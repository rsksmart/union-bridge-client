use alloy_primitives::hex;
use alloy_signer::k256::ecdsa::{SigningKey, VerifyingKey};
use alloy_signer_local::LocalSigner;
use anyhow::{Context, Result};
use rand::rngs::OsRng;
use rand::thread_rng;
use std::path::Path;

pub struct KeyManager {
    // TODO(iago) instantiate with a path to the keystore so the methods below are instance ones (but for the generate one)
}

impl KeyManager {
    pub fn generate_key(destination: &Path, password: &str) -> Result<(String, String, String)> {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);
        let public_key = &verifying_key.to_sec1_bytes();
        let private_key_bytes = signing_key.to_bytes().to_vec();

        let mut rng = thread_rng();

        let (wallet, file_name) = LocalSigner::encrypt_keystore(
            destination,
            &mut rng,
            private_key_bytes,
            password,
            None,
        )?;

        let public_key_str = hex::encode(public_key);
        let address_str = hex::encode(wallet.address());
        let file_path = destination.join(file_name).to_str().unwrap().to_string();
        Ok((file_path, public_key_str, address_str))
    }

    pub fn get_signer(location: &Path, password: &str) -> Result<LocalSigner<SigningKey>> {
        LocalSigner::decrypt_keystore(location, password).context("Getting signer")
    }

    pub fn derive_public_key_and_address(
        location: &Path,
        password: &str,
    ) -> Result<(String, String)> {
        let wallet = LocalSigner::decrypt_keystore(location, password)?;

        let private_key_bytes = wallet.to_field_bytes();

        let signing_key = SigningKey::from_slice(&private_key_bytes)?;
        let verifying_key = VerifyingKey::from(&signing_key);
        let public_key = verifying_key.to_sec1_bytes();

        let address_str = hex::encode(wallet.address());
        let public_key_str = hex::encode(public_key);

        Ok((public_key_str, address_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn test_generate_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let destination = temp_dir.path().join("keystore");
        fs::create_dir_all(&destination).unwrap();

        let password = "test_password";

        let result = KeyManager::generate_key(&destination, password);
        assert!(result.is_ok());

        let (file_path, public_key, address) = result.unwrap();

        assert!(Path::new(&file_path).exists());
        assert!(!public_key.is_empty());
        assert!(!address.is_empty());

        fs::remove_file(file_path).unwrap();
    }

    #[test]
    fn test_derive_public_key_and_address() {
        let temp_dir = tempfile::tempdir().unwrap();
        let destination = temp_dir.path().join("keystore");
        fs::create_dir_all(&destination).unwrap();

        let password = "test_password";

        let result = KeyManager::generate_key(&destination, password);
        assert!(result.is_ok());
        let (file_path, expected_public_key, expected_address) = result.unwrap();

        let result =
            KeyManager::derive_public_key_and_address(&PathBuf::from(file_path.clone()), password);
        assert!(result.is_ok());

        let (public_key, address) = result.unwrap();
        assert_eq!(expected_public_key, public_key);
        assert_eq!(expected_address, address);

        fs::remove_file(file_path).unwrap();
    }
}
