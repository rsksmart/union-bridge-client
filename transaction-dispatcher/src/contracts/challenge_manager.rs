use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, Bytes, FixedBytes, TxHash};
use alloy_provider::Provider;
use log::{error, info};
#[cfg(test)]
use mockall::automock;
use union_contracts::bindings::challenge_manager::ChallengeManager::{
    self, BtcTransaction, BtcTxIn, BtcTxOut, BtcTxSPVProof, ChallengeManagerErrors,
    ChallengeManagerInstance, ChallengeTempInfo,
};

use crate::contracts::bitcoin_manager::ParseFieldError;
use crate::contracts::common::send_tx_with_gas_bump;
pub(crate) use crate::contracts::interactions::register_challenge::RegisterChallengeInvoke;
pub(crate) use crate::contracts::interactions::register_input_revealed::RegisterInputRevealedInvoke;
use crate::rsk_gateway::DomainErrors;
use crate::types::BtcTxSPVProofInput;

#[cfg_attr(test, automock)]
pub trait ChallengeManagerContractApi {
    async fn invoke_register_challenge(
        &self,
        accept_pegin_txid: FixedBytes<32>,
        challenge: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn invoke_register_input_revealed(
        &self,
        accept_pegin_txid: FixedBytes<32>,
        input_revealed: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash>;

    async fn call_get_challenge_temp_info(
        &self,
        accept_pegin_txid: FixedBytes<32>,
    ) -> alloy_contract::Result<ChallengeTempInfo>;
}

#[derive(Clone)]
pub struct ChallengeManagerContract<P: Provider> {
    contract_instance: ChallengeManagerInstance<P>,
}

impl<P: Provider> ChallengeManagerContract<P> {
    pub fn new(provider: P, contract_address: Address) -> Self {
        info!("Connecting to ChallengeManagerContract @ {contract_address}");
        let contract_instance = ChallengeManager::new(contract_address, provider);
        ChallengeManagerContract { contract_instance }
    }
}

impl<P: Provider> ChallengeManagerContractApi for ChallengeManagerContract<P> {
    async fn invoke_register_challenge(
        &self,
        accept_pegin_txid: FixedBytes<32>,
        challenge: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || self.contract_instance.registerChallenge(accept_pegin_txid, challenge.clone()),
            gas_bumps,
        )
        .await
    }

    async fn invoke_register_input_revealed(
        &self,
        accept_pegin_txid: FixedBytes<32>,
        input_revealed: BtcTxSPVProof,
        gas_bumps: u8,
    ) -> alloy_contract::Result<TxHash> {
        send_tx_with_gas_bump(
            &self.contract_instance.provider(),
            || {
                self.contract_instance
                    .registerInputRevealed(accept_pegin_txid, input_revealed.clone())
            },
            gas_bumps,
        )
        .await
    }

    async fn call_get_challenge_temp_info(
        &self,
        accept_pegin_txid: FixedBytes<32>,
    ) -> alloy_contract::Result<ChallengeTempInfo> {
        self.contract_instance.getChallengeTempInfo(accept_pegin_txid).call().await
    }
}

impl TryFrom<BtcTxSPVProofInput> for BtcTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: BtcTxSPVProofInput) -> Result<Self, Self::Error> {
        build_btc_tx_spv_proof(value)
    }
}

fn build_btc_tx_spv_proof(input: BtcTxSPVProofInput) -> Result<BtcTxSPVProof, ParseFieldError> {
    let block_hash =
        FixedBytes::<32>::from_hex(&input.block_hash).map_err(ParseFieldError::ParseHex)?;

    let inputs = input
        .btc_tx
        .inputs
        .into_iter()
        .map(|i| {
            let txid = i.tx_id.parse().map_err(ParseFieldError::ParseHex)?;
            let script_sig = Bytes::from_hex(i.script_sig).map_err(ParseFieldError::ParseHex)?;
            Ok(BtcTxIn { txId: txid, vout: i.v_out, sequence: i.sequence, scriptSig: script_sig })
        })
        .collect::<Result<Vec<BtcTxIn>, ParseFieldError>>()?;

    let outputs = input
        .btc_tx
        .outputs
        .into_iter()
        .map(|o| {
            let script_pub_key =
                Bytes::from_hex(o.script_pub_key).map_err(ParseFieldError::ParseHex)?;
            Ok(BtcTxOut { amount: o.amount, scriptPubKey: script_pub_key })
        })
        .collect::<Result<Vec<BtcTxOut>, ParseFieldError>>()?;

    let btc_tx = BtcTransaction {
        version: input.btc_tx.version,
        inputs,
        outputs,
        locktime: input.btc_tx.lock_time,
    };

    let merkle_branches_hashes = input
        .merkle_branch_hashes
        .into_iter()
        .map(|hash| hash.parse::<FixedBytes<32>>().map_err(ParseFieldError::ParseHex))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            error!("Failed to convert merkle_branch_hashes: {e:?}");
            e
        })?;

    Ok(BtcTxSPVProof {
        blockHash: block_hash,
        btcTx: btc_tx,
        merkleBranchPath: input.merkle_branch_path.parse()?,
        merkleBranchHashes: merkle_branches_hashes,
    })
}

pub(crate) fn decode_error(err: &alloy_contract::Error) -> Option<DomainErrors> {
    let decoded_err = err.as_decoded_interface_error::<ChallengeManagerErrors>()?;

    Some(match decoded_err {
        ChallengeManagerErrors::PeginNotRequested(e) => {
            DomainErrors::PeginNotRequested(format!("{e:?}"))
        }
        ChallengeManagerErrors::ChallengeTxidNotMatch(e) => {
            DomainErrors::ChallengeTxidNotMatch(format!("{e:?}"))
        }
        ChallengeManagerErrors::InvalidChallengeInputCount(e) => {
            DomainErrors::InvalidChallengeInputCount(format!("{e:?}"))
        }
        ChallengeManagerErrors::InvalidRevealedInputCount(e) => {
            DomainErrors::InvalidRevealedInputCount(format!("{e:?}"))
        }
        ChallengeManagerErrors::ReimbursementKickoffTxidNotMatch(e) => {
            DomainErrors::ReimbursementKickoffTxidNotMatch(format!("{e:?}"))
        }
        ChallengeManagerErrors::InvalidPegStatus(e) => {
            DomainErrors::InvalidPegStatus(format!("{e:?}"))
        }
        ChallengeManagerErrors::MemberNotInCommittee(e) => {
            DomainErrors::MemberNotInCommittee(format!("{e:?}"))
        }
        _ => DomainErrors::UnhandledContractError(format!("{decoded_err:?}")),
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, FixedBytes};
    use union_contracts::bindings::challenge_manager::ChallengeManager::{
        ChallengeTxidNotMatch, ChallengeManagerErrors, InvalidChallengeInputCount,
        InvalidPegStatus, InvalidRevealedInputCount, MemberNotInCommittee, PeginNotRequested,
        ReimbursementKickoffTxidNotMatch,
    };

    use super::*;
    use crate::contracts::common::tests::generate_contract_revert_error;

    #[test]
    fn test_pegin_not_requested_error() {
        let err_data = ChallengeManagerErrors::PeginNotRequested(PeginNotRequested {
            btcTxid: FixedBytes::<32>::from([1u8; 32]),
        });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::PeginNotRequested(_)));
    }

    #[test]
    fn test_challenge_txid_not_match_error() {
        let err_data = ChallengeManagerErrors::ChallengeTxidNotMatch(ChallengeTxidNotMatch {
            actual: FixedBytes::<32>::from([1u8; 32]),
            expected: FixedBytes::<32>::from([2u8; 32]),
        });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::ChallengeTxidNotMatch(_)));
    }

    #[test]
    fn test_invalid_challenge_input_count_error() {
        let err_data =
            ChallengeManagerErrors::InvalidChallengeInputCount(InvalidChallengeInputCount {
                actual: alloy_primitives::U256::from(1),
                expected: alloy_primitives::U256::from(2),
            });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidChallengeInputCount(_)));
    }

    #[test]
    fn test_invalid_revealed_input_count_error() {
        let err_data =
            ChallengeManagerErrors::InvalidRevealedInputCount(InvalidRevealedInputCount {
                actual: alloy_primitives::U256::from(3),
                expected: alloy_primitives::U256::from(4),
            });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidRevealedInputCount(_)));
    }

    #[test]
    fn test_reimbursement_kickoff_txid_not_match_error() {
        let err_data = ChallengeManagerErrors::ReimbursementKickoffTxidNotMatch(
            ReimbursementKickoffTxidNotMatch {
                actual: FixedBytes::<32>::from([3u8; 32]),
                expected: FixedBytes::<32>::from([4u8; 32]),
            },
        );
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::ReimbursementKickoffTxidNotMatch(_)));
    }

    #[test]
    fn test_invalid_peg_status_error() {
        let err_data = ChallengeManagerErrors::InvalidPegStatus(InvalidPegStatus { actual: 0 });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::InvalidPegStatus(_)));
    }

    #[test]
    fn test_member_not_in_committee_error() {
        let err_data = ChallengeManagerErrors::MemberNotInCommittee(MemberNotInCommittee {
            committeeId: 1u128,
            memberAddress: Address::default(),
        });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::MemberNotInCommittee(_)));
    }

    #[test]
    fn test_unhandled_error() {
        use union_contracts::bindings::challenge_manager::ChallengeManager::ERC1967InvalidImplementation;

        let err_data =
            ChallengeManagerErrors::ERC1967InvalidImplementation(ERC1967InvalidImplementation {
                implementation: Address::default(),
            });
        let result = generate_contract_revert_error(&err_data);
        let domain_error = decode_error(&result).unwrap();
        assert!(matches!(domain_error, DomainErrors::UnhandledContractError(_)));
    }
}
