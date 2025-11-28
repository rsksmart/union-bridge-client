use anyhow::{Context, Result, anyhow};
use bitcoin::{Txid, hashes::Hash};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generate official flow ID for pegin based on committee and slot
pub fn get_accept_pegin_pid(committee_id: Uuid, slot_index: usize) -> Result<Uuid> {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(slot_index.to_be_bytes());
    hasher.update("accept_pegin");

    // Get the result as a byte array
    let hash = hasher.finalize();
    let slice = hash
        .as_slice()
        .get(..16)
        .ok_or_else(|| anyhow!("SHA256 hash too short for UUID generation"))?;
    let uuid_bytes: [u8; 16] = slice
        .try_into()
        .context("Failed to convert hash slice to UUID bytes")?;
    Ok(Uuid::from_bytes(uuid_bytes))
}

/// Generate temporary flow ID for pegin based on Bitcoin transaction ID
pub fn get_temp_pegin_pid(btc_tx_id: Txid) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(btc_tx_id.to_byte_array());
    hasher.update("temp_pegin");

    // Get the result as a byte array
    let hash = hasher.finalize();
    let slice = &hash.as_slice()[..16];
    let uuid_bytes: [u8; 16] = slice.try_into().expect("SHA256 hash is always 32 bytes");
    Uuid::from_bytes(uuid_bytes)
}
