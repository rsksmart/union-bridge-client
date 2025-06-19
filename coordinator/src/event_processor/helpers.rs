use common::types::{BlockNumber, RskBlock, RskBlockAndUncles};
use log::{error, info};
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
    pub(crate) fn accum(&self) -> u32 {
        self.accum
    }
}

impl BlockchainObserver for Confirmations {
    fn get_id(&self) -> String {
        self.flow_id.clone()
    }

    fn update_with_block(
        &mut self,
        new_block: &RskBlockAndUncles,
        removed_block: &Option<RskBlockAndUncles>,
    ) {
        if removed_block.is_some() {
            self.accum = self.accum.saturating_sub(1);
            info!(
                "Replaced block {} ({}) for {}, keeping confirmations. Status: {}/{}",
                new_block.number(),
                new_block.hash(),
                self.flow_id,
                self.accum,
                self.req
            );
        } else {
            self.accum = self.accum.saturating_add(1);
            info!(
                "New block {} ({}) for {}, increasing confirmations. Status: {}/{}",
                new_block.number(),
                new_block.hash(),
                self.flow_id,
                self.accum,
                self.req
            );
        }
    }
}

pub trait BlockchainObserver {
    fn get_id(&self) -> String;

    fn update_with_block(
        &mut self,
        new_block: &RskBlockAndUncles,
        removed_block: &Option<RskBlockAndUncles>,
    );
}

// TODO(iago) move to another place
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
        let id = observer.borrow().get_id();
        self.observers.insert(id, observer);
    }

    pub fn remove_observer(&mut self, observer_id: &str) {
        self.observers.remove(observer_id);
    }

    pub fn add(&mut self, block: RskBlockAndUncles) {
        self.validate_consecutive_block(&block.block());

        let new_block = block.clone();

        let removed_block = self.blocks.insert(block.block().number(), block);

        // update all visitors when adding or removing a new block
        for observer in self.observers.values() {
            observer
                .borrow_mut()
                .update_with_block(&new_block, &removed_block);
        }
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

    #[cfg(test)]
    pub fn get(&self, number: &BlockNumber) -> Option<&RskBlockAndUncles> {
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

    fn validate_consecutive_block(&self, block: &RskBlock) {
        // validate that blocks are consecutive
        if let Some(prev_block) = self.get_tip() {
            if block.number() != prev_block.number() + 1
                || block.parent_hash() != prev_block.block().hash()
            {
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) we should properly react to this fact - https://rsklabs.atlassian.net/browse/UB-132
                error!(
                    "Non-consecutive block or parent hash mismatch: block {} after {}, parent_hash: {:?}, expected: {:?}",
                    block.number(),
                    prev_block.number(),
                    block.parent_hash(),
                    prev_block.block().hash()
                );
            }
        }
    }
}
