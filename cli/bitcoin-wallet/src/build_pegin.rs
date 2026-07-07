//! Stateless `build_pegin` helper: build and sign a pegin transaction from a JSON request.
//!
//! It reuses [`Wallet::create_pegin_transaction`] against a throwaway tempdir store (mirroring the
//! tempdir pattern in the wallet integration tests), so it adds no transaction logic and never
//! configures RPC, broadcasts, or commits anything — the store is discarded on return.

use std::str::FromStr;

use anyhow::{Result, bail};
use bitcoin::consensus::encode::serialize_hex;
use bitcoin::network::Network;
use bitcoin::{OutPoint, Txid};
use serde::{Deserialize, Serialize};

use crate::cli::WalletMode;
use crate::wallet::{DEFAULT_ENABLER_AMOUNT, DEFAULT_SATS_PER_BYTE, Wallet};

#[derive(Deserialize)]
pub struct UtxoInput {
    pub txid: String,
    pub vout: u32,
    pub value_sat: u64,
}

#[derive(Deserialize)]
pub struct BuildPeginRequest {
    pub wif: String,
    pub network: String,
    pub utxos: Vec<UtxoInput>,
    pub stream_value_sat: u64,
    pub packet_number: u64,
    pub pegin_address: String,
    pub rsk_address_hex: String,
    pub enabler_script_pubkey_hex: String,
    pub sats_per_byte: Option<u64>,
    pub enabler_amount_sat: Option<u64>,
}

#[derive(Serialize)]
pub struct OutpointOut {
    pub txid: String,
    pub vout: u32,
}

#[derive(Serialize)]
pub struct BuildPeginResponse {
    pub raw_tx_hex: String,
    pub tx_id: String,
    pub fee_sat: u64,
    pub change_value_sat: u64,
    pub spent_outpoints: Vec<OutpointOut>,
}

pub fn build_pegin(req: BuildPeginRequest) -> Result<BuildPeginResponse> {
    let network = match req.network.as_str() {
        "regtest" => Network::Regtest,
        "testnet" => Network::Testnet,
        "bitcoin" => Network::Bitcoin,
        other => bail!("unsupported network: {other}"),
    };

    // Throwaway store: the wallet is built, used to sign, then dropped with the tempdir.
    let temp = tempfile::tempdir()?;
    let db_root = temp.path().join("utxo-db");

    let mut wallet = Wallet::new_with_network(db_root, network, WalletMode::User)?;
    wallet.import_private_key(&req.wif)?;
    wallet.set_sats_per_byte(req.sats_per_byte.unwrap_or(DEFAULT_SATS_PER_BYTE));
    wallet.set_enabler_amount(req.enabler_amount_sat.unwrap_or(DEFAULT_ENABLER_AMOUNT));

    for utxo in &req.utxos {
        let outpoint = OutPoint::new(Txid::from_str(&utxo.txid)?, utxo.vout);
        wallet.register_utxo(outpoint, utxo.value_sat)?;
    }

    let created = wallet.create_pegin_transaction(
        req.stream_value_sat,
        req.packet_number,
        req.pegin_address,
        req.rsk_address_hex,
        req.enabler_script_pubkey_hex,
    )?;

    let spent_outpoints = created
        .spent_utxos
        .iter()
        .map(|u| OutpointOut { txid: u.outpoint.txid.to_string(), vout: u.outpoint.vout })
        .collect();

    Ok(BuildPeginResponse {
        raw_tx_hex: serialize_hex(&created.transaction),
        tx_id: created.transaction.compute_txid().to_string(),
        fee_sat: created.fee_sat,
        change_value_sat: created.change_value,
        spent_outpoints,
    })
}
