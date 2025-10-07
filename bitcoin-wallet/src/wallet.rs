use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::bitcoin::reqwest_https::ReqwestHttpsTransport;
use crate::bitcoin::utils::fetch_utxo_amount;
use crate::config::Config;
use crate::utxo_store::UtxoStore;
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
use bitcoin::{Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey};
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
    private_keys: HashMap<String, PrivateKey>,
    active_address: Option<Address>,
    utxos: Vec<Utxo>,
    secp: Secp256k1<secp256k1::All>,
    sats_per_byte: u64,
    utxo_db_root: PathBuf,
    utxo_store: UtxoStore,
    rpc_client: Option<Client>,
}

impl Wallet {
    pub fn new(utxo_db_root: impl Into<PathBuf>) -> Result<Self> {
        Self::new_with_network(utxo_db_root, Network::Regtest)
    }

    pub fn new_with_network(utxo_db_root: impl Into<PathBuf>, network: Network) -> Result<Self> {
        let utxo_db_root = utxo_db_root.into();
        let utxo_store = open_network_store(&utxo_db_root, network)?;

        Ok(Self {
            network,
            private_keys: HashMap::new(),
            active_address: None,
            utxos: Vec::new(),
            secp: Secp256k1::new(),
            sats_per_byte: DEFAULT_SATS_PER_BYTE,
            utxo_db_root,
            utxo_store,
            rpc_client: None,
        })
    }

    pub fn from_config(config: &Config) -> Result<Self> {
        let mut wallet = if let Some(network) = config.network {
            Wallet::new_with_network(config.utxo_db_path.clone(), network)?
        } else {
            Wallet::new(config.utxo_db_path.clone())?
        };

        if let Some(sats_per_byte) = config.sats_per_byte {
            wallet.set_sats_per_byte(sats_per_byte);
        }

        if let Some(wif) = config.private_key_wif.as_deref() {
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

    pub fn fetch_utxo_amount(&self, txid: Txid, vout: u32) -> Result<u64> {
        let client = self.require_rpc_client()?;
        fetch_utxo_amount(client, txid, vout)
    }

    pub fn broadcast_transaction(&mut self, created: &CreatedTransaction) -> Result<Txid> {
        let client = self.require_rpc_client()?;
        let raw_hex = serialize_hex(&created.transaction);
        let rpc_txid = client.send_raw_transaction(raw_hex)?;
        let txid =
            Txid::from_str(&rpc_txid.to_string()).context("failed to convert broadcast txid")?;
        if let Err(err) = self.commit_spend(created) {
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

    // Apply UTXO mutations only after a successful broadcast
    pub fn commit_spend(&mut self, created: &CreatedTransaction) -> Result<Option<Utxo>> {
        self.update_utxos_after_spend(
            created.spent_indices.clone(),
            &created.spent_utxos,
            &created.transaction,
            created.change_value,
            &created.change_address,
        )
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

        let new_store = open_network_store(&self.utxo_db_root, network)?;

        self.network = network;
        self.utxo_store = new_store;
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
        let private_key = self.require_active_private_key()?;
        let address = self.require_active_address()?.clone();
        let pubkey = private_key.public_key(&self.secp);
        let wpkh = pubkey
            .wpubkey_hash()
            .map_err(|_| anyhow!("private key must correspond to a compressed public key"))?;
        let change_script = ScriptBuf::new_p2wpkh(&wpkh);

        if target_scripts.is_empty() {
            bail!("at least one target script is required");
        }
        let outputs_count = target_scripts.len() as u64;
        let total_amount = amount_sat
            .checked_mul(outputs_count)
            .context("total amount overflowed")?;
        let mut required = total_amount;

        'select: loop {
            let (selected_indices, selected_utxos, total_input) = self.select_utxos(required)?;

            let mut include_change =
                total_input.saturating_sub(total_amount) >= P2WPKH_DUST_LIMIT_SATS;
            let mut change_value = total_input.saturating_sub(total_amount);
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
                    output: target_scripts
                        .iter()
                        .map(|script| TxOut {
                            value: Amount::from_sat(amount_sat),
                            script_pubkey: script.clone(),
                        })
                        .collect(),
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
                let total_required = total_amount
                    .checked_add(fee)
                    .context("amount plus fee overflowed")?;

                if total_input < total_required {
                    required = total_required;
                    continue 'select;
                }

                let new_change_value = total_input - total_amount - fee;
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
                    .checked_sub(total_amount)
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
        reimbursement_pubkey_hex: String,
    ) -> Result<CreatedTransaction> {
        // Constants matching user-api
        const KEY_SPEND_FEE: u64 = 335;
        const OP_RETURN_FEE: u64 = 300;
        const EXTRA_FEE: u64 = 1000;

        let private_key = self.require_active_private_key()?;
        let address = self.require_active_address()?.clone();
        let pubkey = private_key.public_key(&self.secp);

        // Parse the destination address
        let dest_addr: Address<NetworkUnchecked> =
            Address::from_str(&tmp_addr).context("invalid destination address")?;
        let checked_addr = dest_addr
            .require_network(self.network)
            .context("destination address network mismatch")?;

        // Parse RSK address (20 bytes)
        let rsk_address_clean = rsk_address_hex.trim_start_matches("0x");
        let rsk_address_bytes = Vec::<u8>::from_hex(rsk_address_clean)
            .context("invalid RSK address hex")?;
        if rsk_address_bytes.len() != 20 {
            bail!("RSK address must be 20 bytes, got {}", rsk_address_bytes.len());
        }
        let mut rsk_address = [0u8; 20];
        rsk_address.copy_from_slice(&rsk_address_bytes);

        // Parse the provided reimbursement public key
        let reimbursement_pubkey_clean = reimbursement_pubkey_hex.trim_start_matches("0x");
        let reimbursement_xpk = XOnlyPublicKey::from_str(reimbursement_pubkey_clean)
            .context("invalid reimbursement public key hex")?;

        // Calculate total amount needed
        let fee = KEY_SPEND_FEE + EXTRA_FEE;
        let total_amount = stream_value + fee + OP_RETURN_FEE;

        // Select UTXOs
        let (selected_indices, selected_utxos, total_input) = self.select_utxos(total_amount)?;

        // Create OP_RETURN data
        let op_return_data = Self::create_pegin_op_return_data(
            packet_number,
            rsk_address,
            reimbursement_xpk,
        )?;

        // Build transaction
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
            output: vec![
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
            ],
        };

        // Add change output if necessary
        let change_value = total_input.saturating_sub(total_amount);
        if change_value >= P2WPKH_DUST_LIMIT_SATS {
            let wpkh = pubkey
                .wpubkey_hash()
                .context("key must be compressed")?;
            let change_script = ScriptBuf::new_p2wpkh(&wpkh);
            tx.output.push(TxOut {
                value: Amount::from_sat(change_value),
                script_pubkey: change_script.clone(),
            });
        }

        // Sign the transaction
        let wpkh = pubkey
            .wpubkey_hash()
            .context("key must be compressed")?;
        let script_code = ScriptBuf::new_p2wpkh(&wpkh);
        let signed = self.sign_transaction(tx, &selected_utxos, &pubkey, &script_code)?;

        // Calculate actual fee
        let final_change = if change_value >= P2WPKH_DUST_LIMIT_SATS {
            change_value
        } else {
            0
        };
        let fee_sat = total_input.saturating_sub(stream_value).saturating_sub(final_change);

        // Create change UTXO info if applicable
        let change_preview = if final_change > 0 {
            let txid = signed.compute_txid();
            let change_vout = (signed.output.len() - 1) as u32;
            Some(Utxo {
                outpoint: OutPoint::new(txid, change_vout),
                value_sat: final_change,
            })
        } else {
            None
        };

        Ok(CreatedTransaction {
            transaction: signed,
            change: change_preview,
            fee_sat,
            spent_indices: selected_indices,
            spent_utxos: selected_utxos,
            change_value: final_change,
            change_address: address,
        })
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
        let push_bytes = PushBytesBuf::try_from(data)
            .context("OP_RETURN data too large")?;
        let script = ScriptBuilder::new()
            .push_opcode(OP_RETURN)
            .push_slice(push_bytes)
            .into_script();
        Ok(script)
    }
}

fn open_network_store(root: &Path, network: Network) -> Result<UtxoStore> {
    fs::create_dir_all(root).with_context(|| {
        format!(
            "failed to create UTXO database root directory {}",
            root.display()
        )
    })?;

    let path = utxo_db_path(root, network);
    fs::create_dir_all(&path).with_context(|| {
        format!(
            "failed to create UTXO database directory {}",
            path.display()
        )
    })?;

    println!("Opening UTXO database at {} ", path.display());

    UtxoStore::open(&path)
}

fn utxo_db_path(root: &Path, network: Network) -> PathBuf {
    let nw = network_suffix(network);
    root.join(nw)
}

fn network_suffix(network: Network) -> &'static str {
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
        let mut wallet = Wallet::new(root)?;
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
        let mut wallet = Wallet::new(root)?;

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
        let mut wallet = Wallet::new(root)?;

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
}
