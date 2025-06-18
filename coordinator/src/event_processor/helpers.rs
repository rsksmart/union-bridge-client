use common::types::{BlockNumber, RskBlock, RskBlockAndUncles};
use log::{error, info};
use std::collections::BTreeMap;

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

    pub fn update(&mut self, removed: bool) {
        if removed {
            self.accum = self.accum.saturating_sub(1);
            info!(
                "Removed confirmation for {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
        } else {
            self.accum = self.accum.saturating_add(1);
            info!(
                "Added confirmation to {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
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

// TODO(iago) move to another place
#[derive(Debug)]
pub struct BlockchainView {
    blocks: BTreeMap<BlockNumber, RskBlockAndUncles>,
}

impl BlockchainView {
    pub fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
        }
    }

    /// Adds a block to the known blocks.
    ///
    /// # Arguments
    ///
    /// * `block` - The block to add.
    ///
    /// # Returns
    ///
    /// The replaced block if it exists, `None` otherwise.
    pub fn add(&mut self, block: RskBlockAndUncles) -> Option<RskBlockAndUncles> {
        self.validate_consecutive_block(&block.block());
        self.blocks.insert(block.block().number(), block)
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
