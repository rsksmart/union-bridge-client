use crate::{
    contracts::peg_manager::PegManagerContractApi,
    rsk_gateway::DomainErrors,
    types::{TriggerOperatorTakeInput, TriggerOperatorTakeOutput},
};
use alloy_primitives::FixedBytes;
use anyhow::Result;
use log::info;

#[derive(Clone)]
pub(crate) struct TriggerOperatorTakeInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> TriggerOperatorTakeInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        Self { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: TriggerOperatorTakeInput,
    ) -> Result<TriggerOperatorTakeOutput, DomainErrors> {
        info!("Init triggerOperatorTake for: {:?}", input);

        let pegout_txid = input.pegout_txid.parse::<FixedBytes<32>>().map_err(|e| {
            DomainErrors::InvalidValue(format!("Failed to parse pegout_txid: {}", e))
        })?;

        let transaction_hash =
            self.contract.invoke_trigger_operator_take(pegout_txid, self.gas_bumps).await?;

        Ok(TriggerOperatorTakeOutput { transaction_hash: transaction_hash.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::common::tests::generate_contract_revert_error;
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use alloy_primitives::{FixedBytes, TxHash};
    use std::str::FromStr;
    use union_contracts::bindings::peg_manager::PegManager::{
        PegManagerErrors, PegoutTxidNotFound,
    };

    impl TriggerOperatorTakeInvoke<MockPegManagerContractApi> {
        fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            Self { contract, gas_bumps: 3 }
        }
    }

    #[tokio::test]
    async fn test_run_successful() {
        let mut mock = MockPegManagerContractApi::new();
        let input = base_input();
        let expected_txid = input.pegout_txid.parse::<FixedBytes<32>>().unwrap();
        let expected_tx_hash =
            TxHash::from_str("0xfeedfacecafebeef000000000000000000000000000000000000000000000000")
                .expect("invalid tx hash");

        mock.expect_invoke_trigger_operator_take()
            .withf(move |hash, _| hash == &expected_txid)
            .returning(move |_, _| Ok(expected_tx_hash))
            .times(1);

        let invoke = TriggerOperatorTakeInvoke::new_for_tests(mock);
        let result = invoke.run(base_input()).await;

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            TriggerOperatorTakeOutput { transaction_hash: expected_tx_hash.to_string() }
        );
    }

    #[tokio::test]
    async fn test_run_failure() {
        let mut mock = MockPegManagerContractApi::new();

        let expected_tx_id = "0x6b8f74fe9c66c9c3a6c3d0b7111d9b6aaac0ea3db1bdbd6a38eb0e7d8b8bba3e";

        mock.expect_invoke_trigger_operator_take()
            .returning(move |_, _| {
                let expected_err = PegManagerErrors::PegoutTxidNotFound(PegoutTxidNotFound {
                    pegoutTxid: expected_tx_id.parse().expect("Failed to parse tx hash"),
                });
                Err(generate_contract_revert_error(&expected_err))
            })
            .times(1);

        let invoke = TriggerOperatorTakeInvoke::new_for_tests(mock);
        let err = invoke.run(base_input()).await.unwrap_err();

        match err {
            DomainErrors::UnhandledContractError(msg) => {
                assert!(msg.contains("PegoutTxidNotFound"));
                assert!(msg.contains(&expected_tx_id));
            }
            _ => panic!("Expected TransactionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_run_invalid_hash() {
        let mut mock = MockPegManagerContractApi::new();
        mock.expect_invoke_trigger_operator_take().times(0);

        let invoke = TriggerOperatorTakeInvoke::new_for_tests(mock);
        let bad_input = TriggerOperatorTakeInput { pegout_txid: "not-a-hash".to_string() };

        let err = invoke.run(bad_input).await.unwrap_err();
        match err {
            DomainErrors::InvalidValue(msg) => {
                assert!(msg.contains("pegout_txid"));
            }
            _ => panic!("Expected InvalidValue error"),
        }
    }

    fn base_input() -> TriggerOperatorTakeInput {
        TriggerOperatorTakeInput { pegout_txid: format!("0x{}", "11".repeat(32)) }
    }
}
