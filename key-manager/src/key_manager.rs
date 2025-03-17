use alloy_primitives::hex;
use alloy_signer::k256::ecdsa::{SigningKey, VerifyingKey};
use alloy_signer_local::LocalSigner;
use anyhow::Result;
use rand::rngs::OsRng;
use rand::thread_rng;
use std::path::Path;

pub struct KeyManager {}

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
