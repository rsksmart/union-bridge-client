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

                // we always save the block by hash
                self.store.save_block(&new_block)?;

                let extends_canonical = new_block.parent() == tip_block.hash();
                let requires_local_reorg = !extends_canonical
                    && new_block.total_difficulty() > tip_block.total_difficulty();

                if extends_canonical {
                    // set canonical fields
                    self.store.set_best_block(&new_block)?;
                    self.store.set_canonical_block(&new_block)?;
                    // set last connected block to this new best block
                    self.store.set_last_connected_block(&new_block)?;

                    info!(
                        "Processed block {} ({}): new tip",
                        new_block.number(),
                        new_block.hash()
                    );

                    tip_block = new_block;
                } else if requires_local_reorg {
                    info!(
                        "Processed block {} ({}): local reorg, run backward sync",
                        new_block.number(),
                        new_block.hash()
                    );
                    // backward_sync fixes reorgs internally (if any)
                    tip_block = self.backward_sync(&shutdown_flag)?;
                } else {
                    info!(
                        "Processing block {} ({}): non extending, non competing",
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
        let mut connection_point_reached = false;

        let mut node_block: RskBlock = best_block.clone();
        let mut store_block_opt = self.store.get_block_by_number(node_block.number())?;

        while !shutdown_flag.is_on() && !connection_point_reached {
            let is_reorg = store_block_opt
                .as_ref()
                .map(|sb| sb.hash() != node_block.hash())
                .unwrap_or(false);

            if store_block_opt.is_none() || is_reorg {
                info!(
                    "Creating or updating block {} ({})...",
                    node_block.number(),
                    node_block.hash()
                );

                // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
                self.store.save_block(&node_block)?;
                self.store.set_canonical_block(&node_block)?;
            } else {
                debug!(
                "Already stored block {} ({}) while trying to reach connection_hash, nothing to do",
                node_block.number(),
                node_block.hash()
            );
            }

            // TODO(Jira) request and persist uncles: https://rsklabs.atlassian.net/browse/UB-16

            let parent_block_num = node_block.number() - 1;
            node_block = self.rsk_provider.get_block_by_number(parent_block_num)?;
            store_block_opt = self.store.get_block_by_number(parent_block_num)?;

            let is_target_num_reached = store_block_opt
                .as_ref()
                .map(|sb| sb.number() <= target_block_num)
                .unwrap_or(false);

            if is_target_num_reached {
                let store_block = store_block_opt.as_ref().unwrap();
                if store_block.hash() == node_block.hash() {
                    connection_point_reached = true;
                    self.store.set_last_connected_block(&best_block)?;
                    // TODO(iago) improve this log for the usage within subscription
                    info!("backward_sync completed!");
                } else {
                    target_block_num -= 1;
                    debug!("decrementing target block: {}", target_block_num);
                }
            }
        }

        // set the best block now that we are done
        self.store.set_best_block(&best_block)?;

        Ok(best_block)
    }

    fn initialize_db_if_required(&self) -> Result<()> {
        let initial_block: Option<RskBlock> = self
            .store
            .get_block_by_hash(self.initial_block_hash.deref())?
            .or(None);

        match initial_block {
            Some(_) => Ok(()),
            None => {
                let initial_block = self
                    .rsk_provider
                    .get_block_by_hash(self.initial_block_hash.deref())?; // TODO(iago) resilient to not found

                info!(
                    "First backward sync, setting last_connected_block to initial block {} ({})",
                    initial_block.number(),
                    initial_block.hash()
                );

                // TODO(Jira) take care of transactionality: https://rsklabs.atlassian.net/browse/UB-11
                // initialize the store with the initial block info
                self.store.set_canonical_block(&initial_block)?;
                self.store.set_best_block(&initial_block)?;
                self.store.set_last_connected_block(&initial_block)?;
                // should go last, as it will be used to determine if the DB is initialized
                self.store.save_block(&initial_block)
            }
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
