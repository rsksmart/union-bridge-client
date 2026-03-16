use anyhow::{bail, Context, Result};
use block_indexer::config::{Config, Logger};
use block_indexer::indexer::BlockIndexer;
use block_indexer::store::CachedBlockStore;
use clap::Parser;
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::config::IndexerStartFrom;
use common::rsk_indexer::RskIndexer;
use common::rsk_provider::RskProvider;
use common::shutdown_flag::ShutdownFlag;
use common::types::BlockNumber;
use log::{error, info};

#[derive(Parser)]
#[command(
    name = "block-indexer-runner",
    about = "Run block-indexer with CLI overrides for manual testing"
)]
struct Args {
    /// Start N blocks behind the current best
    #[arg(short = 'f', long, conflicts_with = "block_height")]
    finality: Option<u64>,

    /// Start from a specific block height
    #[arg(short = 'b', long, conflicts_with = "finality")]
    block_height: Option<u64>,

    /// Override cache size
    #[arg(long)]
    cache_size: Option<usize>,

    /// Override provider URL (otherwise uses config / UB__ env vars)
    #[arg(long)]
    provider_url: Option<String>,

    /// Tag for storage isolation (stored under /tmp/manual-tests/<tag>/)
    #[arg(short = 't', long)]
    tag: String,

    /// Environment name for base config loading (maps to config/environment/<env>.toml)
    #[arg(short = 'e', long)]
    env: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    Logger::init(None).context("Failed to initialize logger")?;

    let mut config = Config::load(args.env).context("Failed to load config")?;

    config.indexer.storage.path = format!("/tmp/manual-tests/{}/database", args.tag);

    if let Some(size) = args.cache_size {
        config.indexer.cache.size = size;
    }

    if let Some(ref url) = args.provider_url {
        config.provider.rootstock.url = url.clone();
    }

    let shutdown_flag = ShutdownFlag::init();

    let provider = AlloyProvider::new(&config.provider.rootstock.url, shutdown_flag.clone())
        .context("Failed to connect to provider")?;

    if let Some(finality) = args.finality {
        let best = provider.get_best_block().context("Failed to get best block")?;
        let target_height = best.number().value().saturating_sub(finality);
        let block =
            provider.get_block_by_number(BlockNumber::from(target_height))?.with_context(|| {
                format!(
                    "Block at height {target_height} not found (best={}, finality={finality})",
                    best.number()
                )
            })?;
        println!("Resolved -f {finality}: height={target_height}, hash={}", block.hash());
        config.indexer.initial_block_hash = Some(block.hash().to_string());
    } else if let Some(height) = args.block_height {
        let block = provider
            .get_block_by_number(BlockNumber::from(height))?
            .with_context(|| format!("Block at height {height} not found"))?;
        println!("Resolved -b {height}: hash={}", block.hash());
        config.indexer.initial_block_hash = Some(block.hash().to_string());
    } else {
        bail!("Either -f (finality) or -b (block_height) must be provided");
    }

    config.indexer.start_from = IndexerStartFrom::Hash;

    let store_path = format!("{}/blocks", config.indexer.storage.path);

    println!("Storage:    {}", config.indexer.storage.path);
    println!("Cache size: {}", config.indexer.cache.size);
    println!("Provider:   {}", config.provider.rootstock.url);
    println!("Initial:    {:?}", config.indexer.initial_block_hash);

    let store = CachedBlockStore::new(&store_path, config.indexer.cache.size)
        .context("Failed to create block store")?;

    let indexer = BlockIndexer::new(store, provider, &config.indexer, shutdown_flag.clone())
        .context("Failed to create BlockIndexer")?;

    indexer.run().inspect_err(|e| {
        error!("Block indexer failed: {e:?}");
        shutdown_flag.set();
    })?;

    info!("Block indexer stopped.");
    log::logger().flush();

    Ok(())
}
