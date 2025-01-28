use crate::rsk_provider::provider::{RskProvider, RskSubscription};
use crate::store::BlockStore;
use crate::types::RskBlock;
use anyhow::{anyhow, bail, Result};
use log::{debug, error, info, warn};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct Indexer<P: RskProvider, S: BlockStore> {
    // TODO Arc<S> needed because of this piece in storage_backend/src/storage.rs: "transactions: RefCell<HashMap<usize, Box<rocksdb::Transaction<'static, TransactionDB>>>>"
    store: Arc<S>,
    rsk_provider: P,
    initial_block_hash: String,
    is_running: Arc<AtomicBool>,
}

// TODO(Jira) review this file and take care of transactionality on storage saving: https://rsklabs.atlassian.net/browse/UB-11
// TODO(Jira) allow changing the initial_block_hash on a running instance: https://rsklabs.atlassian.net/browse/UB-32

impl<P: RskProvider, S: BlockStore> Indexer<P, S> {
    pub fn new(store: Arc<S>, provider: P, initial_block_hash: &str) -> Self {
        Self {
            store,
            rsk_provider: provider,
            initial_block_hash: initial_block_hash.to_string(),
            is_running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn run(&self) -> Result<()> {
        self.initialize_db_if_required()?;

        self.startup_backward_sync()?;

        self.subscribe_blocks()
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    fn initialize_db_if_required(&self) -> Result<()> {
        let best_block: Option<RskBlock> = self.store.get_best_block()?;
        if best_block.is_some() {
            return Ok(());
        }

        let initial_block_node = self
            .rsk_provider
            .get_block_by_hash(self.initial_block_hash.deref())
            .expect("Provider errored getting initial_block_node (startup -> quit)")
            .expect("initial_block_node not found on provider (startup -> quit)");

        info!(
            "[initialize_db_if_required] New instance: initializing DB with {} ({})",
            initial_block_node.number(),
            initial_block_node.hash()
        );

        // initialize the store with the initial block info
        self.save_as_best_block(&initial_block_node)
    }

    fn startup_backward_sync(&self) -> Result<()> {
        // In case of an interrupted backward_sync, a back_sync_checkpoint will be created.
        // If it is still canonical when we restart the application, we will run:
        //     1. a backward_sync to finish connecting such checkpoint, that becomes the new connection point
        //     2. another backward_sync to connect the provider best block and achieve the full sync
        // We do this in order to save some time by not re-processing blocks we already know/have.
        // If the checkpoint does not exist, or it is not canonical anymore, we will just start a
        // new backward_sync from the provider best block.
        self.resume_pending_backward_sync()?;

        // In case of a long backward_sync, we may be far from the tip when it completes to rely on
        // eth_subscribe for catch up. So we run many backward_syncs until we get close to the tip.
        self.full_sync_backward_syncs()?;

        Ok(())
    }

    fn subscribe_blocks(&self) -> Result<()> {
        if !self.is_running() {
            info!("[subscribe_blocks] Shutdown requested, skipping subscribe_blocks");
            return Ok(());
        }

        info!("[subscribe_blocks] Start subscribe_blocks...");

        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        let mut rsk_block_subscription = self
            .rsk_provider
            .subscribe_blocks()
            .expect("Failed to subscribe to blocks (unrecoverable)"); // TODO retry mechanism in scope of UB-15

        let loop_result = (|| {
            while self.is_running() {
                let new_block = rsk_block_subscription.next()?;
                if new_block.is_none() {
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }

                let new_block = new_block.unwrap();

                // TODO(Jira) do batched writes in backward sync: https://rsklabs.atlassian.net/browse/UB-24

                // no need to keep track of it between iters as it is cached and can be re-fetched
                let local_best_block = self
                    .store
                    .get_best_block()?
                    .ok_or_else(|| anyhow!("Failed to get local_best_block from store"))?;

                let extends_canonical = new_block.parent() == local_best_block.hash();
                let is_reorg = !extends_canonical
                    && new_block.total_difficulty() > local_best_block.total_difficulty();

                if extends_canonical {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): setting new best",
                        new_block.number(),
                        new_block.hash()
                    );
                    self.save_as_best_block(&new_block)?;
                } else if is_reorg {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): fixing local reorg",
                        new_block.number(),
                        new_block.hash()
                    );
                    let provider_best_block = self.rsk_provider.get_best_block()?;
                    self.backward_sync(&provider_best_block)?;
                } else {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): neither extending, nor competing",
                        new_block.number(),
                        new_block.hash()
                    );
                    // just save the block as it is not part of the main chain (at least yet)
                    self.store.save_block(&new_block)?;
                }
            }

            Ok(())
        })();

        rsk_block_subscription
            .unsubscribe()
            .and_then(|_| loop_result)
    }

    fn backward_sync(&self, starting_block: &RskBlock) -> Result<()> {
        if !self.is_running() {
            info!("[backward_sync] Shutdown requested, skipping backward_sync");
            return Ok(());
        }

        let store_best_block = self
            .store
            .get_best_block()?
            .expect("Could not get best block from store (unrecoverable)");

        info!(
            "[backward_sync] Connecting blocks {} ({}) and {} ({})",
            starting_block.number(),
            starting_block.hash(),
            store_best_block.number(),
            store_best_block.hash(),
        );

        // TODO(Jira) request and persist uncles during this process: https://rsklabs.atlassian.net/browse/UB-16

        let mut new_block = starting_block.clone();
        loop {
            let store_block = self.store.get_canonical_block(new_block.number())?;

            let is_missing = store_block.is_none();
            let is_reorg = store_block.map_or(false, |sb| sb.hash() != new_block.hash());
            let reached_connection_height = new_block.number() <= store_best_block.number();

            if is_missing || is_reorg {
                info!(
                    "[backward_sync] {} block {} ({})...",
                    if is_reorg { "Replacing" } else { "Creating" },
                    new_block.number(),
                    new_block.hash(),
                );
                self.save_as_canonical(&new_block)?;
            } else if !reached_connection_height {
                debug!(
                    "[backward_sync] Skipping known block {} ({}) while checking if fully connected",
                    new_block.number(),
                    new_block.hash()
                );
            } else {
                info!(
                    "[backward_sync] Completed at block {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // we are complete, so we remove the checkpoint if any
                self.store.reset_back_sync_checkpoint()?;
                // it represents also the connection point to achieve full sync
                self.store.set_best_block(&starting_block)?;

                break;
            }

            if !self.is_running() {
                warn!(
                    "[backward_sync] Shutdown requested, setting back_sync_checkpoint to {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // define backward_sync checkpoint to resume from
                self.store.set_back_sync_checkpoint(&new_block)?;

                break;
            }

            if self.initial_block_hash == new_block.hash() || new_block.number() == 0 {
                error!("[backward_sync] Reached genesis or starting block, aborting backward_sync");
                break;
            }

            // no exit condition met, keep searching backwards
            new_block = self.get_next_backward_sync_block(new_block.number() - 1)?;
        }

        Ok(())
    }

    fn resume_pending_backward_sync(&self) -> Result<()> {
        if let Some(checkpoint) = self.store.get_back_sync_checkpoint()? {
            match self.rsk_provider.get_block_by_hash(checkpoint.parent())? {
                Some(checkpoint_parent) => {
                    info!("[startup_backward_sync] Resuming previous backward_sync");
                    self.backward_sync(&checkpoint_parent)?;
                }
                None => {
                    warn!(
                        "[startup_backward_sync] Cannot resume backward_sync from non canonical checkpoint {} ({})",
                        checkpoint.number(),
                        checkpoint.hash(),
                    );
                }
            }
        }
        Ok(())
    }

    fn full_sync_backward_syncs(&self) -> Result<()> {
        let max_attempts = 10;
        for i in 1..max_attempts {
            let provider_best_block = self.rsk_provider.get_best_block()?;
            if let Some(store_best_block) = self.store.get_best_block()? {
                let is_full_sync = provider_best_block.hash() == store_best_block.hash();
                if is_full_sync {
                    debug!("[startup_backward_sync] No more backward_sync needed",);
                    return Ok(());
                } else if !self.is_running() {
                    return Ok(());
                } else {
                    info!("[startup_backward_sync] Running tip backward_sync-{}", i);
                    self.backward_sync(&provider_best_block)?;
                }
            }
        }

        bail!(
            "Could not catch up to the tip after {} backward_sync attempts",
            max_attempts
        )
    }

    fn save_as_canonical(&self, canonical_block: &RskBlock) -> Result<()> {
        self.store.save_block(&canonical_block)?;
        // last, to avoid requiring db transactionality, as it is used to distinguish new block from reorgs
        self.store.set_canonical_block(&canonical_block)
    }

    fn save_as_best_block(&self, new_block: &RskBlock) -> Result<()> {
        self.save_as_canonical(&new_block)?;
        // last is preferred to not mark as best a block that was not yet stored
        // furthermore, if this line is fails for any reason (or app quits on error right before
        // running), soon a new block will become best (either one extending or reorg)
        self.store.set_best_block(&new_block)
    }

    fn get_next_backward_sync_block(&self, block_num: u64) -> Result<RskBlock> {
        match self.rsk_provider.get_block_by_number(block_num)? {
            Some(block) => Ok(block),
            None => {
                // this means a reorg just have happened to a lower block num, so we start again from the best block
                warn!(
                    "[backward_sync] Could not get block {} from provider, retrying from best block",
                    block_num,
                );
                self.rsk_provider.get_best_block()
            }
        }
    }
}

impl<P: RskProvider, S: BlockStore> Drop for Indexer<P, S> {
    fn drop(&mut self) {
        if let Err(e) = self.rsk_provider.disconnect() {
            error!("Failed to disconnect rsk_provider: {:?}", e);
        }
    }
}
