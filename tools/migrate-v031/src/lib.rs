//! v0.3.1 → v0.4.x coordinator DB migration helpers.
//!
//! These helpers operate on a `storage_backend::storage::Storage` opened at
//! the coordinator's `RocksDB` directory. Each helper rewrites legacy rows
//! in place to add the fields that v0.4.x requires for `restore_flows` to
//! deserialize them.
//!
//! Field additions are gated by `is_none()` checks, and step rewrites are
//! gated by exact legacy names, so reruns are no-ops. The migration does not
//! remove existing values; downgrade remains possible because v0.3.1 ignores
//! the added fields, but step names rewritten for v0.4.x should be restored
//! from the operator snapshot before a downgrade.
//!
//! Two non-mutation helpers are also provided for the operator runbook:
//! `check_config_no_legacy_bridge` (refuses to migrate if the operator's
//! TOML config still has the legacy `[bridge.*]` section) and
//! `verify_v04x_schema` (probes the DB after migration to confirm every
//! prefix has the v0.4.x-required fields).
//!
//! `RunOptions` and `MigrationReport` are exposed for callers (the binary,
//! tests) that need a dry-run mode or a per-row mutation list.
//!
//! TODO(v0.5.x): delete this crate from the workspace once all operators
//! are on v0.4.x or later. See `docs/v031-to-v04x-migration.md`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use log::{info, warn};
use serde::Serialize;
use serde_json::{Value, json};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;

const COMMITTEE_STEP_RENAMES: &[(&str, &str)] = &[
    ("DepositP2PData", "DepositP2PDataAuthoritativeCheckpoint"),
    ("SetupTakeAggregatedKey", "SetupTakeAggregatedKeyAllConvergeCheckpoint"),
];

const PEGIN_STEP_RENAMES: &[(&str, &str)] = &[
    ("PeginRequested", "WaitPeginRequested"),
    ("GetCommInfo", "GetCommInfoAuthoritativeCheckpoint"),
    ("AddOperatorTakeHash", "WaitAllOperatorTakeTxidsAdded"),
    ("DispatchTransaction", "WaitAcceptPeginSignaturesReadyAllConvergeCheckpoint"),
];

const PEGOUT_STEP_RENAMES: &[(&str, &str)] = &[
    ("PegoutRequested", "WaitPegoutRequested"),
    ("GetCommInfo", "GetCommInfoAuthoritativeCheckpoint"),
    ("DispatchTransaction", "WaitUserTakeSignaturesReady"),
];

/// Knobs for `run_with_options`. Defaults: do mutate the DB.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunOptions {
    /// When `true`, log what would be mutated but do not write back.
    pub dry_run: bool,
}

/// Per-prefix list of keys whose row was (or would be) mutated.
#[derive(Debug, Default, Clone, Serialize)]
pub struct MigrationReport {
    pub dry_run: bool,
    pub committee_mutated: Vec<String>,
    pub pegout_mutated: Vec<String>,
    pub pegin_mutated: Vec<String>,
}

impl MigrationReport {
    #[must_use]
    pub fn total(&self) -> usize {
        self.committee_mutated.len() + self.pegout_mutated.len() + self.pegin_mutated.len()
    }
}

/// Open the coordinator DB at `path`, returning a friendly error if the
/// underlying `RocksDB` rejects the open because the directory is locked
/// by another process (typically a running coordinator).
///
/// # Errors
///
/// Returns an error if the storage cannot be opened, including the
/// translated message when the lock is held by another process.
pub fn open_storage(path: &str) -> Result<Storage> {
    Storage::open(&StorageConfig::new(path.to_string(), None)).map_err(|e| {
        let msg = e.to_string();
        if msg.to_lowercase().contains("lock") {
            anyhow!(
                "Could not open coordinator DB at {path}: it appears to be locked by another process (most likely a running coordinator). Stop the coordinator first (e.g. `docker compose down`) and retry.",
            )
        } else {
            anyhow::Error::from(e).context(format!("opening coordinator DB at {path}"))
        }
    })
}

/// Apply every migration step under `opts`. Returns a structured report.
///
/// # Errors
///
/// Returns an error if reading from or writing to the underlying storage
/// fails, or if a row contains malformed JSON.
pub fn run_with_options(storage: &Storage, opts: RunOptions) -> Result<MigrationReport> {
    let mut report = MigrationReport { dry_run: opts.dry_run, ..MigrationReport::default() };
    migrate_committee_inner(storage, opts, &mut report.committee_mutated)?;
    migrate_pegout_inner(storage, opts, &mut report.pegout_mutated)?;
    migrate_pegin_inner(storage, opts, &mut report.pegin_mutated)?;
    Ok(report)
}

/// Backwards-compatible entry: applies every migration step with default
/// options and returns the total number of mutated rows.
///
/// # Errors
///
/// See `run_with_options`.
pub fn run(storage: &Storage) -> Result<usize> {
    Ok(run_with_options(storage, RunOptions::default())?.total())
}

/// Inject `setup_full_penalization_req: []` into legacy `setup_committee_flows/*` rows.
///
/// # Errors
///
/// See `run_with_options`.
pub fn migrate_committee(storage: &Storage) -> Result<usize> {
    let mut mutated = Vec::new();
    migrate_committee_inner(storage, RunOptions::default(), &mut mutated)?;
    Ok(mutated.len())
}

/// Inject `request_pegout_tx_hash: ""` into legacy `pegout_flows/*` rows; warn for in-flight rows.
///
/// # Errors
///
/// See `run_with_options`.
pub fn migrate_pegout(storage: &Storage) -> Result<usize> {
    let mut mutated = Vec::new();
    migrate_pegout_inner(storage, RunOptions::default(), &mut mutated)?;
    Ok(mutated.len())
}

/// Lift legacy `bitvmx_pegin_accepted.{operator_take_txid, operator_won_txid}` into the new
/// top-level ctx fields. Read-only access on the lift source avoids accidentally promoting a
/// `null` `bitvmx_pegin_accepted` into an object.
///
/// # Errors
///
/// See `run_with_options`.
pub fn migrate_pegin(storage: &Storage) -> Result<usize> {
    let mut mutated = Vec::new();
    migrate_pegin_inner(storage, RunOptions::default(), &mut mutated)?;
    Ok(mutated.len())
}

fn migrate_committee_inner(
    storage: &Storage,
    opts: RunOptions,
    mutated: &mut Vec<String>,
) -> Result<()> {
    for (key, raw) in storage.partial_compare("setup_committee_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        let mut changes = Vec::new();

        if let Some((from, to)) = rewrite_string_field(&mut v, &["step"], COMMITTEE_STEP_RENAMES) {
            changes.push(format!("step {from}->{to}"));
        }

        if string_field(&v, &["step"]) == Some("DepositAggregatedKey") {
            warn!(
                "Committee flow {key} is at legacy DepositAggregatedKey. v0.4.x keeps that serialized name but moved the step after dispute/full-penalization setup; drain this flow before upgrade if it has not already reached NewCommitteeReady.",
            );
        }

        if v["ctx"].get("setup_full_penalization_req").is_none() {
            v["ctx"]["setup_full_penalization_req"] = json!([]);
            changes.push("setup_full_penalization_req=[]".to_string());
        }

        if !changes.is_empty() {
            commit_or_log(storage, &key, &v, opts, &changes.join(", "))?;
            mutated.push(key);
        }
    }
    Ok(())
}

fn migrate_pegout_inner(
    storage: &Storage,
    opts: RunOptions,
    mutated: &mut Vec<String>,
) -> Result<()> {
    for (key, raw) in storage.partial_compare("pegout_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        let mut changes = Vec::new();

        if let Some((from, to)) = rewrite_string_field(&mut v, &["step"], PEGOUT_STEP_RENAMES) {
            changes.push(format!("step {from}->{to}"));
        }

        if v["ctx"].get("request_pegout_tx_hash").is_none() {
            v["ctx"]["request_pegout_tx_hash"] = json!("");
            let step = v.get("step").and_then(Value::as_str).unwrap_or("?");
            if step != "Done" && step != "Failed" {
                warn!(
                    "Migrating in-flight pegout {key} (step={step}) with empty request_pegout_tx_hash; flow may emit an incomplete completion marker",
                );
            }
            changes.push("request_pegout_tx_hash=\"\"".to_string());
        }

        if !changes.is_empty() {
            commit_or_log(storage, &key, &v, opts, &changes.join(", "))?;
            mutated.push(key);
        }
    }
    Ok(())
}

fn migrate_pegin_inner(
    storage: &Storage,
    opts: RunOptions,
    mutated: &mut Vec<String>,
) -> Result<()> {
    for (key, raw) in storage.partial_compare("pegin_flows/")? {
        let mut v: Value = serde_json::from_str(&raw)?;
        let mut changes = Vec::new();

        if let Some((from, to)) = rewrite_string_field(&mut v, &["ctx", "step"], PEGIN_STEP_RENAMES)
        {
            changes.push(format!("ctx.step {from}->{to}"));
        }

        for field in ["operator_take_txid", "operator_won_txid"] {
            if v["ctx"].get(field).is_none() {
                let lifted = v["ctx"]
                    .get("bitvmx_pegin_accepted")
                    .and_then(|b| b.get(field))
                    .cloned()
                    .unwrap_or(Value::Null);
                v["ctx"][field] = lifted;
                changes.push(format!("lift ctx.{field}"));
            }
        }

        if !changes.is_empty() {
            commit_or_log(storage, &key, &v, opts, &changes.join(", "))?;
            mutated.push(key);
        }
    }
    Ok(())
}

fn string_field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}

fn string_field_mut<'a>(value: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
    let mut current = value;
    for segment in path {
        current = current.get_mut(*segment)?;
    }
    Some(current)
}

fn rewrite_string_field(
    value: &mut Value,
    path: &[&str],
    renames: &[(&str, &str)],
) -> Option<(String, String)> {
    let field = string_field_mut(value, path)?;
    let current = field.as_str()?.to_string();
    let (_, to) = renames.iter().find(|(from, _)| current == *from)?;
    *field = Value::String((*to).to_string());
    Some((current, (*to).to_string()))
}

fn commit_or_log(
    storage: &Storage,
    key: &str,
    value: &Value,
    opts: RunOptions,
    summary: &str,
) -> Result<()> {
    if opts.dry_run {
        info!("[dry-run] would mutate {key}: {summary}");
    } else {
        storage.set(key, value, None)?;
    }
    Ok(())
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
/// fields and no legacy step names that would fail `Steps` enum deserialization.
///
/// # Errors
///
/// Returns an error if a row is at the v0.3.1 schema, or if reading the
/// underlying storage fails or a row contains malformed JSON.
pub fn verify_v04x_schema(storage: &Storage) -> Result<()> {
    for (key, raw) in storage.partial_compare("setup_committee_flows/")? {
        let v: Value = serde_json::from_str(&raw)?;
        verify_no_legacy_step(&key, &v, &["step"], COMMITTEE_STEP_RENAMES)?;
        if v["ctx"].get("setup_full_penalization_req").is_none() {
            bail!(
                "DB row {key} is at v0.3.1 schema (missing ctx.setup_full_penalization_req); the DB has not been migrated yet. Run migrate-v031 against this DB first."
            );
        }
    }
    for (key, raw) in storage.partial_compare("pegout_flows/")? {
        let v: Value = serde_json::from_str(&raw)?;
        verify_no_legacy_step(&key, &v, &["step"], PEGOUT_STEP_RENAMES)?;
        if v["ctx"].get("request_pegout_tx_hash").is_none() {
            bail!(
                "DB row {key} is at v0.3.1 schema (missing ctx.request_pegout_tx_hash); the DB has not been migrated yet. Run migrate-v031 against this DB first."
            );
        }
    }
    for (key, raw) in storage.partial_compare("pegin_flows/")? {
        let v: Value = serde_json::from_str(&raw)?;
        verify_no_legacy_step(&key, &v, &["ctx", "step"], PEGIN_STEP_RENAMES)?;
    }
    Ok(())
}

fn verify_no_legacy_step(
    key: &str,
    value: &Value,
    path: &[&str],
    renames: &[(&str, &str)],
) -> Result<()> {
    let Some(step) = string_field(value, path) else {
        return Ok(());
    };

    if renames.iter().any(|(from, _)| step == *from) {
        bail!(
            "DB row {key} is at v0.3.1 schema (legacy step {step}); the DB has not been migrated yet. Run migrate-v031 against this DB first."
        );
    }

    Ok(())
}
