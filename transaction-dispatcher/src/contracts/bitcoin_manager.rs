pub(crate) use crate::contracts::common::ParseFieldError;
use crate::contracts::peg_manager::PegManagerAlloy::{BtcTransaction, BtcTxIn, BtcTxOut};
use alloy_primitives::Bytes;
use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

sol!(
    #[sol(rpc)]
    BitcoinManager,
    "../config/dev/abi/BitcoinManager.json",
);

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionIn {
    pub tx_id: String,
    pub v_out: u32,
    pub sequence: u32,
    pub script_sig: String,
}

impl TryFrom<BitcoinTransactionIn> for BtcTxIn {
    type Error = ParseFieldError;

    fn try_from(value: BitcoinTransactionIn) -> Result<Self, Self::Error> {
        Ok(BtcTxIn {
            txId: value.tx_id.parse().map_err(ParseFieldError::ParseHex)?,
            vout: value.v_out,
            sequence: value.sequence,
            scriptSig: Bytes::from_str(&value.script_sig).map_err(ParseFieldError::ParseHex)?,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransactionOut {
    amount: u64,
    script_pub_key: String,
}

impl TryFrom<BitcoinTransactionOut> for BtcTxOut {
    type Error = ParseFieldError;

    fn try_from(value: BitcoinTransactionOut) -> Result<Self, Self::Error> {
        Ok(BtcTxOut {
            amount: value.amount,
            scriptPubKey: Bytes::from_str(&value.script_pub_key)
                .map_err(ParseFieldError::ParseHex)?,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BitcoinTransaction {
    version: u32,
    inputs: Vec<BitcoinTransactionIn>,
    outputs: Vec<BitcoinTransactionOut>,
    lock_time: u32,
}

impl TryFrom<BitcoinTransaction> for BtcTransaction {
    type Error = ParseFieldError;

    fn try_from(value: BitcoinTransaction) -> Result<Self, Self::Error> {
        Ok(BtcTransaction {
            version: value.version,
            inputs: value
                .inputs
                .into_iter()
                .map(BtcTxIn::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            outputs: value
                .outputs
                .into_iter()
                .map(BtcTxOut::try_from)
                .collect::<Result<Vec<_>, _>>()?,

            locktime: value.lock_time,
        })
    }
}
