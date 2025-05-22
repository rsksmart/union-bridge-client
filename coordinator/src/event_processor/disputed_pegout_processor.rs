use crate::event_processor::EventProcessor;
use crate::event_processor::disputed_pegout_processor::types::Dispute;
use crate::types::RskPegManagerEvents;
use anyhow::{Result, bail};
use check_fork::check_fork;
use common::fake_contracts::FakePegManager::KickoffAdvanceFunds;
use common::types::{BlockNumber, RskBlock};
use log::{error, info, warn};
use std::collections::BTreeMap;

pub struct DisputedPegOutProcessor {
    req_adv_funds_block: Option<BlockNumber>,
    dispute: Option<Dispute>,
    // BTreeMap to sort blocks by number while keeping just the most recent one in case of reorgs
    known_blocks: BTreeMap<BlockNumber, RskBlock>,
}

impl DisputedPegOutProcessor {
    pub fn new() -> Self {
        Self {
            req_adv_funds_block: None,
            dispute: None,
            known_blocks: BTreeMap::new(),
        }
    }

    fn kickoff_dispute(&mut self, event: &KickoffAdvanceFunds, block_number: &BlockNumber) -> () {
        if !self.is_waiting_blocks() {
            error!("Cannot kickoff dispute, RequestAdvanceFunds not yet received");
            return;
        }

        if self.dispute.is_some() {
            // we don't want to err, so we just skip this event
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("More tha one active dispute is not expected on Union Bridge Design",);
            return;
        }

        let post_kickoff_blocks: Vec<&RskBlock> = self
            .known_blocks
            .values()
            .filter(|b| b.number().value() > block_number.value())
            .collect();

        info!("Init dispute for {event:?}");
        let new_dispute = Dispute::new(event.clone(), post_kickoff_blocks);
        self.dispute = Some(new_dispute);
    }

    fn start_processing(&mut self, block_num: &BlockNumber) {
        self.req_adv_funds_block = Some(block_num.clone());
    }

    fn stop_processing(&mut self) {
        self.req_adv_funds_block = None;
    }

    fn close_dispute(&mut self, completed: bool) -> () {
        if let Some(dispute) = &self.dispute {
            info!("Removing active {:?}", dispute);
            self.dispute = None;
        } else {
            info!("Trying to remove unexisting dispute");
        }

        if completed {
            self.stop_processing()
        }
    }

    fn run_check_fork(dispute: &mut Dispute) {
        let args = dispute.check_fork_args();
        // note: check-fork already validates consecutive blocks, etc.
        match check_fork(args) {
            Ok(effort) => {
                info!("CheckFork accepted with effort {}", effort);
                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-89
            }
            Err(e) => {
                error!("CheckFork rejected: {}", e);
                // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            }
        }
    }
}

impl EventProcessor for DisputedPegOutProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(ev, block_num) => {
                info!("Handling {:?} @ block {}, waiting blocks...", ev, block_num);
                self.start_processing(block_num);
            }
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveRequestAdvanceFunds {peg_out_id}...");
                self.stop_processing();
            }
            RskPegManagerEvents::KickoffAdvanceFunds(ev, block_num) => {
                info!("Handling {:?}...", ev);
                self.kickoff_dispute(ev, block_num);
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
        if !self.is_waiting_blocks() {
            bail!("Not waiting blocks, still received {}...", block.number());
        }

        if let Some(req_adv_funds_block) = &self.req_adv_funds_block {
            if block.number().value() < req_adv_funds_block.value() {
                warn!(
                    "Ignoring block {}, older than RequestAdvanceFunds",
                    block.number()
                );
                return Ok(());
            }
        }

        let removed_block = self.known_blocks.insert(block.number(), block.clone());

        if !self.dispute.is_some() {
            info!(
                "No active dispute, ignoring block effort for {}",
                block.number()
            );
            return Ok(());
        }

        let dispute = self.dispute.as_mut().unwrap();

        info!("Accumulating pow for dispute {}", dispute.pegout_id());

        // TODO(Jira) this will probably be removed in scope of // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-144
        if let Some(rb) = &removed_block {
            dispute.update_with_block(rb, true);
        }

        dispute.update_with_block(block, false);

        if dispute.has_enough_pow() {
            info!("Triggering CheckFork for complete dispute {:?}", dispute);
            Self::run_check_fork(dispute);

            info!("Completing dispute {}", dispute.pegout_id());
            self.close_dispute(true);
        }

        Ok(())
    }

    fn is_waiting_blocks(&self) -> bool {
        self.req_adv_funds_block.is_some()
    }

    fn shutdown(&self) {
        if self.dispute.is_some() {
            warn!("Active dispute found on shutdown! {:?}", self.dispute);
        }
    }
}

pub(super) mod types {
    use check_fork::{Block, CheckForkArgs};
    use common::fake_contracts::FakePegManager::KickoffAdvanceFunds;
    use common::types::{BlockPow, RskBlock};
    use log::info;
    use primitive_types::U256;

    #[derive(Debug)]
    pub struct Dispute {
        event: KickoffAdvanceFunds,
        check_fork_args: CheckForkArgs,
        accum_effort: U256,
    }

    impl Dispute {
        pub(crate) fn new(event: KickoffAdvanceFunds, kickoff_blocks: Vec<&RskBlock>) -> Self {
            let check_fork_args = CheckForkArgs {
                utxo_id: event.utxo_id.clone(),
                pegout_id: event.peg_out_id.clone(),
                operator_id: event.operator_id.clone(),
                required_effort: U256::from_big_endian(&event.required_effort.to_be_bytes_vec()),
                required_num_blocks: event.required_num_blocks,
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
            kickoff_blocks
                .iter()
                .for_each(|b| instance.update_check_fork_with_block(b));

            instance
        }

        pub fn pegout_id(&self) -> String {
            self.event.peg_out_id.clone()
        }

        pub fn check_fork_args(&self) -> CheckForkArgs {
            self.check_fork_args.clone()
        }

        pub fn update_with_block(&mut self, block: &RskBlock, removed: bool) -> () {
            let block_effort = Self::pow_to_effort(&block.pow());

            if removed {
                self.decrease_effort(block_effort);
            } else {
                self.increase_effort(block_effort);

                // we received the block that triggered the event after the event itself
                if block.hash() == self.event.block_hash.into() {
                    self.update_check_fork_with_block(block);
                }
            }
        }

        pub fn has_enough_pow(&self) -> bool {
            self.get_missing_effort() == U256::zero()
        }

        fn update_check_fork_with_block(&mut self, block: &RskBlock) {
            self.check_fork_args.init_block_number = block.number().value();
            self.check_fork_args.init_block_time = block.timestamp().value();
            self.check_fork_args
                .block_list
                .push(self.new_check_fork_block(block))
        }

        fn increase_effort(&mut self, block_effort: U256) {
            self.accum_effort = self.accum_effort.saturating_add(block_effort);
            info!(
                "Dispute {}: new block, pending effort {} (+{})",
                self.pegout_id(),
                self.get_missing_effort(),
                block_effort
            );
        }

        fn decrease_effort(&mut self, block_effort: U256) {
            self.accum_effort = self.accum_effort.saturating_sub(block_effort);
            info!(
                "Dispute {}: new block, pending effort {} (-{})",
                self.pegout_id(),
                self.get_missing_effort(),
                block_effort
            );
        }

        fn get_missing_effort(&self) -> U256 {
            let missing_effort = self
                .check_fork_args
                .required_effort
                .saturating_sub(self.accum_effort);
            missing_effort
        }

        fn new_check_fork_block(&self, new_block: &RskBlock) -> Block {
            let bridge_event = (new_block.hash() == self.event.block_hash.into()).then(|| {
                check_fork::BridgeEvent {
                    utxo_id: self.event.utxo_id.clone(),
                    pegout_id: self.event.peg_out_id.clone(),
                    operator_id: self.event.operator_id.clone(),
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
                pow: hex::encode(new_block.pow().value()),
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
            U256::from(250000000000u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256 as AlloyU256;
    use common::fake_contracts::FakePegManager::{KickoffAdvanceFunds, RequestAdvanceFunds};
    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use primitive_types::{H256, U256};
    use std::ops::Mul;

    fn create_fake_block(number: u64, block_effort: U256) -> RskBlock {
        let block_pow_u = U256::MAX.checked_div(block_effort).expect("0 division");
        let pow = BlockPow::from(H256::from_slice(&block_pow_u.to_big_endian()));

        let block_number = BlockNumber::from(number);
        let block_hash = BlockHash::from(H256::from_low_u64_be(number));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number - 1));
        let timestamp = BlockTimestamp::from(number);
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
            block_hash: BlockHash::from(H256::from_low_u64_be(123)).into(),
            peg_out_id: peg_out_id.to_string(),
            amount: 1000,
        }
    }

    fn create_fake_kickoff_event(peg_out_id: &str) -> KickoffAdvanceFunds {
        KickoffAdvanceFunds {
            block_hash: BlockHash::from(H256::from_low_u64_be(123)).into(),
            peg_out_id: peg_out_id.to_string(),
            utxo_id: "utxo123".to_string(),
            operator_id: "op123".to_string(),
            required_effort: AlloyU256::from(1000),
            required_num_blocks: 4,
        }
    }

    #[test]
    fn test_new_processor_initial_state() {
        let processor = DisputedPegOutProcessor::new();
        assert!(processor.req_adv_funds_block.is_none());
        assert!(processor.dispute.is_none());
        assert!(processor.known_blocks.is_empty());
        assert!(!processor.is_waiting_blocks());
    }

    #[test]
    fn test_process_request_advance_funds() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);
        let event = create_fake_request_event("peg123");

        let result = processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(event, block_num));
        assert!(result.is_ok());
        assert!(processor.is_waiting_blocks());
        assert!(processor.dispute.is_none());
        assert_eq!(processor.req_adv_funds_block, Some(block_num));
    }

    #[test]
    fn test_process_kickoff_advance_funds() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let pegout_id = "peg123";

        let req_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();

        let block1 = create_fake_block(101, U256::from(100));
        let block2 = create_fake_block(102, U256::from(105));
        processor.process_new_block(&block1).unwrap();
        processor.process_new_block(&block2).unwrap();

        let kickoff_event = create_fake_kickoff_event(pegout_id);
        let result = processor.process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
            kickoff_event.clone(),
            block_num,
        ));
        assert!(result.is_ok());
        assert!(processor.dispute.is_some());

        let dispute = processor.dispute.as_ref().unwrap();
        assert_eq!(dispute.pegout_id(), pegout_id);
        assert_eq!(dispute.check_fork_args().pegout_id, pegout_id);
    }

    #[test]
    fn test_process_kickoff_without_request() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);
        let kickoff_event = create_fake_kickoff_event("peg123");

        let result = processor.process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
            kickoff_event,
            block_num,
        ));
        assert!(result.is_ok());
        assert!(processor.dispute.is_none()); // Should not create dispute without RequestAdvanceFunds
    }

    #[test]
    fn test_process_blocks_accumulates_effort_and_closes_dispute() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let pegout_id = "peg123";

        let req_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        // to need one more block than required ones to achieve the pow
        let num_of_blocks = kickoff_event.required_num_blocks + 1;
        let block_effort = kickoff_event
            .required_effort
            .checked_div(AlloyU256::from(num_of_blocks))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                kickoff_event,
                block_num,
            ))
            .unwrap();

        assert!(processor.dispute.is_some());
        assert!(processor.req_adv_funds_block.is_some());
        assert!(processor.is_waiting_blocks());

        for i in 1..=num_of_blocks - 1 {
            let block = create_fake_block(block_num.value() + i as u64, block_effort);
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.dispute.is_some());
        assert!(processor.req_adv_funds_block.is_some());
        assert!(processor.is_waiting_blocks());

        // we process the missing block
        let block = create_fake_block(block_num.value() + 5, block_effort);
        processor
            .process_new_block(&block)
            .expect("Should process block");

        assert!(processor.dispute.is_none());
        assert!(processor.req_adv_funds_block.is_none());
        assert!(!processor.is_waiting_blocks());
    }

    #[test]
    fn test_remove_request_advance_funds() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let pegout_id = "peg123";

        let req_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();
        assert!(processor.is_waiting_blocks());

        let result = processor.process_new_event(&RskPegManagerEvents::RemoveRequestAdvanceFunds {
            peg_out_id: pegout_id.to_string(),
        });
        assert!(result.is_ok());
        assert!(!processor.is_waiting_blocks());
        assert!(processor.dispute.is_none());
        assert!(processor.req_adv_funds_block.is_none());
    }

    #[test]
    fn test_remove_kickoff_advance_funds() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let pegout_id = "peg123";

        let req_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();

        let kickoff_event = create_fake_kickoff_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                kickoff_event,
                block_num,
            ))
            .unwrap();
        assert!(processor.dispute.is_some());

        let result = processor.process_new_event(&RskPegManagerEvents::RemoveKickoffAdvanceFunds {
            peg_out_id: pegout_id.to_string(),
        });
        assert!(result.is_ok());
        assert!(processor.dispute.is_none());
        assert!(processor.req_adv_funds_block.is_some());
        assert!(processor.is_waiting_blocks());
    }

    #[test]
    fn test_process_old_block() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let req_event = create_fake_request_event("peg123");
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();

        let old_block = create_fake_block(99, U256::from(100));
        let result = processor.process_new_block(&old_block);
        assert!(result.is_ok());
    }

    #[test]
    fn test_shutdown_with_active_dispute() {
        let mut processor = DisputedPegOutProcessor::new();
        let block_num = BlockNumber::from(100);

        let peg_out_id = "peg123";

        let req_event = create_fake_request_event(peg_out_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                req_event, block_num,
            ))
            .unwrap();

        let kickoff_event = create_fake_kickoff_event(peg_out_id);
        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                kickoff_event,
                block_num,
            ))
            .unwrap();

        assert!(processor.dispute.is_some());

        processor.shutdown();
    }
}
