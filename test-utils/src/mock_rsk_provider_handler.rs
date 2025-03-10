use crate::rsk_log_generator::FakeLogGenerator;
use crate::{rsk_block_generator::FakeBlockGenerator, rsk_utils::DEFAULT_BLOCK_HASH};
use anyhow::anyhow;
use common::{
    rsk_provider::{
        MockRskProvider, MockRskSubscription, RskSubscriptionError, RskSubscriptionFilter,
    },
    shutdown_flag::ShutdownFlag,
    types::{BlockHash, BlockNumber, ContractInfo, LogInfo, RskBlock, RskEvent, RskLog},
};
use log::info;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub struct MockRskProviderHandler<'a> {
    provider: &'a mut MockRskProvider,
    block_generator: FakeBlockGenerator,
    log_generator: FakeLogGenerator,
    is_reorg: Arc<AtomicBool>,
    has_subscribed: Arc<AtomicBool>,
    shutting_down: ShutdownFlag,
    block_height_backward_sync_init: BlockNumber,
    block_height_backward_sync_max: BlockNumber,
    block_height_subscription_max: BlockNumber,
    delay_between_blocks_subscription: u64,
}

impl<'a> MockRskProviderHandler<'a> {
    pub fn new(
        provider: &'a mut MockRskProvider,
        block_generator: &FakeBlockGenerator,
        is_reorg: Arc<AtomicBool>,
        shutting_down: ShutdownFlag,
        block_height_backward_sync_init: BlockNumber,
        block_height_backward_sync_max: BlockNumber,
        block_height_subscription_max: BlockNumber,
        delay_between_blocks_subscription: u64,
    ) -> Self {
        Self {
            provider,
            block_generator: block_generator.clone(),
            log_generator: FakeLogGenerator::new(),
            is_reorg,
            has_subscribed: Arc::new(AtomicBool::new(false)),
            shutting_down,
            block_height_backward_sync_init,
            block_height_backward_sync_max,
            block_height_subscription_max,
            delay_between_blocks_subscription,
        }
    }

    pub fn set_provider_expect_get_block_by_hash(
        &mut self,
        expected_block_hash: BlockHash,
        block_height: BlockNumber,
    ) {
        let generator = self.block_generator.clone();
        self.provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(expected_block_hash))
            .returning(move |_hash| Ok(Some(generator.generate_block(block_height))));
    }

    pub fn set_provider_expect_get_best_block(&mut self) {
        let generator = self.block_generator.clone();
        let has_subscribed = self.has_subscribed.clone();
        let block_height_subscription_max = self.block_height_subscription_max;
        let block_height_backward_sync_max = self.block_height_backward_sync_max;
        self.provider.expect_get_best_block().returning(move || {
            // whenever a subscription was activated, the best block is the subscription max
            let block_height = if has_subscribed.load(Ordering::SeqCst) {
                block_height_subscription_max
            } else {
                block_height_backward_sync_max
            };
            Ok(generator.generate_block(block_height))
        });
    }

    pub fn set_provider_expect_get_block_by_number(
        &mut self,
        simul_reorg_happens_at_height: Option<BlockNumber>,
        simul_shutdown_height: Option<BlockNumber>,
    ) {
        let generator = self.block_generator.clone();
        let block_height_backward_sync_init = self.block_height_backward_sync_init;
        let block_height_backward_sync_max = self.block_height_backward_sync_max;
        let block_height_subscription_max = self.block_height_subscription_max;
        let has_subscribed = self.has_subscribed.clone();
        let is_reorg = self.is_reorg.clone();
        let shutting_down = self.shutting_down.clone();
        self.provider
            .expect_get_block_by_number()
            .returning(move |height| {
                // whenever a subscription was activated, the valid range spans to the subscription max
                let mut valid_range =
                    block_height_backward_sync_init..block_height_backward_sync_max;
                if has_subscribed.load(Ordering::SeqCst) {
                    valid_range = block_height_backward_sync_init..block_height_subscription_max;
                }
                if valid_range.contains(&height) {
                    // if a shutdown height is set, the provider will start shutting down at that height
                    if let Some(shutdown_height) = simul_shutdown_height {
                        if height == shutdown_height {
                            shutting_down.set(true);
                            info!("Shutdown initiated at block height {}", height);
                        }
                    }
                    // if a reorg has to happen and the height is the reorg height, activate the reorg
                    if let Some(reorg_happens_at_height) = simul_reorg_happens_at_height {
                        if height == reorg_happens_at_height {
                            is_reorg.store(true, Ordering::SeqCst);
                            info!(
                                "Reorg initiated at block height {} with hash {}",
                                height,
                                generator.generate_hash(height, "alt")
                            );
                        }
                    }
                    Ok(Some(generator.generate_block(height)))
                } else {
                    Ok(None)
                }
            });
    }

    pub fn set_provider_expect_subscribe_blocks(
        &mut self,
        simul_reorg_happens_at_height: Option<BlockNumber>,
    ) {
        let is_reorg = self.is_reorg.clone();
        let generator = self.block_generator.clone();
        let shutting_down = self.shutting_down.clone();
        let mut height_subscr_counter = self.block_height_backward_sync_max + 1;
        let has_subscribed = self.has_subscribed.clone();
        let delay_between_blocks_subscription = self.delay_between_blocks_subscription;
        let block_height_subscription_max = self.block_height_subscription_max;
        self.provider.expect_subscribe_blocks().returning(move || {
            let mut mock_sub = MockRskSubscription::<RskBlock>::new();
            let generator = generator.clone();
            let shutting_down = shutting_down.clone();
            let is_reorg = is_reorg.clone();
            has_subscribed.store(true, Ordering::SeqCst);
            mock_sub.expect_next().returning(move || {
                // if a reorg has to happen and the height is the reorg height, activate the reorg
                if let Some(reorg_happens_at_height) = simul_reorg_happens_at_height {
                    if height_subscr_counter == reorg_happens_at_height {
                        is_reorg.store(true, Ordering::SeqCst);
                        info!(
                            "Reorg initiated at block height {} with hash {}",
                            height_subscr_counter,
                            generator.generate_hash(height_subscr_counter, "alt")
                        );
                    }
                }
                thread::sleep(Duration::from_millis(delay_between_blocks_subscription));
                let block = generator.generate_block(height_subscr_counter);
                height_subscr_counter = height_subscr_counter + 1;
                // if the block height passes the subscription max, start shutting down
                if height_subscr_counter <= block_height_subscription_max {
                    Ok(block)
                } else {
                    shutting_down.set(true);
                    Ok(block)
                }
            });
            mock_sub.expect_unsubscribe().returning(|| Ok(()));
            Ok(mock_sub)
        });
    }

    pub fn set_provider_expect_subscribe_logs(
        &mut self,
        filter: RskSubscriptionFilter,
        event_signature: String,
        log_info_tuples: Vec<LogInfo>,
    ) {
        let log_generator = self.log_generator.clone();
        let has_subscribed = self.has_subscribed.clone();
        let shutting_down = self.shutting_down.clone();
        let delay_between_blocks_subscription = self.delay_between_blocks_subscription;
        let tuples = VecDeque::from(log_info_tuples);
        self.provider
            .expect_subscribe_logs()
            .with(mockall::predicate::function(
                move |f: &RskSubscriptionFilter| matching_filters(filter.clone(), f),
            ))
            .returning(move |_| {
                let mut mock_sub = MockRskSubscription::new();
                let log_generator = log_generator.clone();
                let shutting_down = shutting_down.clone();
                has_subscribed.store(true, Ordering::SeqCst);
                let mut tuples = tuples.clone();
                let event_signature = event_signature.clone();
                mock_sub.expect_next().returning(move || {
                    generate_next_rsk_log(
                        delay_between_blocks_subscription,
                        &log_generator,
                        &shutting_down,
                        &mut tuples,
                        &event_signature,
                    )
                });
                mock_sub.expect_unsubscribe().returning(|| Ok(()));
                Ok(mock_sub)
            });
    }

    pub fn set_provider_expect_decode_log(&mut self) {
        self.provider
            .expect_decode_log()
            .withf(|_log: &RskLog, _contract: &ContractInfo| true)
            .returning(move |_log: RskLog, _contract: &ContractInfo| {
                Ok(Some(RskEvent::new(
                    "TestEvent".to_string(),
                    LogInfo::new(
                        "".to_string(),
                        BlockHash::try_from(DEFAULT_BLOCK_HASH)?,
                        0.into(),
                        "".to_string(),
                        1,
                        true,
                    ),
                    serde_json::Value::Null,
                )))
            });
    }
}

fn generate_next_rsk_log(
    delay_between_blocks_subscription: u64,
    log_generator: &FakeLogGenerator,
    shutting_down: &ShutdownFlag,
    tuples: &mut VecDeque<LogInfo>,
    event_signature: &str,
) -> Result<RskLog, RskSubscriptionError> {
    thread::sleep(Duration::from_millis(delay_between_blocks_subscription));
    if let Some(log_info) = tuples.pop_front() {
        let log = log_generator.generate_log(event_signature, log_info);
        if tuples.is_empty() {
            shutting_down.set(true);
        }
        Ok(log)
    } else {
        Err(RskSubscriptionError::Unexpected(anyhow!(
            "No more logs to generate"
        )))
    }
}

fn matching_filters(filter: RskSubscriptionFilter, f: &RskSubscriptionFilter) -> bool {
    let mut expected = filter.addresses;
    let mut actual = f.addresses.clone();
    expected.sort();
    actual.sort();
    expected == actual && f.topics == filter.topics && f.from_block == filter.from_block
}
