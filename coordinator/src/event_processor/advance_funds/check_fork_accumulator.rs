use crate::config::REQUIRED_CONFIRMATIONS;
use crate::event_processor::blockchain_tracker::{BlockConfirmations, BlockchainObserver};
use crate::types::AdvanceFundsEvent;
use check_fork::{Block, CheckForkArgs};
use common::types::{BlockPow, RskBlock, RskBlockAndUncles};
use log::{debug, info};
use primitive_types::H256;
use primitive_types::U256;

#[derive(Debug)]
pub(super) struct CheckForkAccumulator {
    advance_funds_block_hash: H256,
    args: CheckForkArgs,
    confirmations: Option<BlockConfirmations>,
}

impl BlockchainObserver for CheckForkAccumulator {
    fn get_id(&self) -> String {
        self.args.pegout_id.clone()
    }

    fn on_block_added(&mut self, block: &RskBlockAndUncles) {
        // we only collect confirmations if CheckFork is ready
        if self.is_check_fork_ready() {
            // creates it if it doesn't exist
            self.confirmations
                .get_or_insert_with(|| Self::new_confirmations(block, &self.args.pegout_id))
                .on_block_added(block);
            debug!(
                "Adding confirmation with block {} ({})",
                block.number(),
                block.hash()
            );
        } else {
            self.add_block_to_check_fork(block);
            debug!(
                "CheckFork added block {} ({})",
                block.number(),
                block.hash()
            );
        }
    }

    fn on_block_removed(&mut self, block: &RskBlockAndUncles) {
        // we optimistically try to remove the block from both the check_fork_args and confirmations
        self.remove_block_from_check_fork(block.block());
        debug!(
            "CheckFork removed block {} ({})",
            block.number(),
            block.hash()
        );

        if let Some(confirmations) = &mut self.confirmations {
            confirmations.on_block_removed(block);
            debug!(
                "Removing confirmation with block {} ({})",
                block.number(),
                block.hash()
            );
        }
    }
}

impl CheckForkAccumulator {
    pub(super) fn new(
        event: AdvanceFundsEvent,
        post_advance_funds_blocks: Vec<&RskBlockAndUncles>,
    ) -> Self {
        let check_fork_args = CheckForkArgs {
            // coming from the AdvanceFunds event
            utxo_id: event.inner.utxo_id.clone(),
            pegout_id: event.inner.peg_out_id.clone(),
            operator_id: event.inner.operator_id.clone(),
            required_effort: U256::from_big_endian(&event.inner.required_effort.to_be_bytes_vec()),
            required_num_blocks: event.inner.required_num_blocks,
            // coming from the AdvanceFunds block
            init_block_time: 0,
            init_block_number: 0,
            // coming from AdvanceFunds and post-AdvanceFunds blocks
            block_list: vec![],
        };

        info!(
            "Creating AdvanceFundsChecker with {:?} and block hash: {}",
            check_fork_args, event.block_hash
        );

        let mut instance = Self {
            advance_funds_block_hash: event.block_hash.value(),
            args: check_fork_args,
            confirmations: None,
        };

        // we already received the block that triggered the event, before the event itself
        post_advance_funds_blocks
            .iter()
            .for_each(|b| instance.add_block_to_check_fork(b));

        instance
    }

    pub fn pegout_id(&self) -> String {
        self.args.pegout_id.clone()
    }

    pub fn check_fork_args(&self) -> CheckForkArgs {
        self.args.clone()
    }

    pub fn has_enough_confirmations(&self) -> bool {
        self.confirmations
            .as_ref()
            .map_or(false, |c| c.is_confirmed())
    }

    fn is_check_fork_ready(&self) -> bool {
        let accum_effort = self
            .args
            .block_list
            .iter()
            .flat_map(|b| std::iter::once(b).chain(&b.uncles))
            .map(|b| BlockPow::from(b.pow).into_effort())
            .fold(U256::zero(), |accum, effort| accum.saturating_add(effort));

        let pending_effort = self.args.required_effort.saturating_sub(accum_effort);

        let pending_blocks = self
            .args
            .required_num_blocks
            .saturating_sub(self.args.block_list.len() as u32);

        let is_req_effort_achieved = pending_effort == U256::zero();
        let is_req_blocks_achieved = pending_blocks == 0;

        let ready = is_req_effort_achieved && is_req_blocks_achieved;
        if ready {
            info!(
                "AdvanceFundsChecker {} is ready for checkFork: {:?}",
                self.args.pegout_id, self.args
            );
        } else {
            info!(
                "AdvanceFundsChecker {} is missing {} effort and {} blocks for checkFork",
                self.args.pegout_id, pending_effort, pending_blocks
            );
        }

        ready
    }

    fn add_block_to_check_fork(&mut self, block_with_uncles: &RskBlockAndUncles) {
        let block = &block_with_uncles.block();

        // we received the block that triggered the event after the event itself
        if block.hash() == self.advance_funds_block_hash.into() {
            info!("Setting InitBlock on check_fork_args {:?}", block);
            self.args.init_block_number = block.number().value();
            self.args.init_block_time = block.timestamp().value();
        }

        // include the block in the list, with uncles if any
        self.args
            .block_list
            .push(self.new_check_fork_block(block_with_uncles));
    }

    fn remove_block_from_check_fork(&mut self, block: &RskBlock) {
        info!(
            "Removing block {} ({}) from checkFork",
            block.number(),
            block.hash()
        );
        self.args
            .block_list
            .retain(|b| b.hash != block.hash().value());
    }

    fn new_check_fork_block(&self, block_with_uncles: &RskBlockAndUncles) -> Block {
        let block = &block_with_uncles.block();

        let bridge_event = (block.hash() == self.advance_funds_block_hash.into()).then(|| {
            let bridge_event = check_fork::BridgeEvent {
                utxo_id: self.args.utxo_id.clone(),
                pegout_id: self.args.pegout_id.clone(),
                operator_id: self.args.operator_id.clone(),
            };
            info!("Setting check_fork_args {:?}", bridge_event);
            bridge_event
        });

        let uncle_blocks: Vec<Block> = block_with_uncles
            .uncles()
            .iter()
            .map(|uncle| {
                info!(
                    "Adding uncle {} ({}) to checkFork with effort {}",
                    uncle.number(),
                    uncle.hash(),
                    block.pow().into_effort(),
                );
                // convert each uncle to a checkFork Block: they have neither bridge event nor uncles
                self.rsk_block_to_check_fork_block(uncle, None, vec![])
            })
            .collect();

        info!(
            "Adding block {} ({}) to checkFork with effort {}",
            block.number(),
            block.hash(),
            block.pow().into_effort(),
        );

        // create a checkFork Block with bridge_event and uncles if any
        self.rsk_block_to_check_fork_block(block, bridge_event, uncle_blocks)
    }

    fn rsk_block_to_check_fork_block(
        &self,
        block: &RskBlock,
        bridge_event: Option<check_fork::BridgeEvent>,
        uncles: Vec<Block>,
    ) -> Block {
        Block {
            number: block.number().value(),
            hash: block.hash().value(),
            parent: block.parent_hash().value(),
            difficulty: block.difficulty().value(),
            timestamp: block.timestamp().value(),
            bridge_event,
            uncles,
            pow: block.pow().value(),
        }
    }

    fn new_confirmations(block: &RskBlockAndUncles, pegout_id: &String) -> BlockConfirmations {
        BlockConfirmations::new(
            pegout_id.to_string(),
            block.number(),
            REQUIRED_CONFIRMATIONS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processor::advance_funds::tests::create_fake_block;
    use actors_mocking::fake_contracts::FakePegManager::AdvanceFunds;
    use common::types::{BlockHash, BlockNumber, RskBlockAndUncles, TxHash};
    use primitive_types::H256;

    fn create_fake_advance_funds_event(
        pegout_id: &str,
        utxo_id: &str,
        operator_id: &str,
        required_effort: U256,
        required_num_blocks: u32,
        block_hash: H256,
        block_number: u64,
        tx_hash: H256,
    ) -> AdvanceFundsEvent {
        AdvanceFundsEvent {
            inner: AdvanceFunds {
                peg_out_id: pegout_id.to_string(),
                utxo_id: utxo_id.to_string(),
                operator_id: operator_id.to_string(),
                required_effort: alloy_primitives::U256::from_be_bytes(
                    required_effort.to_big_endian(),
                ),
                required_num_blocks,
            },
            block_number: BlockNumber::from(block_number),
            block_hash: BlockHash::from(block_hash),
            removed: false,
            tx_hash: TxHash::from(tx_hash),
        }
    }

    fn create_fake_block_with_uncles(
        number: u64,
        effort: U256,
        uncles: Vec<RskBlock>,
    ) -> RskBlockAndUncles {
        let block = create_fake_block(BlockNumber::from(number), effort);
        RskBlockAndUncles::new(block, uncles)
    }

    #[test]
    fn test_new_with_empty_post_advance_funds_blocks() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 5;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let checker = CheckForkAccumulator::new(event.clone(), vec![]);

        assert_eq!(checker.pegout_id(), pegout_id);
        assert_eq!(checker.args.utxo_id, utxo_id);
        assert_eq!(checker.args.operator_id, operator_id);
        assert_eq!(checker.args.required_effort, required_effort);
        assert_eq!(checker.args.required_num_blocks, required_num_blocks);
        assert_eq!(checker.args.init_block_time, 0);
        assert_eq!(checker.args.init_block_number, 0);
        assert_eq!(checker.args.block_list.len(), 0);
        assert_eq!(checker.advance_funds_block_hash, block_hash);
        assert!(!checker.has_enough_confirmations());
    }

    #[test]
    fn test_new_with_post_advance_funds_blocks() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 2;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let block1_number = 101;
        let block2_number = 102;
        let block1 = create_fake_block_with_uncles(block1_number, U256::from(300), vec![]);
        let block2 = create_fake_block_with_uncles(block2_number, U256::from(400), vec![]);
        let post_advance_funds_blocks = vec![&block1, &block2];

        let checker = CheckForkAccumulator::new(event, post_advance_funds_blocks);

        assert_eq!(checker.args.block_list.len(), 2);
        assert_eq!(checker.args.block_list[0].number, block1_number);
        assert_eq!(checker.args.block_list[1].number, block2_number);
    }

    #[test]
    fn test_pegout_id() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 5;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let checker = CheckForkAccumulator::new(event, vec![]);
        assert_eq!(checker.pegout_id(), pegout_id);
    }

    #[test]
    fn test_check_fork_args() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 5;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let checker = CheckForkAccumulator::new(event, vec![]);
        let args = checker.check_fork_args();

        assert_eq!(args.utxo_id, utxo_id);
        assert_eq!(args.operator_id, operator_id);
        assert_eq!(args.pegout_id, pegout_id);
        assert_eq!(args.required_effort, required_effort);
        assert_eq!(args.required_num_blocks, required_num_blocks);
    }

    #[test]
    fn test_update_with_block_before_check_fork_ready() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 3;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add a block
        let block_number = 101;
        let block = create_fake_block_with_uncles(block_number, U256::from(300), vec![]);
        checker.on_block_added(&block);

        assert_eq!(checker.args.block_list.len(), 1);
        assert_eq!(checker.args.block_list[0].number, block_number);

        // remove the block
        checker.on_block_removed(&block);
        assert_eq!(checker.args.block_list.len(), 0);
    }

    #[test]
    fn test_update_with_advance_funds_block_sets_init_values() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 3;
        let advance_funds_blockk_number = 100;
        let advance_funds_block_hash = H256::from_low_u64_be(advance_funds_blockk_number);
        let expected_timestamp = advance_funds_blockk_number * 1000; // timestamp is number * 1000
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            advance_funds_block_hash,
            advance_funds_blockk_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        let advance_funds_block = create_fake_block(
            BlockNumber::from(advance_funds_blockk_number),
            U256::from(300),
        );
        let advance_funds_block_with_uncles = RskBlockAndUncles::new(advance_funds_block, vec![]);

        checker.on_block_added(&advance_funds_block_with_uncles);

        assert_eq!(checker.args.init_block_number, advance_funds_blockk_number);
        assert_eq!(checker.args.init_block_time, expected_timestamp);
        assert_eq!(checker.args.block_list.len(), 1);
    }

    #[test]
    fn test_update_with_block_with_uncles() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(2000);
        let required_num_blocks = 3;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        let uncle1_number = 200;
        let uncle2_number = 201;
        let main_block_number = 202;
        let uncle1_effort = U256::from(200);
        let uncle2_effort = U256::from(250);
        let main_block_effort = U256::from(300);

        let uncle1 = create_fake_block(BlockNumber::from(uncle1_number), uncle1_effort);
        let uncle2 = create_fake_block(BlockNumber::from(uncle2_number), uncle2_effort);
        let block = create_fake_block_with_uncles(
            main_block_number,
            main_block_effort,
            vec![uncle1, uncle2],
        );

        checker.on_block_added(&block);

        assert_eq!(checker.args.block_list.len(), 1);
        let added_block = &checker.args.block_list[0];
        assert_eq!(added_block.number, main_block_number);
        assert_eq!(added_block.uncles.len(), 2);
        assert_eq!(added_block.uncles[0].number, uncle1_number);
        assert_eq!(added_block.uncles[1].number, uncle2_number);
    }

    #[test]
    fn test_is_check_fork_ready_insufficient_effort() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(10000);
        let required_num_blocks = 1;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add a block with low effort
        let test_block_number = 101;
        let low_effort = U256::from(100);
        let block = create_fake_block_with_uncles(test_block_number, low_effort, vec![]);
        checker.on_block_added(&block);

        assert!(!checker.is_check_fork_ready());
    }

    #[test]
    fn test_is_check_fork_ready_insufficient_blocks() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(100);
        let required_num_blocks = 10;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add a block with sufficient effort but not enough blocks
        let test_block_number = 101;
        let block = create_fake_block_with_uncles(test_block_number, required_effort, vec![]);
        checker.on_block_added(&block);

        assert!(!checker.is_check_fork_ready());
    }

    #[test]
    fn test_is_check_fork_ready_sufficient_effort_and_blocks() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(500);
        let required_num_blocks = 2;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        let block1_number = 101;
        let block1_effort = U256::from(300);
        let block1 = create_fake_block_with_uncles(block1_number, block1_effort, vec![]);
        checker.on_block_added(&block1);
        assert!(!checker.is_check_fork_ready());

        let block2_number = 102;
        let block2_effort = U256::from(200);
        let block2 = create_fake_block_with_uncles(block2_number, block2_effort, vec![]);
        checker.on_block_added(&block2);
        assert!(checker.is_check_fork_ready());
    }

    #[test]
    fn test_is_check_fork_ready_with_uncles_contributing_effort() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(800);
        let required_num_blocks = 1;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // create a block with uncles that contribute to effort
        let uncle1_number = 200;
        let uncle2_number = 201;
        let main_block_number = 202;
        let uncle1_effort = U256::from(200);
        let uncle2_effort = U256::from(250);
        let main_block_effort = U256::from(350);

        let uncle1 = create_fake_block(BlockNumber::from(uncle1_number), uncle1_effort);
        let uncle2 = create_fake_block(BlockNumber::from(uncle2_number), uncle2_effort);
        let block = create_fake_block_with_uncles(
            main_block_number,
            main_block_effort,
            vec![uncle1, uncle2],
        );

        checker.on_block_added(&block);

        // total effort should be 350 (block) + 200 (uncle1) + 250 (uncle2) = 800
        assert!(checker.is_check_fork_ready());
    }

    #[test]
    fn test_update_with_block_after_check_fork_ready() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(300);
        let required_num_blocks = 1;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add a block to make check fork ready
        let block1_number = 101;
        let block1 = create_fake_block_with_uncles(block1_number, required_effort, vec![]);
        checker.on_block_added(&block1);
        assert!(checker.is_check_fork_ready());

        let initial_block_count = checker.args.block_list.len();

        // try to add another block - it should not be added but confirmations should be updated
        let block2_number = 102;
        let block2_effort = U256::from(200);
        let block2 = create_fake_block_with_uncles(block2_number, block2_effort, vec![]);
        checker.on_block_added(&block2);

        // block list should not have grown
        assert_eq!(checker.args.block_list.len(), initial_block_count);
        // but confirmations should have been updated
        let expected_confirmations = 1;
        assert_eq!(
            checker.confirmations.unwrap().accum(),
            expected_confirmations
        );
    }

    #[test]
    fn test_has_enough_confirmations_progression() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(300);
        let required_num_blocks = 1;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // initially no confirmations
        assert!(!checker.has_enough_confirmations());

        // make check fork ready
        let initial_block_number = 101;
        let block1 = create_fake_block_with_uncles(initial_block_number, required_effort, vec![]);
        checker.on_block_added(&block1);
        assert!(checker.is_check_fork_ready());

        // still no confirmations after just becoming ready
        assert!(!checker.has_enough_confirmations());

        // add confirmations (assuming REQUIRED_CONFIRMATIONS is reasonable)
        let confirmation_effort = U256::from(100);
        for i in 0..REQUIRED_CONFIRMATIONS {
            let confirmation_block_number = initial_block_number + 1 + i as u64;
            let block = create_fake_block_with_uncles(
                confirmation_block_number,
                confirmation_effort,
                vec![],
            );
            checker.on_block_added(&block);
        }

        // should now have enough confirmations
        assert!(checker.has_enough_confirmations());
    }

    #[test]
    fn test_confirmations_removed_blocks() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(300);
        let required_num_blocks = 1;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // make check fork ready
        let initial_block_number = 101;
        let block1 = create_fake_block_with_uncles(initial_block_number, required_effort, vec![]);
        checker.on_block_added(&block1);
        assert!(checker.is_check_fork_ready());

        // add some confirmations
        let confirmation_effort = U256::from(100);
        let block2_number = 102;
        let block3_number = 103;
        let block2 = create_fake_block_with_uncles(block2_number, confirmation_effort, vec![]);
        let block3 = create_fake_block_with_uncles(block3_number, confirmation_effort, vec![]);

        checker.on_block_added(&block2);
        checker.on_block_added(&block3);

        let expected_confirmations = 2;
        assert_eq!(
            checker.confirmations.as_ref().unwrap().accum(),
            expected_confirmations
        );

        // remove a confirmation
        checker.on_block_removed(&block2);
        assert_eq!(
            checker.confirmations.as_ref().unwrap().accum(),
            expected_confirmations - 1
        );

        // remove another confirmation
        checker.on_block_removed(&block1);
        assert_eq!(checker.confirmations.unwrap().accum(), 0);
    }

    #[test]
    fn test_remove_block_from_check_fork() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(2000);
        let required_num_blocks = 3;
        let block_hash = H256::from_low_u64_be(100);
        let block_number = 100;
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            block_hash,
            block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add multiple blocks with lower effort so check fork doesn't become ready
        let block1_number = 101;
        let block2_number = 102;
        let block3_number = 103;
        let block1_effort = U256::from(200);
        let block2_effort = U256::from(250);
        let block3_effort = U256::from(200);

        let block1 = create_fake_block_with_uncles(block1_number, block1_effort, vec![]);
        let block2 = create_fake_block_with_uncles(block2_number, block2_effort, vec![]);
        let block3 = create_fake_block_with_uncles(block3_number, block3_effort, vec![]);

        checker.on_block_added(&block1);
        checker.on_block_added(&block2);
        checker.on_block_added(&block3);

        let expected_initial_blocks = 3;
        assert_eq!(checker.args.block_list.len(), expected_initial_blocks);
        assert!(!checker.is_check_fork_ready()); // ensure check fork is not ready yet

        // remove middle block
        checker.on_block_removed(&block2);
        assert_eq!(checker.args.block_list.len(), expected_initial_blocks - 1);

        // verify the correct block was removed
        let remaining_numbers: Vec<u64> =
            checker.args.block_list.iter().map(|b| b.number).collect();
        assert!(remaining_numbers.contains(&block1_number));
        assert!(!remaining_numbers.contains(&block2_number));
        assert!(remaining_numbers.contains(&block3_number));
    }

    #[test]
    fn test_advance_funds_block_has_bridge_event() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 3;
        let advance_funds_block_number = 100;
        let advance_funds_hash = H256::from_low_u64_be(advance_funds_block_number);
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            advance_funds_hash,
            advance_funds_block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add the kickoff block
        let advance_funds_block = create_fake_block(
            BlockNumber::from(advance_funds_block_number),
            U256::from(300),
        );
        let advance_funds_block_with_uncles = RskBlockAndUncles::new(advance_funds_block, vec![]);

        checker.on_block_added(&advance_funds_block_with_uncles);

        // verify bridge event is set
        let check_fork_block = &checker.args.block_list[0];
        assert!(check_fork_block.bridge_event.is_some());

        let bridge_event = check_fork_block.bridge_event.as_ref().unwrap();
        assert_eq!(bridge_event.utxo_id, utxo_id);
        assert_eq!(bridge_event.pegout_id, pegout_id);
        assert_eq!(bridge_event.operator_id, operator_id);
    }

    #[test]
    fn test_non_advance_funds_block_has_no_bridge_event() {
        let pegout_id = "pegout_123";
        let utxo_id = "utxo_456";
        let operator_id = "operator_789";
        let required_effort = U256::from(1000);
        let required_num_blocks = 3;
        let advance_funds_block_number = 100;
        let advance_funds_hash = H256::from_low_u64_be(advance_funds_block_number);
        let tx_hash = H256::from_low_u64_be(100);

        let event = create_fake_advance_funds_event(
            pegout_id,
            utxo_id,
            operator_id,
            required_effort,
            required_num_blocks,
            advance_funds_hash,
            advance_funds_block_number,
            tx_hash,
        );

        let mut checker = CheckForkAccumulator::new(event, vec![]);

        // add a non-kickoff block
        let non_advance_funds_block_number = 101;
        let block =
            create_fake_block_with_uncles(non_advance_funds_block_number, U256::from(300), vec![]);
        checker.on_block_added(&block);

        // verify no bridge event is set
        let check_fork_block = &checker.args.block_list[0];
        assert!(check_fork_block.bridge_event.is_none());
    }
}
