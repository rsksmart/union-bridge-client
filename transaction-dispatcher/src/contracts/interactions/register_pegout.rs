use crate::contracts::peg_manager::PegManagerContractApi;
use crate::rsk_gateway::DomainErrors;
use crate::types::{RegisterPegoutInput, RegisterPegoutOutput};
use anyhow::Result;
use log::{error, info};
use union_contracts::bindings::peg_manager::PegManager::BtcTxSPVProof;

#[derive(Clone)]
pub(crate) struct RegisterPegoutInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterPegoutInvoke<C> {
    pub(crate) fn new(contract: C, gas_bumps: u8) -> Self {
        RegisterPegoutInvoke {
            contract,
            gas_bumps,
        }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegoutInput,
    ) -> Result<RegisterPegoutOutput, DomainErrors> {
        info!("Init RegisterPegout for: {:?}", input);

        let parsed_input: BtcTxSPVProof = input.try_into().map_err(|e| {
            DomainErrors::InvalidBtcTxSpvProof(format!(
                "Failed to parse RegisterPegoutInput: {}",
                e
            ))
        })?;

        let receipt = self
            .contract
            .invoke_register_pegout(parsed_input, self.gas_bumps)
            .await?;

        let result = RegisterPegoutOutput {
            transaction_hash: receipt.transaction_hash.to_string(),
            success: receipt.status(),
        };

        if result.success {
            info!(
                "invoke_register_pegout successful at tx {}",
                receipt.transaction_hash
            );
        } else {
            error!(
                "invoke_register_pegout failed at tx {}",
                receipt.transaction_hash
            );
        }
        Ok(result)
    }
}

//todo test
