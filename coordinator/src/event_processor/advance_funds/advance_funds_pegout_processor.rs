use crate::event_processor::EventProcessor;
use crate::event_processor::advance_funds::advance_funds_checker::AdvanceFundsChecker;
use crate::types::{KickoffAdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents};
use anyhow::Result;
use check_fork::check_fork;
use common::types::{BlockNumber, RskBlock};
use log::{debug, error, info, warn};
use primitive_types::U256;
use std::collections::BTreeMap;

pub struct PegOutAdvanceFundsProcessor {
    request_event: Option<RequestAdvanceFundsEvent>,
    adv_funds_checker: Option<AdvanceFundsChecker>,
    // BTreeMap to sort blocks by number while keeping just the most recent one in case of reorgs
    known_blocks: BTreeMap<BlockNumber, RskBlock>,
}

impl PegOutAdvanceFundsProcessor {
    pub fn new() -> Self {
        Self {
            request_event: None, // TODO(iago) convert to list
            adv_funds_checker: None,
            known_blocks: BTreeMap::new(),
        }
    }

    fn request_advance_funds(&mut self, event1: RequestAdvanceFundsEvent) {
        // TODO(Jira) we should allow several RequestAdvanceFunds - https://rsklabs.atlassian.net/browse/UB-150
        self.request_event = Some(event1);
    }

    fn kickoff_advance_funds(&mut self, event2: KickoffAdvanceFundsEvent) -> () {
        if self.known_blocks.is_empty() {
            // this happens when a kickoff is received before any block
            // it should not happen in real life because RequestAdvanceFunds must be received many blocks before KickoffAdvanceFunds
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("No blocks received yet, cannot kickoff advance funds");
            return;
        }

        let Some(event1) = self.request_event.as_ref() else {
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

        if self.adv_funds_checker.is_some() {
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("More than one active advance funds is not expected on Union Bridge Design",);
            return;
        }

        let post_kickoff_blocks: Vec<&RskBlock> = self
            .known_blocks
            .values()
            .filter(|b| b.number() >= event2.block_number)
            .collect();

        info!("Init advance funds with {event2:?} and {post_kickoff_blocks:?}");
        let new_advance_funds = AdvanceFundsChecker::new(event2, post_kickoff_blocks);
        self.adv_funds_checker = Some(new_advance_funds);
    }

    fn deactivate_monitoring(&mut self) {
        // TODO(Jira) deactivate only if we don't have more RequestAdvanceFunds pending - https://rsklabs.atlassian.net/browse/UB-150
        self.request_event = None;
        self.known_blocks.clear();
    }

    fn close_advance_funds(&mut self, done: bool) -> () {
        if let Some(afc) = &self.adv_funds_checker {
            info!("Removing active {:?}", afc);
            self.adv_funds_checker = None;
        } else {
            info!("Trying to remove unexisting advance funds");
        }

        if done {
            self.deactivate_monitoring()
        }
    }

    fn run_check_fork(adv_funds_approver: &AdvanceFundsChecker) {
        let args = adv_funds_approver.check_fork_args();
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

impl EventProcessor for PegOutAdvanceFundsProcessor {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(data) => {
                info!("Handling {:?}, waiting blocks...", data);
                self.request_advance_funds(data.clone());
            }
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveRequestAdvanceFunds {peg_out_id}...");
                self.deactivate_monitoring();
            }
            RskPegManagerEvents::KickoffAdvanceFunds(data) => {
                info!("Handling {:?}...", data);
                self.kickoff_advance_funds(data.clone());
            }
            RskPegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveKickoffAdvanceFunds {peg_out_id}...");
                self.close_advance_funds(false);
            }
            _ => {
                info!("Ignoring {:?}...", event);
                return Ok(()); // ignore unrelated events
            }
        }
        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlock) -> Result<()> {
        let Some(event1) = self.request_event.as_ref() else {
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

        let Some(afc) = self.adv_funds_checker.as_mut() else {
            info!(
                "No active advance funds, ignoring block's {} pow",
                block.number()
            );
            return Ok(());
        };

        info!("Accumulating pow for advance funds {}", afc.pegout_id());

        if let Some(rb) = &removed_block {
            afc.update_with_block(rb, true);
        }

        afc.update_with_block(block, false);

        if afc.is_ready_for_check_fork() {
            info!("Triggering CheckFork for complete advance funds {:?}", afc);
            Self::run_check_fork(afc);

            info!("Completing advance funds {}", afc.pegout_id());
            self.close_advance_funds(true);
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        if self.adv_funds_checker.is_some() {
            warn!(
                "Active advance funds found on shutdown! {:?}",
                self.adv_funds_checker
            );
        }
        self.adv_funds_checker = None;
        self.request_event = None;
        self.known_blocks.clear();
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
        let processor = PegOutAdvanceFundsProcessor::new();
        assert!(processor.request_event.is_none());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_new_event_request_advance_funds_stores_event() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

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
        assert_eq!(processor.request_event, Some(event_with_block));
        assert!(processor.known_blocks.is_empty());
        assert!(processor.adv_funds_checker.is_none());
    }

    #[test]
    fn test_process_new_event_kickoff_advance_funds_creates_advance_funds() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(51));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
            KickoffAdvanceFundsEvent {
                inner: kickoff_event,
                block_number: kickoff_block.number(),
                block_hash: kickoff_block.hash(),
            },
        ));

        processor
            .process_new_block(&kickoff_block)
            .expect("Should have processed request");

        assert!(result.is_ok());
        assert!(processor.adv_funds_checker.is_some());

        let adv_funds = processor
            .adv_funds_checker
            .as_ref()
            .expect("AdvanceFundsPowChecker should exist");
        assert_eq!(adv_funds.pegout_id(), pegout_id);
        assert_eq!(adv_funds.check_fork_args().pegout_id, pegout_id);

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
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let kickoff_block = create_fake_block(110.into(), U256::from(51));
        let kickoff_event = create_fake_kickoff_event("peg123");

        let result = processor.process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
            KickoffAdvanceFundsEvent {
                inner: kickoff_event,
                block_number: kickoff_block.number(),
                block_hash: kickoff_block.parent_hash(),
            },
        ));
        assert!(result.is_ok());
        assert!(processor.request_event.is_none());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_kickoff_block_after_event_accumulates_effort_and_closes_advance_funds() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
                KickoffAdvanceFundsEvent {
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

        assert!(processor.adv_funds_checker.is_some());
        assert!(processor.request_event.is_some());

        // -2: the one created after the kickoff counts, and we will create the last one out of this loop
        for i in 1..=total_blocks - 2 {
            let block = create_fake_block(kickoff_block.number() + i as u64, block_effort);
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.adv_funds_checker.is_some());
        assert!(processor.request_event.is_some());

        // we process the missing block
        let block = create_fake_block(kickoff_block.number() + 5, block_effort);
        processor
            .process_new_block(&block)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_none());
    }

    #[test]
    fn test_process_kickoff_block_before_event_accumulates_effort_and_closes_advance_funds() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.adv_funds_checker.is_some());
        assert!(processor.request_event.is_some());

        // -2: the one created after the kickoff event counts, and we will create the last one out of this loop
        for i in 1..=total_blocks - 2 {
            let block = create_fake_block(kickoff_block.number() + i as u64, block_effort);
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.adv_funds_checker.is_some());
        assert!(processor.request_event.is_some());

        // we process the missing block
        let block = create_fake_block(kickoff_block.number() + 5, block_effort);
        processor
            .process_new_block(&block)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_none());
    }

    #[test]
    fn test_process_kickoff_event_without_blocks_early_exits() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.request_event.is_some());
        assert!(processor.known_blocks.is_empty());
        assert!(processor.adv_funds_checker.is_none());
    }

    #[test]
    fn test_process_old_block_early_exits() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let request_event = create_fake_request_event("peg123");
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
    fn test_shutdown_with_active_advance_funds_works() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(100));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        assert!(processor.request_event.is_some());
        assert!(processor.adv_funds_checker.is_some());
        assert_eq!(processor.known_blocks.len(), 2);

        processor.shutdown();

        assert!(processor.request_event.is_none());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_remove_request_advance_funds_event_removes_it() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_none());
    }

    #[test]
    fn test_remove_request_advance_funds_block_removes_it() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let adv_funds_block_2 = create_kickoff_block_2(&request_block);

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_some());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&adv_funds_block_2)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_some());
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_block() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let advance_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(50));
        let kickoff_block_2 = create_kickoff_block_2(&kickoff_block);

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: advance_block.number(),
                    block_hash: advance_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&kickoff_block)
            .expect("Should process block");

        assert!(processor.request_event.is_some());
        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&kickoff_block_2)
            .expect("Should process block");

        assert!(processor.request_event.is_some());
        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_event_stops_advance_funds() {
        let mut processor = PegOutAdvanceFundsProcessor::new();

        let request_block = create_fake_block(100.into(), U256::from(50));
        let kickoff_block = create_fake_block(110.into(), U256::from(100));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
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
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed kickoff");

        assert!(processor.adv_funds_checker.is_some());
        assert!(processor.request_event.is_some());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_event(&RskPegManagerEvents::RemoveKickoffAdvanceFunds {
                peg_out_id: pegout_id.to_string(),
            })
            .expect("Should have processed kickoff");

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_event.is_some());
        assert_eq!(processor.known_blocks.len(), 1);
    }
}
