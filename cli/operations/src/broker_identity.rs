use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

const BROKER_SERVICES: [&str; 4] = ["block-indexer", "log-indexer", "user-api", "coordinator"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedBrokerIdentity {
    pub service: &'static str,
    pub operator_id: u8,
    pub pem_path: PathBuf,
    pub pubkey_hash_path: PathBuf,
    pub pubkey_hash: String,
    pub created: bool,
}

pub fn create_local_broker_identities(
    base_storage_path: &Path,
    operator_ids: &[u8],
) -> Result<Vec<ProvisionedBrokerIdentity>> {
    let mut identities = Vec::new();

    for &operator_id in operator_ids {
        for service in BROKER_SERVICES {
            identities.push(provision_identity(base_storage_path, operator_id, service)?);
        }
    }

    Ok(identities)
}

fn provision_identity(
    base_storage_path: &Path,
    operator_id: u8,
    service: &'static str,
) -> Result<ProvisionedBrokerIdentity> {
    let operator_dir = base_storage_path.join(".union_bridge").join(format!("op_{operator_id}"));
    let broker_dir = operator_dir.join("broker");
    fs::create_dir_all(&broker_dir).with_context(|| {
        format!("Failed to create broker identity directory {}", broker_dir.display())
    })?;

    let pem_path = broker_dir.join(format!("{service}.pem"));
    let pubkey_hash_path = broker_dir.join(format!("{service}.pubkey_hash"));

    let created = if pem_path.exists() {
        false
    } else {
        generate_private_key(&pem_path)?;
        true
    };

    let pubkey_hash = compute_pubkey_hash(&pem_path)?;
    fs::write(&pubkey_hash_path, format!("{pubkey_hash}\n")).with_context(|| {
        format!("Failed to write broker pubkey_hash file {}", pubkey_hash_path.display())
    })?;

    Ok(ProvisionedBrokerIdentity {
        service,
        operator_id,
        pem_path,
        pubkey_hash_path,
        pubkey_hash,
        created,
    })
}

fn generate_private_key(pem_path: &Path) -> Result<()> {
    let output = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-out",
            pem_path.to_str().context("Broker identity PEM path is not valid UTF-8")?,
            "-pkeyopt",
            "rsa_keygen_bits:2048",
        ])
        .output()
        .context("Failed to run openssl genpkey for broker identity")?;

    ensure!(
        output.status.success(),
        "openssl genpkey failed for {}: {}",
        pem_path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );

    restrict_owner_only_permissions(pem_path)?;

    Ok(())
}

#[cfg(unix)]
fn restrict_owner_only_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to set 0600 permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_owner_only_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn compute_pubkey_hash(pem_path: &Path) -> Result<String> {
    let output = Command::new("openssl")
        .args([
            "pkey",
            "-pubout",
            "-outform",
            "DER",
            "-in",
            pem_path.to_str().context("Broker identity PEM path is not valid UTF-8")?,
        ])
        .output()
        .context("Failed to run openssl pkey for broker identity")?;

    ensure!(
        output.status.success(),
        "openssl pkey failed for {}: {}",
        pem_path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );

    Ok(format!("{:x}", Sha256::digest(output.stdout)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_provision_local_broker_identities_is_idempotent_and_distinct() {
        let temp_dir = tempfile::tempdir().expect("tempdir");

        let first_run =
            create_local_broker_identities(temp_dir.path(), &[1]).expect("first provision");
        assert_eq!(4, first_run.len());

        let unique_hashes: HashSet<&str> =
            first_run.iter().map(|identity| identity.pubkey_hash.as_str()).collect();
        assert_eq!(4, unique_hashes.len());
        assert!(first_run.iter().all(|identity| identity.created));

        let second_run =
            create_local_broker_identities(temp_dir.path(), &[1]).expect("second provision");
        assert_eq!(4, second_run.len());
        assert!(second_run.iter().all(|identity| !identity.created));

        for (first, second) in first_run.iter().zip(second_run.iter()) {
            assert_eq!(first.pem_path, second.pem_path);
            assert_eq!(first.pubkey_hash_path, second.pubkey_hash_path);
            assert_eq!(first.pubkey_hash, second.pubkey_hash);
            assert!(first.pem_path.to_string_lossy().contains("/.union_bridge/op_1/broker/"));
            assert!(first.pem_path.exists());
            assert!(first.pubkey_hash_path.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_generated_private_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let identities =
            create_local_broker_identities(temp_dir.path(), &[1]).expect("provision identities");

        for identity in identities {
            let mode =
                fs::metadata(&identity.pem_path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(0o600, mode, "unexpected mode for {}", identity.pem_path.display());
        }
    }
}
