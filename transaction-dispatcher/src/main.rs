use crate::PegManager::PegManagerErrors;
use alloy_contract::Error::TransportError;
use alloy_primitives::{FixedBytes, address, fixed_bytes};
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_sol_types::SolInterface;
use alloy_sol_types::sol;
use anyhow::Result;

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

#[tokio::main]
async fn main() -> Result<()> {
    let ws = WsConnect::new("ws://127.0.0.1:8545");
    let provider: RootProvider = ProviderBuilder::default().on_ws(ws).await?;
    println!("block_number: {:?}", provider.get_block_number().await?);

    let contract = PegManager::new(
        address!("0x21df544947ba3e8b3c32561399e88b52dc8b2823"),
        provider,
    );

    let rootstock_deposit_address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");
    let value = 100_000_000;
    let btc_reimbursement_pub_key: FixedBytes<32> =
        fixed_bytes!("0xc72a9f6fc8e57f1de528a48b6c4ad7a6db30b24a7bbf8cdd74b0a3b248b6f7f1");

    let call = contract.getTemporaryPegInAddress(
        rootstock_deposit_address,
        value,
        btc_reimbursement_pub_key,
    );

    let result = call.call().await;

    match result {
        Ok(data) => {
            println!(
                "Bitcoin Deposit Address for input [{} - {} - {}]: {}",
                rootstock_deposit_address,
                value,
                btc_reimbursement_pub_key,
                data.bitcoinDepositAddress
            )
        }
        Err(TransportError(err)) => {
            // TODO(iago) improve this line
            let expected =
                decode_contract_error(&err.as_error_resp().unwrap().as_revert_data().unwrap());
            if !expected {
                println!("Error: {:?}", err);
            }
        }
        Err(e) => println!("Unknown error: {:?}", e),
    }

    Ok(())
}

fn decode_contract_error(data: &[u8]) -> bool {
    if let Ok(decoded_error) = PegManagerErrors::abi_decode(data, true) {
        match decoded_error {
            PegManagerErrors::AddressEmptyCode(e) => {
                println!("Error: AddressEmptyCode {}", e.target);
            }
            PegManagerErrors::AlreadyRegisteredPegIn(e) => {
                println!("Error: AlreadyRegisteredPegIn {}", e.btcTxHash);
            }
            PegManagerErrors::BridgeBtcBlockNotInBestChain(e) => {
                println!("Error: BridgeBtcBlockNotInBestChain {}", e.blockHash);
            }
            PegManagerErrors::BridgeBtcBlockTooOld(e) => {
                println!("Error: BridgeBtcBlockTooOld {}", e.maxDepth);
            }
            PegManagerErrors::BridgeBtcInconsistentBlock(e) => {
                println!("Error: BridgeBtcInconsistentBlock {}", e.blockHash);
            }
            PegManagerErrors::BridgeBtcInexistantBlockHash(e) => {
                println!("Error: BridgeBtcInexistantBlockHash {}", e.blockHash);
            }
            PegManagerErrors::BridgeBtcTxInvalidMerkleBranch(e) => {
                println!(
                    "Error: BridgeBtcTxInvalidMerkleBranch {} - {} - {:?}",
                    e.txHash, e.merkleBranchPath, e.merkleBranchHashes
                );
            }
            PegManagerErrors::BridgeBtcUnknownError(e) => {
                println!("Error: BridgeBtcUnknownError {}", e.errorCode);
            }
            PegManagerErrors::ERC1967InvalidImplementation(e) => {
                println!("Error: ERC1967InvalidImplementation {}", e.implementation);
            }
            PegManagerErrors::ERC1967NonPayable(_e) => {
                println!("Error: ERC1967NonPayable");
            }
            PegManagerErrors::FailedCall(_e) => {
                println!("Error: FailedCall");
            }
            PegManagerErrors::InvalidInitialization(_e) => {
                println!("Error: InvalidInitialization");
            }
            PegManagerErrors::NoEmptySlot(e) => {
                println!("Error: NoEmptySlot {} - {}", e.packetNumber, e.streamId);
            }
            PegManagerErrors::NotEnoughConfirmations(e) => {
                println!(
                    "Error: NotEnoughConfirmations {} - {}",
                    e.expected, e.actual
                );
            }
            PegManagerErrors::NotInitializing(_e) => {
                println!("Error: NotInitializing");
            }
            PegManagerErrors::OwnableInvalidOwner(e) => {
                println!("Error: OwnableInvalidOwner {}", e.owner);
            }
            PegManagerErrors::OwnableUnauthorizedAccount(e) => {
                println!("Error: OwnableUnauthorizedAccount {}", e.account);
            }
            PegManagerErrors::PacketOutOfBound(e) => {
                println!("Error: PacketOutOfBound {}", e.packetNumber);
            }
            PegManagerErrors::StreamNotFoundByDenomination(e) => {
                println!("Error: StreamNotFoundByDenomination {}", e.denomination);
            }
            PegManagerErrors::UUPSUnauthorizedCallContext(_e) => {
                println!("Error: UUPSUnauthorizedCallContext");
            }
            PegManagerErrors::UUPSUnsupportedProxiableUUID(e) => {
                println!("Error: UUPSUnsupportedProxiableUUID {}", e.slot);
            }
        }
        true
    } else {
        false
    }
}
