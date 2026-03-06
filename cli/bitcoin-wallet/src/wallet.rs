use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::bitcoin::reqwest_https::ReqwestHttpsTransport;
use crate::bitcoin::utils::fetch_utxo_amount;
use crate::config::Config;
use crate::pending_tx_store::PendingTransactionStore;
use crate::utxo_store::{UtxoState, UtxoStore};
use anyhow::{Context, Result, anyhow, bail};
use bitcoin::absolute;
use bitcoin::address::{Address, NetworkUnchecked};
use bitcoin::blockdata::transaction::{Sequence, Version};
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::ecdsa;
use bitcoin::hashes::hex::FromHex;
use bitcoin::key::{CompressedPublicKey, PrivateKey, PublicKey};
use bitcoin::network::{Network, NetworkKind};
use bitcoin::opcodes::all::OP_RETURN;
use bitcoin::script::{Builder as ScriptBuilder, PushBytesBuf};
use bitcoin::secp256k1::rand::rngs::OsRng;
use bitcoin::secp256k1::{self, Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{
    Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey,
};
use bitcoincore_rpc::{Client, RpcApi, jsonrpc};

pub const DEFAULT_SATS_PER_BYTE: u64 = 5;
const P2WPKH_DUST_LIMIT_SATS: u64 = 330;

#[derive(Debug, Clone)]
pub struct Utxo {
    pub outpoint: OutPoint,
    pub value_sat: u64,
}

#[derive(Debug, Clone)]
pub struct CreatedTransaction {
    pub transaction: Transaction,
    pub change: Option<Utxo>,
    pub fee_sat: u64,
    // Plan data for committing UTXO changes after successful broadcast
    pub spent_indices: Vec<usize>,
    pub spent_utxos: Vec<Utxo>,
    pub change_value: u64,
    pub change_address: Address,
}

#[derive(Debug, Clone)]
pub struct GeneratedAddress {
    pub address: Address,
    pub private_key_wif: String,
    pub public_key_hex: String,
}

pub struct Wallet {
    network: Network,
    mode: crate::cli::WalletMode,
    private_keys: HashMap<String, PrivateKey>,
    active_address: Option<Address>,
    utxos: Vec<Utxo>,
    secp: Secp256k1<secp256k1::All>,
    sats_per_byte: u64,
    db_root: PathBuf,
    utxo_store: UtxoStore,
    rpc_client: Option<Client>,
    pending_transactions: HashMap<Txid, CreatedTransaction>,
    pending_tx_store: PendingTransactionStore,
}

impl Wallet {
    pub fn new(db_root: impl Into<PathBuf>, mode: crate::cli::WalletMode) -> Result<Self> {
        Self::new_with_network(db_root, Network::Regtest, mode)
    }

    pub fn new_with_network(
        db_root: impl Into<PathBuf>,
        network: Network,
        mode: crate::cli::WalletMode,
    ) -> Result<Self> {
        let db_root = db_root.into();
        let utxo_store = open_network_store(&db_root, network, &mode)?;
        let pending_tx_store = open_pending_tx_store(&db_root, network, &mode)?;

        // load pending transactions from storage
        let pending_transactions: HashMap<Txid, CreatedTransaction> =
            pending_tx_store.load_all(network)?.into_iter().collect();

        Ok(Self {
            network,
            mode,
            private_keys: HashMap::new(),
            active_address: None,
            utxos: Vec::new(),
            secp: Secp256k1::new(),
            sats_per_byte: DEFAULT_SATS_PER_BYTE,
            db_root,
            utxo_store,
            rpc_client: None,
            pending_transactions,
            pending_tx_store,
        })
    }

    pub fn from_config(config: &Config) -> Result<Self> {
        let mut wallet = if let Some(network) = config.network {
            Wallet::new_with_network(config.db_path.clone(), network, config.mode.clone())?
        } else {
            Wallet::new(config.db_path.clone(), config.mode.clone())?
        };

        if let Some(sats_per_byte) = config.sats_per_byte {
            wallet.set_sats_per_byte(sats_per_byte);
        }

        if let Some(ref wif) = config.private_key_wif {
            let address = wallet.import_private_key(wif)?;
            wallet.active_address = Some(address.clone());
            wallet.reload_active_utxos()?;
            println!("Loaded private key. Default P2WPKH address: {address}");
        }

        if let Some(url) = config.rpc_url.as_deref() {
            wallet.configure_rpc(
                url,
                config.rpc_user.as_deref(),
                config.rpc_password.as_deref(),
            )?;
            println!("RPC client configured (URL: {url}).");
        } else if config.rpc_user.is_some() || config.rpc_password.is_some() {
            bail!("RPC URL must be provided when specifying credentials");
        }

        Ok(wallet)
    }

    pub fn configure_rpc(
        &mut self,
        url: &str,
        user: Option<&str>,
        pass: Option<&str>,
    ) -> Result<()> {
        let transport = match (user, pass) {
            (Some(user), Some(pass)) => {
                let transport = ReqwestHttpsTransport::builder()
                    .url(url)?
                    .basic_auth(user.to_owned(), Some(pass.to_string()))
                    .build();

                transport
            }
            (Some(user), None) => {
                let transport = ReqwestHttpsTransport::builder()
                    .url(url)?
                    .basic_auth(user.to_owned(), None)
                    .build();

                transport
            }
            (None, None) => ReqwestHttpsTransport::builder().url(url)?.build(),
            (None, Some(_)) => bail!("RPC password provided without username"),
        };

        // let transport = match user {
        //     Some(user) => ReqwestHttpsTransport::builder()
        //         .url(url)?
        //         .basic_auth(user.to_owned(), pass)
        //         .build(),
        //     _ => ReqwestHttpsTransport::builder().url(url)?.build(),
        // };

        let from_jsonrpc = jsonrpc::client::Client::with_transport(transport);
        let client = Client::from_jsonrpc(from_jsonrpc);

        match client.get_blockchain_info() {
            Ok(_) => {}
            Err(e) => {
                if self.network == Network::Regtest {
                    println!(
                        "Warning! You are running in regtest mode but no node was found at RPC URL: {}. \
            Please ensure a regtest node is running at this URL and port.",
                        url
                    );
                } else {
                    return Err(anyhow!("RPC connection check failed: {e}"));
                }
            }
        }

        self.rpc_client = Some(client);
        Ok(())
    }

    pub fn set_rpc_client(&mut self, client: Client) {
        self.rpc_client = Some(client);
    }

    pub fn clear_rpc_client(&mut self) {
        self.rpc_client = None;
    }

    pub fn rpc_client(&self) -> Option<&Client> {
        self.rpc_client.as_ref()
    }

    pub fn fetch_utxo_amount(
        &self,
        txid: Txid,
        block_hash: Option<&bitcoincore_rpc::bitcoin::BlockHash>,
        vout: u32,
    ) -> Result<u64> {
        let client = self.require_rpc_client()?;
        fetch_utxo_amount(client, txid, block_hash, vout)
    }

    pub fn broadcast_transaction(&mut self, created: &CreatedTransaction) -> Result<Txid> {
        let client = self.require_rpc_client()?;
        let raw_hex = serialize_hex(&created.transaction);
        let rpc_txid = client.send_raw_transaction(raw_hex)?;
        let txid =
            Txid::from_str(&rpc_txid.to_string()).context("failed to convert broadcast txid")?;

        // mark UTXOs as spent-unconfirmed and track the transaction
        if let Err(err) = self.commit_spend_pending(created) {
            eprintln!("  warning: failed to commit local UTXO changes: {err}");
        }

        match self.network {
            Network::Testnet => {
                let url = format!("https://mempool.space/testnet/tx/{}", txid);
                println!("View transaction at: {}", url);
            }
            Network::Bitcoin => {
                let url = format!("https://mempool.space/tx/{}", txid);
                println!("View transaction at: {}", url);
            }
            _ => { /* no-op */ }
        };

        Ok(txid)
    }

    // Apply UTXO mutations only after a successful broadcast (for backwards compatibility)
    pub fn commit_spend(&mut self, created: &CreatedTransaction) -> Result<Option<Utxo>> {
        self.update_utxos_after_spend(
            created.spent_indices.clone(),
            &created.spent_utxos,
            &created.transaction,
            created.change_value,
            &created.change_address,
        )
    }

    // mark UTXOs as pending and track the transaction (for RBF support)
    fn commit_spend_pending(&mut self, created: &CreatedTransaction) -> Result<()> {
        let txid = created.transaction.compute_txid();

        // mark spent UTXOs as spent-unconfirmed in the database
        for utxo in &created.spent_utxos {
            self.utxo_store.mark_spent_unconfirmed(&utxo.outpoint)?;
        }

        // remove from in-memory available list based on outpoints (not indices, which may be stale)
        let spent_outpoints: Vec<OutPoint> =
            created.spent_utxos.iter().map(|u| u.outpoint).collect();
        self.utxos
            .retain(|u| !spent_outpoints.contains(&u.outpoint));

        // add change output as available UTXO immediately (0-conf change is spendable)
        if created.change_value > 0 {
            let change_outpoint =
                OutPoint::new(txid, (created.transaction.output.len() - 1) as u32);
            self.utxo_store.insert(
                &change_outpoint,
                created.change_value,
                &created.change_address,
            )?;
            self.utxos.push(Utxo {
                outpoint: change_outpoint,
                value_sat: created.change_value,
            });
        }

        // track pending transaction in memory and persist to database
        self.pending_transactions.insert(txid, created.clone());
        self.pending_tx_store.save(&txid, created)?;

        Ok(())
    }

    /// Replace an unconfirmed transaction with a new one using the same inputs but higher fee
    /// The new transaction must pay at least the RBF minimum fee increase
    pub fn replace_transaction(
        &mut self,
        original_txid: Txid,
        new_sats_per_byte: u64,
    ) -> Result<CreatedTransaction> {
        // retrieve the original pending transaction
        let original = self
            .pending_transactions
            .get(&original_txid)
            .ok_or_else(|| anyhow!("transaction {} is not pending", original_txid))?
            .clone();

        if new_sats_per_byte <= self.sats_per_byte {
            bail!(
                "new fee rate ({} sats/byte) must be higher than current ({} sats/byte)",
                new_sats_per_byte,
                self.sats_per_byte
            );
        }

        // temporarily mark the original spent UTXOs as available again
        for utxo in &original.spent_utxos {
            self.utxo_store.mark_available(&utxo.outpoint)?;
        }

        // clear current UTXOs and use ONLY the original inputs for replacement
        let saved_utxos = self.utxos.clone();
        self.utxos = original.spent_utxos.clone();

        // temporarily increase fee rate
        let old_sats_per_byte = self.sats_per_byte;
        self.sats_per_byte = new_sats_per_byte;

        // reconstruct the outputs (without the change from original)
        let original_outputs: Vec<TxOut> = original
            .transaction
            .output
            .iter()
            .take(original.transaction.output.len() - if original.change_value > 0 { 1 } else { 0 })
            .cloned()
            .collect();

        // calculate total output amount (excluding change)
        let total_output_amount: u64 = original_outputs.iter().map(|out| out.value.to_sat()).sum();

        // create replacement transaction with higher fee
        let replacement =
            self.build_transaction_with_outputs(original_outputs, total_output_amount);

        // restore original fee rate
        self.sats_per_byte = old_sats_per_byte;

        let replacement = replacement?;

        // restore saved UTXOs and fee rate
        self.utxos = saved_utxos;

        // verify the replacement pays more fee
        if replacement.fee_sat <= original.fee_sat {
            // rollback: mark as spent-unconfirmed again
            for utxo in &original.spent_utxos {
                self.utxo_store.mark_spent_unconfirmed(&utxo.outpoint)?;
            }

            bail!(
                "replacement transaction fee ({} sats) must be higher than original ({} sats)",
                replacement.fee_sat,
                original.fee_sat
            );
        }

        // remove the old pending transaction from memory and storage
        self.pending_transactions.remove(&original_txid);
        self.pending_tx_store.remove(&original_txid)?;

        // remove the old change UTXO if it exists (it's now invalid)
        if original.change_value > 0 {
            let old_change_outpoint = OutPoint::new(
                original_txid,
                (original.transaction.output.len() - 1) as u32,
            );
            // remove from database
            let _ = self.utxo_store.remove(&old_change_outpoint); // ignore error if not found
            // remove from in-memory list
            self.utxos.retain(|u| u.outpoint != old_change_outpoint);
        }

        // the UTXOs are already marked as spent-unconfirmed from the rollback-attempt
        // but we need to ensure they stay that way for the new transaction
        Ok(replacement)
    }

    /// Confirm a transaction, permanently removing its spent UTXOs
    /// Call this after a transaction has been confirmed on-chain
    /// Note: change UTXO was already added when transaction was broadcast
    pub fn confirm_transaction(&mut self, txid: Txid) -> Result<()> {
        let created = self
            .pending_transactions
            .remove(&txid)
            .ok_or_else(|| anyhow!("transaction {} is not pending", txid))?;

        // remove from storage
        self.pending_tx_store.remove(&txid)?;

        // permanently delete spent UTXOs
        for utxo in &created.spent_utxos {
            self.utxo_store.remove(&utxo.outpoint)?;
        }

        // change UTXO was already added when transaction was broadcast, so nothing to do here

        Ok(())
    }

    /// Get list of pending (unconfirmed) transaction IDs
    pub fn pending_transaction_ids(&self) -> Vec<Txid> {
        self.pending_transactions.keys().copied().collect()
    }

    /// Get details of a pending transaction
    pub fn get_pending_transaction(&self, txid: &Txid) -> Option<&CreatedTransaction> {
        self.pending_transactions.get(txid)
    }

    pub fn clear_db(&mut self) -> Result<()> {
        self.utxo_store.clear()?;
        self.utxos.clear();
        Ok(())
    }

    fn reload_active_utxos(&mut self) -> Result<()> {
        if let Some(address) = &self.active_address {
            let entries = self.utxo_store.load_by_address(address)?;
            self.utxos = entries
                .into_iter()
                .filter(|(_, stored)| stored.state == UtxoState::Available)
                .map(|(outpoint, stored)| Utxo {
                    outpoint,
                    value_sat: stored.value_sat,
                })
                .collect();
        } else {
            self.utxos.clear();
        }
        Ok(())
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn sats_per_byte(&self) -> u64 {
        self.sats_per_byte
    }

    pub fn set_sats_per_byte(&mut self, sats: u64) {
        self.sats_per_byte = sats;
    }

    pub fn utxos(&self) -> &[Utxo] {
        &self.utxos
    }

    pub fn utxos_with_timestamps(&self) -> Result<Vec<(Utxo, u64)>> {
        let address = self.require_active_address()?;
        self.require_active_private_key()?;
        self.load_utxos_for_address(address)
    }

    pub fn utxos_with_timestamps_all(&self) -> Result<Vec<(Address, Vec<(Utxo, u64)>)>> {
        let mut grouped: BTreeMap<String, (Address, Vec<(Utxo, u64)>)> = BTreeMap::new();

        let parse_address = |addr: &str| -> Result<Address> {
            let raw: Address<NetworkUnchecked> = Address::from_str(addr)
                .map_err(|_| anyhow!("stored address has invalid format: {addr}"))?;
            raw.require_network(self.network).map_err(|_| {
                anyhow!(
                    "stored address network mismatch for {addr} (wallet network {:?})",
                    self.network
                )
            })
        };

        for addr in self.private_keys.keys() {
            let address = parse_address(addr)?;
            grouped
                .entry(address.to_string())
                .or_insert_with(|| (address, Vec::new()));
        }

        for (outpoint, stored) in self.utxo_store.load_all()? {
            let addr_str = match stored.address {
                Some(ref addr) => addr,
                None => continue,
            };
            let address = parse_address(addr_str)?;
            let entry = grouped
                .entry(address.to_string())
                .or_insert_with(|| (address.clone(), Vec::new()));
            entry.1.push((
                Utxo {
                    outpoint,
                    value_sat: stored.value_sat,
                },
                stored.timestamp,
            ));
        }

        for (_, entry) in grouped.iter_mut() {
            entry
                .1
                .sort_by_key(|(_, timestamp)| std::cmp::Reverse(*timestamp));
        }

        let mut collected: Vec<(Address, Vec<(Utxo, u64)>)> =
            grouped.into_iter().map(|(_, value)| value).collect();

        if let Some(active) = &self.active_address {
            if let Some(pos) = collected.iter().position(|(addr, _)| addr == active) {
                let active_entry = collected.remove(pos);
                collected.insert(0, active_entry);
            }
        }

        Ok(collected)
    }

    pub fn set_network(&mut self, network: Network) -> Result<bool> {
        if self.network == network {
            return Ok(false);
        }

        let new_store = open_network_store(&self.db_root, network, &self.mode)?;
        let new_pending_tx_store = open_pending_tx_store(&self.db_root, network, &self.mode)?;

        // load pending transactions for the new network
        let pending_transactions: HashMap<Txid, CreatedTransaction> = new_pending_tx_store
            .load_all(network)?
            .into_iter()
            .collect();

        self.network = network;
        self.utxo_store = new_store;
        self.pending_tx_store = new_pending_tx_store;
        self.pending_transactions = pending_transactions;
        self.utxos.clear();
        self.private_keys.clear();
        self.active_address = None;
        Ok(true)
    }

    pub fn import_private_key(&mut self, wif: &str) -> Result<Address> {
        let private_key = PrivateKey::from_wif(wif).context("failed to parse WIF private key")?;
        let target_kind = NetworkKind::from(self.network);
        if private_key.network != target_kind {
            bail!(
                "private key network ({:?}) does not match current wallet network ({:?}).",
                private_key.network,
                self.network
            );
        }

        let public_key = private_key.public_key(&self.secp);
        let compressed = CompressedPublicKey::try_from(public_key)
            .map_err(|_| anyhow!("private key must correspond to a compressed public key"))?;
        let address = bitcoin::address::Address::p2wpkh(&compressed, self.network);
        self.private_keys.insert(address.to_string(), private_key);
        self.active_address = Some(address.clone());
        self.reload_active_utxos()?;
        Ok(address)
    }

    pub fn register_utxo(&mut self, outpoint: OutPoint, amount: u64) -> Result<()> {
        let address = self.require_active_address()?;
        self.require_active_private_key()?;

        if self.utxos.iter().any(|u| u.outpoint == outpoint) {
            bail!("UTXO {outpoint} already registered");
        }

        if self.utxo_store.contains(&outpoint)? {
            bail!("UTXO {outpoint} already exists in the database");
        }

        self.utxo_store.insert(&outpoint, amount, address)?;
        self.utxos.push(Utxo {
            outpoint,
            value_sat: amount,
        });
        Ok(())
    }

    pub fn generate_address(&mut self) -> Result<GeneratedAddress> {
        let secret_key = SecretKey::new(&mut OsRng);
        let private_key = PrivateKey::new(secret_key, self.network);
        let public_key = private_key.public_key(&self.secp);
        let compressed = CompressedPublicKey::try_from(public_key)
            .map_err(|_| anyhow!("generated private key must produce a compressed public key"))?;
        let address = bitcoin::address::Address::p2wpkh(&compressed, self.network);

        let previous_active = self.active_address.clone();
        let wif = private_key.to_wif();
        self.private_keys.insert(address.to_string(), private_key);
        self.active_address = match previous_active {
            Some(active) => Some(active),
            None => Some(address.clone()),
        };
        self.reload_active_utxos()?;

        Ok(GeneratedAddress {
            address,
            private_key_wif: wif,
            public_key_hex: public_key.to_string(),
        })
    }

    /// Create one or more transactions that pay the same amount to each provided target script in a single transaction.
    /// If `count` > 1, the whole multi-output payment is repeated `count` times (i.e., `count` separate transactions).
    pub fn create_transactions(
        &mut self,
        target_scripts: Vec<ScriptBuf>,
        amount_sat: u64,
        count: usize,
    ) -> Result<Vec<CreatedTransaction>> {
        if count == 0 {
            bail!("count must be at least 1");
        }
        if target_scripts.is_empty() {
            bail!("at least one target script is required");
        }
        self.require_active_private_key()?;

        let mut created = Vec::with_capacity(count);
        for _ in 0..count {
            let tx = self.create_transaction_once(target_scripts.clone(), amount_sat)?;
            created.push(tx);
        }
        Ok(created)
    }

    pub fn active_address(&self) -> Option<&Address> {
        self.active_address.as_ref()
    }

    pub fn imported_addresses(&self) -> Vec<String> {
        let mut addresses: Vec<String> = self.private_keys.keys().cloned().collect();
        addresses.sort();
        addresses
    }

    pub fn switch_active_address(&mut self, address: Address) -> Result<()> {
        let address_str = address.to_string();
        if !self.private_keys.contains_key(&address_str) {
            bail!("address {address} is not managed by this wallet");
        }

        self.active_address = Some(address);
        self.reload_active_utxos()?;
        Ok(())
    }

    pub fn private_key(&self) -> Option<&PrivateKey> {
        self.active_address
            .as_ref()
            .and_then(|addr| self.private_keys.get(&addr.to_string()))
    }

    fn create_transaction_once(
        &mut self,
        target_scripts: Vec<ScriptBuf>,
        amount_sat: u64,
    ) -> Result<CreatedTransaction> {
        if target_scripts.is_empty() {
            bail!("at least one target script is required");
        }
        let outputs_count = target_scripts.len() as u64;
        let total_amount = amount_sat
            .checked_mul(outputs_count)
            .context("total amount overflowed")?;

        let outputs: Vec<TxOut> = target_scripts
            .iter()
            .map(|script| TxOut {
                value: Amount::from_sat(amount_sat),
                script_pubkey: script.clone(),
            })
            .collect();

        self.build_transaction_with_outputs(outputs, total_amount)
    }

    fn build_transaction_with_outputs(
        &mut self,
        outputs: Vec<TxOut>,
        total_output_amount: u64,
    ) -> Result<CreatedTransaction> {
        let private_key = self.require_active_private_key()?;
        let address = self.require_active_address()?.clone();
        let pubkey = private_key.public_key(&self.secp);
        let wpkh = pubkey
            .wpubkey_hash()
            .map_err(|_| anyhow!("private key must correspond to a compressed public key"))?;
        let change_script = ScriptBuf::new_p2wpkh(&wpkh);

        let mut required = total_output_amount;

        'select: loop {
            let (selected_indices, selected_utxos, total_input) = self.select_utxos(required)?;

            let mut include_change =
                total_input.saturating_sub(total_output_amount) >= P2WPKH_DUST_LIMIT_SATS;
            let mut change_value = total_input.saturating_sub(total_output_amount);
            let result;

            'build: loop {
                let mut tx = Transaction {
                    version: Version::TWO,
                    lock_time: absolute::LockTime::ZERO,
                    input: selected_utxos
                        .iter()
                        .map(|u| TxIn {
                            previous_output: u.outpoint,
                            script_sig: ScriptBuf::new(),
                            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                            witness: Witness::default(),
                        })
                        .collect(),
                    output: outputs.clone(),
                };

                if include_change {
                    tx.output.push(TxOut {
                        value: Amount::from_sat(change_value),
                        script_pubkey: change_script.clone(),
                    });
                }

                let signed = self.sign_transaction(tx, &selected_utxos, &pubkey, &change_script)?;
                let vsize = signed.vsize() as u64;
                let fee = vsize
                    .checked_mul(self.sats_per_byte)
                    .context("fee computation overflowed")?;
                let total_required = total_output_amount
                    .checked_add(fee)
                    .context("amount plus fee overflowed")?;

                if total_input < total_required {
                    required = total_required;
                    continue 'select;
                }

                let new_change_value = total_input - total_output_amount - fee;
                let should_include_change = new_change_value >= P2WPKH_DUST_LIMIT_SATS;

                if should_include_change != include_change {
                    include_change = should_include_change;
                    change_value = new_change_value;
                    continue 'build;
                }

                if include_change && new_change_value != change_value {
                    change_value = new_change_value;
                    continue 'build;
                }

                if !include_change {
                    change_value = 0;
                }

                let effective_fee = total_input
                    .checked_sub(total_output_amount)
                    .and_then(|value| value.checked_sub(change_value))
                    .context("fee computation underflow")?;

                result = (signed, effective_fee, change_value);
                break 'build;
            }

            let (signed_tx, fee_sat, final_change) = result;

            let change_preview = if final_change > 0 {
                let txid = signed_tx.compute_txid();
                let change_vout = (signed_tx.output.len() - 1) as u32; // change is last output
                Some(Utxo {
                    outpoint: OutPoint::new(txid, change_vout),
                    value_sat: final_change,
                })
            } else {
                None
            };

            return Ok(CreatedTransaction {
                transaction: signed_tx,
                change: change_preview,
                fee_sat,
                spent_indices: selected_indices,
                spent_utxos: selected_utxos,
                change_value: final_change,
                change_address: address,
            });
        }
    }

    fn sign_transaction(
        &self,
        tx: Transaction,
        spent_utxos: &[Utxo],
        pubkey: &PublicKey,
        script_code: &ScriptBuf,
    ) -> Result<Transaction> {
        let private_key = self.require_active_private_key()?;

        let sighash_type = EcdsaSighashType::All;
        let mut cache = SighashCache::new(tx);

        for (index, utxo) in spent_utxos.iter().enumerate() {
            let sighash = cache.p2wpkh_signature_hash(
                index,
                script_code,
                Amount::from_sat(utxo.value_sat),
                sighash_type,
            )?;

            let msg = Message::from(sighash);
            let sig = self.secp.sign_ecdsa(&msg, &private_key.inner);
            let signature = ecdsa::Signature {
                signature: sig,
                sighash_type,
            };
            *cache
                .witness_mut(index)
                .expect("witness for existing input") = Witness::p2wpkh(&signature, &pubkey.inner);
        }

        Ok(cache.into_transaction())
    }

    // Simple, unoptimized greedy algorithm to select UTXOs until the required amount is met or exceeded.
    // If needed a better coin selection algorithm can be implemented here.
    fn select_utxos(&self, required: u64) -> Result<(Vec<usize>, Vec<Utxo>, u64)> {
        let mut indices = Vec::new();
        let mut selected = Vec::new();
        let mut total = 0u64;

        for (index, utxo) in self.utxos.iter().enumerate() {
            indices.push(index);
            selected.push(utxo.clone());
            total = total
                .checked_add(utxo.value_sat)
                .context("overflow selecting UTXOs")?;
            if total >= required {
                break;
            }
        }

        if total < required {
            bail!(
                "insufficient funds: selected {} sat but need at least {} sat (including fee)",
                total,
                required
            );
        }

        Ok((indices, selected, total))
    }

    fn update_utxos_after_spend(
        &mut self,
        mut spent_indices: Vec<usize>,
        spent_utxos: &[Utxo],
        signed_tx: &Transaction,
        change_value: u64,
        address: &Address,
    ) -> Result<Option<Utxo>> {
        for utxo in spent_utxos {
            self.utxo_store.remove(&utxo.outpoint)?;
        }

        spent_indices.sort_unstable_by(|a, b| b.cmp(a));
        for index in spent_indices {
            self.utxos.swap_remove(index);
        }

        if change_value > 0 {
            let txid = signed_tx.compute_txid();
            let change_vout = (signed_tx.output.len() - 1) as u32;
            let change_outpoint = OutPoint::new(txid, change_vout);
            self.utxo_store
                .insert(&change_outpoint, change_value, address)?;
            let change_utxo = Utxo {
                outpoint: change_outpoint,
                value_sat: change_value,
            };
            self.utxos.push(change_utxo.clone());
            Ok(Some(change_utxo))
        } else {
            Ok(None)
        }
    }

    fn require_active_address(&self) -> Result<&Address> {
        self.active_address
            .as_ref()
            .ok_or_else(|| anyhow!("import a private key first"))
    }

    fn require_active_private_key(&self) -> Result<&PrivateKey> {
        let address = self.require_active_address()?;
        let address_str = address.to_string();
        match self.private_keys.get(&address_str) {
            Some(pk) => Ok(pk),
            None => Err(anyhow!("no private key loaded for {address_str}")),
        }
    }

    fn require_rpc_client(&self) -> Result<&Client> {
        self.rpc_client
            .as_ref()
            .context("RPC client required but not configured")
    }

    fn load_utxos_for_address(&self, address: &Address) -> Result<Vec<(Utxo, u64)>> {
        let entries = self.utxo_store.load_by_address(address)?;
        let mut utxos: Vec<(Utxo, u64)> = entries
            .into_iter()
            .filter(|(_, stored)| stored.state == UtxoState::Available)
            .map(|(outpoint, stored)| {
                (
                    Utxo {
                        outpoint,
                        value_sat: stored.value_sat,
                    },
                    stored.timestamp,
                )
            })
            .collect();
        utxos.sort_by_key(|(_, timestamp)| std::cmp::Reverse(*timestamp));
        Ok(utxos)
    }

    pub fn create_pegin_transaction(
        &mut self,
        stream_value: u64,
        packet_number: u64,
        tmp_addr: String,
        rsk_address_hex: String,
    ) -> Result<CreatedTransaction> {
        let private_key = self.require_active_private_key()?;
        let pubkey = private_key.public_key(&self.secp);

        // Parse the destination address
        let dest_addr: Address<NetworkUnchecked> =
            Address::from_str(&tmp_addr).context("invalid destination address")?;
        let checked_addr = dest_addr
            .require_network(self.network)
            .context("destination address network mismatch")?;

        // Parse RSK address (20 bytes)
        let rsk_address_clean = rsk_address_hex.trim_start_matches("0x");
        let rsk_address_bytes =
            Vec::<u8>::from_hex(rsk_address_clean).context("invalid RSK address hex")?;
        if rsk_address_bytes.len() != 20 {
            bail!(
                "RSK address must be 20 bytes, got {}",
                rsk_address_bytes.len()
            );
        }
        let mut rsk_address = [0u8; 20];
        rsk_address.copy_from_slice(&rsk_address_bytes);

        // Derive the reimbursement X-only public key from wallet's own key
        let (reimbursement_xpk, _) = pubkey.inner.x_only_public_key();

        // Create OP_RETURN data
        let op_return_data =
            Self::create_pegin_op_return_data(packet_number, rsk_address, reimbursement_xpk)?;

        // Build outputs for pegin transaction
        let outputs = vec![
            // Taproot output
            TxOut {
                value: Amount::from_sat(stream_value),
                script_pubkey: checked_addr.script_pubkey(),
            },
            // OP_RETURN output
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: Self::create_op_return_script(op_return_data)?,
            },
        ];

        self.build_transaction_with_outputs(outputs, stream_value)
    }

    fn create_pegin_op_return_data(
        packet_number: u64,
        rsk_address: [u8; 20],
        reimbursement_xpk: XOnlyPublicKey,
    ) -> Result<Vec<u8>> {
        let mut data = Vec::with_capacity(69);
        data.extend_from_slice(b"RSK_PEGIN");
        data.extend_from_slice(&packet_number.to_be_bytes());
        data.extend_from_slice(&rsk_address);
        data.extend_from_slice(&reimbursement_xpk.serialize());
        Ok(data)
    }

    fn create_op_return_script(data: Vec<u8>) -> Result<ScriptBuf> {
        let push_bytes = PushBytesBuf::try_from(data).context("OP_RETURN data too large")?;
        let script = ScriptBuilder::new()
            .push_opcode(OP_RETURN)
            .push_slice(push_bytes)
            .into_script();
        Ok(script)
    }
}

fn open_network_store(
    root: &Path,
    network: Network,
    mode: &crate::cli::WalletMode,
) -> Result<UtxoStore> {
    fs::create_dir_all(root).with_context(|| {
        format!(
            "failed to create UTXO database root directory {}",
            root.display()
        )
    })?;

    let path = utxo_db_path(root, network, mode);
    fs::create_dir_all(&path).with_context(|| {
        format!(
            "failed to create UTXO database directory {}",
            path.display()
        )
    })?;

    println!("Opening UTXO database at {} ", path.display());

    UtxoStore::open(&path)
}

fn open_pending_tx_store(
    root: &Path,
    network: Network,
    mode: &crate::cli::WalletMode,
) -> Result<PendingTransactionStore> {
    fs::create_dir_all(root).with_context(|| {
        format!(
            "failed to create pending transaction database root directory {}",
            root.display()
        )
    })?;

    let path = pending_tx_db_path(root, network, mode);
    fs::create_dir_all(&path).with_context(|| {
        format!(
            "failed to create pending transaction database directory {}",
            path.display()
        )
    })?;

    PendingTransactionStore::open(&path)
}

fn utxo_db_path(root: &Path, network: Network, mode: &crate::cli::WalletMode) -> PathBuf {
    let mode_name = mode.to_string();
    let network_name = network_suffix(network);
    root.join(mode_name).join(network_name).join("utxo_db")
}

fn pending_tx_db_path(root: &Path, network: Network, mode: &crate::cli::WalletMode) -> PathBuf {
    let mode_name = mode.to_string();
    let network_name = network_suffix(network);
    root.join(mode_name)
        .join(network_name)
        .join("pending_tx_db")
}

pub fn network_suffix(network: Network) -> &'static str {
    match network {
        Network::Bitcoin => "bitcoin",
        Network::Testnet => "testnet",
        Network::Testnet4 => "testnet4",
        Network::Signet => "signet",
        Network::Regtest => "regtest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generate_address_sets_new_active_address() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path().join("utxo-db");
        let mut wallet = Wallet::new(root, crate::cli::WalletMode::User)?;
        assert!(wallet.active_address().is_none());

        let generated = wallet.generate_address()?;

        assert_eq!(
            wallet.active_address().map(|addr| addr.to_string()),
            Some(generated.address.to_string())
        );

        let private_key = wallet
            .private_key()
            .expect("generated key should be available");
        assert_eq!(private_key.to_wif(), generated.private_key_wif);
        assert_eq!(
            private_key.public_key(&wallet.secp).to_string(),
            generated.public_key_hex
        );

        Ok(())
    }

    #[test]
    fn generate_address_twice_yields_unique_keys() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path().join("utxo-db");
        let mut wallet = Wallet::new(root, crate::cli::WalletMode::User)?;

        let first = wallet.generate_address()?;
        let second = wallet.generate_address()?;

        assert_ne!(first.address, second.address);
        assert_ne!(first.private_key_wif, second.private_key_wif);

        let addresses = wallet.imported_addresses();
        assert!(addresses.contains(&first.address.to_string()));
        assert!(addresses.contains(&second.address.to_string()));
        assert_eq!(addresses.len(), 2);

        assert_eq!(
            wallet.active_address().map(|addr| addr.to_string()),
            Some(first.address.to_string())
        );

        Ok(())
    }

    #[test]
    fn generate_address_preserves_existing_active() -> Result<()> {
        let temp = tempdir()?;
        let root = temp.path().join("utxo-db");
        let mut wallet = Wallet::new(root, crate::cli::WalletMode::User)?;

        let secret = secp256k1::SecretKey::from_slice(&[9u8; 32]).expect("secret");
        let imported = PrivateKey::new(secret, Network::Regtest);
        let imported_address = wallet.import_private_key(&imported.to_wif())?;

        let generated = wallet.generate_address()?;

        assert_ne!(imported_address, generated.address);
        assert_eq!(
            wallet.active_address().map(|addr| addr.to_string()),
            Some(imported_address.to_string())
        );
        assert!(
            wallet
                .imported_addresses()
                .contains(&generated.address.to_string())
        );

        Ok(())
    }

    #[test]
    fn rbf_tracking_and_replacement() -> Result<()> {
        use bitcoin::hashes::Hash;

        let temp = tempdir()?;
        let root = temp.path().join("utxo-db");
        let mut wallet = Wallet::new(root, crate::cli::WalletMode::User)?;

        // import a key
        let secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("secret");
        let private_key = PrivateKey::new(secret, Network::Regtest);
        wallet.import_private_key(&private_key.to_wif())?;

        // register a UTXO
        let txid = Txid::from_byte_array([2u8; 32]);
        let outpoint = OutPoint::new(txid, 0);
        wallet.register_utxo(outpoint, 100_000)?;

        // create a transaction
        let dest_addr = wallet.active_address().unwrap().clone();
        let target_scripts = vec![dest_addr.script_pubkey()];
        let mut txs = wallet.create_transactions(target_scripts, 10_000, 1)?;
        let created = txs.remove(0);

        // simulate broadcast by calling commit_spend_pending directly
        wallet.commit_spend_pending(&created)?;
        let tx_id = created.transaction.compute_txid();

        // verify the transaction is tracked as pending
        assert_eq!(wallet.pending_transaction_ids().len(), 1);
        assert!(wallet.get_pending_transaction(&tx_id).is_some());

        // verify change was added immediately (0-conf change is spendable)
        let expected_utxos = if created.change_value > 0 { 1 } else { 0 };
        assert_eq!(wallet.utxos().len(), expected_utxos);

        // verify original UTXO is marked as spent-unconfirmed, change is available
        let stored_entries = wallet.utxo_store.load_all()?;
        let expected_stored = if created.change_value > 0 { 2 } else { 1 };
        assert_eq!(stored_entries.len(), expected_stored);

        // find the spent-unconfirmed entry
        let spent_entry = stored_entries
            .iter()
            .find(|(_, stored)| stored.state == crate::utxo_store::UtxoState::SpentUnconfirmed);
        assert!(spent_entry.is_some());

        // replace the transaction with higher fee
        let original_fee = created.fee_sat;
        // replace with 10 sats/byte (current is 5)
        let replacement = wallet.replace_transaction(tx_id, 10)?;

        // verify replacement has higher fee
        assert!(replacement.fee_sat > original_fee);

        // verify original transaction is no longer pending
        assert!(wallet.get_pending_transaction(&tx_id).is_none());

        // verify replacement is using the same inputs
        assert_eq!(replacement.spent_utxos.len(), created.spent_utxos.len());
        assert_eq!(
            replacement.spent_utxos[0].outpoint,
            created.spent_utxos[0].outpoint
        );

        // simulate broadcasting the replacement
        wallet.commit_spend_pending(&replacement)?;
        let replacement_txid = replacement.transaction.compute_txid();

        // verify replacement is now tracked
        assert_eq!(wallet.pending_transaction_ids().len(), 1);
        assert!(wallet.get_pending_transaction(&replacement_txid).is_some());

        // verify change was added immediately after broadcast
        assert_eq!(
            wallet.utxos().len(),
            if replacement.change_value > 0 { 1 } else { 0 }
        );

        // confirm the replacement transaction
        wallet.confirm_transaction(replacement_txid)?;

        // verify no pending transactions remain
        assert_eq!(wallet.pending_transaction_ids().len(), 0);

        // verify spent UTXOs were permanently removed, change still available
        let stored_entries = wallet.utxo_store.load_all()?;
        if replacement.change_value > 0 {
            // only change UTXO should remain
            assert_eq!(stored_entries.len(), 1);
            assert_eq!(
                stored_entries[0].1.state,
                crate::utxo_store::UtxoState::Available
            );
            assert_eq!(stored_entries[0].1.value_sat, replacement.change_value);
        } else {
            assert_eq!(stored_entries.len(), 0);
        }

        Ok(())
    }

    #[test]
    fn pending_tx_persistence_across_restart() -> Result<()> {
        use bitcoin::hashes::Hash;

        let temp = tempdir()?;
        let root = temp.path().join("utxo-db");

        // create wallet and transaction
        let txid = {
            let mut wallet = Wallet::new(root.clone(), crate::cli::WalletMode::User)?;

            // import a key
            let secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("secret");
            let private_key = PrivateKey::new(secret, Network::Regtest);
            wallet.import_private_key(&private_key.to_wif())?;

            // register a UTXO
            let input_txid = Txid::from_byte_array([2u8; 32]);
            let outpoint = OutPoint::new(input_txid, 0);
            wallet.register_utxo(outpoint, 100_000)?;

            // create a transaction
            let dest_addr = wallet.active_address().unwrap().clone();
            let target_scripts = vec![dest_addr.script_pubkey()];
            let mut txs = wallet.create_transactions(target_scripts, 10_000, 1)?;
            let created = txs.remove(0);

            // simulate broadcast
            wallet.commit_spend_pending(&created)?;
            let tx_id = created.transaction.compute_txid();

            // verify pending transaction is tracked
            assert_eq!(wallet.pending_transaction_ids().len(), 1);
            assert!(wallet.get_pending_transaction(&tx_id).is_some());

            tx_id
        }; // wallet dropped here

        // create new wallet instance (simulating restart)
        {
            let secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("secret");
            let private_key = PrivateKey::new(secret, Network::Regtest);
            let mut wallet = Wallet::new(root.clone(), crate::cli::WalletMode::User)?;
            wallet.import_private_key(&private_key.to_wif())?;

            // verify pending transaction was loaded from storage
            assert_eq!(wallet.pending_transaction_ids().len(), 1);
            let restored_tx = wallet.get_pending_transaction(&txid);
            assert!(restored_tx.is_some());

            // verify transaction details match
            let restored = restored_tx.unwrap();
            assert_eq!(restored.transaction.compute_txid(), txid);
            assert_eq!(restored.spent_utxos.len(), 1);
            let original_fee = restored.fee_sat;

            // verify original UTXO is still marked as spent-unconfirmed, plus change UTXO
            let stored_entries = wallet.utxo_store.load_all()?;
            assert_eq!(stored_entries.len(), 2); // spent input + change

            // one should be spent-unconfirmed, one should be available (change)
            let spent_count = stored_entries
                .iter()
                .filter(|(_, s)| s.state == crate::utxo_store::UtxoState::SpentUnconfirmed)
                .count();
            let available_count = stored_entries
                .iter()
                .filter(|(_, s)| s.state == crate::utxo_store::UtxoState::Available)
                .count();
            assert_eq!(spent_count, 1);
            assert_eq!(available_count, 1);

            // can replace the transaction after restart
            let replacement = wallet.replace_transaction(txid, 10)?;
            assert!(replacement.fee_sat > original_fee);

            // save replacement
            wallet.commit_spend_pending(&replacement)?;
            let replacement_txid = replacement.transaction.compute_txid();

            // original should be gone
            assert!(wallet.get_pending_transaction(&txid).is_none());
            assert!(wallet.get_pending_transaction(&replacement_txid).is_some());
        } // wallet dropped again

        // verify replacement persisted
        {
            let secret = secp256k1::SecretKey::from_slice(&[1u8; 32]).expect("secret");
            let private_key = PrivateKey::new(secret, Network::Regtest);
            let mut wallet = Wallet::new(root, crate::cli::WalletMode::User)?;
            wallet.import_private_key(&private_key.to_wif())?;

            // original tx should not exist
            assert!(wallet.get_pending_transaction(&txid).is_none());

            // only replacement should exist
            assert_eq!(wallet.pending_transaction_ids().len(), 1);
            let pending_ids = wallet.pending_transaction_ids();
            assert_ne!(pending_ids[0], txid);
        }

        Ok(())
    }
}
