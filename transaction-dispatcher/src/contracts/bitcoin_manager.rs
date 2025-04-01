use crate::contracts::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
pub(super) use crate::contracts::common::ParseFieldError;
use crate::contracts::peg_manager::PegManagerErrors;
use crate::contracts::peg_manager::SolPegManager::{BtcTransaction, BtcTxIn, BtcTxOut};
use alloy_primitives::Bytes;
use alloy_sol_types::{SolInterface, sol};
use log::error;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

sol!(
    #[sol(rpc)]
    BitcoinManager,
    "../config/dev/abi/BitcoinManager.json",
);

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct BitcoinTransactionIn {
    pub(crate) tx_id: String,
    pub(crate) v_out: u32,
    pub(crate) sequence: u32,
    pub(crate) script_sig: String,
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
pub(crate) struct BitcoinTransactionOut {
    pub(crate) amount: u64,
    pub(crate) script_pub_key: String,
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
pub(crate) struct BitcoinTransaction {
    pub(crate) version: u32,
    pub(crate) inputs: Vec<BitcoinTransactionIn>,
    pub(crate) outputs: Vec<BitcoinTransactionOut>,
    pub(crate) lock_time: u32,
}

impl TryFrom<BitcoinTransaction> for BtcTransaction {
    type Error = ParseFieldError;

    fn try_from(value: BitcoinTransaction) -> Result<Self, Self::Error> {
        let inputs = value
            .inputs
            .into_iter()
            .map(BtcTxIn::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                error!("Failed to convert BitcoinTransactionIn: {:?}", e);
                e
            })?;

        let outputs = value
            .outputs
            .into_iter()
            .map(BtcTxOut::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                error!("Failed to convert BitcoinTransactionOut: {:?}", e);
                e
            })?;

        Ok(BtcTransaction {
            version: value.version,
            inputs,
            outputs,
            locktime: value.lock_time,
        })
    }
}

pub fn decode_contract_error(revert_data: &Bytes) -> Option<PegManagerErrors> {
    let decoded_error = BitcoinManagerErrors::abi_decode(&revert_data, true);
    if decoded_error.is_ok() {
        return Some(match decoded_error.unwrap() {
            BitcoinManagerErrors::AddressEmptyCode(e) => {
                error!("SolBitcoinManagerErrors#AddressEmptyCode {}", e.target);
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::FailedCall(_) => {
                error!("SolBitcoinManagerErrors#FailedCall");
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::IncorrectOutputNumber(e) => {
                error!(
                    "SolBitcoinManagerErrors#IncorrectOutputNumber actual: {}, expected: {}",
                    e.actual, e.expected
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::IncorrectP2TRScriptPub(e) => {
                error!(
                    "SolBitcoinManagerErrors#IncorrectP2TRScriptPub actual: {:?}, expected: {:?}",
                    e.actual, e.expected
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::IncorrectlyFormedOpReturn(e) => {
                error!(
                    "SolBitcoinManagerErrors#IncorrectlyFormedOpReturn index: {}",
                    e.index
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::InvalidAddress(e) => {
                error!("SolBitcoinManagerErrors#InvalidAddress {}", e._address);
                PegManagerErrors::InvalidAddress
            }
            BitcoinManagerErrors::InvalidInitialization(_) => {
                error!("SolBitcoinManagerErrors#InvalidInitialization");
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::InvalidOpReturnLength(e) => {
                error!(
                    "SolBitcoinManagerErrors#InvalidOpReturnLength actual: {}, expected: {}",
                    e.actual, e.expected
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::InvalidPublicKey(e) => {
                error!("SolBitcoinManagerErrors#InvalidPublicKey {}", e.publicKey);
                PegManagerErrors::InvalidPublicKey
            }
            BitcoinManagerErrors::InvalidValue(e) => {
                error!("SolBitcoinManagerErrors#InvalidValue {}", e._value);
                PegManagerErrors::InvalidValue
            }
            BitcoinManagerErrors::NotInitializing(_) => {
                error!("SolBitcoinManagerErrors#NotInitializing");
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::NumberTooLarge(e) => {
                error!(
                    "SolBitcoinManagerErrors#NumberTooLarge actual: {}, max: {}",
                    e.actual, e.max
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::OwnableInvalidOwner(e) => {
                error!("SolBitcoinManagerErrors#OwnableInvalidOwner {}", e.owner);
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::OwnableUnauthorizedAccount(e) => {
                error!(
                    "SolBitcoinManagerErrors#OwnableUnauthorizedAccount {}",
                    e.account
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::UUPSUnauthorizedCallContext(_) => {
                error!("SolBitcoinManagerErrors#UUPSUnauthorizedCallContext");
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
                error!(
                    "SolBitcoinManagerErrors#UUPSUnsupportedProxiableUUID slot: {:?}",
                    e.slot
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::indexOverflow(e) => {
                error!(
                    "SolBitcoinManagerErrors#indexOverflow length: {}, from: {}, upTo: {}",
                    e.length, e.from, e.upTo
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::ERC1967InvalidImplementation(e) => {
                error!(
                    "SolBitcoinManagerErrors#ERC1967InvalidImplementation {}",
                    e.implementation
                );
                PegManagerErrors::InternalError
            }
            BitcoinManagerErrors::ERC1967NonPayable(_) => {
                error!("SolBitcoinManagerErrors#ERC1967NonPayable");
                PegManagerErrors::InternalError
            }
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        IncorrectOutputNumber, IncorrectP2TRScriptPub, IncorrectlyFormedOpReturn,
        InvalidOpReturnLength, SolBitcoinManagerErrors,
    };
    use crate::contracts::common::tests::generate_contract_expected_error;
    use crate::contracts::peg_manager::SolPegManager::{NotInitializing, SolPegManagerErrors};
    use crate::contracts::peg_manager::{PegManagerErrors, decode_contract_error};

    #[test]
    fn test_incorrect_output_number() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputNumber(IncorrectOutputNumber {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InvalidPegInRequestData);
    }

    #[test]
    fn test_incorrect_p2tr_script_pub() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectP2TRScriptPub(IncorrectP2TRScriptPub {
                actual: alloy_primitives::Bytes::from_static(&[0x00]),
                expected: alloy_primitives::Bytes::from_static(&[0x01]),
            });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InvalidPegInRequestData);
    }

    #[test]
    fn test_incorrectly_formed_op_return() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(IncorrectlyFormedOpReturn {
                index: alloy_primitives::Uint::from(1),
            });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InvalidPegInRequestData);
    }

    #[test]
    fn test_invalid_op_return_length() {
        let expected_err = SolBitcoinManagerErrors::InvalidOpReturnLength(InvalidOpReturnLength {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InvalidPegInRequestData);
    }

    // check one of the errors to ensure the mapping to InternalError keeps working
    // there are more errors that map to InternalError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = SolPegManagerErrors::NotInitializing(NotInitializing {});

        let expected_err_payload = generate_contract_expected_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        assert_eq!(result, PegManagerErrors::InternalError);
    }
}
