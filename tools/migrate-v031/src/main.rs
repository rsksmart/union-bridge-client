//! Stand-alone CLI entry point for the v0.3.1 → v0.4.x DB migrator.
//!
//! Usage:
//!
//!     migrate-v031 <db-path> [--config <toml-path>]
//!
//! For example:
//!
//!     migrate-v031 ~/.union_bridge/op_1/local_database/coordinator \
//!         --config ./config/local-op_1.toml
//!
//! Behaviour, in order:
//!
//! 1. If `--config` was given, refuse to proceed if the TOML still has the
//!    legacy `[bridge.*]` section (v0.4.x silently ignores it and would
//!    otherwise run with `FlowsConfig` defaults).
//! 2. Migrate the DB in place. Idempotent and additive; see the crate-level
//!    docs.
//! 3. Verify post-migration that no row is left at the v0.3.1 schema.
//!
//! Run against a stopped coordinator's database before starting v0.4.x.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use log::info;
use storage_backend::storage::Storage;
use storage_backend::storage_config::StorageConfig;

/// Migrate a v0.3.1 coordinator database to v0.4.x in place.
#[derive(Parser)]
#[command(name = "migrate-v031", version, about, long_about = None)]
struct Args {
    /// Path to the coordinator DB directory.
    /// Typically: `~/.union_bridge/op_NN/local_database/coordinator`.
    db: String,

    /// Optional path to the operator's TOML config.
    /// When provided, the tool refuses to migrate if the config still uses
    /// the legacy `[bridge.*]` section.
    #[arg(long, value_name = "TOML")]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if let Some(toml_path) = args.config.as_ref() {
        migrate_v031::check_config_no_legacy_bridge(toml_path)?;
        info!("Config at {} has no legacy [bridge.*] section", toml_path.display());
    }

    let storage = Storage::open(&StorageConfig::new(args.db.clone(), None))
        .with_context(|| format!("opening coordinator DB at {}", args.db))?;

    let total = migrate_v031::run(&storage)?;
    if total == 0 {
        info!("Nothing to migrate; DB already at v0.4.x shape");
    } else {
        info!("v0.3.1 → v0.4.x migration: {total} rows mutated");
    }

    migrate_v031::verify_v04x_schema(&storage).context("post-migration schema verification")?;
    info!("Post-migration schema verification passed");

    Ok(())
}
