use crate::contracts::bitcoin_manager::SolBitcoinManager::SolBitcoinManagerErrors;
pub(super) use crate::contracts::common::ParseFieldError;
use crate::contracts::peg_manager::SolPegManager::{BtcTransaction, BtcTxIn, BtcTxOut};
use crate::rsk_gateway::PegManagerErrors;
use crate::types::{BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut};
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::Bytes;
use alloy_sol_types::SolInterface;
use log::error;
use std::str::FromStr;

include!(concat!(env!("OUT_DIR"), "/abi.rs"));

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
        // TODO(Jira): Improve error handling https://rsklabs.atlassian.net/browse/UB-107
        return Some(match decoded_error.unwrap() {
            SolBitcoinManagerErrors::AddressEmptyCode(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#AddressEmptyCode {}",
                    e.target
                ))
            }
            SolBitcoinManagerErrors::FailedCall(_) => PegManagerErrors::UnhandledContractError(
                "SolSolBitcoinManagerErrors#FailedCall".to_string(),
            ),
            SolBitcoinManagerErrors::IncorrectOutputScript(e) => {
                PegManagerErrors::InvalidPegInRequestData(format!(
                    "SolSolBitcoinManagerErrors#IncorrectOutputScript actual: {}, expected: {}",
                    e.actual, e.expected
                ))
            }
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(e) => {
                PegManagerErrors::InvalidPegInRequestData(format!(
                    "SolSolBitcoinManagerErrors#IncorrectlyFormedOpReturn index: {}",
                    e.index
                ))
            }
            SolBitcoinManagerErrors::InvalidAddress(e) => PegManagerErrors::InvalidAddress(
                format!("SolSolBitcoinManagerErrors#InvalidAddress {}", e._address),
            ),
            SolBitcoinManagerErrors::InvalidInitialization(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolSolBitcoinManagerErrors#InvalidInitialization".to_string(),
                )
            }
            SolBitcoinManagerErrors::InvalidOpReturnLength(e) => {
                PegManagerErrors::InvalidPegInRequestData(format!(
                    "SolSolBitcoinManagerErrors#InvalidOpReturnLength actual: {}, expected: {}",
                    e.actual, e.expected
                ))
            }
            SolBitcoinManagerErrors::InvalidPublicKey(e) => {
                PegManagerErrors::InvalidPublicKey(format!(
                    "SolSolBitcoinManagerErrors#InvalidPublicKey {}",
                    e.publicKey
                ))
            }
            SolBitcoinManagerErrors::InvalidValue(e) => PegManagerErrors::InvalidValue(format!(
                "SolSolBitcoinManagerErrors#InvalidValue {}",
                e._value
            )),
            SolBitcoinManagerErrors::InvalidOutputAmount(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#InvalidOutputAmount: {} - {}",
                    e.expected, e.actual
                ))
            }
            SolBitcoinManagerErrors::NotInitializing(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolSolBitcoinManagerErrors#NotInitializing".to_string(),
                )
            }
            SolBitcoinManagerErrors::NumberTooLarge(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#NumberTooLarge actual: {}, max: {}",
                    e.actual, e.max
                ))
            }
            SolBitcoinManagerErrors::OwnableInvalidOwner(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#OwnableInvalidOwner {}",
                    e.owner
                ))
            }
            SolBitcoinManagerErrors::OwnableUnauthorizedAccount(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#OwnableUnauthorizedAccount {}",
                    e.account
                ))
            }
            SolBitcoinManagerErrors::UUPSUnauthorizedCallContext(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolSolBitcoinManagerErrors#UUPSUnauthorizedCallContext".to_string(),
                )
            }
            SolBitcoinManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#UUPSUnsupportedProxiableUUID slot: {:?}",
                    e.slot
                ))
            }
            SolBitcoinManagerErrors::indexOverflow(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#indexOverflow length: {}, from: {}, upTo: {}",
                    e.length, e.from, e.upTo
                ))
            }
            SolBitcoinManagerErrors::ERC1967InvalidImplementation(e) => {
                PegManagerErrors::UnhandledContractError(format!(
                    "SolSolBitcoinManagerErrors#ERC1967InvalidImplementation {}",
                    e.implementation
                ))
            }
            SolBitcoinManagerErrors::ERC1967NonPayable(_) => {
                PegManagerErrors::UnhandledContractError(
                    "SolSolBitcoinManagerErrors#ERC1967NonPayable".to_string(),
                )
            }
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        IncorrectOutputScript, IncorrectlyFormedOpReturn, InvalidOpReturnLength, NotInitializing,
        SolBitcoinManagerErrors,
    };
    use crate::contracts::bitcoin_manager::decode_contract_error;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::PegManagerErrors;

    #[test]
    fn test_incorrect_output_number() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputScript(IncorrectOutputScript {
            actual: alloy_primitives::Bytes::from(vec![0x01, 0x2]),
            expected: alloy_primitives::Bytes::from(vec![0x02, 0x3]),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData(_)));
    }

    #[test]
    fn test_incorrectly_formed_op_return() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(IncorrectlyFormedOpReturn {
                index: alloy_primitives::Uint::from(1),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData(_)));
    }

    #[test]
    fn test_invalid_op_return_length() {
        let expected_err = SolBitcoinManagerErrors::InvalidOpReturnLength(InvalidOpReturnLength {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::InvalidPegInRequestData(_)));
    }

    // check one of the errors to ensure the mapping to UnhandledError keeps working
    // there are more errors that map to UnhandledError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = SolBitcoinManagerErrors::NotInitializing(NotInitializing {});

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, Some(PegManagerErrors::UnhandledContractError(_)));
    }
}
