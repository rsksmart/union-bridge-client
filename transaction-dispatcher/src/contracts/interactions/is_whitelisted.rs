use log::info;

use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::contracts::types::Address;
use crate::rsk_gateway::DomainErrors;

#[derive(Clone)]
pub(crate) struct IsWhitelistedCall<C: CommitteeRegistryContractApi> {
    contract: C,
}

impl<C: CommitteeRegistryContractApi> IsWhitelistedCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        Self { contract }
    }

    pub(crate) async fn run(&self, address: Address) -> Result<bool, DomainErrors> {
        info!("Checking whitelist status for address: {address}");

        let is_whitelisted = self.contract.call_is_whitelisted(address).await.map_err(|e| {
            DomainErrors::UnhandledContractError(format!("Failed to check whitelist status: {e}"))
        })?;

        info!("Whitelist check for {address}: {is_whitelisted}");

        Ok(is_whitelisted)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::address;
    use mockall::predicate::eq;

    use super::*;
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;

    #[tokio::test]
    async fn test_is_whitelisted_returns_true() {
        let mut mock = MockCommitteeRegistryContractApi::new();
        let addr = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");

        mock.expect_call_is_whitelisted().with(eq(addr)).returning(|_| Ok(true)).times(1);

        let call = IsWhitelistedCall::new(mock);
        let result = call.run(addr).await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_is_whitelisted_returns_false() {
        let mut mock = MockCommitteeRegistryContractApi::new();
        let addr = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");

        mock.expect_call_is_whitelisted().with(eq(addr)).returning(|_| Ok(false)).times(1);

        let call = IsWhitelistedCall::new(mock);
        let result = call.run(addr).await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_is_whitelisted_contract_error() {
        let mut mock = MockCommitteeRegistryContractApi::new();
        let addr = address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");

        mock.expect_call_is_whitelisted()
            .with(eq(addr))
            .returning(|_| {
                Err(alloy_contract::Error::TransportError(alloy_json_rpc::RpcError::ErrorResp(
                    alloy_json_rpc::ErrorPayload {
                        code: 3,
                        message: "Contract call failed".to_string().into(),
                        data: None,
                    },
                )))
            })
            .times(1);

        let call = IsWhitelistedCall::new(mock);
        let result = call.run(addr).await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DomainErrors::UnhandledContractError(msg) => {
                assert!(
                    msg.contains("Failed to check whitelist status"),
                    "unexpected error message: {msg}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
