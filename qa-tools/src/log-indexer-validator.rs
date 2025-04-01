use anyhow::{Context, Result};
use clap::Parser;

/// Validator Runner for Log Indexer
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct RunnerArgs {
    /// Tag for archiving (required). Example: "custom"
    #[arg(short = 't')]
    tag: String,

    /// Environment (optional, default: "stage")
    #[arg(short = 'e', default_value = "stage")]
    env: String,
}

const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";

fn main() -> Result<()> {
    let args = RunnerArgs::parse();

    if args.tag.trim().is_empty() {
        return Err(anyhow::anyhow!("Error: -t <tag> is mandatory."));
    }

    // Compute target paths.
    let target_folder = format!("{}/{}", ROOT_DIRECTORY, args.tag);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_log_folder = target_folder.clone(); // logs are in the same folder
    let target_log_config_file = format!("{}/log4rs.yaml", target_log_folder);

    println!("Starting log-indexer-validator with:");
    println!("  Log config file: {}", target_log_config_file);
    println!("  Config folder:   {}", target_config_folder);

    // Run the validator logic.
    run_log_indexer_validator(&target_log_config_file, &target_config_folder)
}

fn run_log_indexer_validator(log_config_path: &str, config_folder: &str) -> Result<()> {
    // Initialize log4rs using the provided log config.
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;

    // Load the configuration.
    let config = common::config::Config::load(config_folder)
        .with_context(|| format!("Loading config from {}", config_folder))?;

    // Create the log store. We assume that logs are stored in a "logs" subdirectory
    // within the storage path specified in the configuration.
    let store =
        log_indexer::store::RawLogStore::new(&format!("{}/logs", config.indexer.storage.path))
            .with_context(|| "Creating RawLogStore")?;

    // Retrieve all logs from storage.
    let stored_logs = store
        .get_all_logs()
        .with_context(|| "Failed to retrieve logs from storage")?;

    log::info!("Retrieved {} logs from storage", stored_logs.len());

    // Pretty-print the logs as JSON.
    let pretty_logs = serde_json::to_string_pretty(&stored_logs)
        .with_context(|| "Failed to serialize logs to pretty JSON")?;
    log::info!("Logs:\n{}", pretty_logs);

    log::logger().flush();
    Ok(())
}
