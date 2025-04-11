use crate::rsk_provider::{
    MockRskProvider, MockRskSubscription, RskSubscriptionError, RskSubscriptionFilter,
};
use crate::shutdown_flag::ShutdownFlag;
use crate::test_utils::rsk_block_generator::FakeBlockGenerator;
use crate::test_utils::rsk_log_generator::FakeLogGenerator;
use crate::test_utils::rsk_utils::{UncleBlockInfo, from_hex_to_block_hash};
use crate::types::{BlockHash, BlockNumber, ContractInfo, LogInfo, RskBlock, RskEvent, RskLog};
use anyhow::anyhow;
use log::info;
use std::cell::RefCell;
use std::collections::HashSet;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    uncle_block_info_vec: Option<Vec<UncleBlockInfo>>,
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
        uncle_block_info_vec: Option<Vec<UncleBlockInfo>>,
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
            uncle_block_info_vec,
        }
    }

    pub fn set_provider_expect_get_block_by_hash_uncles(&mut self) {
        let generator = self.block_generator.clone();
        if let Some(uncle_block_info_vec) = self.uncle_block_info_vec.clone() {
            for uncle_info in uncle_block_info_vec {
                let flavor = format!(
                    "uncle_{}{}",
                    if uncle_info.reorg { "alt" } else { "" },
                    uncle_info.id
                );
                let expected_block_hash =
                    from_hex_to_block_hash(&generator.generate_hash(uncle_info.height, &flavor));
                self.provider
                    .expect_get_block_by_hash()
                    .with(mockall::predicate::eq(expected_block_hash))
                    .returning({
                        let generator = generator.clone();
                        move |_hash| {
                            Ok(generator
                                .generate_block(uncle_info.height, Some(uncle_info.clone())))
                        }
                    })
                    .times(0..);
            }
        }
    }

    pub fn set_provider_expect_get_block_by_hash(
        &mut self,
        expected_block_hash: BlockHash,
        block_height: BlockNumber,
    ) {
        info!("Setting hash expectation for block height {}", block_height);
        let generator = self.block_generator.clone();
        self.provider
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(expected_block_hash))
            .returning(move |_hash| Ok(Some(generator.generate_block(block_height, None).unwrap())))
            .times(1..);
    }

    pub fn set_provider_expect_get_best_block(&mut self) {
        let generator = self.block_generator.clone();
        let has_subscribed = self.has_subscribed.clone();
        let block_height_subscription_max = self.block_height_subscription_max;
        let block_height_backward_sync_max = self.block_height_backward_sync_max;
        self.provider
            .expect_get_best_block()
            .returning(move || {
                // whenever a subscription was activated, the best block is the subscription max
                let block_height = if has_subscribed.load(Ordering::SeqCst) {
                    block_height_subscription_max
                } else {
                    block_height_backward_sync_max
                };
                Ok(generator.generate_block(block_height, None).unwrap())
            })
            .times(1..);
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
                    Ok(Some(generator.generate_block(height, None).unwrap()))
                } else {
                    Ok(None)
                }
            })
            .times(1..);
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
        let uncle_block_info_vec = self.uncle_block_info_vec.clone();
        // Create a persistent container for spent uncle flavors.
        let spent_uncle_flavors = RefCell::new(HashSet::new());

        self.provider
            .expect_subscribe_blocks()
            .returning(move || {
                let mut mock_sub = MockRskSubscription::<RskBlock>::new();
                let generator = generator.clone();
                let shutting_down = shutting_down.clone();
                let is_reorg = is_reorg.clone();
                let uncle_block_info_vec = uncle_block_info_vec.clone();
                has_subscribed.store(true, Ordering::SeqCst);
                let spent_uncle_flavors = spent_uncle_flavors.clone();
                mock_sub
                    .expect_next()
                    .returning(move || {
                        // If a reorg should happen at this height, activate it.
                        activate_reorg(
                            simul_reorg_happens_at_height,
                            height_subscr_counter,
                            &generator,
                            is_reorg.clone(),
                        );

                        thread::sleep(Duration::from_millis(delay_between_blocks_subscription));

                        let mut spent_uncle_ids = spent_uncle_flavors.borrow_mut();
                        if let Some(uncle_block) = provide_uncle_block(
                            height_subscr_counter,
                            &generator,
                            uncle_block_info_vec.clone(),
                            &mut *spent_uncle_ids,
                        ) {
                            return Ok(uncle_block);
                        }

                        Ok(provide_block(
                            &mut height_subscr_counter,
                            block_height_subscription_max,
                            &generator,
                            &shutting_down,
                        ))
                    })
                    .times(1..);
                mock_sub.expect_unsubscribe().returning(|| Ok(())).times(1);
                Ok(mock_sub)
            })
            .times(1);
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
                mock_sub
                    .expect_next()
                    .returning(move || {
                        generate_next_rsk_log(
                            delay_between_blocks_subscription,
                            &log_generator,
                            &shutting_down,
                            &mut tuples,
                            &event_signature,
                        )
                    })
                    .times(1..);
                mock_sub.expect_unsubscribe().returning(|| Ok(())).times(1);
                Ok(mock_sub)
            })
            .times(1);
    }

    pub fn set_provider_expect_decode_log(&mut self) {
        self.provider
            .expect_decode_log()
            .withf(|_log: &RskLog, _contract: &ContractInfo| true)
            .returning(move |log: RskLog, _contract: &ContractInfo| {
                Ok(Some(RskEvent::new(
                    "TestEvent".to_string(),
                    log.info().clone(),
                    serde_json::Value::Null,
                )))
            })
            .times(1..);
    }
}

fn provide_block(
    height: &mut BlockNumber,
    block_height_subscription_max: BlockNumber,
    generator: &FakeBlockGenerator,
    shutting_down: &ShutdownFlag,
) -> RskBlock {
    let block = generator.generate_block(*height, None).unwrap();
    *height = *height + 1;
    if *height <= block_height_subscription_max {
        block
    } else {
        shutting_down.set(true);
        block
    }
}

fn activate_reorg(
    simul_reorg_happens_at_height: Option<BlockNumber>,
    height_subscr_counter: BlockNumber,
    generator: &FakeBlockGenerator,
    is_reorg: Arc<AtomicBool>,
) {
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
}
fn provide_uncle_block(
    height: BlockNumber,
    generator: &FakeBlockGenerator,
    uncle_block_info_vec: Option<Vec<UncleBlockInfo>>,
    spent_uncle_ids: &mut HashSet<String>,
) -> Option<RskBlock> {
    if height == 0 {
        return None;
    }
    let uncle_height = height - 1;
    if let Some(uncle_block_info_vec) = &uncle_block_info_vec {
        for uncle_info in uncle_block_info_vec.iter() {
            if uncle_height == uncle_info.height {
                let uncle_id = uncle_info.id.clone();
                if spent_uncle_ids.contains(&uncle_id) {
                    continue;
                }
                if let Some(uncle_block) =
                    generator.generate_block(uncle_height, Some(uncle_info.clone()))
                {
                    spent_uncle_ids.insert(uncle_id.clone());
                    return Some(uncle_block);
                }
            }
        }
    }
    None
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
