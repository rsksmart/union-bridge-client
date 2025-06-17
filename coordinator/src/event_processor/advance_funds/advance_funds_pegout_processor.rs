use crate::{
    event_processor::{EventProcessor, advance_funds::advance_funds_checker::AdvanceFundsChecker},
    types::{KickoffAdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents},
};
use anyhow::Result;
use bincode::config::standard;
use bitvmx_client::types::IncomingBitVMXApiMessages;
use check_fork::{CheckForkArgs, check_fork};
use check_fork_zkp::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
use common::{
    msg_broker::{
        broker::{BROKER_SERVER_ID, BrokerClientApi},
        types::ToServer,
    },
    types::{BlockNumber, RskBlockAndUncles},
};
use log::{debug, error, info, warn};
use primitive_types::U256;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

pub struct PegOutAdvanceFundsProcessor<T: BrokerClientApi> {
    bitvmx_broker: T,
    first_block_to_process: Option<BlockNumber>,
    request_events: HashMap<String, RequestAdvanceFundsEvent>,
    adv_funds_checker: Option<AdvanceFundsChecker>,
    known_blocks: BTreeMap<BlockNumber, RskBlockAndUncles>,
}

impl<T: BrokerClientApi> PegOutAdvanceFundsProcessor<T> {
    pub fn new(bitvmx_broker: T) -> Self {
        Self {
            bitvmx_broker,
            first_block_to_process: None,
            request_events: HashMap::new(),
            adv_funds_checker: None,
            known_blocks: BTreeMap::new(),
        }
    }

    fn request_advance_funds(&mut self, event: RequestAdvanceFundsEvent) {
        let pegout_id = event.inner.peg_out_id.to_string();

        if self.request_events.is_empty() {
            self.first_block_to_process = Some(event.block_number.clone());
        }

        let updated = self.request_events.insert(pegout_id.to_string(), event);
        if updated.is_some() {
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!(
                "RequestAdvanceFunds for peg_out_id {} already exists",
                pegout_id
            );
            return;
        }
    }

    fn kickoff_advance_funds(&mut self, event2: KickoffAdvanceFundsEvent) {
        if self.known_blocks.is_empty() {
            // this happens when a kickoff is received before any block
            // it should not happen in real life because RequestAdvanceFunds must be received many blocks before KickoffAdvanceFunds
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("No blocks received yet, cannot kickoff advance funds");
            return;
        }

        if !self.request_events.contains_key(&event2.inner.peg_out_id) {
            error!(
                "KickoffAdvanceFundsData received for {}, but no RequestAdvanceFunds was",
                &event2.inner.peg_out_id
            );
            return;
        }

        if self.adv_funds_checker.is_some() {
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("More than one active advance funds is not expected on Union Bridge Design",);
            return;
        }

        let post_kickoff_blocks: Vec<&RskBlockAndUncles> = self
            .known_blocks
            .values()
            .filter(|b| b.number() >= event2.block_number)
            .collect();

        info!("Init advance funds with {event2:?} and {post_kickoff_blocks:?}");
        let new_advance_funds = AdvanceFundsChecker::new(event2, post_kickoff_blocks);
        self.adv_funds_checker = Some(new_advance_funds);
    }

    fn update_block_monitoring(&mut self, pegout_id: &String) {
        if self.request_events.remove(pegout_id).is_none() {
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("Removing non-existing RequestAdvanceFunds for pegout_id {pegout_id}");
            return;
        }

        // update first_block_to_process
        let first_block_on_events = self.request_events.values().map(|e| e.block_number).min();
        match first_block_on_events {
            Some(new_fb) => {
                self.first_block_to_process = Some(new_fb);
                self.known_blocks.retain(|_, b| b.number() >= new_fb);
            }
            None => {
                info!("No more RequestAdvanceFunds events, clearing block monitoring");
                self.first_block_to_process = None;
                self.known_blocks.clear();
            }
        }
    }

    fn close_advance_funds(&mut self, pegout_id: &String, done: bool) -> () {
        if let Some(afc) = &self.adv_funds_checker {
            info!("Removing active {:?}", afc);
            self.adv_funds_checker = None;
        } else {
            info!("Trying to remove unexisting advance funds");
        }

        if done {
            self.update_block_monitoring(pegout_id)
        }
    }

    fn schedule_check_fork_zkp(&mut self, args: CheckForkArgs) -> () {
        // note: check-fork already validates consecutive blocks, etc.
        match check_fork(&args) {
            Ok(effort) => {
                info!(
                    "CheckFork accepted with effort {effort} (pow {:#x}). The elf path is {:?}. The image id is {:?}",
                    Self::pow_from_effort(effort),
                    CHECK_FORK_GUEST_PATH,
                    CHECK_FORK_GUEST_ID,
                );

                // TODO when BitVMX API is ready we should also send the CHECK_FORK_GUEST_ELF

                let serialized_args = match Self::serialize_guest_input(&args) {
                    Ok(input) => input,
                    Err(e) => {
                        error!("Error serializing CheckForkArgs: {}", e);
                        return;
                    }
                };

                self.send_zkp_request(serialized_args);
            }
            Err(e) => {
                error!("CheckFork rejected: {}", e);
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) discuss with architects on error handling - https://rsklabs.atlassian.net/browse/UB-149
            }
        }
    }

    fn send_zkp_request(&mut self, serialized_args: Vec<u8>) {
        // TODO clarify with Fairgate what to do with this id, I guess it's for future correlation
        let request_id = Uuid::new_v4();
        let broker_result = self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            ToServer::ToBitVMX(IncomingBitVMXApiMessages::GenerateZKP(
                request_id,
                serialized_args,
            )),
        );

        match broker_result {
            Ok(true) => info!("Successfully sent GenerateCheckForkZKP"),
            Ok(false) => {
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
                error!("Could not send GenerateCheckForkZKP, broker returned false");
            }
            Err(e) => {
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
                error!("Error sending GenerateCheckForkZKP: {:?}", e)
            }
        }
    }

    pub fn serialize_guest_input<S: serde::Serialize>(data: &S) -> Result<Vec<u8>> {
        bincode::serde::encode_to_vec(data, standard()).map_err(|e| {
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            // TODO(Jira) discuss with architects on error handling - https://rsklabs.atlassian.net/browse/UB-149
            error!("Error serializing guest input: {}", e);
            e.into()
        })
    }

    fn pow_from_effort(effort: U256) -> U256 {
        let pow = U256::MAX.checked_div(effort).unwrap_or_else(|| {
            // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
            error!("CheckFork accepted with 0 effort",);
            U256::zero()
        });
        pow
    }
}

impl<T: BrokerClientApi> EventProcessor for PegOutAdvanceFundsProcessor<T> {
    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(data) => {
                info!("Handling {:?}, waiting blocks...", data);
                self.request_advance_funds(data.clone());
            }
            RskPegManagerEvents::RemoveRequestAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveRequestAdvanceFunds {peg_out_id}...");
                self.update_block_monitoring(peg_out_id);
            }
            RskPegManagerEvents::KickoffAdvanceFunds(data) => {
                info!("Handling {:?}...", data);
                self.kickoff_advance_funds(data.clone());
            }
            RskPegManagerEvents::RemoveKickoffAdvanceFunds { peg_out_id } => {
                info!("Handling RemoveKickoffAdvanceFunds {peg_out_id}...");
                self.close_advance_funds(peg_out_id, false);
            }
            _ => {
                info!("Ignoring {:?}...", event);
                return Ok(()); // ignore unrelated events
            }
        }
        Ok(())
    }

    fn process_new_block(&mut self, block_with_uncles: &RskBlockAndUncles) -> Result<()> {
        let block = &block_with_uncles.block();

        if let Some(first_block) = self.first_block_to_process {
            if block.number() < first_block {
                warn!(
                    "Ignoring block {}, older than first RequestAdvanceFunds at {}",
                    block.number(),
                    first_block
                );
                return Ok(());
            }
        } else {
            debug!(
                "Ignoring block {}, no RequestAdvanceFunds events received yet",
                block.number()
            );
            return Ok(());
        }

        let removed_block = self
            .known_blocks
            .insert(block.number(), block_with_uncles.clone());

        let Some(afc) = self.adv_funds_checker.as_mut() else {
            debug!(
                "No active advance funds, ignoring block's {} pow",
                block.number()
            );
            return Ok(());
        };

        if let Some(rb) = &removed_block {
            afc.update_with_block(rb, true);
        }

        afc.update_with_block(block_with_uncles, false);

        if afc.is_ready_for_check_fork() {
            info!("Triggering CheckFork for complete advance funds {:?}", afc);

            let args = afc.check_fork_args();
            let pegout_id = afc.pegout_id();

            self.schedule_check_fork_zkp(args);

            info!("Completing advance funds {}", pegout_id);
            self.close_advance_funds(&pegout_id, true);
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
        self.request_events.clear();
        self.known_blocks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventWithBlock;
    use actors_mocking::fake_contracts::FakePegManager::{
        KickoffAdvanceFunds, RequestAdvanceFunds,
    };
    use alloy_primitives::U256 as AlloyU256;
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::test_utils::rsk_block_generator::create_block_from_template;
    use common::types::{BlockDifficulty, BlockHash, BlockPow, BlockTimestamp, RskBlock};
    use mockall::predicate::{eq, function};
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
        let processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_new_event_request_advance_funds_keeps_events() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = EventWithBlock {
            inner: create_fake_request_event(pegout_id),
            block_number: request_block.number(),
            block_hash: BlockHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(request_event))
            .expect("Should have processed request");

        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );

        let pegout_id_2 = "peg456";
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(EventWithBlock {
                inner: create_fake_request_event(pegout_id_2),
                block_number: request_block.number() + 1,
                block_hash: BlockHash::from(H256::from_low_u64_be(456)),
            }))
            .expect("Should have processed request");

        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id_2.to_string())
        );

        assert!(processor.known_blocks.is_empty());
        assert!(processor.adv_funds_checker.is_none());
    }

    #[test]
    fn test_process_new_event_kickoff_advance_funds_creates_advance_funds_when_one_request_exists()
    {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let kickoff_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(51)));

        let pegout_id = "peg123";

        let request_event = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id),
            block_number: request_block.number(),
            block_hash: request_block.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event.clone(),
            ))
            .expect("Should have processed request");

        let any_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(102.into(), U256::from(105)));
        processor
            .process_new_block(&request_block)
            .expect("Should have processed request");
        processor
            .process_new_block(&any_block)
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
            .expect("Should have processed request");

        processor
            .process_new_block(&kickoff_block.clone())
            .expect("Should have processed request");

        assert_eq!(processor.request_events.len(), 1);
        assert!(processor.request_events.contains_key(pegout_id));
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
    fn test_process_new_event_kickoff_advance_funds_creates_advance_funds_when_two_requests_exist()
    {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block_1 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let request_block_2 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(105.into(), U256::from(52)));
        let kickoff_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(51)));

        let pegout_id_1 = "peg123";
        let pegout_id_2 = "peg456";

        let request_event_1 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_1),
            block_number: request_block_1.number(),
            block_hash: request_block_1.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event_1.clone(),
            ))
            .expect("Should have processed request");
        processor
            .process_new_block(&request_block_1)
            .expect("Should have processed request");

        let request_event_2 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_2),
            block_number: request_block_2.number(),
            block_hash: request_block_2.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event_2.clone(),
            ))
            .expect("Should have processed request");
        processor
            .process_new_block(&request_block_2)
            .expect("Should have processed request");

        let kickoff_event = create_fake_kickoff_event(pegout_id_1);
        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.hash(),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&kickoff_block)
            .expect("Should have processed request");

        assert_eq!(processor.request_events.len(), 2);
        assert!(processor.request_events.contains_key(pegout_id_1),);
        assert!(processor.request_events.contains_key(pegout_id_2));
        assert!(processor.adv_funds_checker.is_some());

        let adv_funds = processor
            .adv_funds_checker
            .as_ref()
            .expect("AdvanceFundsPowChecker should exist");
        assert_eq!(adv_funds.pegout_id(), pegout_id_1);
        assert_eq!(adv_funds.check_fork_args().pegout_id, pegout_id_1);

        assert_eq!(processor.known_blocks.len(), 3);
        assert_eq!(
            processor
                .known_blocks
                .get(&request_block_1.number())
                .expect("Should exist"),
            &request_block_1
        );
        assert_eq!(
            processor
                .known_blocks
                .get(&request_block_2.number())
                .expect("Should exist"),
            &request_block_2
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
    fn test_process_new_event_kickoff_advance_funds_exits_when_no_requests() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let kickoff_block = create_fake_block(110.into(), U256::from(51));
        let kickoff_event = create_fake_kickoff_event("peg123");

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.parent_hash(),
                },
            ))
            .expect("Should have processed request");

        assert!(processor.first_block_to_process.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_new_event_kickoff_advance_funds_exits_when_no_matching_request() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        let pegout_id_req = "peg123";
        let request_event = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_req),
            block_number: request_block.number(),
            block_hash: request_block.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event.clone(),
            ))
            .expect("Should have processed request");
        processor
            .process_new_block(&request_block)
            .expect("Should have processed request");

        let pegout_id_kick = "peg456";
        let kickoff_block = create_fake_block(110.into(), U256::from(51));
        let kickoff_event = create_fake_kickoff_event(pegout_id_kick);

        processor
            .process_new_event(&RskPegManagerEvents::KickoffAdvanceFunds(
                KickoffAdvanceFundsEvent {
                    inner: kickoff_event,
                    block_number: kickoff_block.number(),
                    block_hash: kickoff_block.parent_hash(),
                },
            ))
            .expect("Should have processed request");

        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(processor.request_events.contains_key(pegout_id_req));
        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.known_blocks.len(), 1);
        assert_eq!(processor.known_blocks.values().next(), Some(&request_block));
    }

    #[test]
    fn test_process_kickoff_block_after_event_accumulates_effort_and_closes_advance_funds() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_zkp_bitvmx(&mut bitvmx_broker);

        let mut processor = PegOutAdvanceFundsProcessor::new(bitvmx_broker);

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

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

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_events.contains_key(pegout_id));
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(processor.known_blocks.contains_key(&request_block.number()));

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        // to need one more block than required ones to achieve the pow
        let total_blocks = kickoff_event.required_num_blocks + 1;

        let block_effort = kickoff_event
            .required_effort
            .checked_div(AlloyU256::from(total_blocks + 1)) // +1 because the uncle we will create also counts for the pow
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let kickoff_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

        // we process the kickoff block after the kickoff event
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

        assert!(processor.adv_funds_checker.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.known_blocks.is_empty());

        let kickoff_sibling = create_block_from_template(
            &kickoff_block.block(),
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            kickoff_block.parent(),
            vec![],
        );

        let block_with_uncle = RskBlockAndUncles::new(
            create_fake_block(kickoff_block.number() + 1, block_effort),
            vec![kickoff_sibling],
        )
        .unwrap();
        processor
            .process_new_block(&block_with_uncle)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );

        // -1: the one created after the kickoff event counts, and the one we created before this loop
        for i in 2..=total_blocks {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                kickoff_block.number() + i as u64,
                block_effort,
            ));
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.known_blocks.is_empty());
    }

    fn expect_zkp_bitvmx(bitvmx_broker: &mut MockBrokerClientApi) {
        bitvmx_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &ToServer| {
                    matches!(
                        req,
                        ToServer::ToBitVMX(IncomingBitVMXApiMessages::GenerateZKP(_, _))
                    )
                }),
            )
            .return_once(|_, _| Ok(true));
    }

    #[test]
    fn test_process_kickoff_block_before_event_accumulates_effort_and_closes_advance_funds() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_zkp_bitvmx(&mut bitvmx_broker);

        let mut processor = PegOutAdvanceFundsProcessor::new(bitvmx_broker);

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

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

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_events.contains_key(pegout_id));
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(processor.known_blocks.contains_key(&request_block.number()));

        let kickoff_event = create_fake_kickoff_event(pegout_id);

        // to need one more block than required ones to achieve the pow
        let total_blocks = kickoff_event.required_num_blocks + 1;

        let block_effort = kickoff_event
            .required_effort
            .checked_div(AlloyU256::from(total_blocks + 1)) // +1 because the uncle we will create also counts for the pow
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let kickoff_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

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
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.known_blocks.is_empty());

        let kickoff_sibling = create_block_from_template(
            &kickoff_block.block(),
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            kickoff_block.parent(),
            vec![],
        );

        let block_with_uncle = RskBlockAndUncles::new(
            create_fake_block(kickoff_block.number() + 1, block_effort),
            vec![kickoff_sibling],
        )
        .unwrap();
        processor
            .process_new_block(&block_with_uncle)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.known_blocks.is_empty());

        // -1: the one created after the kickoff event counts, and the one we created before this loop
        for i in 2..=total_blocks {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                kickoff_block.number() + i as u64,
                block_effort,
            ));
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_process_kickoff_event_without_blocks_early_exits() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

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

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.known_blocks.is_empty());
        assert!(processor.adv_funds_checker.is_none());
    }

    #[test]
    fn test_process_old_block_early_exits() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

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

        let old_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(99.into(), U256::from(100)));
        let result = processor.process_new_block(&old_block);

        assert!(result.is_ok());
    }

    #[test]
    fn test_shutdown_with_active_advance_funds_works() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let kickoff_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(100)));

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

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.adv_funds_checker.is_some());
        assert_eq!(processor.known_blocks.len(), 2);

        processor.shutdown();

        assert!(processor.request_events.is_empty());
        assert!(processor.adv_funds_checker.is_none());
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_remove_request_advance_funds_event_removes_it() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block_1 = create_fake_block(100.into(), U256::from(50));
        let request_block_2 = create_fake_block(101.into(), U256::from(51));

        let pegout_id_1 = "peg123";
        let pegout_id_2 = "peg456";

        let request_event_1 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_1),
            block_number: request_block_1.number(),
            block_hash: request_block_1.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(request_event_1))
            .expect("Should have processed request");

        let request_event_2 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_2),
            block_number: request_block_2.number(),
            block_hash: request_block_2.hash(),
        };
        processor
            .process_new_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event_2.clone(),
            ))
            .expect("Should have processed request");

        assert_eq!(processor.request_events.len(), 2);
        assert!(processor.request_events.contains_key(pegout_id_1));
        assert!(processor.request_events.contains_key(pegout_id_2));
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block_1.number())
        );

        processor
            .process_new_event(&RskPegManagerEvents::RemoveRequestAdvanceFunds {
                peg_out_id: pegout_id_1.to_string(),
            })
            .expect("Should have processed request");

        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.request_events.len(), 1);
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block_2.number())
        );
        assert!(processor.request_events.contains_key(pegout_id_2));
        assert!(processor.known_blocks.is_empty());
    }

    #[test]
    fn test_remove_request_advance_funds_block_removes_it() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let adv_funds_block_2 =
            RskBlockAndUncles::new_no_uncles(create_kickoff_block_2(&request_block.block()));

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
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&adv_funds_block_2)
            .expect("Should process block");

        assert!(processor.adv_funds_checker.is_none());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_block_removes_it() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let advance_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let kickoff_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(50)));
        let kickoff_block_2 =
            RskBlockAndUncles::new_no_uncles(create_kickoff_block_2(&kickoff_block.block()));

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

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_block(&kickoff_block_2)
            .expect("Should process block");

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.adv_funds_checker.is_none());
        assert_eq!(processor.known_blocks.len(), 1);
    }

    #[test]
    fn test_remove_kickoff_advance_funds_event_stops_advance_funds() {
        let mut processor = PegOutAdvanceFundsProcessor::new(MockBrokerClientApi::new());

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let kickoff_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(100)));

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
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.known_blocks.len(), 1);

        processor
            .process_new_event(&RskPegManagerEvents::RemoveKickoffAdvanceFunds {
                peg_out_id: pegout_id.to_string(),
            })
            .expect("Should have processed kickoff");

        assert!(processor.adv_funds_checker.is_none());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.known_blocks.len(), 1);
    }
}
