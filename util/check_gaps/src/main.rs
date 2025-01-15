use anyhow::{bail, Context, Ok, Result};
use log::{debug, error, info};
use monitor::provider::{RskApi, RskProvider};
use monitor::store::CachedKeyValueStore;
use monitor::types::RskBlock;

// TODO(Jira) from .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK: &str = "0xd608130f2caf657d11ec5bc2cbe7c17415813cb906714d1f2b4c6079dcf4c39a";
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

    if !find_connection_point(&store_best_block, FINALITY_FOR_CHECK, &store)? {
        bail!(
            "Could not find canonical block for best block {} ({}) after {} attempts",
            store_best_block.number(),
            store_best_block.hash(),
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

    if !find_connection_point(&next_block, 1, &store)? {
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

fn find_connection_point(
    block_ref: &RskBlock,
    attempts: u8,
    store: &CachedKeyValueStore,
) -> Result<bool> {
    let rsk_ws_provider = RskApi::new("wss://public-node.testnet.rsk.co/websocket");

    let mut store_block = block_ref.clone();

    let mut connection_found = false;
    for i in 0..attempts {
        let node_block = rsk_ws_provider
            .get_block_by_number(store_block.number() - i as u64)
            .context("Failed to get store_best_block block from node")?;

        info!(
            "Attempt {} - Node block: {} ({}) - Store block: {} ({})",
            i,
            node_block.number(),
            node_block.hash(),
            store_block.number(),
            store_block.hash()
        );

        connection_found = node_block.hash() == store_block.hash();
        if connection_found {
            break;
        }

        let store_block_opt = store.get_block_by_hash(store_block.parent())?;
        match store_block_opt {
            Some(block) => store_block = block,
            None => {
                error!(
                    "Disconnection found on store block {} ({})",
                    store_block.number() - 1,
                    store_block.parent()
                );
                return Ok(false);
            }
        }
    }

    Ok(connection_found)
}
