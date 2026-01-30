use anyhow::{Context, Result};
use bitcoin::PublicKey;
use common::msg_broker::bitvmx_types::IncomingBitVMXApiMessages;
use common::msg_broker::broker::BitVmxBrokerClientApi;
use log::{debug, error};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const DISPUTE_CORE_SUFFIX: &str = "dispute_core";
pub const DISPUTE_CHANNEL_SUFFIX: &str = "dispute_channel";
pub const UUID_BYTES_LEN: usize = 16;

/// Sends a message to `BitVMX` broker with proper error handling and logging.
/// Returns Result to allow error propagation.
pub fn send_bitvmx_msg<BC: BitVmxBrokerClientApi>(
    broker_client: &BC,
    msg: IncomingBitVMXApiMessages,
) -> Result<()> {
    debug!("Sending to BitVMX: {msg:?}");

    broker_client
        .send(msg)
        .map(|_| ())
        .map_err(|e| {
            // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
            error!("Failed to send msg to BitVMX: {e:?}");
            anyhow::Error::from(e)
        })
        .context("Failed to send message to BitVMX broker")
}

/// Generates the `DisputeCore` protocol ID for a given committee and member's take key.
/// This is a deterministic UUID derived from `committee_id`, `pubkey`, and `dispute_core` suffix.
pub fn get_dispute_core_pid(committee_id: Uuid, pubkey: &PublicKey) -> Result<Uuid> {
    let mut hasher = Sha256::new();
    hasher.update(committee_id.as_bytes());
    hasher.update(pubkey.to_bytes());
    hasher.update(DISPUTE_CORE_SUFFIX);

    let hash = hasher.finalize();
    // Sha256 always produces 32 bytes, so taking the first 16 bytes is always safe
    let bytes = hash[0..UUID_BYTES_LEN].try_into().context("UUID slice conversion failed")?;

    Ok(Uuid::from_bytes(bytes))
}

/// Generates the `DisputeChannel` protocol ID for a given committee and operator/watchtower pair.
/// This is a deterministic UUID derived from `committee_id`, `op_index`, `wt_index`, and `dispute_channel` suffix.
pub fn get_dispute_channel_pid(committee_id: Uuid, op_index: usize, wt_index: usize) -> Uuid {
    let mut hasher = Sha256::new();

    hasher.update(committee_id.as_bytes());
    hasher.update(op_index.to_be_bytes());
    hasher.update(wt_index.to_be_bytes());
    hasher.update(DISPUTE_CHANNEL_SUFFIX);

    let hash = hasher.finalize();
    // Sha256 always produces 32 bytes, so taking the first 16 bytes is always safe
    Uuid::from_bytes(
        hash[0..UUID_BYTES_LEN]
            .try_into()
            .expect("Sha256 hash is always 32 bytes, so first 16 bytes slice is valid"),
    )
}

const PAIRWISE_AGGREGATED_KEY_SUFFIX: &str = "pairwise_aggregated_key";

/// Generates a deterministic UUID for a pairwise aggregated key between two committee members.
/// The UUID is derived from the `committee_id` and the ordered pair of member indices.
/// Both members will derive the same ID regardless of who initiates.
pub fn get_dispute_pair_aggregated_key_pid(committee_id: Uuid, idx_a: usize, idx_b: usize) -> Uuid {
    let mut hasher = Sha256::new();
    // Ensure canonical ordering (min, max) so both parties derive the same id.
    let (min_i, max_i) = if idx_a <= idx_b { (idx_a, idx_b) } else { (idx_b, idx_a) };

    hasher.update(committee_id.as_bytes());
    hasher.update(min_i.to_be_bytes());
    hasher.update(max_i.to_be_bytes());
    hasher.update(PAIRWISE_AGGREGATED_KEY_SUFFIX);

    let hash = hasher.finalize();
    // Sha256 always produces 32 bytes, so taking the first 16 bytes is always safe
    Uuid::from_bytes(
        hash[0..UUID_BYTES_LEN]
            .try_into()
            .expect("Sha256 hash is always 32 bytes, so first 16 bytes slice is valid"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compressed secp256k1 public key (generator point G) for deterministic tests.
    fn test_public_key() -> PublicKey {
        const COMPRESSED_G: [u8; 33] = [
            0x02, 0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce,
            0x87, 0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81,
            0x5b, 0x16, 0xf8, 0x17, 0x98,
        ];
        PublicKey::from_slice(&COMPRESSED_G).expect("valid compressed pubkey")
    }

    #[test]
    fn get_dispute_core_pid_is_deterministic() {
        let committee_id = Uuid::nil();
        let pubkey = test_public_key();
        let pid1 = get_dispute_core_pid(committee_id, &pubkey).unwrap();
        let pid2 = get_dispute_core_pid(committee_id, &pubkey).unwrap();
        assert_eq!(pid1, pid2);
    }

    #[test]
    fn get_dispute_core_pid_different_inputs_different_ids() {
        let committee_id = Uuid::nil();
        let pubkey = test_public_key();
        let pid_nil = get_dispute_core_pid(committee_id, &pubkey).unwrap();
        let other_committee = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let pid_other = get_dispute_core_pid(other_committee, &pubkey).unwrap();
        assert_ne!(pid_nil, pid_other);
    }

    #[test]
    fn get_dispute_channel_pid_is_deterministic() {
        let committee_id = Uuid::nil();
        let pid1 = get_dispute_channel_pid(committee_id, 0, 1);
        let pid2 = get_dispute_channel_pid(committee_id, 0, 1);
        assert_eq!(pid1, pid2);
    }

    #[test]
    fn get_dispute_channel_pid_order_matters() {
        let committee_id = Uuid::nil();
        let pid_0_1 = get_dispute_channel_pid(committee_id, 0, 1);
        let pid_1_0 = get_dispute_channel_pid(committee_id, 1, 0);
        assert_ne!(pid_0_1, pid_1_0);
    }

    #[test]
    fn get_dispute_pair_aggregated_key_pid_symmetric() {
        let committee_id = Uuid::nil();
        let pid_a_b = get_dispute_pair_aggregated_key_pid(committee_id, 0, 1);
        let pid_b_a = get_dispute_pair_aggregated_key_pid(committee_id, 1, 0);
        assert_eq!(pid_a_b, pid_b_a, "both parties must derive the same ID");
    }

    #[test]
    fn get_dispute_pair_aggregated_key_pid_deterministic() {
        let committee_id = Uuid::nil();
        let pid1 = get_dispute_pair_aggregated_key_pid(committee_id, 2, 3);
        let pid2 = get_dispute_pair_aggregated_key_pid(committee_id, 2, 3);
        assert_eq!(pid1, pid2);
    }

    #[test]
    fn get_dispute_pair_aggregated_key_pid_different_pairs_different_ids() {
        let committee_id = Uuid::nil();
        let pid_0_1 = get_dispute_pair_aggregated_key_pid(committee_id, 0, 1);
        let pid_0_2 = get_dispute_pair_aggregated_key_pid(committee_id, 0, 2);
        let pid_1_2 = get_dispute_pair_aggregated_key_pid(committee_id, 1, 2);
        assert_ne!(pid_0_1, pid_0_2);
        assert_ne!(pid_0_1, pid_1_2);
        assert_ne!(pid_0_2, pid_1_2);
    }
}
