use common::types::{BlockNumber, RskBlock, RskBlockAndUncles};
use log::{debug, info};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

#[derive(Debug)]
pub struct Confirmations {
    flow_id: String,
    accum: u32,
    req: u32,
}

impl Confirmations {
    pub fn new(flow_id: String, req_confirmations: u32) -> Self {
        Self {
            flow_id,
            accum: 0,
            req: req_confirmations,
        }
    }

    pub fn is_confirmed(&self) -> bool {
        self.accum >= self.req
    }

    #[cfg(test)]
    pub fn accum(&self) -> u32 {
        self.accum
    }
}

impl BlockchainObserver for Confirmations {
    fn get_id(&self) -> String {
        self.flow_id.clone()
    }

    fn on_block_added(&mut self, block: &RskBlockAndUncles) {
        self.accum = self.accum.saturating_add(1);
        info!(
            "New block {} ({}) for {}, increasing confirmations. Status: {}/{}",
            block.number(),
            block.hash(),
            self.flow_id,
            self.accum,
            self.req
        );
    }

    fn on_block_removed(&mut self, block: &RskBlockAndUncles) {
        self.accum = self.accum.saturating_sub(1);
        info!(
            "Removed block {} ({}) for {}, reducing confirmations. Status: {}/{}",
            block.number(),
            block.hash(),
            self.flow_id,
            self.accum,
            self.req
        );
    }
}

pub trait BlockchainObserver {
    fn get_id(&self) -> String;

    fn on_block_added(&mut self, block: &RskBlockAndUncles);

    fn on_block_removed(&mut self, block: &RskBlockAndUncles);
}

pub struct BlockchainView {
    blocks: BTreeMap<BlockNumber, RskBlockAndUncles>,
    observers: HashMap<String, Rc<RefCell<dyn BlockchainObserver>>>,
}

impl BlockchainView {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            observers: HashMap::new(),
        }
    }

    pub fn add_observer(&mut self, observer: Rc<RefCell<dyn BlockchainObserver>>) {
        debug!(
            "Adding observer to BlockchainView: {}",
            observer.borrow().get_id()
        );
        let id = observer.borrow().get_id();
        self.observers.insert(id, observer);
    }

    pub fn remove_observer(&mut self, observer_id: &str) {
        self.observers.remove(observer_id);
    }

    pub fn update(&mut self, new_block: RskBlockAndUncles) {
        let prev_tip = self.get_tip().map(|b| b.clone());

        let removed_block = self.blocks.insert(new_block.number(), new_block.clone());

        // new tip block
        if removed_block.is_none() {
            if let Some(prev_tip) = prev_tip {
                self.validate_consecutive_block(&new_block.block(), prev_tip.block());
            }

            self.notify_added_block(&new_block);

            return;
        }

        let removed_block = removed_block.unwrap();

        let new_tip = self
            .get_tip()
            .expect("There should be a tip block after adding a new block");

        // tip replaced
        if new_block.number() == new_tip.number() {
            // update all visitors of the replacement
            self.notify_added_block(&new_block);
            self.notify_removed_block(&removed_block);
            return;
        }

        // reorg
        self.rollback_to(new_block);
    }

    pub fn get_from(&self, number: BlockNumber) -> Vec<&RskBlockAndUncles> {
        self.blocks.range(number..).map(|(_, b)| b).collect()
    }

    pub fn get_tip(&self) -> Option<&RskBlockAndUncles> {
        self.blocks.values().rev().next()
    }

    pub fn restart_from(&mut self, first_block: BlockNumber) {
        self.blocks.retain(|_, b| b.number() >= first_block)
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.observers.clear();
    }

    fn rollback_to(&mut self, new_tip: RskBlockAndUncles) {
        let mut blocks_to_rollback = Vec::new();
        for (_, block) in self.blocks.iter().rev() {
            // new tip is already in the chain, so we stop as soon as we reach it
            if block.number() <= new_tip.number() {
                break;
            }
            blocks_to_rollback.push(block.clone());
        }

        for rolled_back_block in &blocks_to_rollback {
            self.blocks.remove(&rolled_back_block.number());

            // notify observers about the removal
            self.notify_removed_block(rolled_back_block);
        }

        // notify observers about the new tip
        self.notify_added_block(&new_tip);
    }

    fn notify_added_block(&mut self, new_block: &RskBlockAndUncles) {
        // update all visitors when adding a new block
        for observer in self.observers.values() {
            observer.borrow_mut().on_block_added(&new_block);
        }
    }

    fn notify_removed_block(&mut self, rolled_back_block: &RskBlockAndUncles) {
        for observer in self.observers.values() {
            observer.borrow_mut().on_block_removed(rolled_back_block);
        }
    }

    #[cfg(test)]
    pub fn get_at(&self, number: &BlockNumber) -> Option<&RskBlockAndUncles> {
        self.blocks.get(number)
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    #[cfg(test)]
    pub fn is_observed(&self) -> bool {
        !self.observers.is_empty()
    }

    #[cfg(test)]
    pub fn has_observer(&self, id: &str) -> bool {
        self.observers.contains_key(id)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    fn validate_consecutive_block(&self, new_tip: &RskBlock, prev_tip: &RskBlock) {
        // validate that blocks are consecutive
        if new_tip.number() != prev_tip.number() + 1 || new_tip.parent_hash() != prev_tip.hash() {
            panic!(
                "Non-consecutive block or parent hash mismatch: block {} after {}, parent_hash: {:?}, expected: {:?}",
                new_tip.number(),
                prev_tip.number(),
                new_tip.parent_hash(),
                prev_tip.hash()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use primitive_types::{H256, U256};
    use std::ops::Mul;

    // mock observer that tracks all notifications for testing
    #[derive(Debug)]
    struct NotificationTracker {
        id: String,
        added_blocks: RefCell<Vec<RskBlockAndUncles>>,
        removed_blocks: RefCell<Vec<RskBlockAndUncles>>,
    }

    impl NotificationTracker {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                added_blocks: RefCell::new(Vec::new()),
                removed_blocks: RefCell::new(Vec::new()),
            }
        }

        fn get_added_blocks(&self) -> Vec<RskBlockAndUncles> {
            self.added_blocks.borrow().clone()
        }

        fn get_removed_blocks(&self) -> Vec<RskBlockAndUncles> {
            self.removed_blocks.borrow().clone()
        }

        fn clear(&self) {
            self.added_blocks.borrow_mut().clear();
            self.removed_blocks.borrow_mut().clear();
        }
    }

    impl BlockchainObserver for NotificationTracker {
        fn get_id(&self) -> String {
            self.id.clone()
        }

        fn on_block_added(&mut self, block: &RskBlockAndUncles) {
            self.added_blocks.borrow_mut().push(block.clone());
        }

        fn on_block_removed(&mut self, block: &RskBlockAndUncles) {
            self.removed_blocks.borrow_mut().push(block.clone());
        }
    }

    fn create_test_block(number: u64) -> RskBlockAndUncles {
        let block_number = BlockNumber::from(number);
        let block_hash = BlockHash::from(H256::from_low_u64_be(number));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number.saturating_sub(1)));
        let timestamp = BlockTimestamp::from(number * 1000);
        let difficulty = BlockDifficulty::from(U256::from(500));
        let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));
        let pow = BlockPow::from(H256::from_low_u64_be(number));
        let uncles = vec![];

        let block = RskBlock::new(
            block_number,
            block_hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow,
            uncles,
        );

        RskBlockAndUncles::new_no_uncles(block)
    }

    fn create_alt_test_block(number: u64) -> RskBlockAndUncles {
        let block_number = BlockNumber::from(number);
        // different hash to make it an alternative block
        let block_hash = BlockHash::from(H256::from_low_u64_be(number + 1000));
        let parent_hash = BlockHash::from(H256::from_low_u64_be(number.saturating_sub(1)));
        let timestamp = BlockTimestamp::from(number * 1000);
        let difficulty = BlockDifficulty::from(U256::from(500));
        let total_difficulty = difficulty.mul(BlockDifficulty::from(U256::from(1000)));
        let pow = BlockPow::from(H256::from_low_u64_be(number + 1000));
        let uncles = vec![];

        let block = RskBlock::new(
            block_number,
            block_hash,
            parent_hash,
            timestamp,
            difficulty,
            total_difficulty,
            pow,
            uncles,
        );

        RskBlockAndUncles::new_no_uncles(block)
    }

    #[test]
    fn test_blockchain_view_normal_block_addition() {
        let mut chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // add consecutive blocks
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);

        chain_view.update(block_100.clone());
        chain_view.update(block_101.clone());
        chain_view.update(block_102.clone());

        let tracker_ref = tracker.borrow();
        assert_eq!(
            tracker_ref.get_added_blocks(),
            vec![block_100.clone(), block_101.clone(), block_102.clone()]
        );
        assert!(tracker_ref.get_removed_blocks().is_empty(),);

        // verify final chain state has correct blocks
        assert_eq!(chain_view.len(), 3);
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(&block_100));
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(&block_101));
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), Some(&block_102));

        // verify tip is the last added block
        assert_eq!(chain_view.get_tip(), Some(&block_102));
    }

    #[test]
    fn test_blockchain_view_block_replacement() {
        let mut chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // add initial blocks
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);

        chain_view.update(block_100.clone());
        chain_view.update(block_101.clone());
        chain_view.update(block_102.clone());

        tracker.borrow().clear();

        // replace block 102 with an alternative
        let alt_block_102 = create_alt_test_block(102);
        chain_view.update(alt_block_102.clone());

        let tracker_ref = tracker.borrow();
        // should see: removed original block 102, added alternative block 102
        assert_eq!(tracker_ref.get_added_blocks(), vec![alt_block_102.clone()]);
        assert_eq!(tracker_ref.get_removed_blocks(), vec![block_102.clone()]);

        // verify final chain state has correct blocks
        assert_eq!(chain_view.len(), 3);
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(&block_100));
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(&block_101));
        assert_eq!(
            chain_view.get_at(&BlockNumber::from(102)),
            Some(&alt_block_102)
        );

        // verify the replacement block is actually different
        assert_ne!(alt_block_102.hash(), block_102.hash()); // different hashes
        assert_eq!(alt_block_102.number(), block_102.number()); // same block number
    }

    #[test]
    fn test_blockchain_view_reorg_observer_notifications() {
        let mut chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);
        let block_103 = create_test_block(103);

        chain_view.update(block_100.clone());
        chain_view.update(block_101.clone());
        chain_view.update(block_102.clone());
        chain_view.update(block_103.clone());

        tracker.borrow().clear();

        // simulate reorg at block 101 (this should rollback blocks 102 and 103)
        let alt_block_101 = create_alt_test_block(101);
        chain_view.update(alt_block_101.clone());

        let tracker_ref = tracker.borrow();

        // verify exact notification sequence during reorg:
        // 1. blocks 102 and 103 should be removed (they have number > 101)
        // 2. alternative block 101 should be added
        assert_eq!(
            tracker_ref.get_removed_blocks(),
            vec![block_103.clone(), block_102.clone()]
        );

        // verify final chain state with actual block values
        assert_eq!(chain_view.len(), 2); // blocks 100 and alt_101
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(&block_100));
        assert_eq!(
            chain_view.get_at(&BlockNumber::from(101)),
            Some(&alt_block_101)
        );
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(103)), None);
    }

    #[test]
    fn test_confirmations_observer_reorg_behavior() {
        let mut chain_view = BlockchainView::new();
        let confirmations = Rc::new(RefCell::new(Confirmations::new("test".to_string(), 3)));
        chain_view.add_observer(confirmations.clone());

        // build initial chain and accumulate confirmations
        chain_view.update(create_test_block(100));
        chain_view.update(create_test_block(101));
        chain_view.update(create_test_block(102));
        chain_view.update(create_test_block(103));

        // should have 4 confirmations
        assert_eq!(confirmations.borrow().accum(), 4);
        assert!(confirmations.borrow().is_confirmed());

        // simulate reorg back to block 101
        chain_view.update(create_alt_test_block(101));

        // should have 3 confirmations now:
        // - started with 4 confirmations (blocks 100, 101, 102, 103)
        // - removed blocks 102 and 103 (-2 confirmations)
        // - added alternative block 101 (+1 confirmation)
        // - but block 101 was already there and gets replaced, so:
        //   4 - 2 + 1 = 3 confirmations (blocks 100, alt_101, and the +1 from adding alt_101)
        assert_eq!(confirmations.borrow().accum(), 3);
        assert!(confirmations.borrow().is_confirmed());
    }

    #[test]
    fn test_blockchain_view_deep_reorg_observer_notifications() {
        let mut chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103 -> 104 -> 105
        let mut blocks = Vec::new();
        for i in 100..=105 {
            let block = create_test_block(i);
            blocks.push(block.clone());
            chain_view.update(block);
        }

        tracker.borrow().clear();

        // simulate deep reorg back to block 102
        let alt_block_102 = create_alt_test_block(102);
        chain_view.update(alt_block_102.clone());

        let tracker_ref = tracker.borrow();

        // should remove blocks 105, 104, 103 and add alternative block 102
        assert_eq!(
            tracker_ref.get_removed_blocks(),
            vec![blocks[5].clone(), blocks[4].clone(), blocks[3].clone()]
        ); // blocks 105, 104, 103
        assert_eq!(tracker_ref.get_added_blocks(), vec![alt_block_102.clone()]);

        // verify final chain state with actual block values
        assert_eq!(chain_view.len(), 3); // blocks 100, 101, alt_102
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(&blocks[0])); // original block 100
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(&blocks[1])); // original block 101
        assert_eq!(
            chain_view.get_at(&BlockNumber::from(102)),
            Some(&alt_block_102)
        ); // alternative block 102
        assert_eq!(chain_view.get_at(&BlockNumber::from(103)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(104)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(105)), None);

        // verify the alternative block is actually different from original
        assert_ne!(alt_block_102.hash(), blocks[2].hash()); // different from original block 102
        assert_eq!(alt_block_102.number(), blocks[2].number()); // same block number
    }
}
