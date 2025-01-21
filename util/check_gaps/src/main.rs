use anyhow::{bail, Context, Ok, Result};
use log::{debug, info};
use monitor::rsk_provider::alloy::AlloyProvider;
use monitor::rsk_provider::provider::RskProvider;
use monitor::store::{BlockStore, CachedBlockStore};
use monitor::types::RskBlock;

// TODO(Jira) from .env: https://rsklabs.atlassian.net/browse/UB-14
const INITIAL_BLOCK: &str = "0x551c09b6d4e35008a83016a16922676059eab39ba1c72d2c634c1c9119158a4a";
const FINALITY_FOR_CHECK: u8 = 10;

fn main() -> Result<()> {
    env_logger::init();

    let store =
        CachedBlockStore::new("/Users/illuque/tmp/").expect("Failed to create CachedKeyValueStore");

    let initial_block = store
        .get_block_by_hash(INITIAL_BLOCK)?
        .context("Failed to get initial block")?;
    let store_best_block = store
        .get_best_block()?
        .context("Failed to get best block")?;

    if !find_canonical_connection(&store_best_block, FINALITY_FOR_CHECK, &store)? {
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
        next_block = match store.get_canonical_block(next_block_num)? {
            Some(block) => block,
            None => {
                bail!("Missing block at: {}", next_block_num);
            }
        };
    }

    if !find_canonical_connection(&next_block, 1, &store)? {
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

fn find_canonical_connection(
    block_ref: &RskBlock,
    block_margin: u8,
    store: &CachedBlockStore,
) -> Result<bool> {
    let rsk_ws_provider = AlloyProvider::new("wss://public-node.testnet.rsk.co/websocket")
        .expect("Failed to create AlloyProvider");

    info!(
        "Finding connection point for block {} ({})",
        block_ref.number(),
        block_ref.hash()
    );

    let mut store_block = block_ref.clone();
    let mut node_block = rsk_ws_provider
        .get_block_by_number(store_block.number())
        .with_context(|| {
            format!(
                "Provider error getting block by num: {}",
                store_block.number()
            )
        })?
        .with_context(|| format!("Block not found by num: {}", store_block.number()))?;

    let mut connection_found = false;
    for i in 1..=block_margin {
        debug!(
            "Checking local block {} ({}) against node block {} ({})",
            store_block.number(),
            store_block.hash(),
            node_block.number(),
            node_block.hash(),
        );

        connection_found = node_block.hash() == store_block.hash();
        if connection_found {
            break;
        }

        node_block = rsk_ws_provider
            .get_block_by_number(store_block.number() - i as u64)
            .with_context(|| {
                format!(
                    "Provider error getting block by num: {}",
                    store_block.number() - i as u64
                )
            })?
            .with_context(|| {
                format!(
                    "Failed to get block by num: {}",
                    store_block.number() - i as u64
                )
            })?;

        store_block = store
            .get_block_by_hash(store_block.parent())?
            .expect("Failed to get block's parent from store");
    }

    Ok(connection_found)
}
