use crate::rsk_connector::PegManager::PegManagerErrors;
use crate::types::{PeginAddressInput, PeginAddressOutput};
use alloy_contract::Error::TransportError;
use alloy_json_rpc::ErrorPayload;
use alloy_primitives::{Address, FixedBytes};
use alloy_provider::{ProviderBuilder, RootProvider, WsConnect};
use alloy_sol_types::SolInterface;
use alloy_sol_types::sol;
use anyhow::{Result, anyhow, bail};
use log::{debug, error, info};

sol!(
    #[sol(rpc)]
    PegManager,
    "/Users/illuque/workspace/union-bridge/fairgate/bitvmx-union-bridge-contracts/out/PegManager.sol/PegManager.json"
);

// TODO(iago) dynamically get path from config
// fn get_contract_path() -> String {
//     let settings = Config::builder()
//         .add_source(config::File::with_name("Settings"))
//         .build()
//         .unwrap();
//     settings.get_string("contract_path").unwrap()
// }

// TODO(iago) extract one class for each contract

pub async fn get_temporary_pegin_address(input: PeginAddressInput) -> Result<PeginAddressOutput> {
    let ws_url = "ws://127.0.0.1:8545";
    let ws = WsConnect::new(ws_url);
    let provider: RootProvider = ProviderBuilder::default().on_ws(ws).await?;

    info!("Connected to Rootstock at {ws_url}");

    let contract_address: Address = "0x21df544947ba3e8b3c32561399e88b52dc8b2823".parse()?; // TODO(iago) config
    let contract = PegManager::new(contract_address, provider);

    info!("Interacting with PegManager @ {contract_address}");

    let rootstock_deposit_address: Address = input.rootstock_deposit_address.parse()?;
    let value = input.value;
    let btc_reimbursement_pub_key: FixedBytes<32> = input.btc_reimbursement_pub_key.parse()?;

    let call_builder = contract.getTemporaryPegInAddress(
        rootstock_deposit_address,
        value,
        btc_reimbursement_pub_key,
    );

    let result = call_builder.call().await;

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
        Err(TransportError(err)) => {
            let error_resp = match err.as_error_resp() {
                Some(e) => e,
                None => {
                    bail!("Missing ErrorPayload in PegManager error {:?}", err)
                }
            };
            Err(decode_contract_error(error_resp))
        }
        Err(e) => Err(anyhow!("An unknown error occurred: {:?}", e)),
    }
}

fn decode_contract_error(error_payload: &ErrorPayload) -> anyhow::Error {
    let revert_data = error_payload.as_revert_data();
    if revert_data.is_none() {
        error!("No revert data found in PegManager error {error_payload}");
        return anyhow!("Could not derive PegManager error from ErrorPayload");
    }

    let decoded_error = PegManagerErrors::abi_decode(&revert_data.unwrap(), true);
    if decoded_error.is_err() {
        error!("Could not decode PegManager error {error_payload}");
        return anyhow!("Could not decode PegManager error {error_payload}");
    }

    // TODO(iago) handle each error type properly
    match decoded_error.unwrap() {
        PegManagerErrors::AddressEmptyCode(e) => {
            error!("AddressEmptyCode {}", e.target);
        }
        PegManagerErrors::AlreadyRegisteredPegIn(e) => {
            error!("AlreadyRegisteredPegIn {}", e.btcTxHash);
        }
        PegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
            error!("BridgeBtcBlockNotInBestChain {}", e.blockHash);
        }
        PegManagerErrors::BridgeBtcBlockTooOld(e) => {
            error!("BridgeBtcBlockTooOld {}", e.maxDepth);
        }
        PegManagerErrors::BridgeBtcInconsistentBlock(e) => {
            error!("BridgeBtcInconsistentBlock {}", e.blockHash);
        }
        PegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
            error!("BridgeBtcInexistantBlockHash {}", e.blockHash);
        }
        PegManagerErrors::BridgeBtcTxInvalidMerkleBranch(e) => {
            error!(
                "Error: BridgeBtcTxInvalidMerkleBranch {} - {} - {:?}",
                e.txHash, e.merkleBranchPath, e.merkleBranchHashes
            );
        }
        PegManagerErrors::BridgeBtcUnknownError(e) => {
            error!("BridgeBtcUnknownError {}", e.errorCode);
        }
        PegManagerErrors::ERC1967InvalidImplementation(e) => {
            error!("ERC1967InvalidImplementation {}", e.implementation);
        }
        PegManagerErrors::ERC1967NonPayable(_e) => {
            error!("ERC1967NonPayable");
        }
        PegManagerErrors::FailedCall(_e) => {
            error!("FailedCall");
        }
        PegManagerErrors::InvalidInitialization(_e) => {
            error!("InvalidInitialization");
        }
        PegManagerErrors::NoEmptySlot(e) => {
            error!("NoEmptySlot {} - {}", e.packetNumber, e.streamId);
        }
        PegManagerErrors::NotEnoughConfirmations(e) => {
            error!(
                "Error: NotEnoughConfirmations {} - {}",
                e.expected, e.actual
            );
        }
        PegManagerErrors::NotInitializing(_e) => {
            error!("NotInitializing");
        }
        PegManagerErrors::OwnableInvalidOwner(e) => {
            error!("OwnableInvalidOwner {}", e.owner);
        }
        PegManagerErrors::OwnableUnauthorizedAccount(e) => {
            error!("OwnableUnauthorizedAccount {}", e.account);
        }
        PegManagerErrors::PacketOutOfBound(e) => {
            error!("PacketOutOfBound {}", e.packetNumber);
        }
        PegManagerErrors::StreamNotFoundByDenomination(e) => {
            error!("StreamNotFoundByDenomination {}", e.denomination);
        }
        PegManagerErrors::UUPSUnauthorizedCallContext(_e) => {
            error!("UUPSUnauthorizedCallContext");
        }
        PegManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
            error!("UUPSUnsupportedProxiableUUID {}", e.slot);
        }
    }

    anyhow!("Expected error interacting with PegManager")
}
