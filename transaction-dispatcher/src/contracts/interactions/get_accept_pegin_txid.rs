use alloy_primitives::FixedBytes;
use log::info;

use crate::contracts::pegout_manager::PegoutManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{GetAcceptPeginTxidInput, GetAcceptPeginTxidOutput};

#[derive(Clone)]
pub(crate) struct GetAcceptPeginTxidCall<C: PegoutManagerContractApi> {
    contract: C,
}

impl<C: PegoutManagerContractApi> GetAcceptPeginTxidCall<C> {
    pub(crate) fn new(contract: C) -> Self {
        GetAcceptPeginTxidCall { contract }
    }

    pub(crate) async fn run(
        &self,
        input: GetAcceptPeginTxidInput,
    ) -> Result<GetAcceptPeginTxidOutput, DomainErrors> {
        info!("Init GetAcceptPeginTxid for pegout_txid: {:?}", input.pegout_txid);

        let accept_pegin_txid =
            self.contract.call_get_accept_pegin_txid(input.pegout_txid).await.map_err(|e| {
                DomainErrors::UnhandledContractError(format!(
                    "Failed to get accept pegin txid: {e}"
                ))
            })?;

        if accept_pegin_txid == FixedBytes::ZERO {
            return Err(DomainErrors::AcceptPeginTxidNotFound(
                "No accept pegin txid found for the given pegout txid".to_string(),
            ));
        }

        info!("GetAcceptPeginTxid successful: {accept_pegin_txid:?}");

        Ok(GetAcceptPeginTxidOutput { accept_pegin_txid })
    }
}
