use alloy_primitives::{Address, Bytes, FixedBytes};
use anyhow::{Context, Result, bail};
use bitcoin::{Transaction, TxIn, TxOut, Txid};
use common::msg_broker::bitvmx_types::PeerId;
use common::types::{CommitteeId, StreamId};
use common::{msg_broker::bitvmx_types::BtcTxSPVProof, types::Hash256};
use multiaddr::Multiaddr;
use musig2::{PartialSignature, PubNonce};
use serde::{Deserialize, Serialize};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    Committee, CommunicationData, RSAPublicKey, UTXO,
};
use union_contracts::bindings::member_registry::MemberRegistry::RSAPublicKey as MemberRSAPublicKey;
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
pub type RequestPegoutOutput = TxSentOutput;
pub type ApplyToStreamOutput = TxSentOutput;
pub type DepositCommunicationDataOutput = TxSentOutput;
pub type DepositAggregatedKeyOutput = TxSentOutput;

pub type RequestPeginInput = BtcTxSPVProofInput;
pub type RegisterPegInInput = BtcTxSPVProofInput;
pub type AcceptPeginInput = BtcTxSPVProofInput;
pub type RegisterPegoutInput = BtcTxSPVProofInput;

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
    pub peer_id: PeerId,
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
        let multi_addr = Multiaddr::try_from(bytes)?;
        Ok(multi_addr.to_string())
    }

    /// Convert peer ID string to RSA public key
    ///
    /// # Errors
    ///
    /// Returns an error if the peer ID cannot be decoded or converted
    pub fn peer_id_to_contracts(peer_id: &str) -> Result<RSAPublicKey> {
        let peer_id_hex = hex::decode(peer_id)
            .with_context(|| format!("Failed to decode peer_id hex: {peer_id}"))?;
        let data = bytes_to_fb_array::<10>(&peer_id_hex)?;
        Ok(RSAPublicKey { rsaPublicKey: data })
    }

    /// Convert member RSA public key to peer ID string
    ///
    /// # Errors
    ///
    /// Returns an error if the communication data cannot be converted
    pub fn peer_id_from_member_contracts(comm_data: &MemberRSAPublicKey) -> Result<String> {
        let bytes = fb_array_to_bytes(&comm_data.rsaPublicKey)?;
        Ok(hex::encode(bytes))
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
    fn roundtrip_peer_id() {
        let peer = PeerId(
            "30820122300d06092a864886f70d01010105000382010f003082010a0282010100c96872f74e913fbcf2e068d7f508e52dad5a278123ad6546d9735e3f35163e836427ef6ea14ff28d4ca30e7f0d4e251ddf4724668675052d6adb8581550b0adb11f0dcb78a4e9d6ad00f68bf21851d590d88d9fff1d8d7678454f9df4a1daad2f8ebfe69b4ea99160a9e2d43a98cdaaaf380bc4de9f9dec6bedc9351c89c43e4d5d89abbef98664f5d57cdf5c68d93e928203c84fd038fedddac5bbe2b243378141edec442e83c57f0bab437336586f6d6bc01bee222ee8f67dfacb2d94d7a4e406d05446c9f84de055d6175217de19d1005203674b1693f1df2d3dacd11839a782c343c33e86b952740812da624f2ddfd71edf9eb5e9ddf7944b9afc3a08b2f0203010001".to_string(),
        );

        let encoded = P2PAddressParser::peer_id_to_contracts(&peer.0).unwrap();
        let member_encoded = MemberRSAPublicKey {
            rsaPublicKey: encoded.rsaPublicKey,
        };
        let decoded = P2PAddressParser::peer_id_from_member_contracts(&member_encoded).unwrap();
        assert_eq!(decoded, peer.0);
    }

    #[test]
    fn roundtrip_peer_id_ending_zero() {
        let peer = PeerId(
            "30820122300d06092a864886f70d01010105000382010f003082010a0282010100c96872f74e913fbcf2e068d7f508e52dad5a278123ad6546d9735e3f35163e836427ef6ea14ff28d4ca30e7f0d4e251ddf4724668675052d6adb8581550b0adb11f0dcb78a4e9d6ad00f68bf21851d590d88d9fff1d8d7678454f9df4a1daad2f8ebfe69b4ea99160a9e2d43a98cdaaaf380bc4de9f9dec6bedc9351c89c43e4d5d89abbef98664f5d57cdf5c68d93e928203c84fd038fedddac5bbe2b243378141edec442e83c57f0bab437336586f6d6bc01bee222ee8f67dfacb2d94d7a4e406d05446c9f84de055d6175217de19d1005203674b1693f1df2d3dacd11839a782c343c33e86b952740812da624f2ddfd71edf9eb5e9ddf7944b9afc3a08b2f0203010000".to_string(),
        );

        let encoded = P2PAddressParser::peer_id_to_contracts(&peer.0).unwrap();
        let member_encoded = MemberRSAPublicKey {
            rsaPublicKey: encoded.rsaPublicKey,
        };
        let decoded = P2PAddressParser::peer_id_from_member_contracts(&member_encoded).unwrap();
        assert_eq!(decoded, peer.0);
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
}
