use crate::rsk_provider::provider::{RskProvider, RskSubscription};
use crate::store::BlockStore;
use crate::types::RskBlock;
use crate::utils::ShutdownFlag;
use anyhow::{anyhow, Result};
use log::{error, info, warn};
use std::ops::Deref;
use std::thread;
use std::time::Duration;

pub struct Indexer<P: RskProvider, S: BlockStore> {
    store: S,
    rsk_provider: P,
    initial_block_hash: String,
}

// TODO(Jira) review this file and take care of transactionality on storage saving: https://rsklabs.atlassian.net/browse/UB-11

impl<P: RskProvider, S: BlockStore> Indexer<P, S> {
    pub fn new(store: S, provider: P, initial_block_hash: &str) -> Self {
        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        Self {
            store,
            rsk_provider: provider,
            initial_block_hash: initial_block_hash.to_string(),
        }
    }

    pub fn run(&self, shutdown_flag: ShutdownFlag) -> Result<()> {
        self.initialize_db_if_required()?;

        if !shutdown_flag.is_on() {
            self.startup_backward_sync(&shutdown_flag)?;
        }

        if !shutdown_flag.is_on() {
            self.subscribe_blocks(shutdown_flag)?;
        }

        Ok(())
    }

    fn startup_backward_sync(&self, shutdown_flag: &ShutdownFlag) -> Result<()> {
        // if a partial/interrupted backward_sync is found a back_sync_checkpoint will exist, so we:
        //      1. finish connecting that checkpoint (backward_sync from checkpoint)
        //      2. complete the full connection by connecting the provider best block (backward_sync from provider best block)
        //      (this way we save some time by not re-processing blocks we already know/have)
        // otherwise we simply run a backward sync from the provider best block

        let back_sync_checkpoint = self.get_back_sync_checkpoint()?;
        if back_sync_checkpoint.is_some() {
            let back_sync_checkpoint = back_sync_checkpoint.unwrap();
            info!(
                "[startup_backward_sync] Resuming backward_sync from checkpoint {} ({})",
                back_sync_checkpoint.number(),
                back_sync_checkpoint.hash()
            );

            let checkpoint_parent = self
                .rsk_provider
                .get_block_by_hash(back_sync_checkpoint.parent())
                .expect("Provider errored getting checkpoint_parent (startup -> quit)")
                .expect("checkpoint_parent not found on provider (startup -> quit)");
            self.backward_sync(&checkpoint_parent, &shutdown_flag)?;
        }

        if shutdown_flag.is_on() {
            return Ok(());
        }

        let provider_best_block = self.rsk_provider.get_best_block()?;

        info!(
            "[backward_sync] Running backward_sync from tip {} ({})",
            provider_best_block.number(),
            provider_best_block.hash()
        );
        // connect provider best block
        self.backward_sync(&provider_best_block, &shutdown_flag)?;

        Ok(())
    }

    /**
     * Retrieves the last block processed by a previously interrupted backward_sync operation,
     * if such a block exists and is still part of the canonical chain.
     *
     * Returns:
     * - The last block processed by such interrupted sync if the block remains canonical.
     * - `None` in all other scenarios.
     */
    fn get_back_sync_checkpoint(&self) -> Result<Option<RskBlock>> {
        // back_sync_checkpoint will exist only if a backward_sync was interrupted and store its last processed block
        let back_sync_checkpoint = self.store.get_back_sync_checkpoint()?;
        if back_sync_checkpoint.is_none() {
            return Ok(None);
        }

        let back_sync_checkpoint = back_sync_checkpoint.unwrap();
        let canonical_block = self
            .rsk_provider
            .get_block_by_number(back_sync_checkpoint.number())
            .expect("Provider errored getting back_sync_checkpoint (startup -> quit)");

        if canonical_block.is_none() {
            return Ok(None);
        }

        let canonical_block = canonical_block.unwrap();
        if canonical_block.hash() != back_sync_checkpoint.hash() {
            warn!(
                    "[backward_sync] Partial backward_sync found with invalid non canonical checkpoint {} ({})",
                    back_sync_checkpoint.number(),
                    back_sync_checkpoint.hash(),
                );
            return Ok(None);
        }

        Ok(Some(back_sync_checkpoint))
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

    fn subscribe_blocks(&self, shutdown_flag: ShutdownFlag) -> Result<()> {
        info!("[subscribe_blocks] Start subscribe_blocks...");

        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        let mut rsk_block_subscription = self
            .rsk_provider
            .subscribe_blocks()
            .expect("Failed to subscribe to blocks (unrecoverable)"); // TODO retry mechanism in scope of UB-15

        let loop_result = (|| {
            while !shutdown_flag.is_on() {
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
                let requires_local_reorg = !extends_canonical
                    && new_block.total_difficulty() > local_best_block.total_difficulty();

                if extends_canonical {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): setting new best",
                        new_block.number(),
                        new_block.hash()
                    );
                    self.save_as_best_block(&new_block)?;
                } else if requires_local_reorg {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): fixing local reorg",
                        new_block.number(),
                        new_block.hash()
                    );
                    let provider_best_block = self.rsk_provider.get_best_block()?;
                    self.backward_sync(&provider_best_block, &shutdown_flag)?;
                } else {
                    info!(
                        "[subscribe_blocks] Processing block {} ({}): neither extending, nor competing",
                        new_block.number(),
                        new_block.hash()
                    );
                    self.save_as_not_canonical(&new_block)?;
                }
            }

            Ok(())
        })();

        rsk_block_subscription
            .unsubscribe()
            .and_then(|_| loop_result)
    }

    fn save_as_not_canonical(&self, new_block: &RskBlock) -> Result<()> {
        self.store.save_block(&new_block)
    }

    fn save_as_canonical(&self, canonical_block: &RskBlock) -> Result<()> {
        self.store.save_block(&canonical_block)?;
        self.store.set_canonical_block(&canonical_block)?;
        Ok(())
    }

    fn save_as_best_block(&self, new_block: &RskBlock) -> Result<()> {
        self.save_as_canonical(&new_block)?;
        self.store.set_best_block(&new_block)
    }

    fn backward_sync(&self, from_block: &RskBlock, shutdown_flag: &ShutdownFlag) -> Result<()> {
        // TODO(Jira) request and persist uncles during this process: https://rsklabs.atlassian.net/browse/UB-16

        let store_best_block = self
            .store
            .get_best_block()?
            .expect("Could not get best block from store (unrecoverable)");

        info!(
            "[backward_sync] Connecting blocks {} ({}) and {} ({})",
            from_block.number(),
            from_block.hash(),
            store_best_block.number(),
            store_best_block.hash(),
        );

        let initial_block = self
            .store
            .get_block_by_hash(&self.initial_block_hash)?
            .expect("initial_block_hash does not exist on provider (unrecoverable)");

        let mut canonical_block: RskBlock = from_block.clone();
        let mut store_block_opt = self.store.get_canonical_block(from_block.number())?;
        let mut connection_point_reached = false;

        while !shutdown_flag.is_on()
            && !connection_point_reached
            && initial_block.number() <= canonical_block.number()
        {
            let is_missing_block = store_block_opt.is_none();
            let is_reorg = !is_missing_block
                && store_block_opt.as_ref().unwrap().hash() != canonical_block.hash();

            if is_missing_block || is_reorg {
                info!(
                    "[backward_sync] {} block {} ({})...",
                    if is_reorg { "Replacing" } else { "Creating" },
                    canonical_block.number(),
                    canonical_block.hash(),
                );

                self.save_as_canonical(&canonical_block)?;
                self.store.set_back_sync_checkpoint(&canonical_block)?;

                canonical_block = match self
                    .rsk_provider
                    .get_block_by_number(canonical_block.number() - 1)?
                {
                    Some(block) => block,
                    None => {
                        // edge case of last minute reorg that makes such block to not exist, retry with best block
                        self.rsk_provider.get_best_block()?
                    }
                };
                store_block_opt = self.store.get_canonical_block(canonical_block.number())?;
            } else {
                connection_point_reached = true;
            }
        }

        if connection_point_reached {
            info!(
                "[backward_sync] Completed at block {} ({}), early={}",
                store_best_block.number(),
                store_best_block.hash(),
                store_best_block.number() < canonical_block.number()
            );

            self.store.reset_back_sync_checkpoint()?;
            self.store.set_best_block(&from_block)?;
        } else {
            warn!("[backward_sync] Finished before completing!");
        }

        Ok(())
    }
}

impl<P: RskProvider, S: BlockStore> Drop for Indexer<P, S> {
    fn drop(&mut self) {
        if let Err(e) = self.rsk_provider.disconnect() {
            error!("Failed to disconnect rsk_provider: {:?}", e);
        }
    }
}
