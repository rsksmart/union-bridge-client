use crate::types::{BlockWithUncles, KickoffAdvanceFundsEvent};
use check_fork::{Block, CheckForkArgs};
use common::types::{BlockPow, RskBlock};
use log::{debug, info};
#[cfg(feature = "anvil")]
use primitive_types::H256;
use primitive_types::U256;

#[derive(Debug)]
pub(super) struct AdvanceFundsChecker {
    kickoff_event: KickoffAdvanceFundsEvent,
    check_fork_args: CheckForkArgs,
    accum_effort: U256,
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

        let mut instance = Self {
            kickoff_event: event,
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
        self.kickoff_event.inner.peg_out_id.clone()
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

    fn add_block_to_check_fork(&mut self, block_with_uncles: &BlockWithUncles) {
        let block = &block_with_uncles.block();

        // we received the block that triggered the event after the event itself
        if block.hash() == self.kickoff_event.block_hash.into() {
            info!("Setting InitBlock on check_fork_args {:?}", block);
            self.check_fork_args.init_block_number = block.number().value();
            self.check_fork_args.init_block_time = block.timestamp().value();
        }

        // include the block in the list, with uncles if any
        self.check_fork_args
            .block_list
            .push(self.new_check_fork_block(block_with_uncles));

        // TODO(iago) pensar si realmente hace falta esto o si podemos calcularlo a partir de check_fork blocks

        // increase the effort with the block and its uncles
        self.increase_effort(Self::pow_to_effort(&block.pow()));
        for uncle in block_with_uncles.uncles() {
            self.increase_effort(Self::pow_to_effort(&uncle.pow()));
        }
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
            "AdvanceFundsPowChecker {} got new block. Pending effort {} (+{}). Pending blocks {}.",
            self.pegout_id(),
            self.get_missing_effort(),
            block_effort,
            self.get_missing_blocks()
        );
    }

    fn decrease_effort(&mut self, block_effort: U256) {
        self.accum_effort = self.accum_effort.saturating_sub(block_effort);
        info!(
            "AdvanceFundsPowChecker {}: new block, pending effort {} (-{}), blocks {}",
            self.pegout_id(),
            self.get_missing_effort(),
            block_effort,
            self.get_missing_blocks()
        );
    }

    fn new_check_fork_block(&self, block_with_uncles: &BlockWithUncles) -> Block {
        debug!(
            "hash {} - block {:?}",
            self.kickoff_event.block_hash, block_with_uncles
        );

        let block = &block_with_uncles.block();

        let bridge_event = (block.hash() == self.kickoff_event.block_hash.into()).then(|| {
            let bridge_event = check_fork::BridgeEvent {
                utxo_id: self.kickoff_event.inner.utxo_id.clone(),
                pegout_id: self.kickoff_event.inner.peg_out_id.clone(),
                operator_id: self.kickoff_event.inner.operator_id.clone(),
            };
            info!("Set check_fork_args {:?}", bridge_event);
            bridge_event
        });

        let uncle_blocks: Vec<Block> = block_with_uncles
            .uncles()
            .iter()
            .map(|uncle| {
                // convert each uncle to a checkFork Block: they have neither bridge event nor uncles
                self.rsk_block_to_check_fork_block(uncle, None, vec![])
            })
            .collect();

        // create a checkFork Block with bridge_event and uncles if any
        self.rsk_block_to_check_fork_block(block, bridge_event, uncle_blocks)
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

    fn rsk_block_to_check_fork_block(
        &self,
        block: &RskBlock,
        bridge_event: Option<check_fork::BridgeEvent>,
        uncles: Vec<Block>,
    ) -> Block {
        Block {
            number: block.number().value(),
            hash: hex::encode(block.hash().value()),
            parent: hex::encode(block.parent_hash().value()),
            difficulty: block.difficulty().value(),
            timestamp: block.timestamp().value(),
            bridge_event,
            uncles,
            pow: Self::get_block_pow(&block.pow()),
        }
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
