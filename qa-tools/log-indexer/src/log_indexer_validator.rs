use anyhow::{Context, Result};
use clap::Parser;
use log_indexer::config::Config;
use qa_tools_common::common::config_consts;

/// Validator Runner for Log Indexer
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Tag for archiving (required). Example: "custom"
    #[arg(short = 't')]
    tag: String,

    /// Environment (optional, default: "qa")
    #[arg(short = 'e', default_value = "qa")]
    env: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.tag.trim().is_empty() {
        return Err(anyhow::anyhow!("Error: -t <tag> is mandatory."));
    }

    let target_folder = format!("{}/{}", config_consts::ROOT_DIRECTORY, args.tag);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_log_folder = target_folder.clone(); // logs are in the same folder
    let target_log_config_file = format!("{}/log4rs.yaml", target_log_folder);
    println!("Starting log_indexer_validator with:");
    println!("  Log config file: {}", target_log_config_file);
    println!("  Config folder:   {}", target_config_folder);
    run_log_indexer_validator(&target_log_config_file, &target_config_folder)
}

fn run_log_indexer_validator(log_config_path: &str, config_folder: &str) -> Result<()> {
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;
    let config = Config::load(Some(&config_folder.to_string()))
        .with_context(|| format!("Loading config from {}", config_folder))?;
    let store =
        log_indexer::store::RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))
            .with_context(|| "Creating RawLogStore")?;
    let stored_logs = store
        .get_all_logs()
        .with_context(|| "Failed to retrieve logs from storage")?;
    log::info!("Retrieved {} logs from storage", stored_logs.len());
    let pretty_logs = serde_json::to_string_pretty(&stored_logs)
        .with_context(|| "Failed to serialize logs to pretty JSON")?;
    log::info!("Logs:\n{}", pretty_logs);
    log::logger().flush();
    Ok(())
}
