use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use std::fs;
use std::path::Path;
use qa_tools_common::common::config_consts;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Tag for archiving (required). Example: happy_path
    #[arg(short = 't', long = "tag")]
    tag: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();

    println!("Archiving with timestamp suffix: {}", timestamp);
    println!("Tag: {}", args.tag);
    println!();

    let folder_to_archive = format!("{}/{}", config_consts::ROOT_DIRECTORY, args.tag);
    archive_folder(&folder_to_archive, &timestamp)?;

    Ok(())
}

fn archive_folder(folder: &str, timestamp: &str) -> Result<()> {
    let path = Path::new(folder);
    if path.is_dir() {
        let new_folder = format!("{}_{}", folder, timestamp);
        println!("Archiving folder {} -> {}", folder, new_folder);
        fs::rename(folder, &new_folder)
            .with_context(|| format!("Failed to archive folder {} to {}", folder, new_folder))?;
    } else {
        println!(
            "Folder {} does not exist or is not a directory. Skipping.",
            folder
        );
    }
    Ok(())
}
