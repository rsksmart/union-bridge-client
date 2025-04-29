use crate::{
    contracts::peg_manager::PegManagerContractApi,
    rsk_gateway::PegManagerErrors,
    types::{RegisterPegOutInput, RegisterPegOutOutput},
};
use alloy_primitives::FixedBytes;
use anyhow::Result;
use log::{debug, error, info};

pub struct RegisterPegOutRequestInvoke<C: PegManagerContractApi> {
    contract: C,
    gas_bumps: u8,
}

impl<C: PegManagerContractApi> RegisterPegOutRequestInvoke<C> {
    pub fn new(contract: C, gas_bumps: u8) -> Self {
        Self {
            contract,
            gas_bumps,
        }
    }

    pub async fn run(
        &self,
        input: RegisterPegOutInput,
    ) -> Result<RegisterPegOutOutput, PegManagerErrors> {
        info!("Init RegisterPegOut for: {:?}", input);

        let msg_value = input.amount_in_wei;

        let usr_pub_key: FixedBytes<33> =
            input.usr_pub_key.parse::<FixedBytes<33>>().map_err(|e| {
                PegManagerErrors::InvalidPublicKey(format!("Failed to parse usr_pub_key: {}", e))
            })?;

        let batch_flag = input.batch_flag;

        debug!(
            "Calling register_peg_out_request_send: value = {}, usr_pub_key = {:?}, batch_flag = {}, gas_bumps = {}",
            msg_value, usr_pub_key, batch_flag, self.gas_bumps
        );

        let receipt = self
            .contract
            .register_peg_out_request_send(msg_value, usr_pub_key, batch_flag, self.gas_bumps)
            .await?;

        let result = if receipt.status() {
            info!(
                "RegisterPegInRequest successful at tx {}",
                receipt.transaction_hash
            );
            RegisterPegOutOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
                success: true,
            }
        } else {
            error!(
                "RegisterPegInRequest failed at tx {}",
                receipt.transaction_hash
            );
            RegisterPegOutOutput {
                transaction_hash: receipt.transaction_hash.to_string(),
                success: false,
            }
        };

        Ok(result)
    }
}
