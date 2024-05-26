use flate2::read::ZlibDecoder;
use inflate::{inflate_bytes, inflate_bytes_zlib};
use png::{Decoder, Encoder};
use std::{
    fs::File,
    io::{BufWriter, Cursor, Error, Read, Write},
};
use thiserror::*;
use tracing::info;

extern crate crc;
use crc::{Crc, CRC_32_ISO_HDLC};

#[derive(Error, Debug)]
pub enum PngError {
    #[error("The file provided is not a png file.")]
    NotPng,
    #[error("The file provided is not a iOS CgBI file.")]
    NotCgBI,
    #[error("{0}")]
    IoError(#[from] Error),
    #[error("{0}")]
    PngDecodingError(#[from] png::DecodingError),
    #[error("{0}")]
    PngEncodingError(#[from] png::EncodingError),
}

/// idfk why some of the pngs are broken, but this should fix them. dont quote me on that.
pub fn fixup_png<R: Read>(mut reader: R) -> Result<Vec<u8>, PngError> {
    let (info, data) = parse_ios_png(&mut reader)?;
    let corrected_data = correct_pixel_data(data, info.0 as usize, info.1 as usize);

    let data = build_png(info, &corrected_data)?;

    Ok(data)
}

const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug)]
struct Chunk {
    length: u32,
    chunk_type: [u8; 4],
    data: Vec<u8>,
}

fn parse_ios_png<R: Read>(mut reader: R) -> Result<((u32, u32), Vec<u8>), PngError> {
    // Verify PNG header
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    if &header != PNG_HEADER {
        return Err(PngError::NotPng);
    }

    let mut chunks = Vec::new();
    let mut width = 0;
    let mut height = 0;

    let mut is_ios = false;

    // Read chunks
    while let Ok(chunk) = read_chunk(&mut reader) {
        if &chunk.chunk_type == b"IHDR" {
            width =
                u32::from_be_bytes([chunk.data[0], chunk.data[1], chunk.data[2], chunk.data[3]]);
            height =
                u32::from_be_bytes([chunk.data[4], chunk.data[5], chunk.data[6], chunk.data[7]]);
        }
        if &chunk.chunk_type == b"CgBI" {
            is_ios = true;
            info!("Found CgBI chunk, not adding it back");
            continue;
        } else if &chunk.chunk_type == b"IDAT" {
            chunks.push(chunk);
        } else if &chunk.chunk_type == b"IEND" {
            break;
        }
    }

    if !is_ios {
        return Err(PngError::NotCgBI);
    }

    let idat_data = chunks
        .into_iter()
        .flat_map(|chunk| chunk.data)
        .collect::<Vec<u8>>();
    let decompressed_data = decompress_idat_chunks(&idat_data)?;

    Ok(((width, height), decompressed_data))
}

fn read_chunk<R: Read>(reader: &mut R) -> Result<Chunk, Box<dyn std::error::Error>> {
    let mut length_bytes = [0; 4];
    let mut type_bytes = [0; 4];
    reader.read_exact(&mut length_bytes)?;
    reader.read_exact(&mut type_bytes)?;
    let length = u32::from_be_bytes(length_bytes);
    let mut data = vec![0; length as usize];
    reader.read_exact(&mut data)?;
    let mut crc_bytes = [0; 4];
    reader.read_exact(&mut crc_bytes)?;
    //let crc = u32::from_be_bytes(crc_bytes);
    // info!("crc: 0x{:x}", crc);

    Ok(Chunk {
        length,
        chunk_type: type_bytes,
        data,
    })
}

fn decompress_idat_chunks(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    match inflate_bytes(data) {
        Ok(decompressed_data) => {
            info!("Decompressed IDAT chunk: {} bytes", decompressed_data.len());
            Ok(decompressed_data)
        }
        Err(msg) => Err(std::io::Error::new(std::io::ErrorKind::Other, msg)),
    }
}

fn correct_pixel_data(data: Vec<u8>, width: usize, height: usize) -> Vec<u8> {
    let mut corrected_data = data.clone();

    // for y in 0..height {
    //     for x in 0..width {
    //         let i = (y * width + x) * 4;
    //         let b = data[i];
    //         let g = data[i + 1];
    //         let r = data[i + 2];
    //         let a = data[i + 3];
    //         corrected_data.push(r);
    //         corrected_data.push(g);
    //         corrected_data.push(b);
    //         corrected_data.push(a);
    //     }
    // }

    // unpremultiply_alpha(&mut corrected_data);

    corrected_data
}

fn unpremultiply_alpha(data: &mut Vec<u8>) {
    for chunk in data.chunks_exact_mut(4) {
        let a = chunk[3] as u16;
        if a > 0 {
            chunk[0] = ((chunk[0] as u16 * 255) / a).min(255) as u8;
            chunk[1] = ((chunk[1] as u16 * 255) / a).min(255) as u8;
            chunk[2] = ((chunk[2] as u16 * 255) / a).min(255) as u8;
        }
    }
}

fn build_png(info: (u32, u32), data: &[u8]) -> Result<Vec<u8>, PngError> {
    let mut buffer = Vec::new();

    // Write PNG header
    buffer.write_all(PNG_HEADER)?;

    // Write IHDR chunk
    let mut ihdr_data = Vec::new();
    ihdr_data.extend_from_slice(&info.0.to_be_bytes());
    ihdr_data.extend_from_slice(&info.1.to_be_bytes());
    ihdr_data.extend_from_slice(&[8, 6, 0, 0, 0]); // Bit depth, color type, compression, filter, interlace
    write_chunk(&mut buffer, b"IHDR", &ihdr_data)?;

    // Write IDAT chunk
    let mut compressor =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    compressor.write_all(data)?;
    let compressed_data = compressor.finish()?;
    write_chunk(&mut buffer, b"IDAT", &compressed_data)?;

    // Write IEND chunk
    write_chunk(&mut buffer, b"IEND", &[])?;

    Ok(buffer)
}

fn write_chunk<W: Write>(
    writer: &mut W,
    chunk_type: &[u8; 4],
    data: &[u8],
) -> Result<(), PngError> {
    let length = data.len() as u32;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(chunk_type)?;
    writer.write_all(data)?;

    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let crc = crc.checksum(&[chunk_type, data].concat());
    writer.write_all(&crc.to_be_bytes())?;

    Ok(())
}
