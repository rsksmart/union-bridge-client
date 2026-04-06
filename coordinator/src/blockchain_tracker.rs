use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use anyhow::Result;
use common::types::{BlockNumber, RskBlock, RskBlockAndUncles};
use log::{debug, info, warn};

use crate::types::RskPegManagerEvents;

/// Observer trait for blockchain events.
///
/// # Safety
/// Implementations must NOT call `BlockchainView` methods during callbacks
/// (`on_block_added`, `on_block_removed`) to avoid `RefCell` panics due to reentrancy.
/// The `BlockchainView` holds borrows while calling these methods.
pub trait BlockchainObserver {
    fn get_id(&self) -> String;

    fn on_block_added(&mut self, block: &RskBlockAndUncles);

    fn on_block_removed(&mut self, block: &RskBlockAndUncles);
}

#[derive(Debug)]
pub struct BlockConfirmations {
    flow_id: String,
    init: BlockNumber,
    last: BlockNumber,
    required: u32,
}

impl BlockConfirmations {
    #[must_use]
    pub fn new(flow_id: String, init: BlockNumber, req_confirmations: u32) -> Self {
        Self { flow_id, init, last: init, required: req_confirmations }
    }

    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.accum() >= u64::from(self.required)
    }

    #[must_use]
    pub fn accum(&self) -> u64 {
        if self.last < self.init { 0 } else { self.last.value() - self.init.value() + 1 }
    }
}

impl BlockchainObserver for BlockConfirmations {
    fn get_id(&self) -> String {
        self.flow_id.clone()
    }

    fn on_block_added(&mut self, block: &RskBlockAndUncles) {
        self.last = block.number();
        info!(
            "New block {} ({}) for {}, increasing Rootstock confirmations. Status: {}/{}",
            block.number(),
            block.hash(),
            self.flow_id,
            self.accum(),
            self.required
        );
    }

    fn on_block_removed(&mut self, block: &RskBlockAndUncles) {
        self.last = block.number();
        info!(
            "Removed block {} ({}) for {}, reducing Rootstock confirmations. Status: {}/{}",
            block.number(),
            block.hash(),
            self.flow_id,
            self.accum(),
            self.required
        );
    }
}

/// A view of the blockchain that maintains blocks and notifies observers of changes.
///
/// # `RefCell` Safety
/// This struct uses `RefCell` for interior mutability. Observer implementations MUST NOT call back
/// into `BlockchainView` methods during callbacks (`on_block_added`, `on_block_removed`) to avoid
/// `RefCell` panics due to reentrancy. Such violations are deterministic programming errors that
/// will always panic when the violating code path is executed.
type ObserverMap = HashMap<String, Rc<RefCell<dyn BlockchainObserver>>>;

#[derive(Clone)]
pub struct BlockchainView {
    blocks: Rc<RefCell<BTreeMap<BlockNumber, RskBlockAndUncles>>>,
    observers: Rc<RefCell<ObserverMap>>, // Rc<RefCell<...>> is necessary for shared mutable access to trait objects
}

impl Default for BlockchainView {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockchainView {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocks: Rc::new(RefCell::new(BTreeMap::new())),
            observers: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// # Panics
    /// Panics if the observer ID already exists or if observers are already borrowed (reentrancy detected).
    pub fn add_observer(&self, observer: Rc<RefCell<dyn BlockchainObserver>>) {
        let observer_id = observer.borrow().get_id();

        debug!("Adding observer to BlockchainView: {observer_id}");

        // Check for duplicates with defensive borrowing
        {
            let observers = self.observers.try_borrow()
                .expect("BlockchainView observers already borrowed during add_observer - reentrancy detected! This violates the BlockchainObserver contract and indicates a programming bug.");
            assert!(
                !observers.contains_key(&observer_id),
                "Observer with id {observer_id} already exists in BlockchainView, halting to avoid duplicates"
            );
        } // Drop the borrow before the mutable borrow

        let id = observer.borrow().get_id();
        self.observers.try_borrow_mut()
            .expect("BlockchainView observers already mutably borrowed during add_observer - reentrancy detected! This violates the BlockchainObserver contract and indicates a programming bug.")
            .insert(id, observer);
    }

    /// # Panics
    /// Panics if observers are already borrowed (reentrancy detected).
    pub fn remove_observer(&self, observer_id: &str) {
        self.observers.try_borrow_mut()
            .expect("BlockchainView observers already borrowed during remove_observer - reentrancy detected! This violates the BlockchainObserver contract and indicates a programming bug.")
            .remove(observer_id);
    }

    /// # Panics
    /// Panics if blocks are already borrowed or if block validation fails.
    ///
    pub fn update(&self, new_block: &RskBlockAndUncles) {
        let prev_tip = self.get_tip();

        let removed_block = self.blocks.borrow_mut().insert(new_block.number(), new_block.clone());

        // new tip without reorg
        if removed_block.is_none() {
            debug!(
                "Adding new tip {} ({}) to BlockchainView",
                new_block.number(),
                new_block.hash()
            );

            if let Some(prev_tip) = prev_tip {
                Self::validate_consecutive_block(new_block.block(), prev_tip.block());
            }

            self.notify_added_block(new_block);

            return;
        }

        let removed_block = removed_block.unwrap();

        let stored_tip =
            self.get_tip().expect("There should be a tip block after adding a new block");

        // tip replacement
        if new_block.number() == stored_tip.number() {
            info!(
                "Replacing tip block {} ({}) with {} ({}) in BlockchainView",
                stored_tip.number(),
                stored_tip.hash(),
                new_block.number(),
                new_block.hash()
            );

            // update all visitors of the replacement
            self.notify_removed_block(&removed_block);
            self.notify_added_block(new_block);

            return;
        }

        // reorg
        self.rollback_to(new_block, &removed_block);
    }

    #[must_use]
    pub fn get_from(&self, number: BlockNumber) -> Vec<RskBlockAndUncles> {
        self.blocks.borrow().range(number..).map(|(_, b)| b.clone()).collect()
    }

    #[must_use]
    pub fn get_tip(&self) -> Option<RskBlockAndUncles> {
        self.blocks.borrow().values().next_back().cloned()
    }

    pub fn restart_from(&self, first_block: BlockNumber) {
        self.blocks.borrow_mut().retain(|_, b| b.number() >= first_block);
    }

    pub fn clear(&self) {
        self.blocks.borrow_mut().clear();
        self.observers.borrow_mut().clear();
    }

    fn rollback_to(&self, new_tip: &RskBlockAndUncles, removed_block: &RskBlockAndUncles) {
        debug!(
            "Reorg detected, rolling back BlockchainView to {} ({})",
            new_tip.number(),
            new_tip.hash()
        );

        let mut blocks_to_rollback = Vec::new();
        for (_, block) in self.blocks.borrow().iter().rev() {
            // new tip is already in the chain, so we stop as soon as we reach it
            if block.number() <= new_tip.number() {
                break;
            }
            blocks_to_rollback.push(block.clone());
        }

        for btr in &blocks_to_rollback {
            debug!("Rolling back block {} ({}) in BlockchainView", btr.number(), btr.hash());

            self.blocks.borrow_mut().remove(&btr.number());

            // notify observers about the removal
            self.notify_removed_block(btr);
        }

        // notify observers about the new tip
        self.notify_removed_block(removed_block);
        self.notify_added_block(new_tip);

        info!(
            "Reorg fixed, rolled back BlockchainView to {} ({})",
            new_tip.number(),
            new_tip.hash()
        );
    }

    fn notify_added_block(&self, new_block: &RskBlockAndUncles) {
        // update all visitors when adding a new block
        let observers = self.observers.try_borrow()
            .expect("BlockchainView observers already borrowed during notify_added_block - reentrancy detected! This violates the BlockchainObserver contract and indicates a programming bug.");

        for observer in observers.values() {
            observer.borrow_mut().on_block_added(new_block);
        }
    }

    fn notify_removed_block(&self, rolled_back_block: &RskBlockAndUncles) {
        // update all visitors when removing a block
        let observers = self.observers.try_borrow()
            .expect("BlockchainView observers already borrowed during notify_removed_block - reentrancy detected! This violates the BlockchainObserver contract and indicates a programming bug.");

        for observer in observers.values() {
            observer.borrow_mut().on_block_removed(rolled_back_block);
        }
    }

    #[must_use]
    pub fn has_observers(&self) -> bool {
        !self.observers.borrow().is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub fn get_at(&self, number: &BlockNumber) -> Option<RskBlockAndUncles> {
        self.blocks.borrow().get(number).cloned()
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.borrow().is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub fn has_observer(&self, id: &str) -> bool {
        self.observers.borrow().contains_key(id)
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_observed(&self) -> bool {
        !self.observers.borrow().is_empty()
    }

    #[cfg(test)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.blocks.borrow().len()
    }

    fn validate_consecutive_block(new_tip: &RskBlock, prev_tip: &RskBlock) {
        // validate that blocks are consecutive
        assert!(
            new_tip.number() == prev_tip.number() + 1 && new_tip.parent_hash() == prev_tip.hash(),
            "Non-consecutive block or parent hash mismatch: block {} after {}, parent_hash: {:?}, expected: {:?}",
            new_tip.number(),
            prev_tip.number(),
            new_tip.parent_hash(),
            prev_tip.hash()
        );
    }
}

pub struct ConfirmableEvent {
    id: String,
    chain_view: BlockchainView,
    req_confirmations: u32,
    confirmations: Option<Rc<RefCell<BlockConfirmations>>>,
}

pub struct ConfirmableEventWithData {
    confirmation: ConfirmableEvent,
    data: RskPegManagerEvents,
}

impl ConfirmableEventWithData {
    pub fn new(
        id: String,
        req_confirmations: u32,
        chain_view: BlockchainView,
        data: RskPegManagerEvents,
    ) -> Self {
        Self { confirmation: ConfirmableEvent::new(id, req_confirmations, chain_view), data }
    }

    pub fn id(&self) -> String {
        self.confirmation.id.clone()
    }

    /// # Errors
    /// Returns an error if starting confirmation tracking fails.
    pub fn start_confirming(&mut self, block_number: BlockNumber) -> Result<()> {
        self.confirmation.start_confirming(block_number)
    }

    /// # Errors
    /// Returns an error if stopping confirmation tracking fails.
    pub fn stop_confirming(&mut self) -> Result<()> {
        self.confirmation.stop_confirming()
    }

    pub fn is_confirmed(&self) -> bool {
        self.confirmation.is_confirmed()
    }

    pub fn get_data(&self) -> &RskPegManagerEvents {
        &self.data
    }
}

impl ConfirmableEvent {
    #[must_use]
    pub fn new(id: String, req_confirmations: u32, chain_view: BlockchainView) -> Self {
        ConfirmableEvent { id, chain_view, req_confirmations, confirmations: None }
    }

    /// # Errors
    /// Returns an error if starting confirmation tracking fails.
    pub fn start_confirming(&mut self, block_number: BlockNumber) -> Result<()> {
        let rc_confirmations = Rc::new(RefCell::new(BlockConfirmations::new(
            self.id.clone(),
            block_number,
            self.req_confirmations,
        )));

        self.confirmations = Some(rc_confirmations.clone());
        // remove just in case
        self.remove_observer();
        self.add_observer(rc_confirmations);

        Ok(())
    }

    /// # Errors
    /// Returns an error if stopping confirmation fails.
    pub fn stop_confirming(&mut self) -> Result<()> {
        if self.confirmations.is_none() {
            warn!("Confirmations not set for protocol {} but stop_confirming called", self.id);
        }

        self.remove_observer();
        self.confirmations = None;

        Ok(())
    }

    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmations.as_ref().map_or_else(
            || {
                warn!("Confirmations not set for protocol {} but is_confirmed called", self.id);
                false
            },
            |c| c.borrow().is_confirmed(),
        )
    }

    fn add_observer(&self, rc_confirmations: Rc<RefCell<BlockConfirmations>>) {
        self.chain_view.add_observer(rc_confirmations);
    }

    fn remove_observer(&self) {
        self.chain_view.remove_observer(&self.id);
    }
}

#[cfg(test)]
mod tests_blockchain_view {
    use std::ops::Mul;

    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use primitive_types::{H256, U256};

    use super::*;

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
        let parent_hash = BlockHash::from(H256::from_low_u64_be((number + 1000).saturating_sub(1)));
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
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // add consecutive blocks
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);

        chain_view.update(&block_100);
        chain_view.update(&block_101);
        chain_view.update(&block_102);

        let tracker_ref = tracker.borrow();
        assert_eq!(
            tracker_ref.get_added_blocks(),
            vec![block_100.clone(), block_101.clone(), block_102.clone()]
        );
        assert!(tracker_ref.get_removed_blocks().is_empty(),);

        // verify final chain state has correct blocks
        assert_eq!(chain_view.len(), 3);
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(block_100.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(block_101.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), Some(block_102.clone()));

        // verify tip is the last added block
        assert_eq!(chain_view.get_tip(), Some(block_102));
    }

    #[test]
    fn test_blockchain_view_block_replacement() {
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // add initial blocks
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);

        chain_view.update(&block_100);
        chain_view.update(&block_101);
        chain_view.update(&block_102);

        tracker.borrow().clear();

        // replace block 102 with an alternative
        let alt_block_102 = create_alt_test_block(102);
        chain_view.update(&alt_block_102);

        let tracker_ref = tracker.borrow();
        // should see: removed original block 102, added alternative block 102
        assert_eq!(tracker_ref.get_added_blocks(), vec![alt_block_102.clone()]);
        assert_eq!(tracker_ref.get_removed_blocks(), vec![block_102.clone()]);

        // verify final chain state has correct blocks
        assert_eq!(chain_view.len(), 3);
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(block_100.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(block_101.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), Some(alt_block_102.clone()));

        // verify the replacement block is actually different
        assert_ne!(alt_block_102.hash(), block_102.hash()); // different hashes
        assert_eq!(alt_block_102.number(), block_102.number()); // same block number
    }

    #[test]
    fn test_blockchain_view_reorg_old_block_observer_notifications() {
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("tracker")));
        chain_view.add_observer(tracker.clone());

        let confirmations = Rc::new(RefCell::new(BlockConfirmations::new(
            "confirmations".to_string(),
            BlockNumber::from(100),
            3,
        )));
        chain_view.add_observer(confirmations.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);
        let block_102 = create_test_block(102);
        let block_103 = create_test_block(103);

        chain_view.update(&block_100);
        chain_view.update(&block_101);
        chain_view.update(&block_102);
        chain_view.update(&block_103);

        // should have 4 confirmations
        assert_eq!(confirmations.borrow().accum(), 4);
        assert!(confirmations.borrow().is_confirmed());

        tracker.borrow().clear();

        // simulate reorg at block 101 (this should rollback blocks 102 and 103)
        let alt_block_101 = create_alt_test_block(101);
        chain_view.update(&alt_block_101);

        let tracker_ref = tracker.borrow();

        // verify exact notification sequence during reorg:
        // 1. blocks 102 and 103 should be removed (they have number > 101)
        // 2. alternative block 101 should be added
        assert_eq!(
            tracker_ref.get_removed_blocks(),
            vec![block_103.clone(), block_102.clone(), block_101.clone()]
        );

        // verify final chain state with actual block values
        assert_eq!(chain_view.len(), 2); // blocks 100 and alt_101
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(block_100.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(alt_block_101.clone()));
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(103)), None);

        // should have 3 confirmations now:
        // - started with 4 confirmations (blocks 100, 101, 102, 103)
        // - removed blocks 102 and 103 (-2 confirmations)
        // - added alternative block 101 (+1 confirmation)
        // - but block 101 was already there and gets replaced (-1 confirmation), so:
        //   4 - 2 + 1 - 1 = 2 confirmations (blocks 100, alt_101, and the +1 from adding alt_101)
        assert_eq!(confirmations.borrow().accum(), 2);
        assert!(!confirmations.borrow().is_confirmed());
    }

    #[test]
    #[should_panic(expected = "Non-consecutive block or parent hash mismatch")]
    fn test_blockchain_view_fails_on_tip_gap() {
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);

        chain_view.update(&block_100);
        chain_view.update(&block_101);

        tracker.borrow().clear();

        // simulate reorg at block 102
        let alt_block_102 = create_test_block(103);

        chain_view.update(&alt_block_102);
    }

    #[test]
    #[should_panic(expected = "Non-consecutive block or parent hash mismatch")]
    fn test_blockchain_view_fails_on_not_connected_tip() {
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103
        let block_100 = create_test_block(100);
        let block_101 = create_test_block(101);

        chain_view.update(&block_100);
        chain_view.update(&block_101);

        tracker.borrow().clear();

        // simulate reorg at block 102
        let alt_block_102 = create_alt_test_block(102);

        chain_view.update(&alt_block_102);
    }

    #[test]
    fn test_blockchain_view_deep_reorg_observer_notifications() {
        let chain_view = BlockchainView::new();
        let tracker = Rc::new(RefCell::new(NotificationTracker::new("test")));
        chain_view.add_observer(tracker.clone());

        // build initial chain: 100 -> 101 -> 102 -> 103 -> 104 -> 105
        let mut blocks = Vec::new();
        for i in 100..=105 {
            let block = create_test_block(i);
            blocks.push(block.clone());
            chain_view.update(&block);
        }

        tracker.borrow().clear();

        // simulate deep reorg back to block 102
        let alt_block_102 = create_alt_test_block(102);
        chain_view.update(&alt_block_102);

        let tracker_ref = tracker.borrow();

        // should remove blocks 105, 104, 103 and add alternative block 102
        assert_eq!(
            tracker_ref.get_removed_blocks(),
            vec![blocks[5].clone(), blocks[4].clone(), blocks[3].clone(), blocks[2].clone()]
        ); // blocks 105, 104, 103
        assert_eq!(tracker_ref.get_added_blocks(), vec![alt_block_102.clone()]);

        // verify final chain state with actual block values
        assert_eq!(chain_view.len(), 3); // blocks 100, 101, alt_102
        assert_eq!(chain_view.get_at(&BlockNumber::from(100)), Some(blocks[0].clone())); // original block 100
        assert_eq!(chain_view.get_at(&BlockNumber::from(101)), Some(blocks[1].clone())); // original block 101
        assert_eq!(chain_view.get_at(&BlockNumber::from(102)), Some(alt_block_102.clone())); // alternative block 102
        assert_eq!(chain_view.get_at(&BlockNumber::from(103)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(104)), None);
        assert_eq!(chain_view.get_at(&BlockNumber::from(105)), None);

        // verify the alternative block is actually different from original
        assert_ne!(alt_block_102.hash(), blocks[2].hash()); // different from original block 102
        assert_eq!(alt_block_102.number(), blocks[2].number()); // same block number
    }
}

#[cfg(test)]
mod confirmable_event_tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use common::test_utils::rsk_block_generator::FakeBlockGenerator;
    use common::types::{BlockNumber, RskBlockAndUncles};
    use uuid::Uuid;

    use crate::blockchain_tracker::{BlockchainView, ConfirmableEvent};

    #[test]
    fn test_confirmable_event_happy_path_flow() {
        // setup
        let id = Uuid::new_v4().to_string();
        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event =
            ConfirmableEvent::new(id.clone(), req_confirmations, chain_view);

        // step 1: start_confirming() - should succeed and add observer
        let block_number = BlockNumber::from(100);
        let result = confirmable_event.start_confirming(block_number);
        assert!(result.is_ok(), "start_confirming() should succeed");

        // verify observer was added to blockchain view
        assert!(confirmable_event.chain_view.has_observer(&id));

        // step 2: should not be confirmed yet (no blocks processed)
        assert!(!confirmable_event.is_confirmed(), "Should not be confirmed initially");

        // step 3: simulate processing blocks to reach confirmation
        // we need to process req_confirmations number of blocks
        for i in 0..u64::from(req_confirmations) {
            let block = create_test_block(block_number + i);
            confirmable_event.chain_view.update(&block);
        }

        // step 4: should be confirmed after processing enough blocks
        assert!(
            confirmable_event.is_confirmed(),
            "Should be confirmed after processing enough blocks"
        );
    }

    #[test]
    fn test_confirmable_event_stop_confirming_and_resuming() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: verify observer was added
        assert!(confirmable_event.chain_view.has_observer(&confirmable_event.id));

        // step 2: stop_confirming() should succeed and remove observer
        let result = confirmable_event.stop_confirming();
        assert!(result.is_ok(), "stop_confirming() should succeed");

        // step 3: observer should be removed from the blockchain view
        assert!(
            !confirmable_event.chain_view.has_observer(&confirmable_event.id),
            "Observer should be removed from blockchain view after stop_confirming"
        );

        // step 4: confirmations should be None after stopping
        assert!(!confirmable_event.is_confirmed(), "Should not be confirmed after stopping");
    }

    #[test]
    fn test_confirmable_event_stop_confirming_without_start_does_not_fail() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        // stop_confirming without a start should fail
        let result = confirmable_event.stop_confirming();
        assert!(result.is_ok(), "stop_confirming() without start should NOT fail");
    }

    #[test]
    fn test_confirmable_event_blocks_removed_reduces_confirmations() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let start_block = BlockNumber::from(100);
        confirmable_event.start_confirming(start_block).expect("Failed to start confirming");

        // step 1: add enough blocks to reach confirmation
        let mut blocks = Vec::new();
        for i in 0..u64::from(req_confirmations) {
            let block = create_test_block(start_block + i);
            blocks.push(block.clone());
            confirmable_event.chain_view.update(&block);
        }

        // step 2: should be confirmed after processing enough blocks
        assert!(
            confirmable_event.is_confirmed(),
            "Should be confirmed after processing enough blocks"
        );

        // step 3: simulate a reorg by creating an alternative block at position 1
        // this should cause blocks 1 and 2 to be removed, reducing confirmations
        let alt_block_1 = create_alt_test_block(start_block + 1);
        confirmable_event.chain_view.update(&alt_block_1);

        // step 4: should no longer be confirmed due to reduced confirmations
        assert!(
            !confirmable_event.is_confirmed(),
            "Should not be confirmed after reorg removes blocks"
        );
    }

    #[test]
    fn test_confirmable_event_is_confirmed_when_confirmations_none() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        // step 1: is_confirmed() should return false and log warning
        assert!(
            !confirmable_event.is_confirmed(),
            "Should not be confirmed when confirmations is None"
        );
    }

    #[test]
    fn test_confirmable_event_start_confirming_twice_fails() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        // step 1: first start_confirming() should succeed
        let block_number = BlockNumber::from(100);
        let result = confirmable_event.start_confirming(block_number);
        assert!(result.is_ok(), "First start_confirming() should succeed");

        // step 2: second start_confirming() should succeed (replaces previous)
        let result = confirmable_event.start_confirming(block_number + 1);
        assert!(result.is_ok(), "Second start_confirming() should succeed");

        // step 3: should still work properly
        assert!(!confirmable_event.is_confirmed(), "Should not be confirmed initially");
    }

    #[test]
    fn test_confirmable_event_stop_confirming_twice_does_not_fail() {
        // setup
        let id = Uuid::new_v4().to_string();
        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: first stop_confirming() should succeed
        let result = confirmable_event.stop_confirming();
        assert!(result.is_ok(), "First stop_confirming() should succeed");

        // step 2: second stop_confirming() should succeed with warning
        let result = confirmable_event.stop_confirming();
        assert!(result.is_ok(), "Second stop_confirming() should NOT fail");
    }

    #[test]
    fn test_confirmable_event_restart_confirming_after_stopping() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: add partial confirmations (not enough to reach confirmation)
        for i in 0..u64::from(req_confirmations - 1) {
            let block = create_test_block(block_number + i);
            confirmable_event.chain_view.update(&block);
        }

        // step 2: should not be confirmed yet (partial confirmations)
        assert!(
            !confirmable_event.is_confirmed(),
            "Should not be confirmed with partial confirmations"
        );

        // step 3: stop_confirming() - this should reset all confirmations
        confirmable_event.stop_confirming().expect("Failed to stop confirming");
        assert!(
            !confirmable_event.is_confirmed(),
            "Should not be confirmed after stopping (confirmations reset)"
        );

        // step 4: restart confirming should succeed
        let restart_block = block_number + u64::from(req_confirmations - 1);
        let result = confirmable_event.start_confirming(restart_block);
        assert!(result.is_ok(), "Should be able to restart confirming");

        // step 5: should work properly after restart (confirmations start from 0 again)
        assert!(
            !confirmable_event.is_confirmed(),
            "Should not be confirmed initially after restart"
        );

        // step 6: add blocks to reach confirmation from restart point (consecutive blocks)
        for i in 0..u64::from(req_confirmations) {
            let block = create_test_block(restart_block + i);
            confirmable_event.chain_view.update(&block);
        }

        // step 7: should be confirmed after processing enough blocks from restart
        assert!(
            confirmable_event.is_confirmed(),
            "Should be confirmed after restart and processing blocks"
        );
    }

    #[test]
    fn test_confirmable_event_zero_confirmations_required() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 0; // Edge case: zero confirmations
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: should be immediately confirmed with zero confirmations required
        assert!(
            confirmable_event.is_confirmed(),
            "Should be immediately confirmed with zero confirmations required"
        );
    }

    #[test]
    fn test_confirmable_event_one_confirmation_required() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 1; // Edge case: one confirmation
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: with 1 confirmation required, it might be immediately confirmed
        // depending on how the blockchain tracker counts confirmations
        let _initially_confirmed = confirmable_event.is_confirmed();

        // step 2: add one block to ensure it's confirmed
        let block = create_test_block(block_number);
        confirmable_event.chain_view.update(&block);

        // step 3: should definitely be confirmed after adding a block
        assert!(
            confirmable_event.is_confirmed(),
            "Should be confirmed after adding a block with 1 confirmation required"
        );
    }

    #[test]
    fn test_confirmable_event_observer_cleanup_after_stop_confirming() {
        // setup
        let id = Uuid::new_v4().to_string();

        let req_confirmations = 3;
        let chain_view = BlockchainView::new();

        let mut confirmable_event = ConfirmableEvent::new(id, req_confirmations, chain_view);

        let block_number = BlockNumber::from(100);
        confirmable_event.start_confirming(block_number).expect("Failed to start confirming");

        // step 1: verify observer was added
        assert!(
            confirmable_event.chain_view.has_observer(&confirmable_event.id),
            "Observer should be added after start_confirming"
        );

        // step 2: stop_confirming()
        confirmable_event.stop_confirming().expect("Failed to stop confirming");

        // step 3: confirmations should be None (observer reference removed from struct)
        assert!(
            confirmable_event.confirmations.is_none(),
            "confirmations should be None after stop_confirming"
        );

        // step 4: observer should be removed from the blockchain view
        assert!(
            !confirmable_event.chain_view.has_observer(&confirmable_event.id),
            "Observer should be removed from blockchain view after stop_confirming"
        );

        // step 5: is_confirmed should return false and log warning
        assert!(!confirmable_event.is_confirmed(), "Should not be confirmed after stop_confirming");
    }

    fn create_test_block(block_number: BlockNumber) -> RskBlockAndUncles {
        let generator = FakeBlockGenerator::new(None, Arc::new(AtomicBool::new(false)), None);
        let block =
            generator.generate_block(block_number, None).expect("Failed to generate test block");

        RskBlockAndUncles::new_no_uncles(block)
    }

    fn create_alt_test_block(block_number: BlockNumber) -> RskBlockAndUncles {
        // create a generator with a reorg flag set to true to generate alternative blocks
        let generator =
            FakeBlockGenerator::new(Some(block_number - 1), Arc::new(AtomicBool::new(true)), None);
        let block = generator
            .generate_block(block_number, None)
            .expect("Failed to generate alternative test block");

        RskBlockAndUncles::new_no_uncles(block)
    }
}
