use anyhow::{Context, Result};
pub use tracing_appender::non_blocking::WorkerGuard as LogGuard;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format::{DefaultFields, JsonFields, Writer};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::CommonConfig;

const LOG_DIR_ENV_VAR: &str = "UB_LOG_DIR";
const CLIENT_ID_ENV_VAR: &str = "CLIENT_ID";
const ENVIRONMENT_ENV_VAR: &str = "ENVIRONMENT";
const LOG_FORMAT_ENV_VAR: &str = "LOG_FORMAT";
const LOCAL_ENVIRONMENT: &str = "local";
/// Default log directory used when neither `--log-dir` nor `UB_LOG_DIR` is set.
/// Resolved relative to the process's current working directory.
const DEFAULT_LOG_DIR: &str = "logs";
/// Default per-crate log filters. Noisy third-party crates are pinned to `warn`
/// so they don't drown out service-level events; see `docs/LOGGING.md`.
const DEFAULT_FILTER: &str = "debug,\
    tarpc=warn,\
    alloy_provider=warn,alloy_pubsub=warn,alloy_rpc_client=warn,alloy_json_rpc=warn,\
    hyper=warn,hyper_util=warn,h2=warn,\
    reqwest=warn,rustls=warn,tower_http=warn,tungstenite=warn";

// `tracing-subscriber` caches per-span formatted fields in span extensions keyed by the
// field-formatter type. When two `fmt::Layer`s share that type, the first layer to format
// a span wins and the other reuses its cached string — including ANSI codes. Wrapping the
// default formatters in newtypes gives the file layer a distinct cache key so its
// `with_ansi(false)` is honored independently of the stdout layer.
struct PlainFields(DefaultFields);

impl<'writer> FormatFields<'writer> for PlainFields {
    fn format_fields<R: RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

struct PlainJsonFields(JsonFields);

impl<'writer> FormatFields<'writer> for PlainJsonFields {
    fn format_fields<R: RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

impl CommonConfig {
    /// Initializes the tracing subscriber. See `docs/LOGGING.md` for the rationale.
    ///
    /// **Format selection** (`LOG_FORMAT` env var, defaults derived from `ENVIRONMENT`):
    /// - `pretty` — human-readable colored output. Default when `ENVIRONMENT=local` (or unset).
    /// - `json`   — one JSON event per line, machine-parseable. Default in any other environment.
    ///
    /// **Outputs**:
    /// - Stdout: always emitted, with the chosen format.
    /// - File: always written, never with ANSI codes. The directory is resolved as
    ///   `log_dir_opt` (CLI arg) → `UB_LOG_DIR` env var → `DEFAULT_LOG_DIR`
    ///   (relative to the current working directory).
    ///
    /// **Operator identification**: when `CLIENT_ID` is set (injected per-operator by the
    /// cli/run launcher), the file name becomes `<crate_name>-<CLIENT_ID>.log` so the
    /// per-operator log file is stable. Otherwise the file falls back to
    /// `<crate_name>-<timestamp>.log`.
    ///
    /// **Log level**: controlled by `RUST_LOG`. When unset, defaults to `DEFAULT_FILTER` —
    /// `debug` for service code, `warn` for noisy third-party crates.
    ///
    /// Also installs [`tracing_log::LogTracer`] (via `tracing-subscriber`'s default
    /// `tracing-log` feature) so dependencies still using the `log` crate flow through
    /// the same subscriber.
    ///
    /// Returns a [`LogGuard`] that must be held alive for the duration of the process; dropping
    /// it flushes and closes the background file-writer thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the log directory cannot be created, or if a global tracing
    /// subscriber has already been installed (e.g. in tests that call this more than once).
    pub fn init_logger(log_dir_opt: Option<&String>, crate_name: &str) -> Result<LogGuard> {
        // `tracing-subscriber` enables the `tracing-log` default feature, so `try_init()`
        // installs `LogTracer` internally. An explicit call here would double-register it
        // and cause try_init() to return "logger already initialized".

        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

        let log_dir = Self::resolve_log_dir(
            log_dir_opt.map(String::as_str),
            std::env::var(LOG_DIR_ENV_VAR).ok().as_deref(),
        );
        let client_id = std::env::var(CLIENT_ID_ENV_VAR).ok().filter(|s| !s.is_empty());
        let json_format = Self::select_json_format(
            std::env::var(LOG_FORMAT_ENV_VAR).ok().as_deref(),
            std::env::var(ENVIRONMENT_ENV_VAR).ok().as_deref(),
        );

        // With CLIENT_ID set, the parent directory is already per-execution
        // (logs/YYMMDD/HHMMSS/) and the launcher expects stable per-operator filenames,
        // so no timestamp suffix.
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("Failed to create log directory: {log_dir}"))?;
        let file_name =
            Self::build_log_file_name(crate_name, client_id.as_deref(), &chrono::Local::now());
        println!("Logging to file: {log_dir}/{file_name}");
        let (file_writer, guard) =
            tracing_appender::non_blocking(tracing_appender::rolling::never(&log_dir, file_name));

        // try_init returns Err when a global subscriber is already set. In production
        // that means the configured filter/layers will not be applied, so we surface
        // the error instead of silently dropping it.
        let init_result = if json_format {
            let stdout_layer = fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true);
            let file_layer = fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_writer(file_writer)
                .with_ansi(false)
                .fmt_fields(PlainJsonFields(JsonFields::new()));
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(file_layer)
                .try_init()
        } else {
            let file_layer = fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false)
                .fmt_fields(PlainFields(DefaultFields::new()));
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .with(file_layer)
                .try_init()
        };

        init_result.context("Failed to install tracing subscriber")?;

        Ok(guard)
    }

    /// Resolves the log directory used by [`init_logger`]. CLI arg wins, then
    /// `UB_LOG_DIR`, then `DEFAULT_LOG_DIR`. Empty strings are treated as unset.
    fn resolve_log_dir(arg: Option<&str>, env: Option<&str>) -> String {
        arg.filter(|s| !s.is_empty())
            .or(env.filter(|s| !s.is_empty()))
            .map_or_else(|| DEFAULT_LOG_DIR.to_string(), str::to_string)
    }

    /// Decides whether to use JSON output. `LOG_FORMAT` is the explicit override
    /// (case-insensitive `json` ⇒ true, anything else ⇒ false). When unset,
    /// falls back to `ENVIRONMENT`: `local` (or unset) ⇒ pretty, anything else ⇒ JSON.
    /// Empty strings are treated as unset.
    fn select_json_format(log_format: Option<&str>, environment: Option<&str>) -> bool {
        match log_format.filter(|s| !s.is_empty()) {
            Some(fmt) => fmt.eq_ignore_ascii_case("json"),
            None => {
                environment.filter(|s| !s.is_empty()).is_some_and(|env| env != LOCAL_ENVIRONMENT)
            }
        }
    }

    /// Builds the log file name. With `CLIENT_ID` (operator launcher), the
    /// parent directory is already per-execution so we use a stable
    /// `<crate>-<id>.log` name; otherwise we append a timestamp.
    fn build_log_file_name(
        crate_name: &str,
        client_id: Option<&str>,
        now: &chrono::DateTime<chrono::Local>,
    ) -> String {
        match client_id.filter(|s| !s.is_empty()) {
            Some(id) => format!("{crate_name}-{id}.log"),
            None => format!("{crate_name}-{}.log", now.format("%Y%m%d_%H%M%S")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_log_dir_prefers_cli_arg() {
        assert_eq!("from_arg", CommonConfig::resolve_log_dir(Some("from_arg"), Some("from_env")));
    }

    #[test]
    fn test_resolve_log_dir_falls_back_to_env() {
        assert_eq!("from_env", CommonConfig::resolve_log_dir(None, Some("from_env")));
    }

    #[test]
    fn test_resolve_log_dir_uses_default_when_both_missing() {
        assert_eq!(DEFAULT_LOG_DIR, CommonConfig::resolve_log_dir(None, None));
    }

    #[test]
    fn test_resolve_log_dir_treats_empty_strings_as_unset() {
        // Empty arg falls through to env; empty env falls through to default.
        assert_eq!("from_env", CommonConfig::resolve_log_dir(Some(""), Some("from_env")));
        assert_eq!(DEFAULT_LOG_DIR, CommonConfig::resolve_log_dir(Some(""), Some("")));
    }

    #[test]
    fn test_select_json_format_log_format_overrides_environment() {
        // LOG_FORMAT=json wins regardless of ENVIRONMENT.
        assert!(CommonConfig::select_json_format(Some("json"), Some("local")));
        // LOG_FORMAT=pretty (or anything non-json) wins regardless of ENVIRONMENT.
        assert!(!CommonConfig::select_json_format(Some("pretty"), Some("staging")));
    }

    #[test]
    fn test_select_json_format_log_format_is_case_insensitive() {
        assert!(CommonConfig::select_json_format(Some("JSON"), None));
        assert!(CommonConfig::select_json_format(Some("Json"), None));
    }

    #[test]
    fn test_select_json_format_falls_back_to_environment() {
        // Non-local environment ⇒ JSON.
        assert!(CommonConfig::select_json_format(None, Some("docker")));
        assert!(CommonConfig::select_json_format(None, Some("staging")));
        // Local environment ⇒ pretty.
        assert!(!CommonConfig::select_json_format(None, Some("local")));
    }

    #[test]
    fn test_select_json_format_defaults_to_pretty_when_unset() {
        assert!(!CommonConfig::select_json_format(None, None));
    }

    #[test]
    fn test_select_json_format_treats_empty_strings_as_unset() {
        // Empty LOG_FORMAT falls through to ENVIRONMENT.
        assert!(CommonConfig::select_json_format(Some(""), Some("docker")));
        // Empty ENVIRONMENT falls through to default (pretty).
        assert!(!CommonConfig::select_json_format(None, Some("")));
    }

    #[test]
    fn test_build_log_file_name_with_client_id_uses_stable_name() {
        let now = chrono::Local::now();
        assert_eq!(
            "block-indexer-3.log",
            CommonConfig::build_log_file_name("block-indexer", Some("3"), &now)
        );
    }

    #[test]
    fn test_build_log_file_name_without_client_id_includes_timestamp() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Local);
        let formatted = now.format("%Y%m%d_%H%M%S").to_string();
        let expected = format!("coordinator-{formatted}.log");
        assert_eq!(expected, CommonConfig::build_log_file_name("coordinator", None, &now));
    }

    #[test]
    fn test_build_log_file_name_empty_client_id_treated_as_none() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Local);
        let formatted = now.format("%Y%m%d_%H%M%S").to_string();
        let expected = format!("user-api-{formatted}.log");
        // Empty CLIENT_ID should fall through to the timestamped path.
        assert_eq!(expected, CommonConfig::build_log_file_name("user-api", Some(""), &now));
    }
}
