use crate::event_processor::EventProcessor;
use crate::event_processor::disputed_pegout_processor::types::Dispute;
use crate::types::{KickoffAdvanceFundsData, RequestAdvanceFundsData, RskPegManagerEvents};
use anyhow::Result;
use check_fork::check_fork;
use common::types::{BlockNumber, RskBlock};
use log::{debug, error, info, warn};
use primitive_types::U256;
use std::collections::BTreeMap;

pub struct DisputedPegOutProcessor {
    // TODO(iago) rename dispute to advance_funds or sth like that, all occurrences
    event1: Option<RequestAdvanceFundsData>,
    dispute: Option<Dispute>,
    // BTreeMap to sort blocks by number while keeping just the most recent one in case of reorgs
    known_blocks: BTreeMap<BlockNumber, RskBlock>,
}

impl DisputedPegOutProcessor {
    pub fn new() -> Self {
        Self {
            event1: None, // TODO(iago) convert to list
            dispute: None,
            known_blocks: BTreeMap::new(),
        }
    }

    fn register_advance_funds(&mut self, event1: RequestAdvanceFundsData) {
        // TODO(Jira) we should allow several RequestAdvanceFunds - https://rsklabs.atlassian.net/browse/UB-150
        self.event1 = Some(event1);
    }

    fn create_dispute(&mut self, event2: KickoffAdvanceFundsData) -> () {
        if self.known_blocks.is_empty() {
            // this happens when a kickoff is received before any block
            // it should not happen in real life because RequestAdvanceFunds must be received many blocks before KickoffAdvanceFunds
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("No blocks received yet, cannot kickoff dispute");
            return;
        }

        let Some(event1) = self.event1.as_ref() else {
            error!("KickoffAdvanceFundsData received, but no RequestAdvanceFunds was");
            return;
        };

        if event1.inner.peg_out_id != event2.inner.peg_out_id {
            error!(
                "KickoffAdvanceFundsData received for pegout {}, but RequestAdvanceFunds had pegout {}",
                event1.inner.peg_out_id, event2.inner.peg_out_id
            );
            return;
        }

        if self.dispute.is_some() {
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("More tha one active dispute is not expected on Union Bridge Design",);
            return;
        }

        let post_kickoff_blocks: Vec<&RskBlock> = self
            .known_blocks
            .values()
            .filter(|b| b.number() >= event2.block_number)
            .collect();

        info!("Init dispute with {event2:?} and {post_kickoff_blocks:?}");
        let new_dispute = Dispute::new(event2, post_kickoff_blocks);
        self.dispute = Some(new_dispute);
    }

    fn deactivate_monitoring(&mut self) {
        // TODO(Jira) deactivate only if we don't have more RequestAdvanceFunds pending - https://rsklabs.atlassian.net/browse/UB-150
        self.event1 = None;
        self.known_blocks.clear();
    }

    fn close_dispute(&mut self, done: bool) -> () {
        if let Some(dispute) = &self.dispute {
            info!("Removing active {:?}", dispute);
            self.dispute = None;
        } else {
            info!("Trying to remove unexisting dispute");
        }

        if done {
            self.deactivate_monitoring()
        }
    }

    fn run_check_fork(dispute: &Dispute) {
        let args = dispute.check_fork_args();
        // note: check-fork already validates consecutive blocks, etc.
        match check_fork(args) {
            Ok(effort) if effort != U256::zero() => {
                info!("CheckFork accepted with pow {}", U256::MAX / effort);
                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-89
            }
            Ok(_effort) => {
                error!("CheckFork with 0 effort was accepted");
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) discuss with architects on error handling - https://rsklabs.atlassian.net/browse/UB-149
            }
            Err(e) => {
                error!("CheckFork rejected: {}", e);
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) discuss with architects on error handling - https://rsklabs.atlassian.net/browse/UB-149
            }
        }
    }
}

impl EventProcessor for DisputedPegOutProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(data) => {
                info!("Handling {:?}, waiting blocks...", data);
                self.register_advance_funds(data.clone());
            }
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveRequestAdvanceFunds {peg_out_id}...");
                self.deactivate_monitoring();
            }
            RskPegManagerEvents::KickoffAdvanceFunds(data) => {
                info!("Handling {:?}...", data);
                self.create_dispute(data.clone());
            }
            RskPegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveKickoffAdvanceFunds {peg_out_id}...");
                self.close_dispute(false);
            }
            _ => {
                info!("Ignoring {:?}...", event);
                return Ok(()); // ignore unrelated events
            }
        }
        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlock) -> Result<()> {
        let Some(event1) = self.event1.as_ref() else {
            debug!(
                "No RequestAdvanceFunds event received, ignoring block {}",
                block.number()
            );
            return Ok(());
        };

        if block.number().value() < event1.block_number.value() {
            warn!("Ignoring older block {}", block.number());
            return Ok(());
        }

        let removed_block = self.known_blocks.insert(block.number(), block.clone());

        let Some(dispute) = self.dispute.as_mut() else {
            info!("No active dispute, ignoring block's {} pow", block.number());
            return Ok(());
        };

        info!("Accumulating pow for dispute {}", dispute.pegout_id());

        if let Some(rb) = &removed_block {
            dispute.update_with_block(rb, true);
        }

        dispute.update_with_block(block, false);

        if dispute.is_ready_for_check_fork() {
            info!("Triggering CheckFork for complete dispute {:?}", dispute);
            Self::run_check_fork(dispute);

            info!("Completing dispute {}", dispute.pegout_id());
            self.close_dispute(true);
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        if self.dispute.is_some() {
            warn!("Active dispute found on shutdown! {:?}", self.dispute);
        }
        self.dispute = None;
        self.event1 = None;
        self.known_blocks.clear();
    }
}

pub(super) mod types {
    use crate::types::KickoffAdvanceFundsData;
    use check_fork::{Block, CheckForkArgs};
    use common::types::{BlockPow, RskBlock};
    use log::{debug, info};
    #[cfg(feature = "anvil")]
    use primitive_types::H256;
    use primitive_types::U256;

    #[derive(Debug)]
    pub(super) struct Dispute {
        event: KickoffAdvanceFundsData,
        check_fork_args: CheckForkArgs,
        accum_effort: U256,
    }

    impl Dispute {
        pub(super) fn new(
            event: KickoffAdvanceFundsData,
            post_kickoff_blocks: Vec<&RskBlock>,
        ) -> Self {
            let check_fork_args = CheckForkArgs {
                utxo_id: event.inner.utxo_id.clone(),
                pegout_id: event.inner.peg_out_id.clone(),
                operator_id: event.inner.operator_id.clone(),
                required_effort: U256::from_big_endian(
                    &event.inner.required_effort.to_be_bytes_vec(),
                ),
                required_num_blocks: event.inner.required_num_blocks,
                // fields that can be updated later on
                init_block_time: 0,
                init_block_number: 0,
                block_list: vec![],
            };

            let mut instance = Self {
                event,
                check_fork_args,
                accum_effort: U256::zero(),
            };

            // we already received the block that triggered the event, before the event itself
            post_kickoff_blocks
                .iter()
                .for_each(|b| instance.add_block_to_check_fork(b));

            instance
        }

        pub fn pegout_id(&self) -> String {
            self.event.inner.peg_out_id.clone()
        }

        pub fn check_fork_args(&self) -> CheckForkArgs {
            self.check_fork_args.clone()
        }

        pub fn update_with_block(&mut self, block: &RskBlock, removed: bool) -> () {
            if removed {
                self.remove_block_from_check_fork(block);
            } else {
                self.add_block_to_check_fork(block);
            }
        }

        pub fn is_ready_for_check_fork(&self) -> bool {
            self.has_enough_pow() && self.has_enough_blocks()
        }

        fn get_missing_effort(&self) -> U256 {
            self.check_fork_args
                .required_effort
                .saturating_sub(self.accum_effort)
        }

        fn has_enough_pow(&self) -> bool {
            self.get_missing_effort() == U256::zero()
        }

        fn get_missing_blocks(&self) -> u32 {
            self.check_fork_args
                .required_num_blocks
                .saturating_sub(self.check_fork_args.block_list.len() as u32)
        }

        fn has_enough_blocks(&self) -> bool {
            self.get_missing_blocks() == 0
        }

        fn add_block_to_check_fork(&mut self, block: &RskBlock) {
            // we received the block that triggered the event after the event itself
            if block.hash() == self.event.block_hash.into() {
                self.check_fork_args.init_block_number = block.number().value();
                self.check_fork_args.init_block_time = block.timestamp().value();
            }

            // include the block in the list
            self.check_fork_args
                .block_list
                .push(self.new_check_fork_block(block));

            self.increase_effort(Self::pow_to_effort(&block.pow()));
        }

        fn remove_block_from_check_fork(&mut self, block: &RskBlock) {
            let hash = hex::encode(block.hash().value());

            let before_len = self.check_fork_args.block_list.len();
            self.check_fork_args.block_list.retain(|b| b.hash != hash);

            if self.check_fork_args.block_list.len() < before_len {
                self.decrease_effort(Self::pow_to_effort(&block.pow()));
            }
        }

        fn increase_effort(&mut self, block_effort: U256) {
            self.accum_effort = self.accum_effort.saturating_add(block_effort);
            info!(
                "Dispute {} got new block. Pending effort {} (+{}). Pending blocks {}.",
                self.pegout_id(),
                self.get_missing_effort(),
                block_effort,
                self.get_missing_blocks()
            );
        }

        fn decrease_effort(&mut self, block_effort: U256) {
            self.accum_effort = self.accum_effort.saturating_sub(block_effort);
            info!(
                "Dispute {}: new block, pending effort {} (-{}), blocks {}",
                self.pegout_id(),
                self.get_missing_effort(),
                block_effort,
                self.get_missing_blocks()
            );
        }

        fn new_check_fork_block(&self, new_block: &RskBlock) -> Block {
            debug!("hash {} - block {:?}", self.event.block_hash, new_block);

            let bridge_event = (new_block.hash() == self.event.block_hash.into()).then(|| {
                check_fork::BridgeEvent {
                    utxo_id: self.event.inner.utxo_id.clone(),
                    pegout_id: self.event.inner.peg_out_id.clone(),
                    operator_id: self.event.inner.operator_id.clone(),
                }
            });

            Block {
                number: new_block.number().value(),
                hash: hex::encode(new_block.hash().value()),
                parent: hex::encode(new_block.parent_hash().value()),
                difficulty: new_block.difficulty().value(),
                timestamp: new_block.timestamp().value(),
                bridge_event,
                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-144
                uncles: vec![],
                pow: Self::get_block_pow(&new_block.pow()),
            }
        }

        #[cfg(not(feature = "anvil"))]
        fn pow_to_effort(pow: &BlockPow) -> U256 {
            use log::error;

            let pow_dec: U256 = U256::from_big_endian(pow.value().as_bytes());
            U256::MAX.checked_div(pow_dec).unwrap_or_else(|| {
                error!("0 division on pow_to_effort");
                U256::zero()
            })
        }

        #[cfg(feature = "anvil")]
        fn pow_to_effort(_pow: &BlockPow) -> U256 {
            U256::from(2500000000000u64)
        }

        #[cfg(not(feature = "anvil"))]
        fn get_block_pow(pow: &BlockPow) -> String {
            hex::encode(pow.value())
        }

        #[cfg(feature = "anvil")]
        fn get_block_pow(_pow: &BlockPow) -> String {
            hex::encode(H256::from_low_u64_be(2500000000000u64))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventWithBlock;
    use alloy_primitives::U256 as AlloyU256;
    use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use primitive_types::{H256, U256};
    use std::ops::Mul;

    fn create_fake_block(number: BlockNumber, effort: U256) -> RskBlock {
        let block_pow_u = U256::MAX.checked_div(effort).expect("0 division");
        let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));

        let block_number = number;
        let block_hash = BlockHash::from(H256::from_low_u64_be(number.value()));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number.value() - 1));
        let timestamp = BlockTimestamp::from(number.value() * 1000);
        let difficulty = BlockDifficulty::from(U256::from(500));
        let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));
        let uncles = vec![];

        RskBlock::new(
            block_number,
            block_hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow,
            uncles,
        )
    }

    fn create_fake_request_event(peg_out_id: &str) -> RequestAdvanceFunds {
        RequestAdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
            amount: 1000,
        }
    }

    fn create_kickoff_block_2(original_block: &RskBlock) -> RskBlock {
        RskBlock::new(
            original_block.number(),
            BlockHash::from(H256::from_low_u64_be(123)),
            original_block.parent_hash(),
            original_block.timestamp(),
            original_block.difficulty(),
            original_block.total_difficulty(),
            original_block.pow(),
            original_block.uncles().to_vec(),
        )
    }

    fn create_fake_kickoff_event(peg_out_id: &str) -> KickoffAdvanceFunds {
        KickoffAdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
            utxo_id: "utxo123".to_string(),
            operator_id: "op123".to_string(),
            required_effort: AlloyU256::from(1000),
            required_num_blocks: 4,
        }
    }

    #[test]
    fn test_new_processor_initial_state_is_clear() {
        let processor = DisputedPegOutProcessor::new();
        assert!(processor.event1.is_none());
        assert!(processor.dispute.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_new_event_request_advance_funds_stores_event() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let event_with_block = EventWithBlock {
            inner: create_fake_request_event("peg123"),
            block_number: request_block.number(),
            block_hash: BlockHash::from(H256::from_low_u64_be(123)),
        };
        let result = processor.process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
            event_with_block.clone(),
        ));
        assert!(result.is_ok());
        assert_eq!(processor.event1, Some(event_with_block));
        assert!(processor.known_blocks.is_empty());
        assert!(processor.dispute.is_none());
    }

    #[test]
    fn test_process_new_event_kickoff_advance_funds_creates_dispute() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(51));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        let any_block = create_fake_block(102.into(), U256::from(105));
        processor
            .process_new_block(&request_block)
            .expect("Should have processed request");
        processor
            .process_new_block(&any_block)
            .expect("Should have processed request");

        let kickoff_event = create_fake_kickoff_event(pegout_id);
        let result = processor.process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
            KickoffAdvanceFundsData {
                inner: kickoff_event,
                block_number: kickoff_block.number(),
                block_hash: kickoff_block.hash(),
            },
        ));

        processor
            .process_new_block(&kickoff_block)
            .expect("Should have processed request");

        assert!(result.is_ok());
        assert!(processor.dispute.is_some());

        let dispute = processor.dispute.as_ref().expect("Dispute should exist");
        assert_eq!(dispute.pegout_id(), pegout_id);
        assert_eq!(dispute.check_fork_args().pegout_id, pegout_id);

        assert_eq!(processor.known_blocks.len(), 3);
        assert_eq!(
            processor
                .known_blocks
                .get(&request_block.number())
                .expect("Should exist"),
            &request_block
        );
        assert_eq!(
            processor
                .known_blocks
                .get(&any_block.number())
                .expect("Should exist"),
            &any_block
        );
        assert_eq!(
            processor
                .known_blocks
                .get(&kickoff_block.number())
                .expect("Should exist"),
            &kickoff_block
        );
    }

    #[test]
    fn test_process_new_event_kickoff_advance_funds_without_request_exits() {
        let mut processor = DisputedPegOutProcessor::new();

        let kickoff_block = create_fake_block(110.into(), U256::from(51));
        let kickoff_event = create_fake_kickoff_event("peg123");

        let result = processor.process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
            KickoffAdvanceFundsData {
                inner: kickoff_event,
                block_number: kickoff_block.number(),
                block_hash: kickoff_block.parent_hash(),
            },
        ));
        assert!(result.is_ok());
        assert!(processor.event1.is_none());
        assert!(processor.dispute.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_kickoff_block_after_event_accumulates_effort_and_closes_dispute() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        // we process the advance funds block
        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        // to need one more block than required ones to achieve the pow
        let total_blocks = kickoff_event.required_num_blocks + 1;

        let block_effort = kickoff_event
            .required_effort
            .checked_div(AlloyU256::from(total_blocks))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let kickoff_block = create_fake_block(request_block.number() + 1, block_effort);

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsData {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        // we process the kickoff block after the kickoff event
        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        assert!(processor.dispute.is_some());
        assert!(processor.event1.is_some());

        // -2: the one created after the kickoff counts, and we will create the last one out of this loop
        for i in 1..=total_blocks - 2 {
            let block = create_fake_block(kickoff_block.number() + i as u64, block_effort);
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.dispute.is_some());
        assert!(processor.event1.is_some());

        // we process the missing block
        let block = create_fake_block(kickoff_block.number() + 5, block_effort);
        processor
            .process_new_block(&block)
            .expect("Should process block");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_none());
    }

    #[test]
    fn test_process_kickoff_block_before_event_accumulates_effort_and_closes_dispute() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        // we process the advance funds block
        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        // to need one more block than required ones to achieve the pow
        let total_blocks = kickoff_event.required_num_blocks + 1;

        let block_effort = kickoff_event
            .required_effort
            .checked_div(AlloyU256::from(total_blocks))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let kickoff_block = create_fake_block(request_block.number() + 1, block_effort);

        // we process the kickoff block before the kickoff event
        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsData {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.dispute.is_some());
        assert!(processor.event1.is_some());

        // -2: the one created after the kickoff event counts, and we will create the last one out of this loop
        for i in 1..=total_blocks - 2 {
            let block = create_fake_block(kickoff_block.number() + i as u64, block_effort);
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.dispute.is_some());
        assert!(processor.event1.is_some());

        // we process the missing block
        let block = create_fake_block(kickoff_block.number() + 5, block_effort);
        processor
            .process_new_block(&block)
            .expect("Should process block");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_none());
    }

    #[test]
    fn test_process_kickoff_event_without_blocks_early_exits() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsData {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.event1.is_some());
        assert!(processor.known_blocks.is_empty());
        assert!(processor.dispute.is_none());
    }

    #[test]
    fn test_process_old_block_early_exits() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let request_event = create_fake_request_event("peg123");
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        let old_block = create_fake_block(99.into(), U256::from(100));
        let result = processor.process_new_block(&old_block);
        assert!(result.is_ok());
    }

    #[test]
    fn test_shutdown_with_active_dispute_works() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(100));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let kickoff_event = create_fake_kickoff_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsData {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        assert!(processor.event1.is_some());
        assert!(processor.dispute.is_some());
        assert_eq!(processor.known_blocks.len(), 2);

        processor.shutdown();

        assert!(processor.event1.is_none());
        assert!(processor.dispute.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_remove_request_advance_funds_event_removes_it() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_event(&RskPegManagerEvents::RemoveRequestAdvanceFunds {
                peg_out_id: pegout_id.to_string(),
            })
            .expect("Should have processed request");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_none());
    }

    #[test]
    fn test_remove_request_advance_funds_block_removes_it() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let adv_funds_block_2 = create_kickoff_block_2(&request_block);

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_some());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&adv_funds_block_2)
            .expect("Should process block");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_some());
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_block() {
        let mut processor = DisputedPegOutProcessor::new();

        let advance_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(50));
        let kickoff_block_2 = create_kickoff_block_2(&kickoff_block);

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: advance_block.number(),
                    block_hash: advance_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        assert!(processor.event1.is_some());
        assert!(processor.dispute.is_none());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&kickoff_block_2)
            .expect("Should process block");

        assert!(processor.event1.is_some());
        assert!(processor.dispute.is_none());
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_event_stops_dispute() {
        let mut processor = DisputedPegOutProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(100));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsData {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let kickoff_event = create_fake_kickoff_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsData {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.dispute.is_some());
        assert!(processor.event1.is_some());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_event(&RskPegManagerEvents::RemoveKickoffAdvanceFunds {
                peg_out_id: pegout_id.to_string(),
            })
            .expect("Should have processed kickoff");

        assert!(processor.dispute.is_none());
        assert!(processor.event1.is_some());
        assert_eq!(processor.known_blocks.len(), 1);
    }
}
