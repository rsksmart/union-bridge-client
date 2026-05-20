use std::path::Path;

use alloy_primitives::hex;
use alloy_signer::k256::ecdsa::{SigningKey, VerifyingKey};
use alloy_signer::k256::elliptic_curve::rand_core::OsRng;
use alloy_signer_local::LocalSigner;
use anyhow::{Result, anyhow};
use secrecy::{ExposeSecret, SecretString};

pub struct KeyManager {}

impl KeyManager {
    /// Generate a new signing key and save it to a keystore file
    ///
    /// # Panics
    ///
    /// Panics if the file path cannot be converted to a string (should not happen on valid filesystems)
    ///
    /// # Errors
    ///
    /// Returns an error if keystore encryption or file operations fail
    pub fn generate_key(destination: &Path, password: &str) -> Result<(String, String, String)> {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);
        let public_key = &verifying_key.to_sec1_bytes();
        let private_key_bytes = signing_key.to_bytes().to_vec();

        let (wallet, file_name) = LocalSigner::encrypt_keystore(
            destination,
            &mut OsRng,
            private_key_bytes,
            password,
            None,
        )?;

        let public_key_str = hex::encode(public_key);
        let address_str = hex::encode(wallet.address());
        let file_path = destination.join(file_name).to_str().unwrap().to_string();
        Ok((file_path, public_key_str, address_str))
    }

    /// Retrieves a signer by decrypting the keystore at the specified location.
    ///
    /// The password must be loaded at startup by the caller and passed in;
    /// this function performs no environment-variable lookup itself.
    ///
    /// # Errors
    ///
    /// Returns an error if keystore decryption fails.
    pub fn get_signer(location: &Path, password: &SecretString) -> Result<LocalSigner<SigningKey>> {
        LocalSigner::decrypt_keystore(location, password.expose_secret())
            .map_err(|e| anyhow!("Failed to decrypt keystore: {e}"))
    }

    /// Derive public key and address from a keystore file
    ///
    /// # Errors
    ///
    /// Returns an error if keystore decryption fails or key derivation fails
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
    use std::fs;
    use std::path::PathBuf;

    use super::*;

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
