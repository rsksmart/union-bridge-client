use anyhow::Result;
use tracing::info;
use union_contracts::bindings::pegin_manager::PeginManager::BtcTxSPVProof;

use crate::contracts::pegin_manager::PeginManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RejectPeginInput, RejectPeginOutput};

#[derive(Clone)]
pub(crate) struct RejectPeginInvoke<C: PeginManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PeginManagerContractApi> RejectPeginInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RejectPeginInvoke { contract, gas_bumps }
    }

    pub(crate) async fn run(
        &self,
        input: RejectPeginInput,
    ) -> Result<RejectPeginOutput, DomainErrors> {
        info!("Init RejectPeginInvoke for: {input:?}");

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!("Failed to parse RejectPeginInput: {e}"))
        })?;

        let tx_hash = self.contract.invoke_reject_pegin(parsed_input, self.gas_bumps).await?;

        info!("RejectPegin successful at tx {tx_hash}");
        Ok(RejectPeginOutput { transaction_hash: tx_hash.to_string() })
    }
}
