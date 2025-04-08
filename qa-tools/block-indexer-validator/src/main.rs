use anyhow::{bail, Context, Ok, Result};
use block_indexer::config::Config;
use block_indexer::store::{BlockStore, CachedBlockStore};
use clap::{Arg, Command};
use common::{
    cache::LruCache,
    rsk_provider::RskProvider,
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, RskBlock},
};
use log::{debug, info, warn};
use rsk_provider::rpc::AlloyProvider;

const LOGGER_CLI_FLAG: &str = "logger-path";
const CONFIG_CLI_FLAG: &str = "config-path";
const FINALITY_FOR_CHECK: u8 = 10;

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn main() -> Result<()> {
    let matches = Command::new("Check Fork Tool")
        .arg(
            Arg::new(LOGGER_CLI_FLAG)
                .short('l')
                .long(LOGGER_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the log4rs configuration file"),
        )
        .arg(
            Arg::new(CONFIG_CLI_FLAG)
                .short('c')
                .long(CONFIG_CLI_FLAG)
                .value_name("PATH")
                .help("Sets the path to the configuration directory")
                .default_value("../config/local"), // for local usage within the crate
        )
        .get_matches();

    let default_logger = format!("{}/log4rs.yaml", CARGO_MANIFEST_DIR);
    let logger_path: &str = matches
        .get_one::<String>(LOGGER_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_logger);
    log4rs::init_file(logger_path, Default::default()).expect("Failed to load log4rs config");

    let default_config = format!("{}/../config/local", CARGO_MANIFEST_DIR);
    let config_path: &str = matches
        .get_one::<String>(CONFIG_CLI_FLAG)
        .map(|s| s.as_str())
        .unwrap_or(&default_config);
    let config: Config = Config::load(config_path).expect("Failed to load config");

    let store = CachedBlockStore::new(
        &format!("{}/blocks", config.indexer.storage.path),
        config.indexer.cache.size,
    )?;
    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .expect(&format!(
            "Invalid initial block hash: {}",
            config.indexer.initial_block_hash
        ));

    let initial_block = store
        .get_block_by_hash(initial_block_hash)?
        .context("Failed to get initial block")?;
    let store_best_block = store
        .get_best_block()?
        .context("Failed to get best block")?;

    let provider = AlloyProvider::new(&config.provider.rootstock.url, ShutdownFlag::init())
        .expect("Failed to create AlloyProvider");

    compare_best_blocks(&store_best_block, &provider)?;

    let back_sync_checkpoint = store.get_back_sync_checkpoint()?;

    if back_sync_checkpoint.is_some() {
        warn!("Found partial backward sync (checkpoint). Check if this is expected. Quitting.");
        return Ok(());
    } else {
        info!("No partial backward sync (checkpoint) found.");
    }

    if !find_canonical_connection(&store_best_block, FINALITY_FOR_CHECK, &store, &provider)? {
        bail!(
            "Could not find canonical block for best block {} ({}) after {} attempts",
            store_best_block.number(),
            store_best_block.hash(),
            FINALITY_FOR_CHECK
        );
    }

    let mut next_block = store_best_block;
    let mut expected_hash = next_block.hash();

    while next_block.number() > initial_block.number() {
        if next_block.hash() != expected_hash {
            bail!(
                "Parent hash mismatch at block: {} ({}), expected {}",
                next_block.number(),
                next_block.hash(),
                expected_hash
            );
        }

        expected_hash = next_block.parent_hash();

        let next_block_num = next_block.number() - 1;
        next_block = match store.get_canonical_block(next_block_num)? {
            Some(block) => block,
            None => {
                bail!("Missing block at: {}", next_block_num);
            }
        };
    }

    if !find_canonical_connection(&next_block, 1, &store, &provider)? {
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
        next_block.parent_hash()
    );

    Ok(())
}

fn find_canonical_connection(
    block_ref: &RskBlock,
    block_margin: u8,
    store: &CachedBlockStore<LruCache<RskBlock>>,
    provider: &AlloyProvider,
) -> Result<bool> {
    info!(
        "Finding connection point for block {} ({})",
        block_ref.number(),
        block_ref.hash()
    );

    let mut store_block = block_ref.clone();
    let mut node_block = provider
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

        node_block = provider
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
            .get_block_by_hash(store_block.parent_hash())?
            .expect("Failed to get block's parent from store");
    }

    Ok(connection_found)
}

fn compare_best_blocks(store_best_block: &RskBlock, provider: &AlloyProvider) -> Result<()> {
    // Check 1: Compare the store best block with the provider block at the same height.
    let provider_block_at_store = provider
        .get_block_by_number(store_best_block.number())
        .with_context(|| {
            format!(
                "Provider error getting block by num: {}",
                store_best_block.number()
            )
        })?
        .with_context(|| format!("Block not found by num: {}", store_best_block.number()))?;

    if store_best_block.hash() != provider_block_at_store.hash() {
        let height_diff: u64 = if provider_block_at_store.number() > store_best_block.number() {
            (provider_block_at_store.number() - store_best_block.number().value()).value()
        } else {
            (store_best_block.number() - provider_block_at_store.number().value()).value()
        };
        warn!(
            "Mismatch at store best height:\n  Store best block: {} ({})\n  Provider block at same height: {} ({}).\nDifference in block height: {} - check if this is expected.",
            store_best_block.number(),
            store_best_block.hash(),
            provider_block_at_store.number(),
            provider_block_at_store.hash(),
            height_diff
        );
    } else {
        info!(
            "Store best block matches provider block at same height: {} ({})",
            store_best_block.number(),
            store_best_block.hash()
        );
    }

    // Check 2: Compare the store best block with the provider's best block.
    let provider_best_block = provider
        .get_best_block()
        .with_context(|| "Provider error getting best block")?;

    if store_best_block.hash() != provider_best_block.hash() {
        let height_diff: u64 = if provider_best_block.number() > store_best_block.number() {
            (provider_best_block.number() - store_best_block.number().value()).value()
        } else {
            (store_best_block.number() - provider_best_block.number().value()).value()
        };
        warn!(
            "Mismatch between store best block and provider best block:\n  Store best block: {} ({})\n  Provider best block: {} ({}).\nDifference in block height: {}",
            store_best_block.number(),
            store_best_block.hash(),
            provider_best_block.number(),
            provider_best_block.hash(),
            height_diff
        );
    } else {
        info!(
            "Store best block matches provider best block: {} ({})",
            store_best_block.number(),
            store_best_block.hash()
        );
    }
    Ok(())
}
