use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use common_core::types::{BlockHash, BlockNumber, RskBlock};
use common_rsk::rsk_indexer::RskIndexer;
use common_rsk::rsk_provider::{
    RskProvider, RskSubscription, RskSubscriptionError, resolve_initial_block,
};
use common_runtime::config::{IndexerConfig, IndexerStartFrom};
use common_runtime::shutdown_flag::ShutdownFlag;
use tracing::{debug, error, info, instrument, warn};

use crate::notifier::BlockNotification;
use crate::store::BlockStore;

const BLOCK_DELIVERY_ACK_TIMEOUT: Duration = Duration::from_secs(30);

pub struct BlockIndexer<P: RskProvider, S: BlockStore> {
    store: S,
    rsk_provider: P,
    new_block_sender: Option<mpsc::Sender<BlockNotification>>,
    start_from: IndexerStartFrom,
    initial_block_hash: BlockHash,
    shutdown_flag: ShutdownFlag,
}

impl<P: RskProvider, S: BlockStore> BlockIndexer<P, S> {
    /// Create a new `BlockIndexer` with a notifier channel
    ///
    /// # Errors
    ///
    /// Returns an error if `start_from = "best"` and best block retrieval fails
    pub fn new_with_notifier(
        store: S,
        provider: P,
        new_block_sender: mpsc::Sender<BlockNotification>,
        indexer_config: &IndexerConfig,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block = resolve_initial_block(indexer_config, &provider)?;

        Ok(Self {
            store,
            rsk_provider: provider,
            new_block_sender: Some(new_block_sender),
            start_from: indexer_config.start_from,
            initial_block_hash: initial_block.hash(),
            shutdown_flag,
        })
    }

    /// Create a new `BlockIndexer`
    ///
    /// # Errors
    ///
    /// Returns an error if the initial block cannot be resolved from configuration/provider.
    pub fn new(
        store: S,
        provider: P,
        indexer_config: &IndexerConfig,
        shutdown_flag: ShutdownFlag,
    ) -> Result<Self> {
        let initial_block = resolve_initial_block(indexer_config, &provider)?;

        Ok(Self {
            store,
            rsk_provider: provider,
            new_block_sender: None,
            start_from: indexer_config.start_from,
            initial_block_hash: initial_block.hash(),
            shutdown_flag,
        })
    }

    fn get_initial_block(&self, provider: &P) -> RskBlock {
        let opt_block = provider.get_block_by_hash(self.initial_block_hash).unwrap_or_else(|_| {
            panic!(
                "Precondition failed: error fetching initial block {:?}",
                self.initial_block_hash
            )
        });

        opt_block.unwrap_or_else(|| {
            panic!("Precondition failed: initial block {:?} not found", self.initial_block_hash)
        })
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }

    fn init_db_if_required(&self, initial_block_node: &RskBlock) -> Result<()> {
        let best_block: Option<RskBlock> =
            self.store.get_best_block().context("Initialising DB")?;
        if best_block.is_some() {
            return Ok(());
        }

        info!(
            "[initialize_db_if_required] New instance: initializing DB with {} ({})",
            initial_block_node.number(),
            initial_block_node.hash()
        );

        // initialize the store with the initial block info
        self.save_as_best_block(initial_block_node).context("Initialising DB")
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

    fn init_for_best(&self, initial_block_node: &RskBlock) -> Result<()> {
        if let Some(db_best_block) = self.store.get_best_block().context("Checking DB state")? {
            info!(
                "[initialize_for_best] Existing best block {} ({}) found in DB; start_from='best' will catch up to provider best from the persisted tip",
                db_best_block.number(),
                db_best_block.hash(),
            );
            return self.startup_backward_sync();
        }

        info!(
            "[initialize_for_best] New instance: initializing DB with best block {} ({})",
            initial_block_node.number(),
            initial_block_node.hash()
        );

        self.save_as_best_block(initial_block_node).context("Initialising DB for best")
    }

    fn start_block_subscription(&self) -> Result<()> {
        if !self.is_running() {
            info!("[subscribe_blocks] Shutdown requested, skipping...");
            return Ok(());
        }

        info!("[subscribe_blocks] Start subscribe_blocks...");

        let mut rsk_block_subscription =
            self.rsk_provider.subscribe_blocks().context("Failed to subscribe to blocks")?; // do not retry, this is the application startup

        let loop_result = self.listen_blocks(&mut rsk_block_subscription);

        rsk_block_subscription.unsubscribe().and(loop_result)
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
                    }
                    info!("[subscribe_blocks] Shutdown requested, quitting...");
                    break;
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

            // no need to keep track of it between iters as it is cached and can be re-fetched
            let local_best_block = self
                .store
                .get_best_block()
                .context("On Block subscription")?
                .context("Best block not found while listening blocks")?;

            let extends_canonical = new_block.parent_hash() == local_best_block.hash();
            // We need to catch up whenever the new block does not directly
            // extend the canonical chain but is either taller than our local tip
            // or has higher Total Difficulty. Height alone is necessary because
            // Total Difficulty does not always grow strictly between blocks
            // (e.g. regtest), so a forward gap with flat Total Difficulty would
            // otherwise be misclassified as "neither extending, nor competing"
            // and leave local_best stuck.
            //
            // At this layer we cannot tell whether the trigger is a missed
            // canonical block or a reorg — backward_sync resolves that per
            // block and logs "Creating"/"Replacing" accordingly. We log
            // neutrally here.
            let needs_catch_up = new_block.number() > local_best_block.number()
                || new_block.total_difficulty() > local_best_block.total_difficulty();

            if extends_canonical {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): setting new best",
                    new_block.number(),
                    new_block.hash()
                );
                // order matters: 1) process uncles, 2) notify, 3) save as the best block
                // once save_as_best_block is called, the block won't be re-queried again (unless reorgs)
                let uncles = self.process_uncle_blocks(&new_block).context("On Backward Sync")?;
                // consumer should be resilient to re-notifications
                if !self.notify_block(new_block.clone(), uncles).context("On Block subscription")? {
                    break;
                }
                self.save_as_best_block(&new_block).context("On Block subscription")?;
            } else if needs_catch_up {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): catching up from local best {}",
                    new_block.number(),
                    new_block.hash(),
                    local_best_block.number()
                );
                let provider_best_block =
                    self.rsk_provider.get_best_block().context("On Block subscription")?;
                self.backward_sync(&provider_best_block).context("On Block subscription")?;
            } else {
                info!(
                    "[subscribe_blocks] Processing block {} ({}): neither extending, nor competing",
                    new_block.number(),
                    new_block.hash()
                );
                // just save the block as it is not part of the main chain (at least yet)
                self.store.save_block(&new_block).context("On Block subscription")?;
            }
        }

        Ok(())
    }

    fn notify_block(&self, block: RskBlock, uncles: Vec<RskBlock>) -> Result<bool> {
        #[allow(clippy::collapsible_if)]
        if let Some(channel) = &self.new_block_sender {
            let (delivery_ack_tx, delivery_ack_rx) = mpsc::channel();
            let notification = BlockNotification::new(
                common_core::types::RskBlockAndUncles::new(block, uncles),
                delivery_ack_tx,
            );
            if let Err(e) = channel.send(notification) {
                error!("[notify_block] Failed to send best block through channel: {e:?}");
                if self.is_running() {
                    bail!("[notify_block] Failed to send best block through channel: {e:?}");
                }
                return Ok(false);
            }

            match delivery_ack_rx.recv_timeout(BLOCK_DELIVERY_ACK_TIMEOUT) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    error!("[notify_block] Block delivery failed: {error:#}");
                    if self.is_running() {
                        bail!("[notify_block] Block delivery failed: {error:#}");
                    }
                    return Ok(false);
                }
                Err(RecvTimeoutError::Timeout) => {
                    error!("[notify_block] Timed out waiting for block delivery acknowledgement");
                    if self.is_running() {
                        bail!(
                            "[notify_block] Timed out waiting for block delivery acknowledgement"
                        );
                    }
                    return Ok(false);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    error!("[notify_block] Block delivery acknowledgement channel disconnected");
                    if self.is_running() {
                        bail!("[notify_block] Block delivery acknowledgement channel disconnected");
                    }
                    return Ok(false);
                }
            }
        }
        Ok(true)
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

        let mut blocks_to_notify = Vec::new();

        let mut new_block = starting_block.clone();
        loop {
            let store_block =
                self.store.get_canonical_block(new_block.number()).context("On Backward Sync")?;

            let is_missing = store_block.is_none();
            let is_reorg = store_block.is_some_and(|sb| sb.hash() != new_block.hash());
            let reached_connection_height = new_block.number() <= store_best_block.number();

            if is_missing || is_reorg {
                info!(
                    "[block_backward_sync] {} block {} ({})...",
                    if is_reorg { "Replacing" } else { "Creating" },
                    new_block.number(),
                    new_block.hash(),
                );
                // order matters: 1) process uncles, 2) notify, 3) save as canonical
                // once save_as_canonical is called, the block won't be re-queried again (unless reorgs)
                let uncles = self.process_uncle_blocks(&new_block).context("On Backward Sync")?;
                // consumer should be resilient to re-notifications
                blocks_to_notify.push((new_block.clone(), uncles.clone()));
                self.save_as_canonical(&new_block).context("On Backward Sync")?;
            } else if !reached_connection_height {
                debug!(
                    "[block_backward_sync] Skipping known block {} ({}) while checking if fully connected",
                    new_block.number(),
                    new_block.hash()
                );
                let uncles = self.process_uncle_blocks(&new_block).context("On Backward Sync")?;
                blocks_to_notify.push((new_block.clone(), uncles));
            } else {
                info!(
                    "[block_backward_sync] Completed at block {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // we are complete, so we remove the checkpoint if any
                self.store.reset_back_sync_checkpoint().context("On Backward Sync")?;
                // notify the consumer about the new chain in ascending order
                let mut all_blocks_notified = true;
                for (block, uncles) in blocks_to_notify.into_iter().rev() {
                    if !self.notify_block(block, uncles).context("On Backward Sync")? {
                        all_blocks_notified = false;
                        break;
                    }
                }
                if !all_blocks_notified {
                    break;
                }

                // it represents also the connection point to achieve full sync
                self.store.set_best_block(starting_block).context("On Backward Sync")?;

                break;
            }

            if !self.is_running() {
                warn!(
                    "[block_backward_sync] Shutdown requested, setting back_sync_checkpoint to {} ({})",
                    new_block.number(),
                    new_block.hash()
                );

                // define backward_sync checkpoint to resume from
                self.store.set_back_sync_checkpoint(&new_block).context("On Backward Sync")?;

                break;
            }

            let reached_initial_block = self.start_from == IndexerStartFrom::Hash
                && self.initial_block_hash == new_block.hash();
            if reached_initial_block || new_block.number() == 0 {
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
        if let Some(checkpoint) =
            self.store.get_back_sync_checkpoint().context("Resuming Pending Backward Sync")?
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
            let provider_best_block =
                self.rsk_provider.get_best_block().context("On Full Sync Backward Sync rounds")?;
            if let Some(store_best_block) =
                self.store.get_best_block().context("On Full Sync Backward Sync rounds")?
            {
                let is_full_sync = provider_best_block.hash() == store_best_block.hash();
                if is_full_sync {
                    debug!("[startup_backward_sync] No more rounds needed");
                    return Ok(());
                }

                if !self.is_running() {
                    return Ok(());
                }

                info!("[startup_backward_sync] Running from tip round-{i}");
                self.backward_sync(&provider_best_block)
                    .context("On Full Sync Backward Sync rounds")?;
            }
        }

        bail!("Could not catch up to the tip after {max_attempts} rounds")
    }

    fn save_as_canonical(&self, canonical_block: &RskBlock) -> Result<()> {
        self.store.save_block(canonical_block).context("Storing canonical block")?;
        // last, to avoid requiring db transactionality, as it is used to distinguish new block from reorgs
        self.store.set_canonical_block(canonical_block).context("Setting canonical block")
    }

    fn save_as_best_block(&self, new_block: &RskBlock) -> Result<()> {
        self.save_as_canonical(new_block).context("Saving canonical")?;
        // last is preferred to not mark as best a block that was not yet stored
        // furthermore, if this line is fails for any reason (or app quits on error right before
        // running), soon a new block will become best (either one extending or reorg)
        self.store.set_best_block(new_block).context("Saving as best block")
    }

    fn get_next_backward_sync_block(&self, block_num: BlockNumber) -> Result<RskBlock> {
        if let Some(block) = self.rsk_provider.get_block_by_number(block_num)? {
            Ok(block)
        } else {
            // this means a reorg just have happened to a lower block num, so we start again from the best block
            warn!(
                "[block_backward_sync] Could not get block {block_num} from provider, retrying from best block"
            );
            self.rsk_provider.get_best_block()
        }
    }

    fn process_uncle_blocks(&self, new_block: &RskBlock) -> Result<Vec<RskBlock>> {
        if new_block.uncles().is_empty() {
            return Ok(Vec::new());
        }

        let nephew_hash = new_block.hash();
        let nephew_number = new_block.number();

        debug!(
            "[block_backward_sync] Nephew {} (#{}) has uncles: {:?}",
            nephew_hash,
            nephew_number,
            new_block.uncles()
        );

        let mut uncle_blocks = Vec::new();

        let uncle_amount = new_block.uncles().len();
        for i in 0..uncle_amount {
            if let Some(uncle) = self
                .rsk_provider
                .get_uncle_by_hash_and_index(nephew_hash, i as u64)
                .context("Fetching uncle block")?
            {
                if new_block.uncles().contains(&uncle.hash()) {
                    self.store.save_block(&uncle).context("Saving uncle block")?;
                    uncle_blocks.push(uncle);
                } else {
                    warn!(
                        "[block_backward_sync] Received uncle {} is not in nephew {} (#{}) uncles list: {:?}",
                        uncle.hash(),
                        nephew_hash,
                        nephew_number,
                        new_block.uncles()
                    );
                }
            } else {
                warn!(
                    "[block_backward_sync] Possible orphan – nephew {} (#{}) references missing uncle {}",
                    nephew_hash,
                    nephew_number,
                    new_block.uncles()[i]
                );
            }
        }
        Ok(uncle_blocks)
    }
}

impl<P: RskProvider, S: BlockStore> RskIndexer<P, S> for BlockIndexer<P, S> {
    #[instrument(skip_all)]
    fn run(&self) -> Result<()> {
        let initial_block = self.get_initial_block(&self.rsk_provider);

        match self.start_from {
            IndexerStartFrom::Best => {
                info!("[run] start_from='best': initializing from provider best");
                self.init_for_best(&initial_block)?;
            }
            IndexerStartFrom::Hash => {
                info!("[run] start_from='hash': running startup backward sync");
                self.init_db_if_required(&initial_block)?;
                self.startup_backward_sync()?;
            }
        }

        self.start_block_subscription()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    use common_broker::broker::{BrokerError, Identifier, MockBrokerServerApi};
    use common_core::types::RskBlockAndUncles;
    use common_dev::rsk_block_generator::{
        FakeBlockGenerator, get_first_default_rsk_block, get_second_default_rsk_block,
        get_third_default_rsk_block,
    };
    use common_rsk::rsk_provider::{MockRskProvider, MockRskSubscription};
    use mockall::predicate::eq;

    use super::*;
    use crate::notifier::{BlockNotification, FromServer, Notifier, ToServer};
    use crate::store::MockBlockStore;

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
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Hash,
            initial_block_hash: block.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        assert!(idx.process_uncle_blocks(&block).is_ok());
    }

    #[test]
    fn saves_uncle_when_found() {
        let base: RskBlock = get_second_default_rsk_block();
        let uncle_block = get_first_default_rsk_block();
        let uncle_hash = uncle_block.hash();

        let mut store = MockBlockStore::new();
        store.expect_save_block().with(eq(uncle_block.clone())).times(1).returning(|_| Ok(()));

        let mut provider: MockRskProvider = MockRskProvider::new();
        provider
            .expect_get_uncle_by_hash_and_index()
            .with(eq(base.hash()), eq(0))
            .times(1)
            .returning(move |_, _| Ok(Some(uncle_block.clone())));

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
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Hash,
            initial_block_hash: block_with_uncle.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let res = idx.process_uncle_blocks(&block_with_uncle);
        assert!(res.is_ok());
    }

    #[test]
    fn warns_but_ok_if_uncle_missing() {
        let base = get_second_default_rsk_block();
        let missing_hash = get_first_default_rsk_block().hash();

        let mut provider = MockRskProvider::new();
        provider
            .expect_get_uncle_by_hash_and_index()
            .with(eq(base.hash()), eq(0))
            .times(1)
            .returning(|_, _| Ok(None));

        // store.save_block should never be called
        let mut store = MockBlockStore::new();
        store.expect_save_block().never();

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
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Hash,
            initial_block_hash: block_with_missing.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let res = idx.process_uncle_blocks(&block_with_missing);
        assert!(res.is_ok());
    }

    #[test]
    fn warns_but_ok_if_uncle_mismatch_listed_hashes() {
        let base = get_third_default_rsk_block();
        let uncle_hash = get_second_default_rsk_block().hash();

        let mut provider = MockRskProvider::new();
        provider
            .expect_get_uncle_by_hash_and_index()
            .with(eq(base.hash()), eq(0))
            .times(1)
            .returning(move |_, _| Ok(Some(get_first_default_rsk_block())));

        let mut store: MockBlockStore = MockBlockStore::new();
        store.expect_save_block().never();

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
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Hash,
            initial_block_hash: block_with_uncle.parent_hash(),
            shutdown_flag: ShutdownFlag::init(),
        };
        let res = idx.process_uncle_blocks(&block_with_uncle);
        assert!(res.is_ok());
    }

    #[test]
    fn returns_err_when_initial_block_hash_not_found() {
        // Given a random hash that the provider won't find...
        use primitive_types::H256;
        let missing_hash = BlockHash::from(H256::random());
        let indexer_config = IndexerConfig {
            start_from: IndexerStartFrom::Hash,
            initial_block_hash: Some(missing_hash.to_string()),
            sync: common_runtime::config::SyncConfig { finality_depth: 0, batch_size: 0 },
            storage: common_runtime::config::StorageConfig { path: String::new() },
            cache: common_runtime::config::CacheConfig { size: 0 },
        };

        // Provider that returns Ok(None) for our missing hash
        let mut provider = MockRskProvider::new();
        provider.expect_get_block_by_hash().with(eq(missing_hash)).times(1).returning(|_| Ok(None));

        let result = BlockIndexer::new(
            MockBlockStore::new(),
            provider,
            &indexer_config,
            ShutdownFlag::init(),
        );

        match result {
            Ok(_) => {
                panic!("Expected constructor to fail when initial hash is missing on provider")
            }
            Err(err) => assert_eq!("Initial block not found on provider", err.to_string()),
        }
    }

    #[test]
    fn backward_sync_with_start_from_best_connects_below_initial_best() {
        let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        let connected_block =
            generator.generate_block(100.into(), None).expect("failed to generate block 100");
        let middle_block =
            generator.generate_block(101.into(), None).expect("failed to generate block 101");
        let starting_block =
            generator.generate_block(102.into(), None).expect("failed to generate block 102");

        let mut provider = MockRskProvider::new();
        let middle_block_for_provider = middle_block.clone();
        let connected_block_for_provider = connected_block.clone();
        provider.expect_get_block_by_number().times(2).returning(move |block_num| {
            if block_num == middle_block_for_provider.number() {
                Ok(Some(middle_block_for_provider.clone()))
            } else if block_num == connected_block_for_provider.number() {
                Ok(Some(connected_block_for_provider.clone()))
            } else {
                Ok(None)
            }
        });
        provider.expect_get_uncle_by_hash_and_index().never();

        let mut store = MockBlockStore::new();
        let store_best_block = connected_block.clone();
        store
            .expect_get_best_block()
            .times(1)
            .returning(move || Ok(Some(store_best_block.clone())));

        let connected_block_for_store = connected_block.clone();
        store.expect_get_canonical_block().times(3).returning(move |block_num| {
            if block_num == connected_block_for_store.number() {
                Ok(Some(connected_block_for_store.clone()))
            } else {
                Ok(None)
            }
        });

        store.expect_save_block().times(2).returning(|_| Ok(()));
        store.expect_set_canonical_block().times(2).returning(|_| Ok(()));
        store.expect_reset_back_sync_checkpoint().times(1).returning(|| Ok(()));
        store
            .expect_set_best_block()
            .with(eq(starting_block.clone()))
            .times(1)
            .returning(|_| Ok(()));
        store.expect_set_back_sync_checkpoint().never();

        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: starting_block.hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = idx.backward_sync(&starting_block);
        assert!(result.is_ok());
    }

    #[test]
    fn listen_blocks_does_not_advance_best_when_notifier_channel_is_closed() {
        let local_best = get_first_default_rsk_block();
        let new_block = get_second_default_rsk_block();

        let mut provider = MockRskProvider::new();
        provider.expect_get_uncle_by_hash_and_index().never();

        let mut store = MockBlockStore::new();
        let local_best_for_store = local_best.clone();
        store
            .expect_get_best_block()
            .times(1)
            .returning(move || Ok(Some(local_best_for_store.clone())));
        store.expect_save_block().never();
        store.expect_set_canonical_block().never();
        store.expect_set_best_block().never();

        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);

        let mut subscription = MockRskSubscription::<RskBlock>::new();
        subscription.expect_next().times(1).returning(move || Ok(new_block.clone()));

        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: Some(tx),
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: local_best.hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = idx.listen_blocks(&mut subscription);
        assert!(result.is_err());
    }

    #[test]
    fn listen_blocks_does_not_advance_best_when_broker_delivery_fails() {
        let local_best = get_first_default_rsk_block();
        let new_block = get_second_default_rsk_block();

        let mut provider = MockRskProvider::new();
        provider.expect_get_uncle_by_hash_and_index().never();

        let mut store = MockBlockStore::new();
        let local_best_for_store = local_best.clone();
        store
            .expect_get_best_block()
            .times(1)
            .returning(move || Ok(Some(local_best_for_store.clone())));
        store.expect_save_block().never();
        store.expect_set_canonical_block().never();
        store.expect_set_best_block().never();

        let (tx, rx) = std::sync::mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();
        let coordinator_id = Identifier::new("coordinator".to_string(), 1);

        let mut mock_broker = MockBrokerServerApi::<ToServer, FromServer>::new();
        mock_broker.expect_try_recv().returning(|| Ok(None));
        mock_broker.expect_send().times(1).returning(|_, _| Err(BrokerError::disconnected()));

        let mut notifier =
            Notifier::new_with_consumer(rx, mock_broker, shutdown_flag.clone(), coordinator_id);
        let notifier_handle = thread::spawn(move || notifier.run());

        let mut subscription = MockRskSubscription::<RskBlock>::new();
        subscription.expect_next().times(1).returning(move || Ok(new_block.clone()));

        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: Some(tx),
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: local_best.hash(),
            shutdown_flag: shutdown_flag.clone(),
        };

        let result = idx.listen_blocks(&mut subscription);
        shutdown_flag.set();

        assert!(result.is_err());
        assert!(notifier_handle.join().expect("notifier thread panicked").is_err());
    }

    #[test]
    fn backward_sync_renotifies_known_blocks_above_best_before_advancing_best() {
        let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        let connected_block =
            generator.generate_block(100.into(), None).expect("failed to generate block 100");
        let middle_block =
            generator.generate_block(101.into(), None).expect("failed to generate block 101");
        let starting_block =
            generator.generate_block(102.into(), None).expect("failed to generate block 102");

        let mut provider = MockRskProvider::new();
        let middle_block_for_provider = middle_block.clone();
        let connected_block_for_provider = connected_block.clone();
        provider.expect_get_block_by_number().times(2).returning(move |block_num| {
            if block_num == middle_block_for_provider.number() {
                Ok(Some(middle_block_for_provider.clone()))
            } else if block_num == connected_block_for_provider.number() {
                Ok(Some(connected_block_for_provider.clone()))
            } else {
                Ok(None)
            }
        });
        provider.expect_get_uncle_by_hash_and_index().never();

        let mut store = MockBlockStore::new();
        let store_best_block = connected_block.clone();
        store
            .expect_get_best_block()
            .times(1)
            .returning(move || Ok(Some(store_best_block.clone())));

        let starting_block_for_store = starting_block.clone();
        let middle_block_for_store = middle_block.clone();
        let connected_block_for_store = connected_block.clone();
        store.expect_get_canonical_block().times(3).returning(move |block_num| {
            if block_num == starting_block_for_store.number() {
                Ok(Some(starting_block_for_store.clone()))
            } else if block_num == middle_block_for_store.number() {
                Ok(Some(middle_block_for_store.clone()))
            } else if block_num == connected_block_for_store.number() {
                Ok(Some(connected_block_for_store.clone()))
            } else {
                Ok(None)
            }
        });

        store.expect_save_block().never();
        store.expect_set_canonical_block().never();
        store.expect_reset_back_sync_checkpoint().times(1).returning(|| Ok(()));
        store
            .expect_set_best_block()
            .with(eq(starting_block.clone()))
            .times(1)
            .returning(|_| Ok(()));
        store.expect_set_back_sync_checkpoint().never();

        let (tx, rx) = std::sync::mpsc::channel();
        let ack_handle = collect_and_ack_blocks(rx, 2);
        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: Some(tx),
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: starting_block.hash(),
            shutdown_flag: ShutdownFlag::init(),
        };

        let result = idx.backward_sync(&starting_block);
        assert!(result.is_ok());

        let notified = ack_handle.join().expect("ack thread panicked");
        assert_eq!(notified.len(), 2);
        assert_eq!(notified[0].block(), &middle_block);
        assert_eq!(notified[1].block(), &starting_block);
    }

    #[test]
    fn backward_sync_does_not_advance_best_when_broker_delivery_fails() {
        let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        let connected_block =
            generator.generate_block(100.into(), None).expect("failed to generate block 100");
        let middle_block =
            generator.generate_block(101.into(), None).expect("failed to generate block 101");
        let starting_block =
            generator.generate_block(102.into(), None).expect("failed to generate block 102");

        let mut provider = MockRskProvider::new();
        let middle_block_for_provider = middle_block.clone();
        let connected_block_for_provider = connected_block.clone();
        provider.expect_get_block_by_number().times(2).returning(move |block_num| {
            if block_num == middle_block_for_provider.number() {
                Ok(Some(middle_block_for_provider.clone()))
            } else if block_num == connected_block_for_provider.number() {
                Ok(Some(connected_block_for_provider.clone()))
            } else {
                Ok(None)
            }
        });
        provider.expect_get_uncle_by_hash_and_index().never();

        let mut store = MockBlockStore::new();
        let store_best_block = connected_block.clone();
        store
            .expect_get_best_block()
            .times(1)
            .returning(move || Ok(Some(store_best_block.clone())));

        let starting_block_for_store = starting_block.clone();
        let middle_block_for_store = middle_block.clone();
        let connected_block_for_store = connected_block.clone();
        store.expect_get_canonical_block().times(3).returning(move |block_num| {
            if block_num == starting_block_for_store.number() {
                Ok(Some(starting_block_for_store.clone()))
            } else if block_num == middle_block_for_store.number() {
                Ok(Some(middle_block_for_store.clone()))
            } else if block_num == connected_block_for_store.number() {
                Ok(Some(connected_block_for_store.clone()))
            } else {
                Ok(None)
            }
        });

        store.expect_save_block().never();
        store.expect_set_canonical_block().never();
        store.expect_reset_back_sync_checkpoint().times(1).returning(|| Ok(()));
        store.expect_set_best_block().never();
        store.expect_set_back_sync_checkpoint().never();

        let (tx, rx) = std::sync::mpsc::channel();
        let shutdown_flag = ShutdownFlag::init();
        let coordinator_id = Identifier::new("coordinator".to_string(), 1);

        let mut mock_broker = MockBrokerServerApi::<ToServer, FromServer>::new();
        mock_broker.expect_try_recv().returning(|| Ok(None));
        mock_broker.expect_send().times(1).returning(|_, _| Err(BrokerError::disconnected()));

        let mut notifier =
            Notifier::new_with_consumer(rx, mock_broker, shutdown_flag.clone(), coordinator_id);
        let notifier_handle = thread::spawn(move || notifier.run());

        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: Some(tx),
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: starting_block.hash(),
            shutdown_flag: shutdown_flag.clone(),
        };

        let result = idx.backward_sync(&starting_block);
        shutdown_flag.set();

        assert!(result.is_err());
        assert!(notifier_handle.join().expect("notifier thread panicked").is_err());
    }

    fn collect_and_ack_blocks(
        rx: std::sync::mpsc::Receiver<BlockNotification>,
        expected_blocks: usize,
    ) -> thread::JoinHandle<Vec<RskBlockAndUncles>> {
        thread::spawn(move || {
            let mut blocks = Vec::with_capacity(expected_blocks);
            for block_index in 0..expected_blocks {
                let notification = rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap_or_else(|_| panic!("expected block notification {block_index}"));
                blocks.push(notification.block().clone());
                notification.acknowledge(Ok(()));
            }
            blocks
        })
    }

    #[test]
    fn run_with_start_from_best_existing_db_does_not_reset_to_provider_best() {
        let initial_block = get_second_default_rsk_block();
        let initial_hash = initial_block.hash();
        let db_best_block = get_first_default_rsk_block();

        let mut provider = MockRskProvider::new();
        let initial_block_for_hash = initial_block.clone();
        provider
            .expect_get_block_by_hash()
            .with(eq(initial_hash))
            .times(1)
            .returning(move |_| Ok(Some(initial_block_for_hash.clone())));
        provider.expect_get_best_block().times(1).returning(move || Ok(initial_block.clone()));
        provider.expect_get_block_by_number().never();
        provider.expect_subscribe_blocks().never();

        let mut store = MockBlockStore::new();
        store.expect_get_best_block().times(2).returning(move || Ok(Some(db_best_block.clone())));
        store.expect_get_back_sync_checkpoint().times(1).returning(|| Ok(None));
        store.expect_get_canonical_block().never();
        store.expect_set_back_sync_checkpoint().never();
        store.expect_save_block().never();
        store.expect_set_canonical_block().never();
        store.expect_set_best_block().never();
        store.expect_reset_back_sync_checkpoint().never();

        let shutdown_flag = ShutdownFlag::init();
        shutdown_flag.set();

        let idx = BlockIndexer {
            rsk_provider: provider,
            new_block_sender: None,
            store,
            start_from: IndexerStartFrom::Best,
            initial_block_hash: initial_hash,
            shutdown_flag,
        };

        let result = idx.run();
        assert!(result.is_ok());
    }
}
