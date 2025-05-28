use crate::types::{BlockWithUncles, KickoffAdvanceFundsEvent};
use check_fork::{Block, CheckForkArgs};
use common::types::{BlockPow, RskBlock};
use log::{debug, info};
use primitive_types::H256;
use primitive_types::U256;

#[derive(Debug)]
pub(super) struct AdvanceFundsChecker {
    kickoff_block_hash: H256,
    check_fork_args: CheckForkArgs,
}

impl AdvanceFundsChecker {
    pub(super) fn new(
        event: KickoffAdvanceFundsEvent,
        post_kickoff_blocks: Vec<&BlockWithUncles>,
    ) -> Self {
        let check_fork_args = CheckForkArgs {
            // coming from the kickoff event
            utxo_id: event.inner.utxo_id.clone(),
            pegout_id: event.inner.peg_out_id.clone(),
            operator_id: event.inner.operator_id.clone(),
            required_effort: U256::from_big_endian(&event.inner.required_effort.to_be_bytes_vec()),
            required_num_blocks: event.inner.required_num_blocks,
            // coming from the kickoff block
            init_block_time: 0,
            init_block_number: 0,
            // coming from kickoff and post-kickoff blocks
            block_list: vec![],
        };

        info!(
            "Creating AdvanceFundsChecker with {:?} and kickoff_block_hash: {}",
            check_fork_args, event.block_hash
        );

        let mut instance = Self {
            kickoff_block_hash: event.block_hash.value(),
            check_fork_args,
        };

        // we already received the block that triggered the event, before the event itself
        post_kickoff_blocks
            .iter()
            .for_each(|b| instance.add_block_to_check_fork(b));

        instance
    }

    pub fn pegout_id(&self) -> String {
        self.check_fork_args.pegout_id.clone()
    }

    pub fn check_fork_args(&self) -> CheckForkArgs {
        self.check_fork_args.clone()
    }

    pub fn update_with_block(&mut self, block_with_uncles: &BlockWithUncles, removed: bool) -> () {
        if removed {
            self.remove_block_from_check_fork(&block_with_uncles.block());
        } else {
            self.add_block_to_check_fork(block_with_uncles);
        }
    }

    pub fn is_ready_for_check_fork(&self) -> bool {
        let accum_effort = self
            .check_fork_args
            .block_list
            .iter()
            .flat_map(|b| std::iter::once(b).chain(&b.uncles))
            .map(|b| BlockPow::from(b.pow).into_effort())
            .fold(U256::zero(), |accum, effort| accum.saturating_add(effort));

        let pending_effort = self
            .check_fork_args
            .required_effort
            .saturating_sub(accum_effort);

        let pending_blocks = self
            .check_fork_args
            .required_num_blocks
            .saturating_sub(self.check_fork_args.block_list.len() as u32);

        let is_req_effort_achieved = pending_effort == U256::zero();
        let is_req_blocks_achieved = pending_blocks == 0;

        let ready = is_req_effort_achieved && is_req_blocks_achieved;
        if ready {
            info!(
                "AdvanceFundsChecker {} is ready for checkFork: {:?}",
                self.check_fork_args.pegout_id, self.check_fork_args
            );
        } else {
            debug!(
                "AdvanceFundsChecker {} is missing {} effort and {} blocks for checkFork",
                self.check_fork_args.pegout_id, pending_effort, pending_blocks
            );
        }

        ready
    }

    fn add_block_to_check_fork(&mut self, block_with_uncles: &BlockWithUncles) {
        let block = &block_with_uncles.block();

        // we received the block that triggered the event after the event itself
        if block.hash() == self.kickoff_block_hash.into() {
            info!("Setting InitBlock on check_fork_args {:?}", block);
            self.check_fork_args.init_block_number = block.number().value();
            self.check_fork_args.init_block_time = block.timestamp().value();
        }

        // include the block in the list, with uncles if any
        self.check_fork_args
            .block_list
            .push(self.new_check_fork_block(block_with_uncles));
    }

    fn remove_block_from_check_fork(&mut self, block: &RskBlock) {
        self.check_fork_args
            .block_list
            .retain(|b| b.hash != block.hash().value());
    }

    fn new_check_fork_block(&self, block_with_uncles: &BlockWithUncles) -> Block {
        debug!(
            "hash {} - block {:?}",
            self.kickoff_block_hash, block_with_uncles
        );

        let block = &block_with_uncles.block();

        let bridge_event = (block.hash() == self.kickoff_block_hash.into()).then(|| {
            let bridge_event = check_fork::BridgeEvent {
                utxo_id: self.check_fork_args.utxo_id.clone(),
                pegout_id: self.check_fork_args.pegout_id.clone(),
                operator_id: self.check_fork_args.operator_id.clone(),
            };
            info!("Set check_fork_args {:?}", bridge_event);
            bridge_event
        });

        let uncle_blocks: Vec<Block> = block_with_uncles
            .uncles()
            .iter()
            .map(|uncle| {
                info!(
                    "Adding to checkFork uncle {} ({}) with effort {}",
                    uncle.number(),
                    uncle.hash(),
                    block.pow().into_effort(),
                );
                // convert each uncle to a checkFork Block: they have neither bridge event nor uncles
                self.rsk_block_to_check_fork_block(uncle, None, vec![])
            })
            .collect();

        info!(
            "Adding to checkFork block {} ({}) with effort {}",
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
}
