use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

const COMPLETION_MARKER_DIR_NAME: &str = "union-bridge-flow-completion-markers";

#[derive(Debug, Clone, Default)]
pub struct DisabledSignal;

impl DisabledSignal {
    pub fn signal_done(_flow_kind: &str, _flow_id: Uuid, _payload: &Value) {}
}

#[derive(Debug, Clone)]
pub struct FileSignal {
    directory: PathBuf,
}

impl FileSignal {
    pub fn new(storage_root: impl AsRef<Path>) -> Self {
        Self { directory: storage_root.as_ref().join(COMPLETION_MARKER_DIR_NAME) }
    }

    pub fn signal_done(&self, flow_kind: &str, flow_id: Uuid, payload: &Value) -> Result<()> {
        fs::create_dir_all(&self.directory).with_context(|| {
            format!("Failed to create completion marker dir {}", self.directory.display())
        })?;

        let completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let document = json!({
            "status": "done",
            "flow_kind": flow_kind,
            "flow_id": flow_id,
            "completed_at": completed_at,
            "payload": payload,
        });

        let marker_path = self.directory.join(format!("{flow_kind}-{flow_id}.json"));
        let tmp_path = self.directory.join(format!("{flow_kind}-{flow_id}.json.tmp"));

        let bytes = serde_json::to_vec_pretty(&document)
            .context("Failed to serialize completion marker")?;
        fs::write(&tmp_path, bytes)
            .with_context(|| format!("Failed to write temp marker {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &marker_path).with_context(|| {
            format!("Failed to atomically publish completion marker {}", marker_path.display())
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Signaling {
    Disabled(DisabledSignal),
    File(FileSignal),
    // Additional backends can be added here over time, for example Prometheus-
    // oriented signaling or other operational integrations.
}

impl Signaling {
    pub fn new(storage_root: impl AsRef<Path>, runtime_environment: &str) -> Self {
        if runtime_environment.eq_ignore_ascii_case("local")
            || runtime_environment.eq_ignore_ascii_case("docker")
        {
            Self::File(FileSignal::new(storage_root))
        } else {
            Self::Disabled(DisabledSignal)
        }
    }

    // This abstraction is intentionally broader than completion markers. Today
    // it emits flow-completion signals, but it can also evolve to surface
    // operational signals such as too many open flows, repeated failures, or
    // other coordinator health conditions.

    pub fn signal_done(&self, flow_kind: &str, flow_id: Uuid, payload: &Value) -> Result<()> {
        match self {
            Self::Disabled(_) => {
                DisabledSignal::signal_done(flow_kind, flow_id, payload);
                Ok(())
            }
            Self::File(signal) => signal.signal_done(flow_kind, flow_id, payload),
        }
    }
}
