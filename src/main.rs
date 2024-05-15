// Written by freehelpdesk

use api::AppInfo;
use async_zip::{tokio::read::seek::ZipFileReader, ZipFile};
use clap::{error, Parser};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use tokio::io::BufReader;
use tracing::*;
use zip::ZipArchive;

mod api;

#[derive(Parser)]
#[command(version, about, author = "freehelpdesk")]
struct Cli {
    /// Directory of the IPA library
    input: Vec<PathBuf>,
    /// Directory of the output metatdata
    #[arg(short, long)]
    output: PathBuf,
    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Info {
    c_f_bundle_identifier: String,
    c_f_bundle_display_name: Option<String>,
    c_f_bundle_name: Option<String>,
    c_f_bundle_icon_files: Option<Vec<String>>,
    c_f_bundle_icons: Option<CFBundleIcons>,
    c_f_bundle_short_version_string: Option<String>,
    c_f_bundle_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CFBundleIcons {
    c_f_bundle_primary_icon: Option<CFBundlePrimaryIcon>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CFBundlePrimaryIcon {
    c_f_bundle_icon_files: Option<Vec<String>>,
    c_f_bundle_icon_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Metadata {
    file_name: String,
    identifier: String,
    display_name: Option<String>,
    name: Option<String>,
    author: Option<String>,
    version: Option<String>,
    appstore_icon: Option<String>,
    icons: Vec<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter("imetadata=trace")
        .init();

    let mut ipas: Vec<PathBuf> = vec![];

    for input in cli.input {
        if cli.debug > 0 {
            println!("{}", &input.to_string_lossy())
        }
        // are we a directory?????

        if input.is_dir() {
            // we need to go lookin now
            let Ok(paths) = fs::read_dir(input) else {
                error!("Not a directory, how did we get here?");
                return;
            };
            for path in paths {
                let Ok(entry) = path else {
                    error!("Error in entry.");
                    return;
                };
                //if cli.debug > 0 { println!("{}", &entry.path().to_string_lossy()) }
                if let Some(extension) = entry.path().extension() {
                    if extension.to_string_lossy() == "ipa" {
                        info!("{} is ipa", entry.path().to_string_lossy());
                        ipas.push(entry.path())
                    }
                }
            }
        }

        let mut app_metadata: Vec<Metadata> = vec![];

        let cpus = num_cpus::get();
        let mut tasks = vec![];
        for chunk in ipas.chunks(ipas.len() / cpus + 1) {
            let chunk = chunk.to_vec();
            let out = cli.output.clone();
            tasks.push(tokio::spawn(async move { process_ipas(chunk, &out).await }));
        }

        for task in tasks {
            app_metadata.extend(task.await.unwrap());
        }

        let mut output = cli.output.clone();
        output.push("metadata.json");

        fs::write(output, serde_json::to_string_pretty(&app_metadata).unwrap()).unwrap();
    }
}

async fn process_ipas(path: Vec<PathBuf>, output: &PathBuf) -> Vec<Metadata> {
    // Each task will have its own api instance
    let api = api::Api::new("us");
    let mut app_metadata: Vec<Metadata> = vec![];
    for ipa in &path {
        let Ok(file_handle) = tokio::fs::File::open(ipa).await else {
            error!("Failed to open IPA file :(");
            continue;
        };

        let mut reader = BufReader::new(file_handle);

        let Ok(mut archive) = ZipFileReader::with_tokio(&mut reader).await else {
            error!("unable to open file as an archive");
            continue;
        };

        let archive_len = archive.file().entries().len();

        for i in 0..archive_len {
            let Ok(mut entry) = archive.reader_with_entry(i).await else {
                error!("Failed to get entry idx: {i}");
                continue;
            };

            let Ok(filename_string) = entry.entry().filename().as_str() else {
                error!("Failed to parse filename string");
                continue;
            };

            let path = Path::new(filename_string);

            //println!("{}", entry.entry().filename().as_str().unwrap());

            if path.file_name() == Some(OsStr::new("Info.plist")) && path.components().count() == 3
            {
                info!("{} found", path.to_string_lossy());
                let mut buf = Vec::with_capacity(entry.entry().uncompressed_size() as usize);
                entry.read_to_end_checked(&mut buf).await.unwrap();
                //println!("{}", std::str::from_utf8(&buf).unwrap());
                let info: Info = plist::from_bytes(&buf).unwrap();

                info!(
                    "[{}] {} {}",
                    &info.c_f_bundle_identifier,
                    info.c_f_bundle_display_name
                        .as_ref()
                        .unwrap_or(&info.c_f_bundle_name.clone().unwrap_or("N/A".to_string())),
                    &info
                        .c_f_bundle_short_version_string
                        .clone()
                        .unwrap_or(info.c_f_bundle_version.unwrap_or("N/A".to_string()))
                );

                let api_info = if let Ok(info) = api.lookup(&info.c_f_bundle_identifier).await {
                    info!("Developer: {}", info.artist_name);
                    Some(info)
                } else {
                    warn!(
                        "Appstore info not found for {}, not adding a developer.",
                        &info.c_f_bundle_identifier
                    );
                    None
                };

                // let name = None;

                let mut icons: Vec<String> = vec![];

                let mut icon_file_list: Vec<String> = vec![];

                // far out, now we have to find all the fucking icons somehow lmao
                if let Some(bundle_icons) = info.c_f_bundle_icon_files {
                    icons.append(&mut bundle_icons.clone());
                } else if let Some(bundle_icons) = &info.c_f_bundle_icons {
                    if let Some(primary_icons) = &bundle_icons.c_f_bundle_primary_icon {
                        if let Some(icon_files) = &primary_icons.c_f_bundle_icon_files {
                            icons.append(&mut icon_files.clone())
                        }
                    }
                }

                let mut modify = output.clone();
                modify.push(info.c_f_bundle_identifier.clone());

                fs::create_dir_all(&modify).unwrap();
                for j in 0..archive_len {
                    let Ok(mut entry) = archive.reader_with_entry(j).await else {
                        error!("Failed to get entry idx: {j}");
                        continue;
                    };

                    let name = entry.entry().filename().clone();
                    let Ok(filename_string) = &name.as_str() else {
                        error!("Failed to parse filename string");
                        continue;
                    };

                    let path = Path::new(filename_string);
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    for icon in &icons {
                        let Some(extension) = path.extension() else {
                            continue;
                        };
                        if name.starts_with(icon) && (extension.to_string_lossy() == "png") {
                            let mut buf =
                                Vec::with_capacity(entry.entry().uncompressed_size() as usize);
                            if let Err(err) = entry.read_to_end_checked(&mut buf).await {
                                error!("Failed to read entry: {}", err);
                                continue;
                            }
                            let mut name_buf = modify.clone();
                            name_buf.push(&name);
                            icon_file_list.push(name.to_string());
                            fs::write(name_buf, buf).unwrap();
                        }
                    }
                }

                icon_file_list.sort();
                icon_file_list.dedup();

                app_metadata.push(Metadata {
                    file_name: ipa
                        .clone()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    identifier: info.c_f_bundle_identifier.clone(),
                    display_name: info.c_f_bundle_display_name,
                    name: info.c_f_bundle_name,
                    author: api_info.as_ref().map(|info| info.artist_name.clone()),
                    appstore_icon: api_info.as_ref().map(|info| info.artwork_url_512.clone()),
                    version: info.c_f_bundle_short_version_string.clone(),
                    icons: icon_file_list,
                });
            }
        }
    }

    app_metadata
}
