// Written by freehelpdesk

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::PathBuf,
};
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
    c_f_bundle_short_version_string: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CFBundleIcons {
    c_f_bundle_primary_icon: CFBundlePrimaryIcon,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CFBundlePrimaryIcon {
    c_f_bundle_icon_files: Vec<String>,
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
    icons: Vec<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let api = api::Api::new("us");

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

        for ipa in &ipas {
            let Ok(file_archive) = File::open(ipa) else {
                error!("Failed to open IPA file :(");
                return;
            };

            let Ok(mut archive) = ZipArchive::new(&file_archive) else {
                error!("is not an actual ipa lol");
                continue;
            };

            let Ok(mut archive_dup) = ZipArchive::new(&file_archive) else {
                error!("is not an actual ipa lol");
                continue;
            };

            for i in 0..archive.len() {
                //find dat plist
                let mut entry = archive.by_index(i).unwrap();
                let name = entry.enclosed_name();

                if let Some(name) = name {
                    // println!("name: {}", name.to_string_lossy());
                    if name.file_name() == Some(OsStr::new("Info.plist"))
                        && name.components().count() == 3
                    {
                        info!("{} found", name.to_string_lossy());
                        let mut buf = Vec::with_capacity(entry.size() as usize);
                        entry.read_to_end(&mut buf).unwrap();
                        //println!("{}", std::str::from_utf8(&buf).unwrap());
                        let info: Info = plist::from_bytes(&buf).unwrap();

                        info!(
                            "[{}] {} {}",
                            &info.c_f_bundle_identifier,
                            info.c_f_bundle_display_name.as_ref().unwrap_or(
                                &info.c_f_bundle_name.clone().unwrap_or("N/A".to_string())
                            ),
                            &info.c_f_bundle_short_version_string
                        );

                        let name = if let Ok(e) = api.lookup(&info.c_f_bundle_identifier).await {
                            info!("Developer: {}", e.artist_name);
                            Some(e.artist_name)
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
                            icons.append(
                                &mut bundle_icons
                                    .c_f_bundle_primary_icon
                                    .c_f_bundle_icon_files
                                    .clone(),
                            )
                        }

                        let mut modify = cli.output.clone();
                        modify.push(info.c_f_bundle_identifier.clone());

                        fs::create_dir_all(&modify).unwrap();
                        for j in 0..archive_dup.len() {
                            let mut entry = archive_dup.by_index(j).unwrap();
                            let name = entry.enclosed_name();
                            if let Some(path) = name {
                                let name = path.file_name().unwrap().to_string_lossy().to_string();
                                for icon in &icons {
                                    if name.starts_with(icon)
                                        && (path.extension().unwrap().to_string_lossy() == "png")
                                    {
                                        let mut buf = Vec::with_capacity(entry.size() as usize);
                                        entry.read_to_end(&mut buf).unwrap();
                                        let mut name_buf = modify.clone();
                                        name_buf.push(&name);
                                        icon_file_list.push(name.to_string());
                                        fs::write(name_buf, buf).unwrap();
                                    }
                                }
                            }
                        }

                        icon_file_list.sort();
                        icon_file_list.dedup();

                        app_metadata.push(Metadata {
                            file_name: ipa.file_name().unwrap().to_string_lossy().to_string(),
                            identifier: info.c_f_bundle_identifier.clone(),
                            display_name: info.c_f_bundle_display_name,
                            name: info.c_f_bundle_name,
                            author: name,
                            icons: icon_file_list,
                        });
                    }
                }
            }
        }

        let mut output = cli.output.clone();
        output.push("metadata.json");

        fs::write(output, serde_json::to_string_pretty(&app_metadata).unwrap()).unwrap();
    }
}
