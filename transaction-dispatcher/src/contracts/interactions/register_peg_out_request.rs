use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterPegOutInput, RegisterPegOutOutput};
use anyhow::Result;
use log::{error, info};
use union_contracts::bindings::peg_manager::PegManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct RegisterPegOutRequestInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterPegOutRequestInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterPegOutRequestInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegOutInput,
    ) -> Result<RegisterPegOutOutput, DomainErrors> {
        info!("Init RegisterPegOut for: {:?}", input);

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterPegOutInput: {}",
                e
            ))
        })?;

        let receipt = self
            .contract
            .register_peg_out_request_send(parsed_input, self.gas_bumps)
            .await?;

        let result = RegisterPegOutOutput {
            transaction_hash: receipt.transaction_hash.to_string(),
            success: receipt.status(),
        };

        if result.success {
            info!(
                "RegisterPegOutRequest successful at tx {}",
                receipt.transaction_hash
            );
        } else {
            error!(
                "RegisterPegOutRequest failed at tx {}",
                receipt.transaction_hash
            );
        }
        Ok(result)
    }
}

//todo test
