use crate::provider::{
    AlloyProvider, RskApi, RskBlockSubscription, RskBlockSubscriptionApi, RskProvider,
};
use crate::store::CachedKeyValueStore;
use crate::types::RskBlock;
use crate::utils::ShutdownFlag;
use anyhow::{anyhow, Result};
use log::{debug, error, info};
use std::ops::Deref;
use std::thread;
use std::time::Duration;

pub struct Indexer {
    store: CachedKeyValueStore,
    rsk_provider: RskApi<AlloyProvider>,
    initial_block_hash: String,
}

impl Indexer {
    pub fn new(store: CachedKeyValueStore, ws_url: &str, initial_block_hash: &str) -> Self {
        // TODO(Jira) WS resilience: https://rsklabs.atlassian.net/browse/UB-15
        let rsk_provider = RskApi::new(ws_url);

        Self {
            store,
            rsk_provider,
            initial_block_hash: initial_block_hash.to_string(),
        }
    }

    pub fn run(&self, shutdown_flag: ShutdownFlag) -> Result<()> {
        self.initialize_db_if_required()?;

        // after boot, we do a backward_sync to catch up with the node
        self.backward_sync(&shutdown_flag)?;
        info!("Initial backward sync completed!");

        if shutdown_flag.is_on() {
            return Ok(());
        }

        self.subscribe_blocks(shutdown_flag)
    }

    fn subscribe_blocks(&self, shutdown_flag: ShutdownFlag) -> Result<()> {
        info!("Start subscribe_blocks...");

        let mut rsk_block_subscription = RskBlockSubscriptionApi::new(&self.rsk_provider);

        let mut tip_block = self
            .store
            .get_best_block()?
            .ok_or_else(|| anyhow!("Failed to get best_block from store"))?;

        let loop_result = (|| {
            while !shutdown_flag.is_on() {
                let new_block = rsk_block_subscription.try_next()?;
                if new_block.is_none() {
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }

                let new_block = new_block.unwrap();

                debug!("Fetched RSK block on subscription: {:?}", new_block);

                // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
                // TODO(Jira) do batched writes in backward sync: https://rsklabs.atlassian.net/browse/UB-24

                // we always save the block by hash for potential future connection
                self.store.save_block(&new_block)?;

                let extends_canonical = new_block.parent() == tip_block.hash();
                let requires_local_reorg = !extends_canonical
                    && new_block.total_difficulty() > tip_block.total_difficulty();

                if extends_canonical {
                    info!(
                        "Processing block {} ({}): setting new tip",
                        new_block.number(),
                        new_block.hash()
                    );

                    // set canonical fields
                    self.store.set_best_block(&new_block)?;
                    self.store.set_canonical_block(&new_block)?;
                    // set last connected block to this new best block
                    self.store.set_last_connected_block(&new_block)?;

                    tip_block = new_block;
                } else if requires_local_reorg {
                    info!(
                        "Processing block {} ({}): fixing local reorg",
                        new_block.number(),
                        new_block.hash()
                    );
                    // backward_sync fixes reorgs internally (if any)
                    tip_block = self.backward_sync(&shutdown_flag)?;
                    debug!("Local reorg fixed!");
                } else {
                    info!(
                        "Processing block {} ({}): neither extending, nor competing",
                        new_block.number(),
                        new_block.hash()
                    );
                }
            }

            Ok(())
        })();

        match rsk_block_subscription.unsubscribe() {
            Ok(_) => (),
            Err(e) => error!("Failed to unsubscribe from rsk_block_subscription: {:?}", e),
        }

        loop_result
    }

    fn backward_sync(&self, shutdown_flag: &ShutdownFlag) -> Result<RskBlock> {
        let last_connected_block = self
            .store
            .get_last_connected_block()?
            .ok_or_else(|| anyhow!("Failed to get last_connected_block from store"))?;

        let best_block = self.rsk_provider.get_best_block()?; // TODO(iago) resilient to not found

        if best_block.hash() == last_connected_block.hash() {
            info!(
                "No backward sync needed, already at block {} ({})",
                best_block.number(),
                best_block.hash()
            );
            return Ok(best_block);
        }

        info!(
            "Running backward_sync from block {} ({}) to {} ({})",
            best_block.number(),
            best_block.hash(),
            last_connected_block.number(),
            last_connected_block.hash(),
        );

        let mut target_block_num = last_connected_block.number();
        let mut node_block: RskBlock = best_block.clone();
        let mut store_block_opt = self.store.get_block_by_number(node_block.number())?;

        // TODO(Jira) request and persist uncles during this process: https://rsklabs.atlassian.net/browse/UB-16

        while !shutdown_flag.is_on() {
            let (is_missing_block, is_reorg) = match store_block_opt {
                None => (true, false),
                Some(sb) => (false, sb.hash() != node_block.hash()),
            };

            if is_missing_block || is_reorg {
                info!(
                    "{} block {} ({})...",
                    if is_reorg { "Replacing" } else { "Creating" },
                    node_block.number(),
                    node_block.hash(),
                );

                // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
                self.store.save_block(&node_block)?;
                self.store.set_canonical_block(&node_block)?;
            } else {
                debug!(
                    "No need to store existing block {} ({}) (while trying to reach connection point)",
                    node_block.number(),
                    node_block.hash()
                );
            }

            // at this point, we have stored the same block provided by the node, reassigning for clarity
            let store_block = node_block;

            let target_num_reached = store_block.number() == target_block_num;
            let connection_found = target_num_reached && !is_reorg;
            if connection_found {
                // connection point found, done, setting last_connected_block to best_block
                self.store.set_last_connected_block(&best_block)?;
                break;
            }

            Self::safety_bound_check(target_block_num, &self.initial_block_hash, &store_block);

            if target_num_reached {
                target_block_num -= 1;
                debug!("decrementing target block num to {}", target_block_num);
            }

            let parent_block_num = store_block.number() - 1;
            node_block = self.rsk_provider.get_block_by_number(parent_block_num)?;
            store_block_opt = self.store.get_block_by_number(parent_block_num)?;
        }

        // set the best block now that we are done
        self.store.set_best_block(&best_block)?;

        Ok(best_block)
    }

    fn initialize_db_if_required(&self) -> Result<()> {
        let initial_block_store_opt: Option<RskBlock> = self
            .store
            .get_block_by_hash(self.initial_block_hash.deref())?
            .or(None);

        if initial_block_store_opt.is_some() {
            return Ok(());
        }

        let initial_block_node = self
            .rsk_provider
            .get_block_by_hash(self.initial_block_hash.deref())?; // TODO(iago) resilient to not found

        info!(
            "First backward sync, setting last_connected_block to initial block {} ({})",
            initial_block_node.number(),
            initial_block_node.hash()
        );

        // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
        // initialize the store with the initial block info
        self.store.set_canonical_block(&initial_block_node)?;
        self.store.set_best_block(&initial_block_node)?;
        self.store.set_last_connected_block(&initial_block_node)?;
        // should go last, as it will be used to determine if the DB is initialized
        self.store.save_block(&initial_block_node)
    }

    fn safety_bound_check(target_block_num: u64, initial_block_hash: &str, store_block: &RskBlock) {
        // TODO decide what to do with this safety checks: now it panics, which will turn the monitor off

        if store_block.hash() == initial_block_hash {
            panic!(
                "Reached initial block {} without connection point",
                store_block.hash()
            );
        }

        if store_block.number() == 0 || target_block_num == 0 {
            panic!("Reached block 0 without finding connection point");
        }
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        if let Err(e) = self.rsk_provider.disconnect() {
            error!("Failed to disconnect provider: {:?}", e);
        }
    }
}
