use crate::store::BlockStore;
use anyhow::{Context, Result, bail};
use common::{
    rsk_indexer::RskIndexer,
    rsk_provider::{RskProvider, RskSubscription, RskSubscriptionError},
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, BlockNumber, RskBlock},
};
use log::{debug, error, info, warn};

pub struct BlockIndexer<P: RskProvider, S: BlockStore> {
    store: S,
    rsk_provider: P,
    initial_block_hash: BlockHash,
    shutdown_flag: ShutdownFlag,
}

// TODO(Jira) review this file and take care of transactionality on storage saving: https://rsklabs.atlassian.net/browse/UB-11
// TODO(Jira) allow changing the initial_block_hash on a running instance: https://rsklabs.atlassian.net/browse/UB-32

impl<P: RskProvider, S: BlockStore> BlockIndexer<P, S> {
    pub fn new(
        store: S,
        provider: P,
        initial_block_hash: BlockHash,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            store,
            rsk_provider: provider,
            initial_block_hash,
            shutdown_flag,
        }
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }

    fn init_db_if_required(&self) -> Result<()> {
        let best_block: Option<RskBlock> =
            self.store.get_best_block().context("Initialising DB")?;
        if best_block.is_some() {
            return Ok(());
        }

        let initial_block_node = self
            .rsk_provider
            .get_block_by_hash(self.initial_block_hash)
            .context("Initialising DB")?
            .context("Initial block hash not found on provider while initialising DB")?;

        info!(
            "[initialize_db_if_required] New instance: initializing DB with {} ({})",
            initial_block_node.number(),
            initial_block_node.hash()
        );

        // initialize the store with the initial block info
        self.save_as_best_block(&initial_block_node)
            .context("Initialising DB")
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

    fn start_block_subscription(&self) -> Result<()> {
        if !self.is_running() {
            info!("[subscribe_blocks] Shutdown requested, skipping...");
            return Ok(());
        }

        info!("[subscribe_blocks] Start subscribe_blocks...");

        let mut rsk_block_subscription = self
            .rsk_provider
            .subscribe_blocks()
            .context("Failed to subscribe to blocks")?; // do not retry, this is the application startup

        let loop_result = self.listen_blocks(&mut rsk_block_subscription);

        rsk_block_subscription
            .unsubscribe()
            .and_then(|_| loop_result)
    }

    fn listen_blocks(
        &self,
        rsk_block_subscription: &mut impl RskSubscription<RskBlock>,
    ) -> Result<()> {
        while self.is_running() {
            let new_block = match rsk_block_subscription.next() {
                Ok(block) => block,
                Err(RskSubscriptionError::ClosedConnection) => {
                    if self.is_running() {
                        bail!("Provider closed unexpectedly!");
                    } else {
                        info!("[subscribe_blocks] Shutdown requested, quitting...");
                        break;
                    }
                }
                Err(RskSubscriptionError::Transient(err)) => {
                    error!("[subscribe_blocks] Ignoring problematic block: {err:?}");
                    continue;
                }
                Err(RskSubscriptionError::Lagged(err)) => {
                    error!(
                        "[subscribe_blocks] Subscription lagged, a backward_sync will be needed: {err:?}"
                    );
                    continue;
                }
                Err(RskSubscriptionError::Unexpected(err)) => {
                    bail!("[subscribe_blocks] Unknown error on block subs, quiting: {err:?}");
                }
            };

            // TODO(Jira) do batched writes in backward sync: https://rsklabs.atlassian.net/browse/UB-24

            // no need to keep track of it between iters as it is cached and can be re-fetched
            let local_best_block = self
                .store
                .get_best_block()
                .context("On Block subscription")?
                .context("Best block not found while listening blocks")?;

            let extends_canonical = new_block.parent_hash() == local_best_block.hash();
            let is_reorg = !extends_canonical
                && new_block.total_difficulty() > local_best_block.total_difficulty();

            if extends_canonical {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): setting new best",
                    new_block.number(),
                    new_block.hash()
                );
                self.save_as_best_block(&new_block)
                    .context("On Block subscription")?;
            } else if is_reorg {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): fixing local reorg",
                    new_block.number(),
                    new_block.hash()
                );
                let provider_best_block = self
                    .rsk_provider
                    .get_best_block()
                    .context("On Block subscription")?;
                self.backward_sync(&provider_best_block)
                    .context("On Block subscription")?;
            } else {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): neither extending, nor competing",
                    new_block.number(),
                    new_block.hash()
                );
                // just save the block as it is not part of the main chain (at least yet)
                self.store
                    .save_block(&new_block)
                    .context("On Block subscription")?;
            }
        }

        Ok(())
    }

    fn backward_sync(&self, starting_block: &RskBlock) -> Result<()> {
        if !self.is_running() {
            info!("[block_backward_sync] Shutdown requested, skipping...");
            return Ok(());
        }

        let store_best_block = self
            .store
            .get_best_block()
            .context("On Backward Sync")?
            .context("Best block not found in store during backward sync")?;

        info!(
            "[block_backward_sync] Connecting blocks {} ({}) and {} ({})",
            starting_block.number(),
            starting_block.hash(),
            store_best_block.number(),
            store_best_block.hash(),
        );

        let mut new_block = starting_block.clone();
        loop {
            let store_block = self
                .store
                .get_canonical_block(new_block.number())
                .context("On Backward Sync")?;

            let is_missing = store_block.is_none();
            let is_reorg = store_block.map_or(false, |sb| sb.hash() != new_block.hash());
            let reached_connection_height = new_block.number() <= store_best_block.number();

            if is_missing || is_reorg {
                info!(
                    "[block_backward_sync] {} block {} ({})...",
                    if is_reorg { "Replacing" } else { "Creating" },
                    new_block.number(),
                    new_block.hash(),
                );
                self.get_and_save_uncle_blocks(&new_block)
                    .context("On Backward Sync")?;
                self.save_as_canonical(&new_block)
                    .context("On Backward Sync")?;
            } else if !reached_connection_height {
                debug!(
                    "[block_backward_sync] Skipping known block {} ({}) while checking if fully connected",
                    new_block.number(),
                    new_block.hash()
                );
            } else {
                info!(
                    "[block_backward_sync] Completed at block {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // we are complete, so we remove the checkpoint if any
                self.store
                    .reset_back_sync_checkpoint()
                    .context("On Backward Sync")?;
                // it represents also the connection point to achieve full sync
                self.store
                    .set_best_block(&starting_block)
                    .context("On Backward Sync")?;

                break;
            }

            if !self.is_running() {
                warn!(
                    "[block_backward_sync] Shutdown requested, setting back_sync_checkpoint to {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // define backward_sync checkpoint to resume from
                self.store
                    .set_back_sync_checkpoint(&new_block)
                    .context("On Backward Sync")?;

                break;
            }

            if self.initial_block_hash == new_block.hash() || new_block.number() == 0 {
                error!("[block_backward_sync] Reached genesis or starting block, aborting...");
                break;
            }

            // no exit condition met, keep searching backwards
            new_block = self
                .get_next_backward_sync_block(new_block.number() - 1)
                .context("On Backward Sync")?;
        }

        Ok(())
    }

    fn resume_pending_backward_sync(&self) -> Result<()> {
        if let Some(checkpoint) = self
            .store
            .get_back_sync_checkpoint()
            .context("Resuming Pending Backward Sync")?
        {
            match self
                .rsk_provider
                .get_block_by_hash(checkpoint.parent_hash())
                .context("Resuming Backward Sync")?
            {
                Some(checkpoint_parent) => {
                    info!("[startup_backward_sync] Resuming previous...");
                    self.backward_sync(&checkpoint_parent)?;
                }
                None => {
                    warn!(
                        "[startup_backward_sync] Cannot resume from non canonical checkpoint {} ({})",
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
            let provider_best_block = self
                .rsk_provider
                .get_best_block()
                .context("On Full Sync Backward Sync rounds")?;
            if let Some(store_best_block) = self
                .store
                .get_best_block()
                .context("On Full Sync Backward Sync rounds")?
            {
                let is_full_sync = provider_best_block.hash() == store_best_block.hash();
                if is_full_sync {
                    debug!("[startup_backward_sync] No more rounds needed",);
                    return Ok(());
                } else if !self.is_running() {
                    return Ok(());
                } else {
                    info!("[startup_backward_sync] Running from tip round-{}", i);
                    self.backward_sync(&provider_best_block)
                        .context("On Full Sync Backward Sync rounds")?;
                }
            }
        }

        bail!(
            "Could not catch up to the tip after {} rounds",
            max_attempts
        )
    }

    fn save_as_canonical(&self, canonical_block: &RskBlock) -> Result<()> {
        self.store
            .save_block(&canonical_block)
            .context("Storing canonical block")?;
        // last, to avoid requiring db transactionality, as it is used to distinguish new block from reorgs
        self.store
            .set_canonical_block(&canonical_block)
            .context("Setting canonical block")
    }

    fn save_as_best_block(&self, new_block: &RskBlock) -> Result<()> {
        self.save_as_canonical(&new_block)
            .context("Saving canonical")?;
        // last is preferred to not mark as best a block that was not yet stored
        // furthermore, if this line is fails for any reason (or app quits on error right before
        // running), soon a new block will become best (either one extending or reorg)
        self.store
            .set_best_block(&new_block)
            .context("Saving as best block")
    }

    fn get_next_backward_sync_block(&self, block_num: BlockNumber) -> Result<RskBlock> {
        match self.rsk_provider.get_block_by_number(block_num)? {
            Some(block) => Ok(block),
            None => {
                // this means a reorg just have happened to a lower block num, so we start again from the best block
                warn!(
                    "[block_backward_sync] Could not get block {} from provider, retrying from best block",
                    block_num,
                );
                self.rsk_provider.get_best_block()
            }
        }
    }

    fn get_and_save_uncle_blocks(&self, new_block: &RskBlock) -> Result<()> {
        if new_block.uncles().is_empty() {
            return Ok(());
        }

        debug!(
            "[block_backward_sync] Attempting to get and save uncles blocks ({:?})",
            new_block.uncles()
        );

        new_block
            .uncles()
            .into_iter()
            .try_for_each(|uncle_hash| -> Result<()> {
                if let Some(uncle) = self
                    .rsk_provider
                    .get_block_by_hash(uncle_hash)
                    .context("Fetching uncle block")?
                {
                    self.store
                        .save_block(&uncle)
                        .context("Saving uncle block")?;
                } else {
                    warn!(
                        "[block_backward_sync] Possible orphan block detected – uncle not found: {}",
                        uncle_hash);
                }
                Ok(())
            })?;

        Ok(())
    }
}

impl<P: RskProvider, S: BlockStore> RskIndexer<P, S> for BlockIndexer<P, S> {
    fn run(&self) -> Result<()> {
        self.init_db_if_required()?;
        self.startup_backward_sync()?;
        self.start_block_subscription()
    }
}

#[cfg(all(test, feature = "test-mocks"))]
mod tests {
    use super::*;
    use crate::store::MockBlockStore;
    use common::{
        rsk_provider::MockRskProvider,
        test_utils::rsk_block_generator::{
            get_first_default_rsk_block, get_second_default_rsk_block,
        },
    };

    #[test]
    fn returns_ok_if_no_uncles() {
        let mut provider = MockRskProvider::new();
        let mut store = MockBlockStore::new();
        // no uncles
        let block = get_second_default_rsk_block();

        // neither provider nor store should ever be called
        provider.expect_get_block_by_hash().never();
        store.expect_save_block().never();

        let idx = BlockIndexer {
            rsk_provider: provider,
            store,
            initial_block_hash: block.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        assert!(idx.get_and_save_uncle_blocks(&block).is_ok());
    }

    #[test]
    fn saves_uncle_when_found() {
        let uncle_block = get_first_default_rsk_block();
        let uncle_hash = uncle_block.hash();

        let mut provider = MockRskProvider::new();
        provider
            .expect_get_block_by_hash()
            .with(eq(uncle_hash))
            .times(1)
            .returning(move || Ok(Some(uncle_block.clone())));

        let mut store = MockBlockStore::new();
        store
            .expect_save_block()
            .with(eq(uncle_block.clone()))
            .times(1)
            .returning(|_| Ok(()));

        let base = get_second_default_rsk_block();
        let block_with_uncle = RskBlock::new(
            base.number(),
            base.hash(),
            base.parent_hash(),
            base.timestamp(),
            base.difficulty(),
            base.total_difficulty(),
            base.pow(),
            vec![uncle_hash],
        );

        let idx = BlockIndexer {
            rsk_provider: provider,
            store,
            initial_block_hash: block_with_uncle.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let res = idx.get_and_save_uncle_blocks(&block_with_uncle);
        assert!(res.is_ok());
    }

    #[test]
    fn warns_but_ok_if_uncle_missing() {
        let missing_hash = get_first_default_rsk_block().hash();

        let mut provider = MockRskProvider::new();
        provider
            .expect_get_block_by_hash()
            .with(eq(missing_hash))
            .times(1)
            .returning(|| Ok(None));

        // store.save_block should never be called
        let mut store = MockBlockStore::new();
        store.expect_save_block().never();

        let base = get_second_default_rsk_block();
        let block_with_missing = RskBlock::new(
            base.number(),
            base.hash(),
            base.parent_hash(),
            base.timestamp(),
            base.difficulty(),
            base.total_difficulty(),
            base.pow(),
            vec![missing_hash],
        );

        let idx = BlockIndexer {
            rsk_provider: provider,
            store,
            initial_block_hash: block_with_missing.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let res = idx.get_and_save_uncle_blocks(&block_with_missing);
        assert!(res.is_ok());
    }
}
