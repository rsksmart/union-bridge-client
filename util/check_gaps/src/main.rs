use anyhow::{bail, Context, Ok, Result};
use block_indexer::store::{BlockStore, CachedBlockStore};
use common::cache::LruCache;
use common::rsk_provider::RskProvider;
use common::types::RskBlock;
use dotenv::dotenv;
use log::{debug, info, warn};
use rsk_provider::alloy::AlloyProvider;
use std::env;

const FINALITY_FOR_CHECK: u8 = 10;

fn main() -> Result<()> {
    env_logger::init();

    dotenv().expect("Failed to load .env file");

    let store_path = env::var("STORE_PATH").expect("STORE_PATH not set in env");
    let store = CachedBlockStore::new(&store_path).expect("Failed to create CachedKeyValueStore");

    let initial_block_hash =
        env::var("INITIAL_BLOCK_HASH").expect("INITIAL_BLOCK_HASH not set in env");

    let initial_block = store
        .get_block_by_hash(&initial_block_hash)?
        .context("Failed to get initial block")?;
    let store_best_block = store
        .get_best_block()?
        .context("Failed to get best block")?;
    let back_sync_checkpoint = store.get_back_sync_checkpoint()?;

    if back_sync_checkpoint.is_some() {
        warn!("Found partial backward sync. Check if this is expected. Quiting.");
        return Ok(());
    }

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
    store: &CachedBlockStore<LruCache<RskBlock>>,
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
