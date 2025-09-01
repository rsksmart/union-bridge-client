use anyhow::{anyhow, Context, Result};
use bitcoin::key::rand::rngs::OsRng;
use bitcoin::key::{Parity, UntweakedPublicKey};
use bitcoin::taproot::{TaprootBuilder, TaprootSpendInfo};
use bitcoin::{
    absolute,
    hex::FromHex,
    key::Secp256k1,
    secp256k1::{self, All, Message, PublicKey as SecpPublicKey, SecretKey},
    sighash::SighashCache,
    transaction, Address as BitcoinAddress, Amount, Network, OutPoint, PrivateKey, PublicKey,
    ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey,
};
use bitcoin_scriptexec::treepp::*;
pub use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use common::msg_broker::bitvmx_types::{PartialUtxo, SignMode};
use common::types::{Address, ToHexString};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

const REGTEST: Network = Network::Regtest;
const SPEEDUP_VALUE: u64 = 540;

#[derive(Clone)]
pub struct User {
    pub public_key: PublicKey,
    pub bitcoin_client: Arc<BitcoinClient>,
    pub address: BitcoinAddress,
    secret_key: SecretKey,
    pub network: Network,
    pub secp: Secp256k1<All>,
    pub rsk_address: Address,
}

impl User {
    pub fn new(rsk_address: Address, bitcoin_client: BitcoinClient) -> Result<Self> {
        let secp = Secp256k1::new();
        let (user_address, user_pubkey, user_sk) =
            Self::emulated_user_keypair(&secp, &bitcoin_client, REGTEST)?;
        Ok(Self {
            bitcoin_client: Arc::new(bitcoin_client),
            public_key: user_pubkey,
            address: user_address,
            secret_key: user_sk,
            network: REGTEST,
            secp,
            rsk_address,
        })
    }

    pub fn get_rsk_address(&self) -> Address {
        self.rsk_address
    }

    pub fn request_pegin(
        &self,
        stream_value: u64,
        packet_num: u64,
        tmp_addr: String,
    ) -> Result<Txid> {
        info!("Requesting pegin");

        // Create a proper RSK pegin transaction and send it as if it was a user transaction
        let request_pegin_tx =
            self.create_and_send_request_pegin_tx(stream_value, packet_num, tmp_addr)?;

        info!("Sent RSK pegin transaction to bitcoind {request_pegin_tx:?}");

        Ok(request_pegin_tx.compute_txid())
    }

    pub fn public_key(&self) -> Result<PublicKey> {
        Ok(self.public_key)
    }

    fn create_and_send_request_pegin_tx(
        &self,
        stream_value: u64,
        packet_number: u64,
        tmp_addr: String,
    ) -> Result<Transaction> {
        info!("Creating RSK pegin transaction");

        // We'll create a transaction that will be detected as RSK pegin by the transaction monitor.
        let signed_transaction =
            self.create_request_pegin_tx(stream_value, packet_number, tmp_addr)?;
        let txid = self.bitcoin_client.send_transaction(&signed_transaction)?;

        // Get the transaction and verify it was created
        let request_pegin_tx = self.bitcoin_client.get_transaction(&txid)?.unwrap();

        // Mine blocks to include the transaction
        self.bitcoin_client
            .mine_blocks_to_address(1, &self.address)?;

        Ok(request_pegin_tx)
    }

    fn dispatch_tx(&self, tx: Transaction) -> Result<Txid> {
        let txid = self
            .bitcoin_client
            .send_transaction(&tx)
            .map_err(|e| anyhow::anyhow!("Failed to dispatch transaction: {}", e))?;

        Ok(txid)
    }

    fn get_funding_utxo(&self, amount: u64) -> Result<PartialUtxo> {
        let (funding_tx, vout) = self
            .bitcoin_client
            .fund_address(&self.address, Amount::from_sat(amount))?;
        Ok((funding_tx.compute_txid(), vout, Some(amount), None))
    }

    fn create_request_pegin_tx(
        &self,
        stream_value: u64,
        packet_number: u64,
        tmp_addr: String,
    ) -> Result<Transaction> {
        // RSK Pegin constants
        pub const KEY_SPEND_FEE: u64 = 335;
        pub const OP_RETURN_FEE: u64 = 300;
        // TODO: This should be based on the actual fee rate from the Bitcoin network
        pub const EXTRA_FEE: u64 = 1000;

        let value = stream_value;
        let fee = KEY_SPEND_FEE + EXTRA_FEE;
        let op_return_fee = OP_RETURN_FEE;
        let total_amount = value + fee + op_return_fee;

        // Fund the user address with enough to cover the taproot output + fees
        let (funding_tx, vout) = self
            .bitcoin_client
            .fund_address(&self.address, Amount::from_sat(total_amount))?;

        // RSK Pegin values
        debug!(
            "Roostock address: {}",
            self.rsk_address.to_string().as_str()
        );
        let rootstock_address =
            self.address_to_bytes(self.rsk_address.to_hex_string().trim_start_matches("0x"))?;
        let reimbursement_xpk = self.public_key.into();

        // Create the Request pegin transaction
        // Inputs
        let funds_input = TxIn {
            previous_output: OutPoint::new(funding_tx.compute_txid(), vout),
            script_sig: ScriptBuf::default(), // For a p2wpkh script_sig is empty.
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME, // we want to be able to replace this transaction
            witness: Witness::default(),                // Filled in after, at signing time.
        };

        // Outputs
        // 1) Taproot output
        let addr = bitcoin::Address::from_str(&tmp_addr)
            .context("Parsing tmp_addr to Bitcoin")?
            .require_network(REGTEST)
            .context("Requiring network for tmp_addr")?;
        let dest_spk: ScriptBuf = addr.script_pubkey();

        let taproot_output = TxOut {
            value: Amount::from_sat(value),
            script_pubkey: dest_spk,
        };

        // 2) OP_RETURN output
        let op_return_data = User::request_pegin_op_return_data(
            packet_number,
            rootstock_address,
            reimbursement_xpk,
        )?;

        let op_return_output = TxOut {
            value: Amount::from_sat(0), // OP_RETURN outputs should have 0 value
            script_pubkey: op_return_script(op_return_data)?.get_script().clone(),
        };

        let mut request_pegin_transaction = Transaction {
            version: transaction::Version::TWO,  // Post BIP-68.
            lock_time: absolute::LockTime::ZERO, // Ignore the transaction lvl absolute locktime.
            input: vec![funds_input],
            output: vec![taproot_output, op_return_output],
        };

        let signed_transaction = self.sign_p2wpkh_transaction(
            &mut request_pegin_transaction,
            [(0usize, total_amount)].to_vec(),
        )?;
        info!("Request pegin txid: {}", signed_transaction.compute_txid());

        Ok(signed_transaction)
    }

    pub fn create_and_dispatch_speedup(&self, tx_output: OutPoint, fee: u64) -> Result<()> {
        let speedup_tx = self.create_speedup_tx(tx_output, fee)?;

        let speedup_txid = self.dispatch_tx(speedup_tx)?;
        info!("Speedup transaction dispatched: {}", speedup_txid);
        Ok(())
    }

    pub fn create_speedup_tx(&self, tx_output: OutPoint, fee: u64) -> Result<Transaction> {
        let funding_utxo = self.get_funding_utxo(10_000_000)?;

        // Create two inputs: one from the funding utxo, one from the output to speed up
        let input_funding = TxIn {
            previous_output: OutPoint::new(funding_utxo.0, funding_utxo.1),
            script_sig: ScriptBuf::default(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(),
        };

        let input_speedup = TxIn {
            previous_output: tx_output,
            script_sig: ScriptBuf::default(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(),
        };

        // Output: all funds (minus fee) to user address
        let total_in = funding_utxo.2.unwrap(); // You may want to add the value of tx_output if known
        let output = TxOut {
            value: Amount::from_sat(total_in - fee),
            script_pubkey: self.address.script_pubkey(),
        };

        let mut transaction = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![input_speedup, input_funding],
            output: vec![output],
        };

        // Sign the transaction (this may need to be adapted to sign both inputs)
        let signed_transaction = self.sign_p2wpkh_transaction(
            &mut transaction,
            [(0, SPEEDUP_VALUE), (1, total_in)].to_vec(),
        )?;
        info!(
            "Speeding up txid: {}. Speedup txid: {}",
            tx_output.txid,
            signed_transaction.compute_txid()
        );

        Ok(signed_transaction)
    }

    fn request_pegin_op_return_data(
        packet_number: u64,
        rootstock_address: [u8; 20],
        reimbursement_xpk: XOnlyPublicKey,
    ) -> Result<Vec<u8>> {
        let mut user_data = [0u8; 69];
        user_data.copy_from_slice(
            [
                b"RSK_PEGIN".as_slice(),
                &packet_number.to_be_bytes(),
                &rootstock_address,
                &reimbursement_xpk.serialize(),
            ]
            .concat()
            .as_slice(),
        );
        Ok(user_data.to_vec())
    }

    fn address_to_bytes(&self, address: &str) -> Result<[u8; 20]> {
        let mut address_bytes = [0u8; 20];
        address_bytes.copy_from_slice(Vec::from_hex(address)?.as_slice());
        Ok(address_bytes)
    }

    fn sign_p2wpkh_transaction(
        &self,
        transaction: &mut Transaction,
        index_amount: Vec<(usize, u64)>,
    ) -> Result<Transaction> {
        let user_bitcoin_privkey = PrivateKey {
            compressed: true,
            network: self.network.into(),
            inner: self.secret_key,
        };

        let user_comp_pubkey =
            bitcoin::CompressedPublicKey::from_private_key(&self.secp, &user_bitcoin_privkey)?;
        let uncompressed_pk = secp256k1::PublicKey::from_slice(&user_comp_pubkey.to_bytes())?;

        // Sign the transactions inputs
        let wpkh = self.public_key.wpubkey_hash().expect("key is compressed");
        let script_pubkey = ScriptBuf::new_p2wpkh(&wpkh);
        let mut sighasher = SighashCache::new(transaction);

        let sighash_type = bitcoin::EcdsaSighashType::All;
        for (input_index, value) in index_amount {
            let sighash = sighasher
                .p2wpkh_signature_hash(
                    input_index,
                    &script_pubkey,
                    Amount::from_sat(value),
                    sighash_type,
                )
                .expect("failed to create rsk request pegin input sighash");

            let signature = bitcoin::ecdsa::Signature {
                signature: self
                    .secp
                    .sign_ecdsa(&Message::from(sighash), &self.secret_key),
                sighash_type,
            };

            *sighasher.witness_mut(input_index).unwrap() =
                Witness::p2wpkh(&signature, &uncompressed_pk);
        }

        // Now the transaction is signed
        let signed_transaction = sighasher.into_transaction().to_owned();
        Ok(signed_transaction)
    }

    // This method changes the parity of a keypair to be even, this is needed for Taproot.
    fn adjust_parity(
        secp: &Secp256k1<All>,
        pubkey: SecpPublicKey,
        seckey: SecretKey,
    ) -> (SecpPublicKey, SecretKey) {
        let (_, parity) = pubkey.x_only_public_key();

        if parity == Parity::Odd {
            (pubkey.negate(secp), seckey.negate())
        } else {
            (pubkey, seckey)
        }
    }

    pub fn emulated_user_keypair(
        secp: &Secp256k1<All>,
        bitcoin_client: &BitcoinClient,
        network: Network,
    ) -> Result<(bitcoin::Address, PublicKey, SecretKey)> {
        let mut rng = OsRng;

        // emulate the user keypair
        let user_sk = SecretKey::new(&mut rng);
        let user_pk = SecpPublicKey::from_secret_key(secp, &user_sk);
        let (user_pk, user_sk) = Self::adjust_parity(secp, user_pk, user_sk);
        let user_pubkey = PublicKey {
            compressed: true,
            inner: user_pk,
        };

        let user_address: bitcoin::Address = bitcoin_client.get_new_address(user_pubkey, network);
        info!(
            "User Address({}): {:?}",
            user_address.address_type().unwrap(),
            user_address
        );
        Ok((user_address, user_pubkey, user_sk))
    }
}

// this is temporary for Regtest stage, required for utxo generation
pub fn build_bitcoin_client_regtest() -> BitcoinClient {
    let config_bitcoin_client = RpcConfig::new(
        REGTEST,
        "http://127.0.0.1:18443".to_string(),
        "foo".to_string(),
        "rpcpassword".to_string(),
        "test_wallet".to_string(),
    );

    let bitcoin_client = BitcoinClient::new_from_config(&config_bitcoin_client)
        .expect("Cannot create Setup Committee Flow without a Bitcoin Client");

    bitcoin_client
}

pub fn build_taproot_spend_info(
    secp: &Secp256k1<All>,
    internal_key: &UntweakedPublicKey,
    leaves: &[ProtocolScript],
) -> Result<TaprootSpendInfo> {
    let scripts_count = leaves.len();

    let mut tr_builder = TaprootBuilder::new();

    // For empty scripts finalize the tree
    if scripts_count == 0 {
        return tr_builder
            .finalize(secp, *internal_key)
            .map_err(|e| anyhow!("ScriptError::TapTreeFinalizeError: {e:?}"));
    }

    // For a single script, add it at depth 0
    if scripts_count == 1 {
        tr_builder = tr_builder.add_leaf(0, leaves[0].get_script().clone())?;
        return tr_builder
            .finalize(secp, *internal_key)
            .map_err(|_| anyhow!("ScriptError::TapTreeFinalizeError"));
    }

    // For multiple scripts, build a balanced tree
    //
    // Example tree structure for 7 scripts:
    //
    //           [Root]
    //          /      \
    //      [1-3]      [4-7]
    //     /     \     /    \
    //   [1-2]  [3]  [4-5] [6-7]
    //   /  \         /  \   /  \
    // [1] [2]     [4] [5] [6] [7]
    //
    // The algorithm calculates the minimum depth needed to hold all scripts
    // and then distributes the scripts between that depth and the next one
    // to maintain a balanced tree structure.

    // Calculate the minimum depth needed to hold all scripts
    let min_depth = (scripts_count as f32 - 1.0).log2().floor() as u8;
    // Calculate how many nodes go at the minimum depth vs minimum depth + 1
    let total_slots = 1 << (min_depth + 1); // 2^(min_depth + 1)
    let nodes_at_min_depth = total_slots - scripts_count;
    // Add leaves at minimum depth
    for i in 0..nodes_at_min_depth {
        tr_builder = tr_builder.add_leaf(min_depth, leaves[i].get_script().clone())?;
    }

    // Add remaining leaves at minimum depth + 1
    for i in nodes_at_min_depth..scripts_count {
        tr_builder = tr_builder.add_leaf(min_depth + 1, leaves[i].get_script().clone())?;
    }

    tr_builder
        .finalize(secp, *internal_key)
        .map_err(|e| anyhow!("ScriptError::TapTreeFinalizeError: {e:?}"))
}

fn op_return_script(data: Vec<u8>) -> Result<ProtocolScript> {
    let script = script!(OP_RETURN { data });

    let protocol_script = ProtocolScript::new_unspendable(script);
    Ok(protocol_script)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolScript {
    script: ScriptBuf,
    keys: HashMap<String, ScriptKey>,
    verifying_key: Option<PublicKey>,
    sign_mode: SignMode,
    items: Vec<StackItem>,
}

impl ProtocolScript {
    pub fn new(script: ScriptBuf, verifying_key: &PublicKey, sign_mode: SignMode) -> Self {
        Self {
            script,
            keys: HashMap::new(),
            verifying_key: Some(*verifying_key),
            sign_mode,
            items: Vec::new(),
        }
    }

    pub fn new_unspendable(script: ScriptBuf) -> Self {
        Self {
            script,
            keys: HashMap::new(),
            verifying_key: None,
            sign_mode: SignMode::Skip,
            items: Vec::new(),
        }
    }

    pub fn get_key(&self, name: &str) -> Option<ScriptKey> {
        self.keys.get(name).cloned()
    }

    pub fn get_verifying_key(&self) -> Option<PublicKey> {
        self.verifying_key
    }

    pub fn get_script(&self) -> &ScriptBuf {
        &self.script
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptKey {
    name: String,
    key_type: KeyType,
    key_position: u32,
    derivation_index: u32,
}

impl ScriptKey {
    pub fn new(name: &str, derivation_index: u32, key_type: KeyType, key_position: u32) -> Self {
        Self {
            name: name.to_string(),
            key_type,
            key_position,
            derivation_index,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn derivation_index(&self) -> u32 {
        self.derivation_index
    }

    pub fn key_type(&self) -> KeyType {
        self.key_type.clone()
    }

    pub fn key_position(&self) -> u32 {
        self.key_position
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum KeyType {
    EcdsaKey,
    XOnlyKey,
}

impl KeyType {
    pub fn ecdsa() -> Self {
        KeyType::EcdsaKey
    }

    pub fn x_only() -> Self {
        KeyType::XOnlyKey
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StackItem {
    /// Schnorr signature (64 bytes +1 if non-default sighash).
    SchnorrSig { non_default_sighash: bool },
    /// DER-encoded ECDSA signature (use 73B worst case) +1 if non-default sighash.
    EcdsaSig { non_default_sighash: bool },
    /// Winternitz signature (size depends on the key type).
    WinternitzSig { size: usize },
    /// Raw item of a known length (e.g., pubkeys, data pushes).
    Raw { size: usize },
}

pub fn timelock(blocks: u16, timelock_key: &PublicKey, sign_mode: SignMode) -> ProtocolScript {
    let script = script!(
        // If blocks have passed since this transaction has been confirmed, the timelocked public key can spend the funds
        { blocks as u32 }
        OP_CSV
        OP_DROP
        { XOnlyPublicKey::from(*timelock_key).serialize().to_vec() }
        OP_CHECKSIG
    );

    ProtocolScript::new(script, timelock_key, sign_mode)
}
