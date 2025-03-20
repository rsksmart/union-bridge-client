use crate::contracts::bitcoin_manager::BitcoinManager::BitcoinManagerErrors;
use crate::contracts::peg_manager::SolPegManager::{
    BtcTransaction, PegInRequestTxSPVProof, SolPegManagerErrors, SolPegManagerInstance,
    getTemporaryPegInAddressReturn, registerPegInRequestCall, registerPegInRequestReturn,
};
use alloy_contract::CallBuilder;
use alloy_contract::Error::TransportError;
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::hex::FromHex;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_provider::Provider;
use alloy_provider::network::{EthereumWallet, ReceiptResponse, TxSigner};
use alloy_rpc_types::TransactionReceipt;
use alloy_sol_types::{SolInterface, sol};
use anyhow::{Context, Result, bail};
use log::{debug, error, info, warn};
use std::marker::PhantomData;
use thiserror::Error;

use crate::contracts::bitcoin_manager::{BitcoinTransaction, ParseFieldError};
#[cfg(feature = "testing")]
use mockall::automock;
use serde::{Deserialize, Serialize};
// TODO(iago) refactor peg_manager to several files

#[cfg_attr(feature = "testing", automock)]
pub trait ContractApi {
    #[allow(async_fn_in_trait)]
    async fn get_temporary_pegin_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn>;

    #[allow(async_fn_in_trait)]
    async fn register_pegin_request_send(
        &self,
        signer: &EthereumWallet,
        input: PegInRequestTxSPVProof,
    ) -> Result<TransactionReceipt>;

    #[allow(async_fn_in_trait)]
    async fn register_pegin_request_call(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn>;
}

#[derive(Serialize, Deserialize, Debug)]
// TODO(iago) use alloy type, otherwise this will grow a lot (and this way it automatically reacts to abi changes)
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String,
    pub value: u64,
    pub btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
// TODO(iago) use alloy type, otherwise this will grow a lot (and this way it automatically reacts to abi changes)
pub struct PeginAddressOutput {
    pub address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RegisterPeginInput {
    pub block_hash: String,
    pub btc_tx: BitcoinTransaction,
    pub merkle_branch_path: String,
    pub merkle_branch_hashes: Vec<String>,
}

impl TryFrom<RegisterPeginInput> for PegInRequestTxSPVProof {
    type Error = ParseFieldError;

    fn try_from(value: RegisterPeginInput) -> Result<Self, Self::Error> {
        let block_hash =
            FixedBytes::<32>::from_hex(&value.block_hash).map_err(ParseFieldError::ParseHex)?;

        let btc_tx: BtcTransaction = value.btc_tx.try_into()?;

        let merkle_branches_hashes = value
            .merkle_branch_hashes
            .into_iter()
            .map(|hash| {
                hash.parse::<FixedBytes<32>>()
                    .map_err(ParseFieldError::ParseHex)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let merkle_branch_path =
            U256::from_str_radix(&value.merkle_branch_path.trim_start_matches("0x"), 16)
                .map_err(ParseFieldError::ParseNum)?;

        Ok(PegInRequestTxSPVProof {
            blockHash: block_hash,
            btcTx: btc_tx,
            merkleBranchPath: merkle_branch_path,
            merkleBranchHashes: merkle_branches_hashes,
        })
    }
}

sol!(
    #[sol(rpc)]
    SolPegManager,
    "../config/dev/abi/PegManager.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);

// needed so we can create a PegManagerApi trait for tests mocking
pub struct ContractWrapper<P: Provider> {
    inner: SolPegManagerInstance<(), P>,
}

impl<P: Provider> ContractApi for ContractWrapper<P> {
    #[allow(non_snake_case)]
    async fn get_temporary_pegin_address_call(
        &self,
        rootstock_deposit_address: Address,
        value: u64,
        btc_reimbursement_pub_key: FixedBytes<32>,
    ) -> alloy_contract::Result<getTemporaryPegInAddressReturn> {
        self.inner
            .getTemporaryPegInAddress(rootstock_deposit_address, value, btc_reimbursement_pub_key)
            .call()
            .await
    }

    #[allow(non_snake_case)]
    async fn register_pegin_request_send(
        &self,
        signer: &EthereumWallet,
        input: PegInRequestTxSPVProof,
    ) -> Result<TransactionReceipt> {
        let chain_id = self
            .inner
            .provider()
            .get_chain_id()
            .await
            .context("getting chain id")?;

        let nonce = self
            .inner
            .provider()
            .get_transaction_count(signer.default_signer().address())
            .await?;

        let tx_builder = self
            .inner
            .registerPegInRequest(input)
            .chain_id(chain_id)
            .nonce(nonce);

        let mut estimated_gas = tx_builder.estimate_gas().await.context("estimating gas")?;

        // TODO(iago) make the retries configurable
        for _ in 0..3 {
            let receipt = self
                .send_with_gas(tx_builder.clone(), estimated_gas)
                .await
                .context("getting receipt")?;

            debug!("Transaction receipt: {:?}", receipt);

            if receipt.status() {
                return Ok(receipt);
            } else if receipt.gas_used() >= estimated_gas {
                warn!("Bumping transaction gas");
                estimated_gas = (estimated_gas as f64 * 1.1) as u64; // TODO(iago) 1.1 configurable
            }
        }

        bail!("Failed to call registerPegInRequest")
    }

    #[allow(non_snake_case)]
    async fn register_pegin_request_call(
        &self,
        input: PegInRequestTxSPVProof,
    ) -> alloy_contract::Result<registerPegInRequestReturn> {
        self.inner.registerPegInRequest(input).call().await
    }
}

impl<P: Provider> ContractWrapper<P> {
    async fn send_with_gas(
        &self,
        tx_builder: CallBuilder<(), &P, PhantomData<registerPegInRequestCall>>,
        gas_to_use: u64,
    ) -> Result<TransactionReceipt> {
        let gas_price = self
            .inner
            .provider()
            .get_gas_price()
            .await
            .context("getting gas price")?;

        let pending_tx_builder = tx_builder
            .gas_price(gas_price)
            .gas(gas_to_use)
            .send()
            .await
            .context("sending tx")?;

        let receipt = pending_tx_builder
            .get_receipt()
            .await
            .context("getting receipt")?;

        Ok(receipt)
    }
}

pub struct PegManagerGateway<I>
where
    I: ContractApi,
{
    signer: EthereumWallet,
    contract_address: Address,
    instance: I,
}

impl<P: Provider> PegManagerGateway<ContractWrapper<P>> {
    pub fn init(provider: P, signer: EthereumWallet, contract_address: Address) -> Result<Self> {
        let instance = SolPegManager::new(contract_address, provider);

        Ok(PegManagerGateway {
            contract_address,
            signer,
            instance: ContractWrapper { inner: instance },
        })
    }
}

impl<I> PegManagerGateway<I>
where
    I: ContractApi,
{
    pub(crate) async fn get_temporary_pegin_address(
        &self,
        input: PeginAddressInput,
    ) -> Result<PeginAddressOutput, PegManagerErrors> {
        info!("Interacting with PegManager @ {}", self.contract_address);
        let rootstock_deposit_address: Address = input
            .rootstock_deposit_address
            .parse::<Address>()
            .map_err(|e| {
                error!("Failed to parse rootstock_deposit_address: {}", e);
                PegManagerErrors::InvalidAddress
            })?;
        let value = input.value;
        let btc_reimbursement_pub_key: FixedBytes<32> = input
            .btc_reimbursement_pub_key
            .parse::<FixedBytes<32>>()
            .map_err(|e| {
                error!("Failed to parse btc_reimbursement_pub_key: {}", e);
                PegManagerErrors::InvalidPublicKey
            })?;

        let result = self
            .instance
            .get_temporary_pegin_address_call(
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
            )
            .await;

        match result {
            Ok(data) => {
                debug!(
                    "Bitcoin Deposit Address for {:?}: {}",
                    input, data.bitcoinDepositAddress
                );

                Ok(PeginAddressOutput {
                    address: data.bitcoinDepositAddress.to_string(),
                })
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => Err(Self::decode_contract_error(e)),
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

    pub(crate) async fn register_peg_in_request(
        &self,
        input: RegisterPeginInput,
    ) -> Result<(), PegManagerErrors> {
        let parsed_input = match PegInRequestTxSPVProof::try_from(input) {
            Ok(i) => i,
            Err(e) => {
                // TODO(iago) distinguish error types
                error!("Failed to parse RegisterPeginInput: {}", e);
                return Err(PegManagerErrors::InternalError);
            }
        };

        let result = self
            .instance
            .register_pegin_request_send(&self.signer, parsed_input.clone())
            .await;

        match result {
            Ok(r) => {
                if r.status() {
                    info!("RegisterPeginRequest created in tx {}", r.transaction_hash);
                    Ok(())
                } else {
                    self.request_pegin_inspect_error(parsed_input).await
                }
            }
            Err(e) => {
                error!("Error sending PeginRequest: {}", e);
                Err(PegManagerErrors::InternalError)
            }
        }
    }

    async fn request_pegin_inspect_error(
        &self,
        parsed_input: PegInRequestTxSPVProof,
    ) -> Result<(), PegManagerErrors> {
        let result = self
            .instance
            .register_pegin_request_call(parsed_input)
            .await;

        match result {
            Ok(_) => {
                // TODO properly handle
                error!("RegisterPeginRequest Call worked but Send failed");
                Err(PegManagerErrors::InternalError)
            }
            Err(TransportError(err)) => match err.as_error_resp() {
                Some(e) => Err(Self::decode_contract_error(e)),
                None => {
                    // TODO(iago) handle errors properly
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

    fn decode_contract_error(error_payload: &ErrorPayload) -> PegManagerErrors {
        let revert_data = if let Some(data) = error_payload.as_revert_data() {
            data
        } else {
            error!("No revert data found in PegManager error {error_payload}");
            return PegManagerErrors::InternalError;
        };

        if let Some(err) = Self::decode_peg_manager_error(&revert_data) {
            return err;
        }

        if let Some(err) = Self::decode_bitcoin_manager_error(&revert_data) {
            return err;
        }

        error!("Unknown error on PegManager: {:?}", error_payload);
        PegManagerErrors::InternalError
    }

    fn decode_peg_manager_error(revert_data: &Bytes) -> Option<PegManagerErrors> {
        let decoded_error = SolPegManagerErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            let decoded_error = decoded_error.unwrap();
            return Some(match decoded_error {
                SolPegManagerErrors::AddressEmptyCode(e) => {
                    error!("SolPegManagerErrors#AddressEmptyCode {}", e.target);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::AlreadyRegisteredPegIn(e) => {
                    error!("SolPegManagerErrors#AlreadyRegisteredPegIn {}", e.btcTxHash);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
                    error!(
                        "SolPegManagerErrors#BridgeBtcBlockNotInBestChain {}",
                        e.blockHash
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcBlockTooOld(e) => {
                    error!("SolPegManagerErrors#BridgeBtcBlockTooOld {}", e.maxDepth);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcInconsistentBlock(e) => {
                    error!(
                        "SolPegManagerErrors#BridgeBtcInconsistentBlock {}",
                        e.blockHash
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
                    error!(
                        "SolPegManagerErrors#BridgeBtcInexistantBlockHash {}",
                        e.blockHash
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcTxInvalidMerkleBranch(e) => {
                    error!(
                        "SolPegManagerErrors#BridgeBtcTxInvalidMerkleBranch {} - {} - {:?}",
                        e.txHash, e.merkleBranchPath, e.merkleBranchHashes
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::BridgeBtcUnknownError(e) => {
                    error!("SolPegManagerErrors#BridgeBtcUnknownError {}", e.errorCode);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::ERC1967InvalidImplementation(e) => {
                    error!(
                        "SolPegManagerErrors#ERC1967InvalidImplementation {}",
                        e.implementation
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::ERC1967NonPayable(_) => {
                    error!("SolPegManagerErrors#ERC1967NonPayable");
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::FailedCall(_) => {
                    error!("SolPegManagerErrors#FailedCall");
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::InvalidInitialization(_) => {
                    error!("SolPegManagerErrors#InvalidInitialization");
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::NoEmptySlot(e) => {
                    error!(
                        "SolPegManagerErrors#NoEmptySlot {} - {}",
                        e.packetNumber, e.streamId
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::NotEnoughConfirmations(e) => {
                    error!(
                        "SolPegManagerErrors#NotEnoughConfirmations {} - {}",
                        e.expected, e.actual
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::NotInitializing(_) => {
                    error!("SolPegManagerErrors#NotInitializing");
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::OwnableInvalidOwner(e) => {
                    error!("SolPegManagerErrors#OwnableInvalidOwner {}", e.owner);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::OwnableUnauthorizedAccount(e) => {
                    error!(
                        "SolPegManagerErrors#OwnableUnauthorizedAccount {}",
                        e.account
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::PacketOutOfBound(e) => {
                    error!("SolPegManagerErrors#PacketOutOfBound {}", e.packetNumber);
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::StreamNotFoundByDenomination(e) => {
                    error!(
                        "SolPegManagerErrors#StreamNotFoundByDenomination {}",
                        e.denomination
                    );
                    PegManagerErrors::StreamNotFoundByDenomination
                }
                SolPegManagerErrors::tooManyDenominations(e) => {
                    error!(
                        "SolPegManagerErrors#tooManyDenominations {}",
                        e.maxDenominationsSize
                    );
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::UUPSUnauthorizedCallContext(_) => {
                    error!("SolPegManagerErrors#UUPSUnauthorizedCallContext");
                    PegManagerErrors::InternalError
                }
                SolPegManagerErrors::UUPSUnsupportedProxiableUUID(_) => {
                    error!("SolPegManagerErrors#UUPSUnsupportedProxiableUUID");
                    PegManagerErrors::InternalError
                }
            });
        }
        None
    }

    fn decode_bitcoin_manager_error(revert_data: &Bytes) -> Option<PegManagerErrors> {
        let decoded_error = BitcoinManagerErrors::abi_decode(&revert_data, true);
        if decoded_error.is_ok() {
            return Some(match decoded_error.unwrap() {
                BitcoinManagerErrors::AddressEmptyCode(e) => {
                    error!("SolBitcoinManagerErrors#AddressEmptyCode {}", e.target);
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::FailedCall(_) => {
                    error!("SolBitcoinManagerErrors#FailedCall");
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::IncorrectOutputNumber(e) => {
                    error!(
                        "SolBitcoinManagerErrors#IncorrectOutputNumber actual: {}, expected: {}",
                        e.actual, e.expected
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::IncorrectP2TRScriptPub(e) => {
                    error!(
                        "SolBitcoinManagerErrors#IncorrectP2TRScriptPub actual: {:?}, expected: {:?}",
                        e.actual, e.expected
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::IncorrectlyFormedOpReturn(e) => {
                    error!(
                        "SolBitcoinManagerErrors#IncorrectlyFormedOpReturn index: {}",
                        e.index
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::InvalidAddress(e) => {
                    error!("SolBitcoinManagerErrors#InvalidAddress {}", e._address);
                    PegManagerErrors::InvalidAddress
                }
                BitcoinManagerErrors::InvalidInitialization(_) => {
                    error!("SolBitcoinManagerErrors#InvalidInitialization");
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::InvalidOpReturnLength(e) => {
                    error!(
                        "SolBitcoinManagerErrors#InvalidOpReturnLength actual: {}, expected: {}",
                        e.actual, e.expected
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::InvalidPublicKey(e) => {
                    error!("SolBitcoinManagerErrors#InvalidPublicKey {}", e.publicKey);
                    PegManagerErrors::InvalidPublicKey
                }
                BitcoinManagerErrors::InvalidValue(e) => {
                    error!("SolBitcoinManagerErrors#InvalidValue {}", e._value);
                    PegManagerErrors::InvalidValue
                }
                BitcoinManagerErrors::NotInitializing(_) => {
                    error!("SolBitcoinManagerErrors#NotInitializing");
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::NumberTooLarge(e) => {
                    error!(
                        "SolBitcoinManagerErrors#NumberTooLarge actual: {}, max: {}",
                        e.actual, e.max
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::OwnableInvalidOwner(e) => {
                    error!("SolBitcoinManagerErrors#OwnableInvalidOwner {}", e.owner);
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::OwnableUnauthorizedAccount(e) => {
                    error!(
                        "SolBitcoinManagerErrors#OwnableUnauthorizedAccount {}",
                        e.account
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::UUPSUnauthorizedCallContext(_) => {
                    error!("SolBitcoinManagerErrors#UUPSUnauthorizedCallContext");
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
                    error!(
                        "SolBitcoinManagerErrors#UUPSUnsupportedProxiableUUID slot: {:?}",
                        e.slot
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::indexOverflow(e) => {
                    error!(
                        "SolBitcoinManagerErrors#indexOverflow length: {}, from: {}, upTo: {}",
                        e.length, e.from, e.upTo
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::ERC1967InvalidImplementation(e) => {
                    error!(
                        "SolBitcoinManagerErrors#ERC1967InvalidImplementation {}",
                        e.implementation
                    );
                    PegManagerErrors::InternalError
                }
                BitcoinManagerErrors::ERC1967NonPayable(_) => {
                    error!("SolBitcoinManagerErrors#ERC1967NonPayable");
                    PegManagerErrors::InternalError
                }
            });
        }
        None
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum PegManagerErrors {
    #[error("Internal Error")]
    InternalError,
    #[error("Stream not found by denomination")]
    StreamNotFoundByDenomination,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid address")]
    InvalidAddress,
    #[error("Invalid value")]
    InvalidValue,
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use crate::contracts::bitcoin_manager::BitcoinManager::{
        BitcoinManagerErrors, InvalidAddress, InvalidPublicKey,
    };
    use crate::contracts::peg_manager::SolPegManager::{
        AlreadyRegisteredPegIn, SolPegManagerErrors, StreamNotFoundByDenomination,
        getTemporaryPegInAddressReturn,
    };
    use crate::contracts::peg_manager::{
        MockContractApi, PegManagerErrors, PegManagerGateway, PeginAddressInput,
    };
    use alloy_contract::Error::TransportError;
    use alloy_json_rpc::ErrorPayload;
    use alloy_json_rpc::RpcError::ErrorResp;
    use alloy_primitives::Address;
    use alloy_primitives::FixedBytes;
    use alloy_provider::network::EthereumWallet;
    use alloy_sol_types::SolInterface;
    use mockall::predicate::{always, eq};

    const CONTRACT_ADDRESS: &str = "0x8c86ead50dc378858163debca4b59b039943f05d";
    const VALID_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const VALID_PUB_KEY: &str =
        "0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1";
    const VALID_VALUE: u64 = 1000;

    const ERROR_TEMPLATE: &str =
        r#"{"code":3,"message":"execution reverted:","data":"<to_replace>"}"#;

    #[tokio::test]
    async fn test_get_temporary_pegin_address_success() {
        let mut mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };
        let expected_deposit_address = "0xfake0deposit0address".to_string();
        let output = getTemporaryPegInAddressReturn {
            bitcoinDepositAddress: expected_deposit_address.clone(),
        };

        mock_instance
            .expect_get_temporary_pegin_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| Ok(output.clone()))
            .times(1);

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().address, expected_deposit_address);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_preliminary_validation() {
        let mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: "0xinvalid_address".to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_address_smart_contract_raised() {
        let mut mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            // it has to be valid here in order to pass the preliminary validation (non SC)
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_pegin_address_call()
            .with(
                always(),
                eq(VALID_VALUE),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidAddress(InvalidAddress {
                    _address: Address::default(),
                });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidAddress);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_preliminary_validation() {
        let mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            btc_reimbursement_pub_key: "0xinvalid_pub_key".to_string(),
        };

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_invalid_public_key_smart_contract_raised() {
        let mut mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_pegin_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err = BitcoinManagerErrors::InvalidPublicKey(InvalidPublicKey {
                    publicKey: FixedBytes::<32>::default(),
                });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InvalidPublicKey);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_internal_server_error() {
        let mut mock_instance = MockContractApi::new();

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: VALID_VALUE,
            // it has to be valid here in order to pass the preliminary validation (non SC)
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_pegin_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(VALID_VALUE),
                always(),
            )
            .returning(move |_, _, _| {
                let expected_err =
                    SolPegManagerErrors::AlreadyRegisteredPegIn(AlreadyRegisteredPegIn {
                        btcTxHash: FixedBytes::<32>::default(),
                    });
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), PegManagerErrors::InternalError);
    }

    #[tokio::test]
    async fn test_get_temporary_pegin_address_stream_not_found_by_denomination_smart_contract_raised()
     {
        let mut mock_instance = MockContractApi::new();

        // just to make it clear that is invalid, but we do not care about the value as we force the SC to error
        let invalid_value = 2;

        let input = PeginAddressInput {
            rootstock_deposit_address: VALID_ADDRESS.to_string(),
            value: invalid_value,
            btc_reimbursement_pub_key: VALID_PUB_KEY.to_string(),
        };

        mock_instance
            .expect_get_temporary_pegin_address_call()
            .with(
                eq(VALID_ADDRESS.parse::<Address>().unwrap()),
                eq(invalid_value),
                eq(VALID_PUB_KEY.parse::<FixedBytes<32>>().unwrap()),
            )
            .returning(move |_, _, _| {
                let expected_err = SolPegManagerErrors::StreamNotFoundByDenomination(
                    StreamNotFoundByDenomination {
                        denomination: alloy_primitives::Uint::from(invalid_value),
                    },
                );
                let expected_err_payload = generate_expected_error(expected_err);
                Err(TransportError(ErrorResp(expected_err_payload)))
            })
            .times(1);

        let peg_manager = PegManagerGateway {
            signer: EthereumWallet::default(),
            contract_address: CONTRACT_ADDRESS.parse::<Address>().unwrap(),
            instance: mock_instance,
        };

        let result = peg_manager.get_temporary_pegin_address(input).await;

        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            PegManagerErrors::StreamNotFoundByDenomination
        );
    }

    #[allow(unused)]
    fn init_logger() {
        env_logger::builder().is_test(true).try_init();
    }

    fn generate_expected_error<T: SolInterface>(test: T) -> ErrorPayload {
        let encoded_error = format!("0x{}", hex::encode(test.abi_encode()));
        let error = ERROR_TEMPLATE.replace("<to_replace>", &encoded_error);
        serde_json::from_str::<ErrorPayload>(&error).unwrap()
    }
}
