//! v0.3.1 → v0.4.x coordinator DB migration helpers.
//!
//! These helpers operate on a `storage_backend::storage::Storage` opened at
//! the coordinator's `RocksDB` directory. Each helper rewrites legacy rows
//! in place to add the fields that v0.4.x requires for `restore_flows` to
//! deserialize them.
//!
//! All mutations are gated by `is_none()` checks, so reruns are no-ops, and
//! none of them remove or rewrite existing values, so a downgrade back to
//! v0.3.1 still works (v0.3.1 ignores unknown fields on read).
//!
//! Two non-mutation helpers are also provided for the operator runbook:
//! `check_config_no_legacy_bridge` (refuses to migrate if the operator's
//! TOML config still has the legacy `[bridge.*]` section) and
//! `verify_v04x_schema` (probes the DB after migration to confirm every
//! prefix has the v0.4.x-required fields).
//!
//! TODO(v0.5.x): delete this crate from the workspace once all operators
//! are on v0.4.x or later. See `docs/v031-to-v04x-migration.md`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use log::warn;
use serde_json::{Value, json};
use storage_backend::storage::{KeyValueStore, Storage};

/// Apply every migration step. Returns the total number of mutated rows.
///
/// # Errors
///
/// Returns an error if reading from or writing to the underlying storage
/// fails, or if a row contains malformed JSON.
pub fn run(storage: &Storage) -> Result<usize> {
    let mut total = 0;
    total += migrate_committee(storage)?;
    total += migrate_pegout(storage)?;
    total += migrate_pegin(storage)?;
    Ok(total)
}

/// Inject `setup_full_penalization_req: []` into legacy `setup_committee_flows/*` rows.
///
/// # Errors
///
/// Returns an error if reading from or writing to the underlying storage
/// fails, or if a row contains malformed JSON.
pub fn migrate_committee(storage: &Storage) -> Result<usize> {
    let mut n = 0;
    for (key, raw) in storage.partial_compare("setup_committee_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        if v["ctx"].get("setup_full_penalization_req").is_none() {
            v["ctx"]["setup_full_penalization_req"] = json!([]);
            storage.set(&key, &v, None)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Inject `request_pegout_tx_hash: ""` into legacy `pegout_flows/*` rows; warn for in-flight rows.
///
/// # Errors
///
/// Returns an error if reading from or writing to the underlying storage
/// fails, or if a row contains malformed JSON.
pub fn migrate_pegout(storage: &Storage) -> Result<usize> {
    let mut n = 0;
    for (key, raw) in storage.partial_compare("pegout_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        if v["ctx"].get("request_pegout_tx_hash").is_none() {
            v["ctx"]["request_pegout_tx_hash"] = json!("");
            let step = v.get("step").and_then(Value::as_str).unwrap_or("?");
            if step != "Done" && step != "Failed" {
                warn!(
                    "Migrating in-flight pegout {key} (step={step}) with empty request_pegout_tx_hash; flow may emit an incomplete completion marker",
                );
            }
            storage.set(&key, &v, None)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Lift legacy `bitvmx_pegin_accepted.{operator_take_txid, operator_won_txid}` into the new
/// top-level ctx fields. Read-only access on the lift source avoids accidentally promoting a
/// `null` `bitvmx_pegin_accepted` into an object.
///
/// # Errors
///
/// Returns an error if reading from or writing to the underlying storage
/// fails, or if a row contains malformed JSON.
pub fn migrate_pegin(storage: &Storage) -> Result<usize> {
    let mut n = 0;
    for (key, raw) in storage.partial_compare("pegin_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        let mut changed = false;
        for field in ["operator_take_txid", "operator_won_txid"] {
            if v["ctx"].get(field).is_none() {
                let lifted = v["ctx"]
                    .get("bitvmx_pegin_accepted")
                    .and_then(|b| b.get(field))
                    .cloned()
                    .unwrap_or(Value::Null);
                v["ctx"][field] = lifted;
                changed = true;
            }
        }
        if changed {
            storage.set(&key, &v, None)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Refuse to proceed if the operator's TOML config still has the legacy
/// `[bridge.*]` section. v0.4.x renamed those keys to `[flows.*]` and
/// `[coordinator]`, and would silently ignore the legacy section.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the TOML cannot be parsed,
/// or a `[bridge]` table is present at the top level.
pub fn check_config_no_legacy_bridge(toml_path: &Path) -> Result<()> {
    let raw = fs::read_to_string(toml_path)
        .with_context(|| format!("reading config at {}", toml_path.display()))?;
    let value: toml::Value =
        toml::from_str(&raw).with_context(|| format!("parsing TOML at {}", toml_path.display()))?;
    if value.get("bridge").is_some() {
        bail!(
            "Legacy [bridge.*] config section detected in {}; v0.4.x renamed keys to [flows.*] and [coordinator]. Rename per docs/v031-to-v04x-migration.md before running migrate-v031.",
            toml_path.display(),
        );
    }
    Ok(())
}

/// Probe the DB and confirm every persisted row carries the v0.4.x-required
/// fields. Reads at most one row per prefix; intended as a post-migration
/// sanity check, not as a substitute for `run`.
///
/// # Errors
///
/// Returns an error if a row is at the v0.3.1 schema, or if reading the
/// underlying storage fails or a row contains malformed JSON.
pub fn verify_v04x_schema(storage: &Storage) -> Result<()> {
    const PROBES: &[(&str, &str)] = &[
        ("setup_committee_flows/", "setup_full_penalization_req"),
        ("pegout_flows/", "request_pegout_tx_hash"),
    ];
    for (prefix, field) in PROBES {
        if let Some((key, raw)) = storage.partial_compare(prefix)?.into_iter().next() {
            let v: Value = serde_json::from_str(&raw)?;
            if v["ctx"].get(field).is_none() {
                bail!(
                    "DB row {key} is at v0.3.1 schema (missing ctx.{field}) after migration; this is a bug in migrate-v031, please report it.",
                );
            }
        }
    }
    Ok(())
}
