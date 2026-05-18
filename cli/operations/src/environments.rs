use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fmt, fs};

use anyhow::{Result, anyhow};

use crate::constants::operator_ids;

/// unified environment enum for all cli commands
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Environment {
    /// local cargo-run services (no docker)
    #[default]
    Local,
    /// local docker compose services
    Docker,
    /// generic remote deployment configured via `cli/.env.<profile>`
    Remote(String),
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = input.trim();
        if normalized.is_empty() {
            return Err("environment must not be empty".to_string());
        }

        match normalized {
            "local" => Ok(Environment::Local),
            "docker" => Ok(Environment::Docker),
            other => {
                validate_remote_profile_name(other)?;
                Ok(Environment::Remote(other.to_string()))
            }
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_name())
    }
}

impl Environment {
    /// returns the name of the environment as a string
    pub fn get_name(&self) -> String {
        match self {
            Environment::Local => "local".to_string(),
            Environment::Docker => "docker".to_string(),
            Environment::Remote(name) => name.clone(),
        }
    }

    /// returns true if this is a remote environment
    pub fn is_remote(&self) -> bool {
        matches!(self, Environment::Remote(_))
    }

    /// returns the remote ssh user for remote environments
    pub fn remote_ssh_user(&self) -> Result<String> {
        required_remote_value(self, "UC_REMOTE_SSH_USER")
    }

    /// returns the remote hosts for remote environments
    pub fn hosts(&self) -> Result<Vec<String>> {
        match self {
            Environment::Remote(_) => read_csv_remote_value(self, "UC_REMOTE_HOSTS"),
            Environment::Local | Environment::Docker => {
                Err(anyhow!("hosts() is only available for remote environments"))
            }
        }
    }

    /// returns the RPC URL for this environment
    pub fn rpc_url(&self) -> Result<String> {
        match self {
            Environment::Local | Environment::Docker => Ok("http://localhost:8545".to_string()),
            Environment::Remote(_) => required_remote_value(self, "UC_REMOTE_RPC_URL"),
        }
    }

    /// returns the bitvmx endpoints for this environment
    pub fn user_api_endpoints(&self) -> Result<Vec<String>> {
        let ports = user_api_ports();
        match self {
            Environment::Local | Environment::Docker => {
                Ok(ports.iter().map(|port| format!("{}:{}", LOCAL_HOST, port)).collect())
            }
            Environment::Remote(_) => read_csv_remote_value(self, "UC_REMOTE_USER_API_ENDPOINTS"),
        }
    }
}

const BASE_USER_API_PORT: u16 = 40001;

fn user_api_ports() -> Vec<u16> {
    operator_ids().iter().map(|&id| BASE_USER_API_PORT + (id as u16) - 1).collect()
}

const LOCAL_HOST: &str = "localhost";

fn required_remote_value(environment: &Environment, key: &str) -> Result<String> {
    if !environment.is_remote() {
        return Err(anyhow!("{} is only available for remote environments", key));
    }

    if let Ok(value) = std::env::var(key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let file_path = remote_profile_file_path(environment)?;
    let value = lookup_key_in_profile(&file_path, key)?
        .ok_or_else(|| anyhow!("{} must be defined in {}", key, file_path.display()))?;

    Ok(value)
}

fn read_csv_remote_value(environment: &Environment, key: &str) -> Result<Vec<String>> {
    let value = required_remote_value(environment, key)?;
    let items: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect();

    if items.is_empty() {
        return Err(anyhow!("{} must contain at least one comma-separated value", key));
    }

    Ok(items)
}

fn remote_profile_file_path(environment: &Environment) -> Result<PathBuf> {
    let Environment::Remote(profile) = environment else {
        return Err(anyhow!("remote profile file is only available for remote environments"));
    };

    let operations_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cli_dir =
        operations_dir.parent().ok_or_else(|| anyhow!("failed to resolve cli directory"))?;

    Ok(cli_dir.join(format!(".env.{profile}")))
}

fn validate_remote_profile_name(profile: &str) -> std::result::Result<(), String> {
    let is_valid = !profile.is_empty()
        && profile.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

    if is_valid {
        Ok(())
    } else {
        Err(format!(
            "invalid remote profile '{profile}': use only ASCII letters, numbers, '-' or '_'"
        ))
    }
}

fn lookup_key_in_profile(profile_path: &Path, key: &str) -> Result<Option<String>> {
    if !profile_path.exists() {
        return Err(anyhow!(
            "missing remote profile file {}. Copy cli/.env.sample to cli/{}",
            profile_path.display(),
            profile_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| String::from("<profile-file>"))
        ));
    }

    let contents = fs::read_to_string(profile_path)
        .map_err(|err| anyhow!("failed to read {}: {}", profile_path.display(), err))?;

    for (line_no, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((raw_key, raw_value)) = stripped.split_once('=') else {
            return Err(anyhow!(
                "invalid line {} in {}: expected KEY=VALUE",
                line_no + 1,
                profile_path.display()
            ));
        };

        if raw_key.trim() != key {
            continue;
        }

        let value = raw_value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')))
            .unwrap_or(value)
            .trim()
            .to_string();

        return Ok(Some(unquoted));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_profile_names_with_path_separators() {
        let err = Environment::from_str("../alphanet").expect_err("profile should be rejected");
        assert!(err.contains("invalid remote profile"));
    }
}
