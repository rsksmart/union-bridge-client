use anyhow::{Context, Ok, Result, bail};
use block_indexer::config::{Config, Logger};
use block_indexer::store::{BlockStore, CachedBlockStore};
use clap::Parser;
use common::{
    alloy_rsk_provider::rpc::AlloyProvider,
    cache::LruCache,
    rsk_provider::RskProvider,
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, RskBlock},
};
use log::{debug, info, warn};

/// Runs block-indexer-validator with the provided log configuration and configuration folder.
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Mandatory tag (e.g. "happy_path")
    #[arg(short = 't')]
    tag: String,

    /// Environment (optional, default: "stage")
    #[arg(short = 'e', default_value = "stage")]
    env: String,
}

const ROOT_DIRECTORY: &str = "/tmp/monitor-executions";
const FINALITY_FOR_CHECK: u8 = 10;

fn main() -> Result<()> {
    let args = Args::parse();

    let target_folder = format!("{}/{}", ROOT_DIRECTORY, args.tag);
    let target_config_folder = format!("{}/config/{}", target_folder, args.env);
    let target_log_folder = target_folder.clone();
    let target_log_config_file = format!("{}/log4rs.yaml", target_folder);

    println!(
        "Starting block-indexer-validator with log config: {} and config folder: {}",
        target_log_config_file, target_config_folder
    );

    run_block_indexer_validator(&target_log_config_file, &target_config_folder)?;

    let app_log_path = format!("{}/app.log", target_log_folder);
    tail_file(&app_log_path, 20)?;

    Ok(())
}

fn run_block_indexer_validator(log_config_path: &str, config_folder: &str) -> Result<()> {
    log4rs::init_file(log_config_path, Default::default())
        .with_context(|| format!("Initializing log4rs from {}", log_config_path))?;
    let config = Config::load(config_folder)
        .with_context(|| format!("Loading config from {}", config_folder))?;
    let store = CachedBlockStore::new(
        &format!("{}/blocks", config.indexer.storage.path),
        config.indexer.cache.size,
    )
    .with_context(|| "Creating block store")?;
    let shutdown_flag = ShutdownFlag::init();

    let provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .with_context(|| "Creating AlloyProvider")?;
    let initial_block_hash = BlockHash::try_from(config.indexer.initial_block_hash.as_str())
        .with_context(|| "Parsing initial block hash")?;
    let initial_block = store
        .get_block_by_hash(initial_block_hash)?
        .context("Failed to get initial block")?;
    let store_best_block = store
        .get_best_block()?
        .context("Failed to get best block")?;

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

    let next_block = find_next_block(&store, initial_block, store_best_block)?;

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

fn find_next_block(
    store: &CachedBlockStore<LruCache<RskBlock>>,
    initial_block: RskBlock,
    store_best_block: RskBlock,
) -> Result<RskBlock, anyhow::Error> {
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
    Ok(next_block)
}

fn tail_file<P: AsRef<Path>>(path: P, n: usize) -> Result<()> {
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Reading log file: {:?}", path.as_ref()))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > n { lines.len() - n } else { 0 };
    println!("--- Last {} lines of log ---", n);
    for line in &lines[start..] {
        println!("{}", line);
    }
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
