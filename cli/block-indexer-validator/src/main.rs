use anyhow::{Context, Result, bail};
use clap::Parser;

use block_indexer::store::{BlockStore, CachedBlockStore};
use common::alloy_rsk_provider::rpc::AlloyProvider;
use common::cache::LruCache;
use common::rsk_provider::RskProvider;
use common::shutdown_flag::ShutdownFlag;
use common::types::{BlockNumber, RskBlock};

#[derive(Parser)]
#[command(name = "block-indexer-validator", about = "Validate block-indexer storage after manual test runs")]
struct Args {
    /// Path to the block store directory (the "blocks" subdirectory inside storage.path)
    #[arg(short = 's', long)]
    storage_path: String,

    /// Cache size for the LRU cache when opening the store
    #[arg(short = 'c', long, default_value_t = 1000)]
    cache_size: usize,

    /// Optional provider URL to compare store state against a live node
    #[arg(short = 'p', long)]
    provider_url: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let store: CachedBlockStore<LruCache<RskBlock>> =
        CachedBlockStore::new(&args.storage_path, args.cache_size)
            .context("Failed to open block store")?;

    let best_block = store
        .get_best_block()?
        .context("No best block in store — is the store empty?")?;

    println!(
        "Store best block: height={}, hash={}",
        best_block.number(),
        best_block.hash()
    );

    let has_checkpoint = check_checkpoint(&store)?;
    let chain_length = walk_chain(&store, &best_block)?;
    println!("Chain walk complete: {chain_length} blocks, no gaps found");

    if let Some(url) = &args.provider_url {
        compare_with_provider(url, &best_block)?;
    }

    if has_checkpoint {
        println!("\nAll chain checks passed, but backward sync is incomplete (checkpoint remains).");
    } else {
        println!("\nAll checks passed.");
    }
    Ok(())
}

fn check_checkpoint(store: &CachedBlockStore<LruCache<RskBlock>>) -> Result<bool> {
    match store.get_back_sync_checkpoint()? {
        Some(cp) => {
            println!(
                "WARNING: partial backward sync checkpoint found at height={}, hash={}",
                cp.number(),
                cp.hash()
            );
            Ok(true)
        }
        None => {
            println!("No backward sync checkpoint (good — sync completed)");
            Ok(false)
        }
    }
}

fn walk_chain(store: &CachedBlockStore<LruCache<RskBlock>>, best_block: &RskBlock) -> Result<u64> {
    let mut current = best_block.clone();
    let mut count: u64 = 1;

    loop {
        let height = current.number();
        let canonical = store.get_canonical_block(height)?;

        match canonical {
            Some(ref canon) if canon.hash() == current.hash() => {}
            Some(ref canon) => {
                bail!(
                    "Canonical block mismatch at height {height}: \
                     expected hash={}, got hash={}",
                    current.hash(),
                    canon.hash()
                );
            }
            None => {
                bail!("Missing canonical block at height {height}");
            }
        }

        let parent_hash = current.parent_hash();
        match store.get_block_by_hash(parent_hash)? {
            Some(parent) => {
                let expected_height = height.value()
                    .checked_sub(1)
                    .map(BlockNumber::from)
                    .context("Block at height 0 has a parent in store — unexpected")?;
                if parent.number() != expected_height {
                    bail!(
                        "Parent height mismatch at height {height}: \
                         expected {expected_height}, got {}",
                        parent.number()
                    );
                }
                current = parent;
                count += 1;
            }
            None => {
                println!(
                    "Reached initial block: height={}, hash={} (parent {} not in store)",
                    height,
                    current.hash(),
                    parent_hash
                );
                break;
            }
        }
    }

    Ok(count)
}

fn compare_with_provider(url: &str, store_best: &RskBlock) -> Result<()> {
    let shutdown = ShutdownFlag::init();
    let provider = AlloyProvider::new(url, shutdown).context("Failed to connect to provider")?;

    let provider_best = provider.get_best_block().context("Failed to get provider best block")?;

    println!(
        "Provider best block: height={}, hash={}",
        provider_best.number(),
        provider_best.hash()
    );

    if provider_best.number() > store_best.number() {
        let diff = provider_best.number().value() - store_best.number().value();
        println!("Store is {diff} blocks behind provider (expected if indexer was stopped)");
    } else if store_best.number() > provider_best.number() {
        let diff = store_best.number().value() - provider_best.number().value();
        println!("WARNING: Store is {diff} blocks ahead of provider — possibly connected to a different or syncing node");
    }

    let provider_block_at_store_height = provider
        .get_block_by_number(store_best.number())
        .context("Failed to get provider block at store height")?
        .context("Provider has no block at store best height")?;

    if store_best.hash() == provider_block_at_store_height.hash() {
        println!("Store best block matches provider at same height (good)");
    } else {
        println!(
            "WARNING: Store best block hash differs from provider at height {}.\n  \
             Store:    {}\n  Provider: {}\n  \
             This may indicate a reorg occurred after the indexer stopped.",
            store_best.number(),
            store_best.hash(),
            provider_block_at_store_height.hash()
        );
    }

    Ok(())
}
