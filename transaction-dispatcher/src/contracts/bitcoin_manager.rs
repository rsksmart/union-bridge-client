use crate::contracts::bitcoin_manager::SolBitcoinManager::SolBitcoinManagerErrors;
pub(super) use crate::contracts::common::ParseFieldError;
use crate::contracts::peg_manager::SolPegManager::{BtcTransaction, BtcTxIn, BtcTxOut};
use crate::format_sol_err;
use crate::rsk_gateway::PegManagerErrors;
use crate::types::{BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut};
use SolBitcoinManagerErrors::*;
use alloy_primitives::Bytes;
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

impl From<SolBitcoinManagerErrors> for PegManagerErrors {
    fn from(err: SolBitcoinManagerErrors) -> Self {
        match err {
            // explicitly mapped variants
            IncorrectOutputScript(e) => {
                PegManagerErrors::InvalidPegInRequestData(format_sol_err!(e, e.actual, e.expected))
            }
            IncorrectlyFormedOpReturn(e) => {
                PegManagerErrors::InvalidPegInRequestData(format_sol_err!(e, e.index))
            }
            InvalidAddress(e) => PegManagerErrors::InvalidAddress(format_sol_err!(e, e._address)),
            InvalidOpReturnLength(e) => {
                PegManagerErrors::InvalidPegInRequestData(format_sol_err!(e, e.actual, e.expected))
            }
            InvalidPublicKey(e) => {
                PegManagerErrors::InvalidPublicKey(format_sol_err!(e, e.publicKey))
            }
            InvalidValue(e) => {
                PegManagerErrors::InvalidValue(format_sol_err!(e, e.expected, e._value))
            }

            // all others default to Unhandled
            AddressEmptyCode(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.target))
            }
            FailedCall(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            InvalidInitialization(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            InvalidOutputAmount(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.expected, e.actual))
            }
            NotInitializing(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
            NumberTooLarge(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.actual, e.max))
            }
            OwnableInvalidOwner(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.owner))
            }
            OwnableUnauthorizedAccount(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.account))
            }
            UUPSUnauthorizedCallContext(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e))
            }
            UUPSUnsupportedProxiableUUID(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.slot))
            }
            indexOverflow(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(
                e, e.length, e.from, e.upTo
            )),
            ERC1967InvalidImplementation(e) => {
                PegManagerErrors::UnhandledContractError(format_sol_err!(e, e.implementation))
            }
            ERC1967NonPayable(e) => PegManagerErrors::UnhandledContractError(format_sol_err!(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::bitcoin_manager::SolBitcoinManager::{
        IncorrectOutputScript, IncorrectlyFormedOpReturn, InvalidOpReturnLength, NotInitializing,
        SolBitcoinManagerErrors,
    };
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::PegManagerErrors;
    use alloy_json_rpc::ErrorPayload;
    use alloy_sol_types::SolInterface;

    #[test]
    fn test_incorrect_output_number() {
        let expected_err = SolBitcoinManagerErrors::IncorrectOutputScript(IncorrectOutputScript {
            actual: alloy_primitives::Bytes::from(vec![0x01, 0x2]),
            expected: alloy_primitives::Bytes::from(vec![0x02, 0x3]),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidPegInRequestData(_));
    }

    #[test]
    fn test_incorrectly_formed_op_return() {
        let expected_err =
            SolBitcoinManagerErrors::IncorrectlyFormedOpReturn(IncorrectlyFormedOpReturn {
                index: alloy_primitives::Uint::from(1),
            });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidPegInRequestData(_));
    }

    #[test]
    fn test_invalid_op_return_length() {
        let expected_err = SolBitcoinManagerErrors::InvalidOpReturnLength(InvalidOpReturnLength {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::InvalidPegInRequestData(_));
    }

    // check one of the errors to ensure the mapping to UnhandledError keeps working
    // there are more errors that map to UnhandledError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let expected_err = SolBitcoinManagerErrors::NotInitializing(NotInitializing {});

        let expected_err_payload = generate_contract_revert_error(expected_err);
        let result = decode_contract_error(&expected_err_payload);

        matches!(result, PegManagerErrors::UnhandledContractError(_));
    }

    fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
        let revert_data = if let Some(data) = error_payload.as_revert_data() {
            data
        } else {
            return PegManagerErrors::NoRevertError(format!(
                "Not a BitcoinManagerError: {:?}",
                error_payload
            ));
        };

        match SolBitcoinManagerErrors::abi_decode(&revert_data, true) {
            Ok(decoded_error) => decoded_error.into(),
            Err(_) => PegManagerErrors::UnknownContractError(format!(
                "Unknown BitcoinManagerError: {:?}",
                error_payload
            )),
        }
    }
}
