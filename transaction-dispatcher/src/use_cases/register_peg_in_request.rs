use crate::contracts::bitcoin_manager::BitcoinTransaction;
use crate::contracts::peg_manager;
use crate::contracts::peg_manager::SolPegManager::PegInRequestTxSPVProof;
use crate::contracts::peg_manager::{PegManagerContractApi, PegManagerErrors};
use alloy_contract::Error::TransportError;
use alloy_provider::network::EthereumWallet;
use alloy_rpc_types::TransactionReceipt;
use anyhow::Result;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct RegisterPegInInput {
    pub(crate) block_hash: String,
    pub(crate) btc_tx: BitcoinTransaction,
    pub(crate) merkle_branch_path: String,
    pub(crate) merkle_branch_hashes: Vec<String>,
}

pub(crate) struct RegisterPegInRequestInvoke<C: PegManagerContractApi> {
    contract: Arc<C>,
    signer: EthereumWallet,
}

impl<C: PegManagerContractApi> RegisterPegInRequestInvoke<C> {
    pub(crate) fn new(contract: Arc<C>, signer: EthereumWallet) -> Self {
        RegisterPegInRequestInvoke { contract, signer }
    }

    pub(crate) async fn run(
        &self,
        input: RegisterPegInInput,
    ) -> Result<TransactionReceipt, PegManagerErrors> {
        let parsed_input: PegInRequestTxSPVProof = input.try_into().map_err(|e| {
            error!("Failed to parse RegisterPegInInput: {}", e);
            PegManagerErrors::InternalError
        })?;

        self.do_call(parsed_input.clone()).await?;

        let result = self
            .contract
            .register_peg_in_request_send(&self.signer, parsed_input)
            .await;
        match result {
            Ok(r) => {
                if r.status() {
                    info!(
                        "RegisterPegInRequest successful at tx {}",
                        r.transaction_hash
                    );
                    Ok(r) // TODO(iago) return legacy receipt (without blob, etc.)
                } else {
                    error!(
                        "RegisterPegInRequest failed after successful call at tx {}",
                        r.transaction_hash
                    );
                    Err(PegManagerErrors::InternalError)
                }
            }
            Err(e) => {
                error!("Error sending PegInRequest: {}", e);
                Err(PegManagerErrors::InternalError)
            }
        }
    }

    async fn do_call(&self, parsed_input: PegInRequestTxSPVProof) -> Result<(), PegManagerErrors> {
        let result = self
            .contract
            .register_peg_in_request_call(parsed_input)
            .await;

        match result {
            Ok(_) => {
                debug!("RegisterPegInRequest call worked fine");
                Ok(())
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => Err(peg_manager::decode_contract_error(e)),
                None => {
                    error!("Missing ErrorPayload in PegManager error {:?}", err);
                    Err(PegManagerErrors::InternalError)
                }
            },
            Err(e) => {
                error!("Error calling PegManager: {:?}", e);
                Err(PegManagerErrors::InternalError)
            }
        }
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use crate::contracts::peg_manager::MockPegManagerContractApi;
    use crate::use_cases::register_peg_in_request::RegisterPegInRequestInvoke;
    use std::sync::Arc;

    #[cfg(test)]
    impl RegisterPegInRequestInvoke<MockPegManagerContractApi> {
        pub(crate) fn new_for_tests(contract: MockPegManagerContractApi) -> Self {
            RegisterPegInRequestInvoke {
                contract: Arc::new(contract),
                signer: Default::default(),
            }
        }
    }

    // TODO(iago) añadir tests
}
