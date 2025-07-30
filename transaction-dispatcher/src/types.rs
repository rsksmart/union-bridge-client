use alloy_primitives::FixedBytes;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, anyhow, ensure};
use bitcoin::{Transaction, TxIn, TxOut};
use common::msg_broker::bitvmx_types::{P2PAddress, PeerId};
use common::{msg_broker::bitvmx_types::BtcTxSPVProof, types::Hash256};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommunicationData,
};

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-214

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransaction {
    pub version: u32,
    pub inputs: Vec<BitcoinTransactionIn>,
    pub outputs: Vec<BitcoinTransactionOut>,
    pub lock_time: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionIn {
    pub tx_id: String,
    pub v_out: u32,
    pub sequence: u32,
    pub script_sig: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionOut {
    pub amount: u64,
    pub script_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PeginAddressOutput {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BtcTxSPVProofInput {
    pub block_hash: String,
    pub btc_tx: BitcoinTransaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct RequestPeginOutput {
    pub transaction_hash: String,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPegoutInput {
    pub amount_in_wei: u64,
    pub usr_pub_key: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct AddMemberNonceInput {
    pub hash_to_sign: Hash256,
    pub nonce: PubNonce,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct AddMemberSignatureInput {
    pub hash_to_sign: Hash256,
    pub signature: PartialSignature,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct TxSentOutput {
    pub transaction_hash: String,
    pub success: bool,
}

pub type AddMemberNonceOutput = TxSentOutput;
pub type AddMemberSignatureOutput = TxSentOutput;
pub type RequestPeginInput = BtcTxSPVProofInput;
pub type RegisterPegInInput = BtcTxSPVProofInput;
pub type AcceptPeginInput = BtcTxSPVProofInput;
pub type AcceptPeginOutput = RequestPeginOutput;
pub type RegisterPegoutInput = BtcTxSPVProofInput;
pub type RegisterPegoutOutput = TxSentOutput;
pub type RequestPegoutOutput = TxSentOutput;

impl From<TxIn> for BitcoinTransactionIn {
    fn from(input: TxIn) -> Self {
        BitcoinTransactionIn {
            tx_id: input.previous_output.txid.to_string(),
            v_out: input.previous_output.vout,
            sequence: input.sequence.0,
            script_sig: hex::encode(input.script_sig.into_bytes()),
        }
    }
}

impl From<TxOut> for BitcoinTransactionOut {
    fn from(output: TxOut) -> Self {
        BitcoinTransactionOut {
            amount: output.value.to_sat(),
            script_pub_key: hex::encode(output.script_pubkey.into_bytes()),
        }
    }
}

impl From<Transaction> for BitcoinTransaction {
    fn from(tx: Transaction) -> Self {
        BitcoinTransaction {
            version: tx.version.0 as u32,
            lock_time: tx.lock_time.to_consensus_u32(),
            inputs: tx.input.into_iter().map(Into::into).collect(),
            outputs: tx.output.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<BtcTxSPVProof> for RequestPeginInput {
    fn from(proof: BtcTxSPVProof) -> Self {
        RequestPeginInput {
            block_hash: proof.block_hash,
            btc_tx: BitcoinTransaction::from(proof.tx),
            merkle_branch_path: proof.merkle_branch_path,
            merkle_branch_hashes: proof
                .merkle_branch_hashes
                .into_iter()
                .map(hex::encode)
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetMemberPublicKeysInput {
    pub member_address: Address,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetMemberPublicKeysOutput {
    pub public_keys: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApplyToStreamInput {
    pub stream_id: u8,
    pub role: u8,
    pub committee_public_keys: [CommitteePublicKey; 3],
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CommitteePublicKey {
    pub x: String,
    pub y: String,
    pub r: String,
    pub s: String,
    pub v: u8,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct ApplyToStreamOutput {
    pub transaction_hash: String,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetCommitteeInput {
    pub committee_id: U256,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetCommitteeOutput {
    pub committee: Committee,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetMemberCommunicationDataOutput {
    pub communication_data: Vec<CommunicationData>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositCommunicationDataInput {
    pub stream_id: u64,
    pub communication_data: Vec<CommunicationData>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct DepositCommunicationDataOutput {
    pub transaction_hash: String,
    pub success: bool,
}

/// Parser and encoder for converting between [`P2PAddress`] (BitVMX) and [`CommunicationData`] (Contracts Bindings)
/// using a fixed-size 256-byte payload (`bytes32[8]` in Solidity terms).
///
/// ## Encoding format
///
/// The payload is **exactly 256 bytes**, laid out as:
///
/// ```text
/// [ u16_BE addr_len ][ addr_bytes UTF-8 ][ u16_BE peer_len ][ peer_bytes UTF-8 ][ zero padding ... ]
/// ```
///
/// - `addr_len` — 16-bit unsigned integer (big-endian) indicating the byte length of the UTF-8-encoded `address` string.
/// - `addr_bytes` — Raw UTF-8 bytes of the `address` string.
/// - `peer_len` — 16-bit unsigned integer (big-endian) indicating the byte length of the UTF-8-encoded `peer_id` string.
/// - `peer_bytes` — Raw UTF-8 bytes of the `peer_id` string.
/// - Padding — All remaining bytes up to a total of 256 bytes are zero-filled.
/// - The final 256 bytes are split into **8 × 32-byte chunks** for ABI compatibility with Solidity's `bytes32[8]` type.
///
/// ## Constraints
/// - `addr_bytes.len()` ≤ `u16::MAX`
/// - `peer_bytes.len()` ≤ `u16::MAX`
/// - `(2 + addr_bytes.len() + 2 + peer_bytes.len()) ≤ 256`
///
/// ## Example
///
/// For:
///
/// ```text
/// address = "/ip4/192.168.0.1/tcp/8888"
/// peer_id = "peer-abc"
/// ```
///
/// The first bytes of the encoded payload are:
///
/// ```text
/// 00 1D                                      # addr_len = 29 bytes (0x001D)
/// 2F 69 70 34 2F 31 39 32 2E 31 36 38 2E 30  # "/ip4/192.168.0"
/// 2E 31 2F 74 63 70 2F 38 38 38 38            # ".1/tcp/8888"
/// 00 08                                      # peer_len = 8 bytes (0x0008)
/// 70 65 65 72 2D 61 62 63                     # "peer-abc"
/// 00 00 00 00 ... (zero padding until total length is 256 bytes)
/// ```
///
/// This format is deterministic and round-trips between Rust ↔ Contracts Bindings.
pub struct P2PAddressParser;

impl P2PAddressParser {
    pub fn contracts_to_bitvmx(input: CommunicationData) -> Result<P2PAddress> {
        let mut buf = [0u8; 256];
        for (dst, fb) in buf.chunks_mut(32).zip(input.data.iter()) {
            let chunk: [u8; 32] = (*fb).into();
            dst.copy_from_slice(&chunk);
        }

        let mut offset: usize = 0;

        // Address length (2 bytes)
        let next = offset.checked_add(2).context("address length overflow")?;
        let addr_len_bytes = buf
            .get(offset..next)
            .context("address length out of bounds")?;
        let addr_len = u16::from_be_bytes(
            addr_len_bytes
                .try_into()
                .context("invalid address length bytes")?,
        ) as usize;
        offset = next;

        // Address string
        let next = offset
            .checked_add(addr_len)
            .context("address bytes length overflow")?;
        let address_bytes = buf
            .get(offset..next)
            .context("address bytes out of bounds")?;
        let address =
            String::from_utf8(address_bytes.to_vec()).context("invalid utf8 in address")?;
        offset = next;

        // Peer ID length (2 bytes)
        let next = offset.checked_add(2).context("peer id length overflow")?;
        let peer_len_bytes = buf
            .get(offset..next)
            .context("peer id length out of bounds")?;
        let peer_len = u16::from_be_bytes(
            peer_len_bytes
                .try_into()
                .context("invalid peer id length bytes")?,
        ) as usize;
        offset = next;

        // Peer ID string
        let next = offset
            .checked_add(peer_len)
            .context("peer id bytes length overflow")?;
        let peer_bytes = buf
            .get(offset..next)
            .context("peer id bytes out of bounds")?;
        let peer_id = String::from_utf8(peer_bytes.to_vec()).context("invalid utf8 in peer id")?;

        Ok(P2PAddress {
            address,
            peer_id: PeerId(peer_id),
        })
    }

    pub fn bitvmx_to_contracts(p2p_address: &P2PAddress) -> Result<CommunicationData> {
        let addr_bytes = p2p_address.address.as_bytes();
        let peer_bytes = p2p_address.peer_id.0.as_bytes();

        ensure!(addr_bytes.len() <= u16::MAX as usize, "Address too long");
        ensure!(peer_bytes.len() <= u16::MAX as usize, "Peer ID too long");

        // ensure total payload fits in 256 bytes to avoid out-of-bounds
        let total_len = 0usize
            .saturating_add(2)
            .saturating_add(addr_bytes.len())
            .saturating_add(2)
            .saturating_add(peer_bytes.len());
        ensure!(total_len <= 256, "Communication data too long");

        let mut buf = [0u8; 256];
        let mut offset: usize = 0;

        // Address length
        let next = offset
            .checked_add(2)
            .context("address length write overflow")?;
        buf.get_mut(offset..next)
            .context("address length write out of bounds")?
            .copy_from_slice(&(addr_bytes.len() as u16).to_be_bytes());
        offset = next;

        // Address bytes
        let next = offset
            .checked_add(addr_bytes.len())
            .context("address bytes write overflow")?;
        buf.get_mut(offset..next)
            .context("address bytes write out of bounds")?
            .copy_from_slice(addr_bytes);
        offset = next;

        // Peer ID length
        let next = offset
            .checked_add(2)
            .context("peer id length write overflow")?;
        buf.get_mut(offset..next)
            .context("peer id length write out of bounds")?
            .copy_from_slice(&(peer_bytes.len() as u16).to_be_bytes());
        offset = next;

        // Peer ID bytes
        let next = offset
            .checked_add(peer_bytes.len())
            .context("peer id bytes write overflow")?;
        buf.get_mut(offset..next)
            .context("peer id bytes write out of bounds")?
            .copy_from_slice(peer_bytes);

        Self::buf_to_comm(buf)
    }

    fn buf_to_comm(buf: [u8; 256]) -> Result<CommunicationData> {
        let mut data = [FixedBytes::<32>::ZERO; 8];
        for (i, chunk) in buf.chunks_exact(32).enumerate() {
            let data_slot = data.get_mut(i).context("chunk index out of bounds")?;
            *data_slot = FixedBytes::from(
                *<&[u8; 32]>::try_from(chunk)
                    .map_err(|e| anyhow!("Address field is not valid UTF-8: {}", e))?,
            );
        }
        Ok(CommunicationData { data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(addr: &str, peer: &str) {
        let original = P2PAddress {
            address: addr.to_string(),
            peer_id: PeerId(peer.to_string()),
        };
        let comm = P2PAddressParser::bitvmx_to_contracts(&original).expect("encode should succeed");
        let decoded = P2PAddressParser::contracts_to_bitvmx(comm).expect("decode should succeed");
        assert_eq!(decoded.address, original.address);
        assert_eq!(decoded.peer_id.0, original.peer_id.0);
    }

    #[test]
    fn roundtrip_ipv4_tcp() {
        roundtrip("/ip4/192.168.0.1/tcp/8888", "peer-abc-123");
    }

    #[test]
    fn roundtrip_ipv6_tcp() {
        roundtrip("/ip6/2001:db8::1/tcp/30303", "v6-peer");
    }

    #[test]
    fn roundtrip_dns_udp() {
        roundtrip("/dns4/example.com/udp/12000", "udp-peer-X");
    }

    #[test]
    fn roundtrip_empty_peer() {
        roundtrip("/ip4/10.0.0.5/tcp/8080", "");
    }

    #[test]
    fn roundtrip_exact_256_boundary() {
        // 2 + addr_len + 2 + peer_len must equal 256
        let addr_len = 100usize;
        let peer_len = 256 - 2 - addr_len - 2; // = 152
        let addr = "a".repeat(addr_len);
        let peer = "p".repeat(peer_len);

        roundtrip(&addr, &peer);
    }

    #[test]
    fn encode_returns_err_when_total_exceeds_256_bytes() {
        let addr = "/ip4/127.0.0.1/tcp/12345"; // any smallish addr
        let addr_len = addr.as_bytes().len();
        // Need: 2 + addr_len + 2 + peer_len > 256 => peer_len > 252 - addr_len
        let overflow_peer = "x".repeat(253 - addr_len);

        let p2p = P2PAddress {
            address: addr.to_string(),
            peer_id: PeerId(overflow_peer),
        };
        let err = P2PAddressParser::bitvmx_to_contracts(&p2p).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Communication data too long"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn decode_err_when_addr_len_out_of_bounds() {
        // addr_len = 300 (> 254 available after the first 2 bytes), triggers OOB
        let mut buf = [0u8; 256];
        buf[0..2].copy_from_slice(&(300u16.to_be_bytes())); // addr_len = 300
        // No addr bytes filled; guard should trip
        let comm = P2PAddressParser::buf_to_comm(buf).expect("buf_to_comm should succeed");
        let err = P2PAddressParser::contracts_to_bitvmx(comm).unwrap_err();
        assert!(
            err.to_string().contains("address bytes out of bounds"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_err_when_peer_len_out_of_bounds() {
        // Layout: [addr_len=1]['a'][peer_len=400](OOB)
        let mut buf = [0u8; 256];
        // addr_len = 1
        buf[0..2].copy_from_slice(&(1u16.to_be_bytes()));
        buf[2] = b'a'; // address content
        // peer_len = 400 (0x0190)
        buf[3..5].copy_from_slice(&(400u16.to_be_bytes()));
        // Guard should trip
        let comm = P2PAddressParser::buf_to_comm(buf).expect("buf_to_comm should succeed");
        let err = P2PAddressParser::contracts_to_bitvmx(comm).unwrap_err();
        assert!(
            err.to_string().contains("peer id bytes out of bounds"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_err_when_address_is_invalid_utf8() {
        // addr_len = 1, addr byte = 0xFF (invalid UTF-8)
        let mut buf = [0u8; 256];
        buf[0..2].copy_from_slice(&(1u16.to_be_bytes())); // addr_len = 1
        buf[2] = 0xFF; // invalid UTF-8
        // peer_len = 0
        buf[3..5].copy_from_slice(&(0u16.to_be_bytes()));
        let comm = P2PAddressParser::buf_to_comm(buf).expect("buf_to_comm should succeed");

        let err = P2PAddressParser::contracts_to_bitvmx(comm).unwrap_err();
        assert!(
            err.to_string()
                .to_lowercase()
                .contains("invalid utf8 in address"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_err_when_peer_is_invalid_utf8() {
        // addr_len = 1, addr = "a"; peer_len = 1, peer byte = 0xFF
        let mut buf = [0u8; 256];
        // addr
        buf[0..2].copy_from_slice(&(1u16.to_be_bytes()));
        buf[2] = b'a';
        // peer_len
        buf[3..5].copy_from_slice(&(1u16.to_be_bytes()));
        // peer content
        buf[5] = 0xFF; // invalid UTF-8
        let comm = P2PAddressParser::buf_to_comm(buf).expect("buf_to_comm should succeed");

        let err = P2PAddressParser::contracts_to_bitvmx(comm).unwrap_err();
        assert!(
            err.to_string()
                .to_lowercase()
                .contains("invalid utf8 in peer id"),
            "unexpected error: {}",
            err
        );
    }
}
