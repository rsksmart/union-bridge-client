use std::str::FromStr;

use alloy_primitives::Bytes;
use log::error;
use union_contracts::bindings::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
use union_contracts::bindings::peg_manager::PegManager::{BtcTransaction, BtcTxIn, BtcTxOut};

pub(super) use crate::contracts::common::ParseFieldError;
use crate::rsk_gateway::DomainErrors;
use crate::types::{BitcoinTransaction, BitcoinTransactionIn, BitcoinTransactionOut};

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

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<BitcoinManagerErrors>();
    decoded_err.map(|e| match e {
        BitcoinManagerErrors::InvalidAddress(e) => DomainErrors::InvalidAddress(format!("{:?}", e)),
        BitcoinManagerErrors::InvalidPublicKey(e) => {
            DomainErrors::InvalidPublicKey(format!("{:?}", e))
        }
        BitcoinManagerErrors::InvalidValue(e) => DomainErrors::InvalidValue(format!("{:?}", e)),
        // TODO handle more based on needs
        _ => DomainErrors::UnhandledContractError(format!("{:?}", e)),
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::FixedBytes;
    use union_contracts::bindings::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, IncorrectOutputScript, IncorrectlyFormedOpReturn, InvalidAddress,
        InvalidOpReturnLength, InvalidOutputAmount, InvalidPublicKey, InvalidValue,
        NotInitializing,
    };

    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::rsk_gateway::DomainErrors;

    #[test]
    fn test_incorrect_output_number() {
        let err_data = BitcoinManagerErrors::IncorrectOutputScript(IncorrectOutputScript {
            actual: alloy_primitives::Bytes::from(vec![0x01, 0x2]),
            expected: alloy_primitives::Bytes::from(vec![0x02, 0x3]),
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_incorrectly_formed_op_return() {
        let err_data = BitcoinManagerErrors::IncorrectlyFormedOpReturn(IncorrectlyFormedOpReturn {
            index: alloy_primitives::Uint::from(1),
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_op_return_length() {
        let err_data = BitcoinManagerErrors::InvalidOpReturnLength(InvalidOpReturnLength {
            actual: alloy_primitives::Uint::from(1),
            expected: alloy_primitives::Uint::from(2),
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    #[test]
    fn test_invalid_address() {
        let err_data = BitcoinManagerErrors::InvalidAddress(InvalidAddress {
            _address: "0x00112233445566778899aabbccddeeff00112233"
                .parse()
                .expect("Failed to parse address"),
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidAddress(_));
    }

    #[test]
    fn test_invalid_public_key() {
        let err_data = BitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
            publicKey: "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1"
                .parse::<FixedBytes<32>>()
                .unwrap(),
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidCompressedPubKey(_));
    }

    #[test]
    fn test_invalid_value() {
        let err_data = BitcoinManagerErrors::InvalidValue(InvalidValue {
            expected: 1,
            _value: 2,
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidValue(_));
    }

    #[test]
    fn test_invalid_output_amount() {
        let err_data = BitcoinManagerErrors::InvalidOutputAmount(InvalidOutputAmount {
            expected: 1,
            actual: 2,
        });

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::InvalidBtcTxSpvProof(_));
    }

    // check one of the errors to ensure the mapping to UnhandledError keeps working
    // there are more errors that map to UnhandledError, but we don't need to test all of them
    // all the ones that have defined mappings must be tested
    #[test]
    fn test_unhandled() {
        let err_data = BitcoinManagerErrors::NotInitializing(NotInitializing {});

        let result = generate_contract_revert_error(err_data);
        matches!(result.into(), DomainErrors::UnhandledContractError(_));
    }
}
