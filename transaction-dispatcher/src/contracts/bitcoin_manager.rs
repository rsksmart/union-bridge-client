use crate::contracts::bitcoin_manager::SolBitcoinManager::SolBitcoinManagerErrors;
pub(super) use crate::contracts::common::ParseFieldError;
use crate::contracts::peg_manager::SolPegManager::{BtcTransaction, BtcTxIn, BtcTxOut};
use crate::rsk_gateway::PegManagerErrors;
use crate::types::{BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut};
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::Bytes;
use alloy_sol_types::{SolInterface, sol};
use log::error;
use std::str::FromStr;

sol!(
    #[sol(rpc)]
    SolBitcoinManager,
    "../config/dev/abi/BitcoinManager.json",
);

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

pub(crate) fn decode_contract_error(error_payload: &ErrorPayload) -> Option<PegManagerErrors> {
    let revert_data = if let Some(data) = error_payload.as_revert_data() {
        data
    } else {
        return Some(PegManagerErrors::NoRevertError(format!(
            "Not a BitcoinManagerError: {:?}",
            error_payload
        )));
    };

    let decoded_error = SolBitcoinManagerErrors::abi_decode(&revert_data, true);
    if decoded_error.is_ok() {
        // TODO(create-Jira) - review all errors and conceptually merge into or create new PegManagerErrors
        return Some(match decoded_error.unwrap() {
            SolBitcoinManagerErrors::AddressEmptyCode(e) => {
                error!("SolSolBitcoinManagerErrors#AddressEmptyCode {}", e.target);
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::FailedCall(_) => {
                error!("SolSolBitcoinManagerErrors#FailedCall");
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::IncorrectOutputNumber(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#IncorrectOutputNumber actual: {}, expected: {}",
                    e.actual, e.expected
                );
                PegManagerErrors::InvalidPegInRequestData
            }
            SolBitcoinManagerErrors::IncorrectP2TRScriptPub(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#IncorrectP2TRScriptPub actual: {:?}, expected: {:?}",
                    e.actual, e.expected
                );
                PegManagerErrors::InvalidPegInRequestData
            }
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#IncorrectlyFormedOpReturn index: {}",
                    e.index
                );
                PegManagerErrors::InvalidPegInRequestData
            }
            SolBitcoinManagerErrors::InvalidAddress(e) => {
                error!("SolSolBitcoinManagerErrors#InvalidAddress {}", e._address);
                PegManagerErrors::InvalidAddress
            }
            SolBitcoinManagerErrors::InvalidInitialization(_) => {
                error!("SolSolBitcoinManagerErrors#InvalidInitialization");
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::InvalidOpReturnLength(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#InvalidOpReturnLength actual: {}, expected: {}",
                    e.actual, e.expected
                );
                PegManagerErrors::InvalidPegInRequestData
            }
            SolBitcoinManagerErrors::InvalidPublicKey(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#InvalidPublicKey {}",
                    e.publicKey
                );
                PegManagerErrors::InvalidPublicKey
            }
            SolBitcoinManagerErrors::InvalidValue(e) => {
                error!("SolSolBitcoinManagerErrors#InvalidValue {}", e._value);
                PegManagerErrors::InvalidValue
            }
            SolBitcoinManagerErrors::NotInitializing(_) => {
                error!("SolSolBitcoinManagerErrors#NotInitializing");
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::NumberTooLarge(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#NumberTooLarge actual: {}, max: {}",
                    e.actual, e.max
                );
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::OwnableInvalidOwner(e) => {
                error!("SolSolBitcoinManagerErrors#OwnableInvalidOwner {}", e.owner);
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::OwnableUnauthorizedAccount(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#OwnableUnauthorizedAccount {}",
                    e.account
                );
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::UUPSUnauthorizedCallContext(_) => {
                error!("SolSolBitcoinManagerErrors#UUPSUnauthorizedCallContext");
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#UUPSUnsupportedProxiableUUID slot: {:?}",
                    e.slot
                );
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::indexOverflow(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#indexOverflow length: {}, from: {}, upTo: {}",
                    e.length, e.from, e.upTo
                );
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::ERC1967InvalidImplementation(e) => {
                error!(
                    "SolSolBitcoinManagerErrors#ERC1967InvalidImplementation {}",
                    e.implementation
                );
                PegManagerErrors::UnhandledContractError
            }
            SolBitcoinManagerErrors::ERC1967NonPayable(_) => {
                error!("SolSolBitcoinManagerErrors#ERC1967NonPayable");
                PegManagerErrors::UnhandledContractError
            }
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        IncorrectOutputNumber, IncorrectP2TRScriptPub, IncorrectlyFormedOpReturn,
        InvalidOpReturnLength, NotInitializing, SolBitcoinManagerErrors,
    };
    use crate::contracts::bitcoin_manager::decode_contract_error;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::PegManagerErrors;

    #[test]
    fn test_incorrect_output_number() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputNumber(IncorrectOutputNumber {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData));
    }

    #[test]
    fn test_incorrect_p2tr_script_pub() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectP2TRScriptPub(IncorrectP2TRScriptPub {
                actual: alloy_primitives::Bytes::from_static(&[0x00]),
                expected: alloy_primitives::Bytes::from_static(&[0x01]),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData));
    }

    #[test]
    fn test_incorrectly_formed_op_return() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(IncorrectlyFormedOpReturn {
                index: alloy_primitives::Uint::from(1),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData));
    }

    #[test]
    fn test_invalid_op_return_length() {
        let expected_err = SolBitcoinManagerErrors::InvalidOpReturnLength(InvalidOpReturnLength {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData));
    }

    // check one of the errors to ensure the mapping to UnhandledError keeps working
    // there are more errors that map to UnhandledError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = SolBitcoinManagerErrors::NotInitializing(NotInitializing {});

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::UnhandledContractError));
    }
}
