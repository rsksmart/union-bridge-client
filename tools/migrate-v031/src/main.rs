//! Stand-alone CLI entry point for the v0.3.1 → v0.4.x DB migrator.
//!
//! Usage:
//!
//!     migrate-v031 <db-path> [--config <toml-path>] [--dry-run] [--report <json>] [--verify-only]
//!
//! For example:
//!
//!     migrate-v031 ~/.union_bridge/op_1/local_database/coordinator \
//!         --config ./config/local-op_1.toml
//!
//! Flags:
//!
//! - `--config <toml-path>`: refuse to proceed if the TOML still has the
//!   legacy `[bridge.*]` section (v0.4.x silently ignores it and would
//!   otherwise run with `FlowsConfig` defaults).
//! - `--dry-run`: log every row that would be mutated without writing
//!   back. Useful for previewing against the real DB without copying it.
//! - `--report <json-path>`: write a machine-readable JSON report listing
//!   the keys mutated under each prefix.
//! - `--verify-only`: skip migration; only run the post-migration schema
//!   probe and report whether the DB is already at the v0.4.x shape.
//!
//! Run against a stopped coordinator's database before starting v0.4.x.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use log::info;

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

    /// Log mutations without writing them back.
    #[arg(long)]
    dry_run: bool,

    /// Write a JSON report listing the mutated keys per prefix.
    #[arg(long, value_name = "JSON")]
    report: Option<PathBuf>,

    /// Skip migration; only run the post-migration schema probe.
    #[arg(long, conflicts_with_all = ["dry_run", "report"])]
    verify_only: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if let Some(toml_path) = args.config.as_ref() {
        migrate_v031::check_config_no_legacy_bridge(toml_path)?;
        info!("Config at {} has no legacy [bridge.*] section", toml_path.display());
    }

    let storage = migrate_v031::open_storage(&args.db)?;

    if args.verify_only {
        migrate_v031::verify_v04x_schema(&storage).context("schema verification")?;
        info!("Schema verification passed");
        return Ok(());
    }

    let report = migrate_v031::run_with_options(
        &storage,
        migrate_v031::RunOptions { dry_run: args.dry_run },
    )?;

    let banner = if args.dry_run { "[dry-run]" } else { "" };
    if report.total() == 0 {
        info!("{banner} Nothing to migrate; DB already at v0.4.x shape");
    } else {
        info!(
            "{banner} v0.3.1 → v0.4.x migration: {} rows mutated (committee={}, pegout={}, pegin={})",
            report.total(),
            report.committee_mutated.len(),
            report.pegout_mutated.len(),
            report.pegin_mutated.len(),
        );
    }

    if let Some(report_path) = args.report.as_ref() {
        let json = serde_json::to_string_pretty(&report)?;
        fs::write(report_path, json)
            .with_context(|| format!("writing report to {}", report_path.display()))?;
        info!("Wrote migration report to {}", report_path.display());
    }

    if !args.dry_run {
        migrate_v031::verify_v04x_schema(&storage).context("post-migration schema verification")?;
        info!("Post-migration schema verification passed");
    }

    Ok(())
}
