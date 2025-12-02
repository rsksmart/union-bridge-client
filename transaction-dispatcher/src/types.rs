use alloy_primitives::{Address, Bytes, FixedBytes};
use anyhow::{Context, Result, bail};
use bitcoin::{Transaction, TxIn, TxOut, Txid};
use std::net::{IpAddr, SocketAddr};
use multiaddr::{Multiaddr, Protocol};
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommunicationData, RSAPublicKey, UTXO,
};
use union_contracts::bindings::member_registry::MemberRegistry::RSAPublicKey as MemberRSAPublicKey;
use common::msg_broker::bitvmx_types::{BtcTxSPVProof, PubKeyHash};
use common::types::{CommitteeId, Hash256, StreamId};
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

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestPegoutInput {
    pub amount_in_wei: u64,
    pub usr_pub_key: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct TriggerOperatorTakeInput {
    pub pegout_txid: String,
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
pub struct AddOperatorTakeTxHashInput {
    pub accept_pegin_tx_hash: Txid,
    pub take_tx_hash: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct TxSentOutput {
    pub transaction_hash: String,
}

pub type AddMemberNonceOutput = TxSentOutput;
pub type AddMemberSignatureOutput = TxSentOutput;
pub type AddOperatorTakeTxHashOutput = TxSentOutput;
pub type RequestPeginOutput = TxSentOutput;
pub type AcceptPeginOutput = TxSentOutput;
pub type RegisterPegoutOutput = TxSentOutput;
pub type RegisterOperatorTakeOutput = TxSentOutput;
pub type TriggerOperatorTakeOutput = TxSentOutput;
pub type RequestPegoutOutput = TxSentOutput;
pub type ApplyToStreamOutput = TxSentOutput;
pub type DepositCommunicationDataOutput = TxSentOutput;
pub type DepositAggregatedKeyOutput = TxSentOutput;

pub type RequestPeginInput = BtcTxSPVProofInput;
pub type RegisterPegInInput = BtcTxSPVProofInput;
pub type AcceptPeginInput = BtcTxSPVProofInput;
pub type RegisterPegoutInput = BtcTxSPVProofInput;
pub type RegisterOperatorTakeInput = BtcTxSPVProofInput;

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
            version: u32::try_from(tx.version.0).expect("Transaction version must fit in u32"),
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
            merkle_branch_hashes: proof.merkle_branch_hashes.into_iter().map(hex::encode).collect(),
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
pub struct GetBtcTransactionConfirmationsInput {
    pub tx_hash: common::types::TxHash,
    pub block_hash: common::types::BlockHash,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetBtcTransactionConfirmationsOutput {
    pub confirmations: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApplyToStreamInput {
    pub stream_id: StreamId, // Matches StreamDenomination enum in contracts
    pub role: u8,
    pub take_key: CommitteeECDSA,
    pub dispute_key: CommitteeECDSA,
    pub pubkey_hash: PubKeyHash,
    pub funding_utxo: UTXO,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CommitteeECDSA {
    pub x: String,
    pub y: String,
    pub r: String,
    pub s: String,
    pub v: u8,
}

pub type CommitteeRSA = String;

#[derive(Serialize, Deserialize, Debug)]
pub struct GetCommitteeInput {
    pub committee_id: CommitteeId,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetCommitteeOutput {
    pub committee: Committee,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetCommunicationDataInput {
    pub committee_id: CommitteeId,
    // TODO rethink if this is needed or a member should only request its own communication data and therefore this param is not required
    pub member_address: Address,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetCommunicationDataOutput {
    pub communication_data: Vec<CommunicationData>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositCommunicationDataInput {
    pub committee_id: CommitteeId,
    pub communication_data: Vec<CommunicationData>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositAggregatedKeyInput {
    pub committee_id: CommitteeId,
    pub aggregated_key: Bytes,
}

/// Flatten bytes → `[FixedBytes<32>; N]` (zero-pad if shorter; error if longer).
fn bytes_to_fb_array<const N: usize>(bytes: &[u8]) -> Result<[FixedBytes<32>; N]> {
    let cap = N * 32;
    if bytes.len() > cap.saturating_sub(2) {
        bail!("payload too large: {} > {}", bytes.len(), cap - 2);
    }
    if bytes.len() > u16::MAX as usize {
        bail!("payload too large for u16 header: {}", bytes.len());
    }

    let mut buf = vec![0u8; cap];
    let len_be = u16::try_from(bytes.len())
        .expect("bytes.len() already checked to be <= u16::MAX")
        .to_be_bytes();
    buf[0..2].copy_from_slice(&len_be);
    buf[2..2 + bytes.len()].copy_from_slice(bytes);

    let mut out: [FixedBytes<32>; N] = [FixedBytes([0u8; 32]); N];
    for (i, chunk) in buf.chunks_exact(32).enumerate() {
        out[i] = FixedBytes::<32>(chunk.try_into().context("chunk != 32")?);
    }
    Ok(out)
}

/// Flatten `[FixedBytes<32>; N]` → Vec<u8> and trim trailing zero padding.
/// NOTE: If real payload could end with 0x00 bytes, prefer storing a length header.
fn fb_array_to_bytes<const N: usize>(arr: &[FixedBytes<32>; N]) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(N * 32);
    for fb in arr {
        buf.extend_from_slice(&fb.0);
    }
    if buf.len() < 2 {
        bail!("buffer too small for length header");
    }
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if len > buf.len().saturating_sub(2) {
        bail!("invalid stored length {len}");
    }
    Ok(buf[2..2 + len].to_vec())
}

pub struct P2PAddressParser;

impl P2PAddressParser {
    /// Convert address string to communication data
    ///
    /// # Errors
    ///
    /// Returns an error if the address cannot be parsed or converted
    pub fn addr_to_contracts(address: &str) -> Result<CommunicationData> {
        let multi_addr: Multiaddr = address.parse()?;
        let multi_addr_bytes = multi_addr.to_vec();
        let data = bytes_to_fb_array::<8>(&multi_addr_bytes)?;
        Ok(CommunicationData { data })
    }

    /// Convert communication data to address string
    ///
    /// # Errors
    ///
    /// Returns an error if the communication data cannot be converted
    pub fn addr_from_contracts(comm_data: &CommunicationData) -> Result<String> {
        let bytes = fb_array_to_bytes::<8>(&comm_data.data)?;
        // Empty bytes means no communication data has been deposited yet
        if bytes.is_empty() {
            return Ok(String::new());
        }
        let multi_addr = Multiaddr::try_from(bytes)?;
        Ok(multi_addr.to_string())
    }

    /// Convert a multiaddr string (e.g., "/ip4/192.168.0.1/tcp/8888") to a `SocketAddr`.
    /// Supports both IPv4 and IPv6 addresses.
    ///
    /// # Errors
    ///
    /// Returns an error if the multiaddr string cannot be parsed or is missing IP/port components.
    pub fn multiaddr_to_socket_addr(multiaddr_str: &str) -> Result<SocketAddr> {
        let multi_addr: Multiaddr = multiaddr_str
            .parse()
            .with_context(|| format!("Failed to parse multiaddr: {multiaddr_str}"))?;

        let mut ip: Option<IpAddr> = None;
        let mut port: Option<u16> = None;

        for protocol in &multi_addr {
            match protocol {
                Protocol::Ip4(addr) => ip = Some(IpAddr::V4(addr)),
                Protocol::Ip6(addr) => ip = Some(IpAddr::V6(addr)),
                Protocol::Tcp(p) | Protocol::Udp(p) => port = Some(p),
                _ => {}
            }
        }

        let ip = ip.with_context(|| format!("No IP address found in multiaddr: {multiaddr_str}"))?;
        let port = port.with_context(|| format!("No port found in multiaddr: {multiaddr_str}"))?;

        Ok(SocketAddr::new(ip, port))
    }

    /// Decode communication data from contracts directly to a `SocketAddr`.
    /// Returns None if no communication data has been deposited yet (zeroed data).
    ///
    /// # Errors
    ///
    /// Returns an error if the stored multiaddr cannot be decoded or parsed.
    pub fn socket_addr_from_contracts(comm_data: &CommunicationData) -> Result<Option<SocketAddr>> {
        let multiaddr_str = Self::addr_from_contracts(comm_data)?;
        if multiaddr_str.is_empty() {
            return Ok(None);
        }
        Self::multiaddr_to_socket_addr(&multiaddr_str).map(Some)
    }

    /// Convert a `SocketAddr` to `CommunicationData` for contract storage.
    /// Converts the `SocketAddr` to multiaddr format internally.
    ///
    /// # Errors
    ///
    /// Returns an error if the address cannot be encoded into contract format.
    pub fn socket_addr_to_contracts(addr: &SocketAddr) -> Result<CommunicationData> {
        let multiaddr_str = match addr {
            SocketAddr::V4(v4) => format!("/ip4/{}/tcp/{}", v4.ip(), v4.port()),
            SocketAddr::V6(v6) => format!("/ip6/{}/tcp/{}", v6.ip(), v6.port()),
        };
        Self::addr_to_contracts(&multiaddr_str)
    }

    /// # Errors
    ///
    /// Returns an error if `pubkey_hash` is not valid hex or is not exactly 32 bytes.
    ///
    /// # Panics
    ///
    /// Panics if the decoded bytes cannot be converted to a `[u8; 32]` (unreachable after the length check).
    pub fn pubkey_hash_to_contracts(pubkey_hash: &str) -> Result<RSAPublicKey> {
        let pubkey_hash_bytes = hex::decode(pubkey_hash)
            .with_context(|| format!("Failed to decode pubkey_hash hex: {pubkey_hash}"))?;
        // pubkey_hash is 32 bytes (SHA-256), contracts expect bytes32[10] (320 bytes)
        // Store the hash in the first slot directly, pad rest with zeros
        if pubkey_hash_bytes.len() != 32 {
            bail!(
                "pubkey_hash must be 32 bytes (SHA-256), got {} bytes",
                pubkey_hash_bytes.len()
            );
        }
        let mut data: [FixedBytes<32>; 10] = [FixedBytes([0u8; 32]); 10];
        data[0] = FixedBytes::<32>(pubkey_hash_bytes.try_into().unwrap());
        Ok(RSAPublicKey { rsaPublicKey: data })
    }

    /// # Errors
    ///
    /// Returns an error if the public key data cannot be decoded.
    pub fn pubkey_hash_from_member_contracts(comm_data: &MemberRSAPublicKey) -> Result<String> {
        // Extract only the first 32 bytes (the pubkey_hash), ignore padding
        let first_slot = comm_data.rsaPublicKey[0];
        Ok(hex::encode(first_slot.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_p2p_addr_parser(addr: &str) {
        let encoded = P2PAddressParser::addr_to_contracts(addr).expect("encode should succeed");
        let decoded =
            P2PAddressParser::addr_from_contracts(&encoded).expect("decode should succeed");
        assert_eq!(decoded, addr);
    }

    #[test]
    fn roundtrip_pubkey_hash() {
        // SHA-256 hash (64 hex chars = 32 bytes)
        let pubkey_hash = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".to_string();

        let encoded = P2PAddressParser::pubkey_hash_to_contracts(&pubkey_hash).unwrap();
        let member_encoded = MemberRSAPublicKey {
            rsaPublicKey: encoded.rsaPublicKey,
        };
        let decoded = P2PAddressParser::pubkey_hash_from_member_contracts(&member_encoded).unwrap();
        assert_eq!(decoded, pubkey_hash);
    }

    #[test]
    fn roundtrip_ipv4_tcp() {
        roundtrip_p2p_addr_parser("/ip4/192.168.0.1/tcp/8888");
    }

    #[test]
    fn roundtrip_ipv6_tcp() {
        roundtrip_p2p_addr_parser("/ip6/2001:db8::1/tcp/30303");
    }

    #[test]
    fn roundtrip_dns_udp() {
        roundtrip_p2p_addr_parser("/dns4/example.com/udp/12000");
    }
    #[test]
    fn roundtrip_addr_ending_zero() {
        roundtrip_p2p_addr_parser("/ip4/127.0.0.1/tcp/1024");
    }

    #[test]
    fn multiaddr_to_socket_addr_ipv4_tcp() {
        let socket = P2PAddressParser::multiaddr_to_socket_addr("/ip4/192.168.0.1/tcp/8888").unwrap();
        assert_eq!(socket.to_string(), "192.168.0.1:8888");
    }

    #[test]
    fn multiaddr_to_socket_addr_ipv6_tcp() {
        let socket = P2PAddressParser::multiaddr_to_socket_addr("/ip6/::1/tcp/30303").unwrap();
        assert_eq!(socket.to_string(), "[::1]:30303");
    }

    #[test]
    fn multiaddr_to_socket_addr_ipv4_udp() {
        let socket = P2PAddressParser::multiaddr_to_socket_addr("/ip4/10.0.0.1/udp/12000").unwrap();
        assert_eq!(socket.to_string(), "10.0.0.1:12000");
    }

    #[test]
    fn socket_addr_from_contracts_roundtrip() {
        let original = "/ip4/192.168.0.1/tcp/8888";
        let encoded = P2PAddressParser::addr_to_contracts(original).unwrap();
        let socket = P2PAddressParser::socket_addr_from_contracts(&encoded)
            .unwrap()
            .expect("should have socket addr");
        assert_eq!(socket.to_string(), "192.168.0.1:8888");
    }

    #[test]
    fn socket_addr_from_contracts_zeroed_data_returns_none() {
        // Simulates contract data that hasn't been deposited yet (all zeros)
        let zeroed_data = CommunicationData {
            data: [FixedBytes([0u8; 32]); 8],
        };
        let result = P2PAddressParser::socket_addr_from_contracts(&zeroed_data).unwrap();
        assert!(result.is_none(), "zeroed data should return None");
    }

    #[test]
    fn addr_from_contracts_zeroed_data_returns_empty_string() {
        // Simulates contract data that hasn't been deposited yet (all zeros)
        let zeroed_data = CommunicationData {
            data: [FixedBytes([0u8; 32]); 8],
        };
        let result = P2PAddressParser::addr_from_contracts(&zeroed_data).unwrap();
        assert!(result.is_empty(), "zeroed data should return empty string");
    }

    #[test]
    fn socket_addr_to_contracts_ipv4_roundtrip() {
        use std::str::FromStr;
        let addr = SocketAddr::from_str("192.168.0.1:8888").unwrap();
        let encoded = P2PAddressParser::socket_addr_to_contracts(&addr).unwrap();
        let decoded = P2PAddressParser::socket_addr_from_contracts(&encoded)
            .unwrap()
            .expect("should have socket addr");
        assert_eq!(decoded, addr);
    }

    #[test]
    fn socket_addr_to_contracts_ipv6_roundtrip() {
        use std::str::FromStr;
        let addr = SocketAddr::from_str("[::1]:30303").unwrap();
        let encoded = P2PAddressParser::socket_addr_to_contracts(&addr).unwrap();
        let decoded = P2PAddressParser::socket_addr_from_contracts(&encoded)
            .unwrap()
            .expect("should have socket addr");
        assert_eq!(decoded, addr);
    }
}
