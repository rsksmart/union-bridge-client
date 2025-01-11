use anyhow::{bail, Context, Ok, Result};
use log::{debug, info};
use monitor::provider::{
    AlloyBlockSubscription, AlloyLogsSubscription, AlloyRskWsProvider, RskWsProvider,
};
use monitor::store::{CachedKeyValueStore, StoreKey};
use monitor::types::RskBlock;
use monitor::utils::RuntimeSync;
use std::sync::Arc;

const INITIAL_BLOCK: &str = "0x5609fff226ca052d12eca7bfdb45edca1c8252ac08b492420990fc8fb82c2868"; // TODO change if changed INITIAL_BLOCK_ENV
const FINALITY_FOR_CHECK: u8 = 10;

fn main() -> Result<()> {
    env_logger::init();

    let store = CachedKeyValueStore::new("/Users/illuque/tmp/")
        .expect("Failed to create CachedKeyValueStore");

    let initial_block = store
        .get_block_by_hash(INITIAL_BLOCK)?
        .context("Failed to get initial block")?;
    let store_best_block = store
        .get_best_block()?
        .context("Failed to get best block")?;

    if !find_connection_point(&store_best_block, FINALITY_FOR_CHECK)? {
        bail!(
            "Could not find canonical block for best block after {} attempts",
            FINALITY_FOR_CHECK
        );
    }

    let mut next_block = store_best_block;
    let mut expected_hash = next_block.hash().to_string();

    while next_block.number() > initial_block.number() {
        debug!(
            "Block {} ({}) with parent {}",
            next_block.number(),
            next_block.hash(),
            next_block.parent()
        );

        if next_block.hash() != expected_hash {
            bail!(
                "Parent hash mismatch at block: {} ({}), expected {}",
                next_block.number(),
                next_block.hash(),
                expected_hash
            );
        }

        expected_hash = next_block.parent().to_string();

        let next_block_num = &next_block.number() - 1;
        next_block = match store.get_block_by_number(next_block_num)? {
            Some(block) => block,
            None => {
                bail!("Missing block at: {}", next_block_num);
            }
        };
    }

    if !find_connection_point(&next_block, 1)? {
        bail!(
            "Could not find canonical block for initial block {} ({})",
            next_block.number(),
            next_block.hash()
        );
    }

    info!(
        "Reached initial block {} ({}) with parent {} without gaps!!!",
        next_block.number(),
        next_block.hash(),
        next_block.parent()
    );

    Ok(())
}

fn find_connection_point(store_block: &RskBlock, attempts: u8) -> Result<bool> {
    let rsk_ws_provider: Box<
        dyn RskWsProvider<BlockSub = AlloyBlockSubscription, LogsSub = AlloyLogsSubscription>,
    > = Box::new(AlloyRskWsProvider::new(
        "wss://public-node.testnet.rsk.co/websocket",
        Arc::new(RuntimeSync::new()?),
    )?);

    let mut connection_found = false;
    for i in 0..attempts {
        let node_block = rsk_ws_provider
            .get_block_by_number(store_block.number())
            .context("Failed to store_best_block block from node")?;

        connection_found = node_block.hash() == store_block.hash();
        if connection_found {
            break;
        }
    }

    Ok(connection_found)
}
