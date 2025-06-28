use anyhow::{Context, Result};
use qa_tools_common::common::config_consts;
use std::{fs, path::Path};

fn main() -> Result<()> {
    let root_directory = config_consts::ROOT_DIRECTORY;
    println!("Clearing {}/...", root_directory);

    let root_path = Path::new(root_directory);
    if root_path.exists() {
        for entry in fs::read_dir(root_path)
            .with_context(|| format!("Reading directory: {}", root_directory))?
        {
            let entry = entry.with_context(|| "Getting directory entry")?;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                fs::remove_dir_all(&entry_path)
                    .with_context(|| format!("Failed to remove directory: {:?}", entry_path))?;
            } else {
                fs::remove_file(&entry_path)
                    .with_context(|| format!("Failed to remove file: {:?}", entry_path))?;
            }
        }
    } else {
        println!("Directory {} does not exist.", root_directory);
    }

    println!("Clear script completed.");
    Ok(())
}
