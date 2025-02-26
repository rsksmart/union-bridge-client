use log::info;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::{thread, time::Duration};

use crate::test_utils::rsk::FakeBlockGenerator;
use crate::{
    rsk_provider::{MockRskProvider, MockRskSubscription},
    shutdown_flag::ShutdownFlag,
    types::RskBlock,
};

#[derive(Clone)]
pub struct MockRskProviderHandler {
    provider: Arc<Mutex<MockRskProvider>>,
    generator: FakeBlockGenerator,
    is_reorg: Arc<AtomicBool>,
    has_subscribed: Arc<AtomicBool>,
    shutting_down: ShutdownFlag,
    block_height_backward_sync_init: u64,
    block_height_backward_sync_max: u64,
    block_height_subscription_max: u64,
    block_height_reorg_happens_at: u64,
    delay_between_blocks_subscription: u64,
}

impl MockRskProviderHandler {
    pub fn new(
        provider: Arc<Mutex<MockRskProvider>>,
        generator: &FakeBlockGenerator,
        is_reorg: Arc<AtomicBool>,
        shutting_down: ShutdownFlag,
        block_height_backward_sync_init: u64,
        block_height_backward_sync_max: u64,
        block_height_subscription_max: u64,
        block_height_reorg_happens_at: u64,
        delay_between_blocks_subscription: u64,
    ) -> Self {
        Self {
            provider,
            generator: generator.clone(),
            is_reorg,
            has_subscribed: Arc::new(AtomicBool::new(false)),
            shutting_down,
            block_height_backward_sync_init,
            block_height_backward_sync_max,
            block_height_subscription_max,
            block_height_reorg_happens_at,
            delay_between_blocks_subscription,
        }
    }

    pub fn set_provider_expect_get_block_by_hash(
        &mut self,
        expected_hash_string: String,
        block_height: u64,
    ) {
        let generator = self.generator.clone();
        self.provider
            .lock()
            .unwrap()
            .expect_get_block_by_hash()
            .with(mockall::predicate::eq(expected_hash_string))
            .returning(move |_hash| Ok(Some(generator.generate_block(block_height))));
    }

    pub fn set_provider_expect_get_best_block(&mut self) {
        let generator = self.generator.clone();
        let has_subscribed = self.has_subscribed.clone();
        let block_height_subscription_max = self.block_height_subscription_max;
        let block_height_backward_sync_max = self.block_height_backward_sync_max;
        self.provider
            .lock()
            .unwrap()
            .expect_get_best_block()
            .returning(move || {
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
        simul_reorg: bool,
        simul_shutdown: Option<u64>,
    ) {
        let generator = self.generator.clone();
        let block_height_backward_sync_init = self.block_height_backward_sync_init;
        let block_height_backward_sync_max = self.block_height_backward_sync_max;
        let block_height_subscription_max = self.block_height_subscription_max;
        let block_height_reorg_happens_at = self.block_height_reorg_happens_at;
        let has_subscribed = self.has_subscribed.clone();
        let is_reorg = self.is_reorg.clone();
        let shutting_down = self.shutting_down.clone();
        self.provider
            .lock()
            .unwrap()
            .expect_get_block_by_number()
            .returning(move |height| {
                let mut valid_range =
                    block_height_backward_sync_init..block_height_backward_sync_max;
                if has_subscribed.load(Ordering::SeqCst) {
                    valid_range = block_height_backward_sync_init..block_height_subscription_max;
                }
                if valid_range.contains(&height) {
                    if let Some(shutdown_height) = simul_shutdown {
                        if height == shutdown_height {
                            shutting_down.set(true);
                        }
                    }
                    if simul_reorg && height == block_height_reorg_happens_at {
                        is_reorg.store(true, Ordering::SeqCst);
                        info!(
                            "Reorg initiated at block height {} with hash {}",
                            height,
                            generator.generate_hash(height, "alt")
                        );
                    }
                    Ok(Some(generator.generate_block(height)))
                } else {
                    Ok(None)
                }
            });
    }

    pub fn set_provider_expect_subscribe_blocks(&mut self, simul_reorg: bool) {
        let block_height_reorg_happens_at = self.block_height_reorg_happens_at;
        let is_reorg = self.is_reorg.clone();
        let generator = self.generator.clone();
        let shutting_down = self.shutting_down.clone();
        let mut height_subscr_counter = self.block_height_backward_sync_max + 1;
        let has_subscribed = self.has_subscribed.clone();
        let delay_between_blocks_subscription = self.delay_between_blocks_subscription;
        let block_height_subscription_max = self.block_height_subscription_max;
        self.provider
            .lock()
            .unwrap()
            .expect_subscribe_blocks()
            .returning(move || {
                let mut mock_sub = MockRskSubscription::<RskBlock>::new();
                let generator = generator.clone();
                let shutting_down = shutting_down.clone();
                let is_reorg = is_reorg.clone();
                has_subscribed.store(true, Ordering::SeqCst);
                mock_sub.expect_next().returning(move || {
                    if simul_reorg && height_subscr_counter == block_height_reorg_happens_at {
                        is_reorg.store(true, Ordering::SeqCst);
                        info!(
                            "Reorg initiated at block height {} with hash {}",
                            height_subscr_counter,
                            generator.generate_hash(height_subscr_counter, "alt")
                        );
                    }
                    thread::sleep(Duration::from_millis(delay_between_blocks_subscription));
                    let block = generator.generate_block(height_subscr_counter);
                    height_subscr_counter += 1;
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
}
