use crate::blockchain_tracker::{BlockchainObserver, BlockchainView};
use crate::flows::advance_funds::check_fork_accumulator::CheckForkAccumulator;
use crate::{
    event_processor::EventProcessor,
    types::{AdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents},
};
use anyhow::Result;
use bincode::config::standard;
use check_fork::{CheckForkArgs, check_fork};
use check_fork_zkp::{CHECK_FORK_GUEST_ID, CHECK_FORK_GUEST_PATH};
use common::msg_broker::bitvmx_types::IncomingBitVMXApiMessages;
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::{
    msg_broker::broker::BROKER_SERVER_ID,
    types::{BlockNumber, RskBlockAndUncles},
};
use log::{debug, error, info, warn};
use primitive_types::U256;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

pub struct AdvanceFundsProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    rt_sync: RuntimeSync,
    contracts: Rc<CG>,
    bitvmx_broker: Rc<BC>,
    first_block_to_process: Option<BlockNumber>,
    request_events: HashMap<String, RequestAdvanceFundsEvent>,
    check_fork_accumulator: Option<Rc<RefCell<CheckForkAccumulator>>>,
    chain_view: BlockchainView,
}

impl<CG, BC> AdvanceFundsProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    pub fn new(rt_sync: RuntimeSync, contracts: Rc<CG>, bitvmx_broker: Rc<BC>) -> Self {
        Self {
            rt_sync,
            contracts,
            bitvmx_broker,
            first_block_to_process: None,
            request_events: HashMap::new(),
            check_fork_accumulator: None,
            chain_view: BlockchainView::new(),
        }
    }

    #[cfg(test)]
    pub fn new_for_test(contracts: Rc<CG>, bitvmx_broker: Rc<BC>) -> Self {
        Self {
            rt_sync: RuntimeSync::new().unwrap(),
            contracts,
            bitvmx_broker,
            first_block_to_process: None,
            request_events: HashMap::new(),
            check_fork_accumulator: None,
            chain_view: BlockchainView::new(),
        }
    }

    fn start_monitoring_blocks_for_pegout(&mut self, event: RequestAdvanceFundsEvent) {
        let pegout_id = event.inner.pegout_id.to_string();

        if self.request_events.is_empty() {
            self.first_block_to_process = Some(event.block_number.clone());
        }

        let updated = self.request_events.insert(pegout_id.to_string(), event);
        if updated.is_some() {
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!(
                "RequestAdvanceFunds for pegout_id {} already exists",
                pegout_id
            );
            return;
        }
    }

    fn start_pow_accum_for_pegout(&mut self, event2: AdvanceFundsEvent) {
        match self.check_fork_accumulator.as_ref() {
            Some(afc) if &afc.borrow().pegout_id() == &event2.inner.pegout_id => {
                warn!("Already monitoring advance funds for {event2:?}");
                return;
            }
            Some(afc) => {
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                error!("A second advance funds was not expected. Closing {:?}", afc);
                let pegout_id = afc.borrow().pegout_id();
                self.close_pegout(&pegout_id);
                return;
            }
            None => {}
        }

        if self.chain_view.get_tip().is_none() {
            // this happens when a AdvanceFunds is received before any block
            // it should not happen in real life because RequestAdvanceFunds must be received many blocks before AdvanceFunds
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("No blocks received yet, cannot start advance funds");
            return;
        }

        if !self.request_events.contains_key(&event2.inner.pegout_id) {
            error!(
                "AdvanceFundsData received for {}, but no RequestAdvanceFunds was",
                &event2.inner.pegout_id
            );
            return;
        }

        let post_advance_funds_blocks: Vec<&RskBlockAndUncles> =
            self.chain_view.get_from(event2.block_number);

        info!("Init advance funds with {event2:?} and {post_advance_funds_blocks:?}");
        let new_advance_funds = CheckForkAccumulator::new(event2, post_advance_funds_blocks);
        let advance_funds_rc = Rc::new(RefCell::new(new_advance_funds));
        self.chain_view.add_observer(advance_funds_rc.clone());
        self.check_fork_accumulator = Some(advance_funds_rc);
    }

    fn stop_monitoring_blocks_for_pegout(&mut self, pegout_id: &String) {
        if self.request_events.remove(pegout_id).is_none() {
            // TODO(Jira) this should be monitored and analysed - https://rsklabs.atlassian.net/browse/UB-127
            error!("Removing non-existing RequestAdvanceFunds for pegout_id {pegout_id}");
            return;
        }

        // update first_block_to_process and restart chain_view to the next RequestAdvanceFunds event block
        let next_request_event_block = self.request_events.values().map(|e| e.block_number).min();
        match next_request_event_block {
            Some(new_fb) => {
                self.first_block_to_process = Some(new_fb);
                self.chain_view.restart_from(new_fb);
            }
            None => {
                info!("No more RequestAdvanceFunds events, clearing block monitoring");
                self.first_block_to_process = None;
                self.chain_view.clear();
            }
        }
    }

    fn stop_pow_accum_for_pegout(&mut self, pegout_id: &String) {
        if let Some(afc) = &self.check_fork_accumulator {
            if &afc.borrow().pegout_id() == pegout_id {
                info!("Removing active {:?}", afc);
                self.chain_view
                    .remove_observer(afc.borrow().get_id().as_str());
                self.check_fork_accumulator = None;
            } else {
                error!(
                    "Trying to remove advance funds for pegout_id {}, but active one is {}. This is not expected on Union Bridge Design",
                    pegout_id,
                    afc.borrow().pegout_id()
                );
            }
        } else {
            info!("Trying to remove unexisting advance funds");
        }
    }

    fn close_pegout(&mut self, pegout_id: &String) {
        self.stop_pow_accum_for_pegout(pegout_id);
        self.stop_monitoring_blocks_for_pegout(pegout_id)
    }

    fn schedule_check_fork_zkp(&mut self, args: CheckForkArgs) {
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
            IncomingBitVMXApiMessages::GenerateZKP(request_id, serialized_args),
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

    fn notify_contracts_advance_funds_complete(&self, pegout_id: &str) {
        debug!(
            "Notifying contracts that advance funds for {} are complete",
            pegout_id
        );

        let result = self
            .rt_sync
            .run(async { self.contracts.notify_check_fork_completion(pegout_id).await });
        match result {
            Ok(_) => {
                info!(
                    "Successfully notified contracts about advance funds completion for {}",
                    pegout_id
                );
            }
            Err(e) => {
                // TODO(Jira) this should be monitored - https://rsklabs.atlassian.net/browse/UB-127
                error!(
                    "Error notifying contracts about advance funds completion for {}: {}",
                    pegout_id, e
                );
            }
        }
    }

    fn serialize_guest_input<S: serde::Serialize>(data: &S) -> Result<Vec<u8>> {
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

impl<CG, BC> EventProcessor for AdvanceFundsProcessor<CG, BC>
where
    CG: RskContractsGatewayApi,
    BC: BitVmxBrokerClientApi,
{
    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RequestAdvanceFunds(data) => {
                if !data.removed {
                    info!("Handling {:?}, waiting blocks...", data);
                    self.start_monitoring_blocks_for_pegout(data.clone());
                } else {
                    info!(
                        "Handling RemoveRequestAdvanceFunds {}...",
                        data.inner.pegout_id
                    );
                    self.stop_monitoring_blocks_for_pegout(&data.inner.pegout_id);
                }
            }
            RskPegManagerEvents::AdvanceFunds(data) => {
                if !data.removed {
                    info!("Handling {:?}...", data);
                    self.start_pow_accum_for_pegout(data.clone());
                } else {
                    info!("Handling RemoveAdvanceFunds {}...", data.inner.pegout_id);
                    self.stop_pow_accum_for_pegout(&data.inner.pegout_id);
                }
            }
            _ => {
                info!("Ignoring {:?}...", event);
                return Ok(()); // ignore unrelated events
            }
        }
        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
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

        self.chain_view.update(block.clone());

        let Some(afc) = self.check_fork_accumulator.as_mut() else {
            debug!(
                "No active advance funds, ignoring block's {} pow",
                block.number()
            );
            return Ok(());
        };

        if afc.borrow().has_enough_confirmations() {
            info!("Triggering CheckFork for complete advance funds {:?}", afc);

            let args = afc.borrow().check_fork_args();
            let pegout_id = afc.borrow().pegout_id();

            // TODO(Jira) if this fails, we should not close the pegout and retry it later - https://rsklabs.atlassian.net/browse/UB-132
            self.schedule_check_fork_zkp(args);

            self.notify_contracts_advance_funds_complete(&pegout_id);

            info!("Completing advance funds {}", pegout_id);
            self.stop_monitoring_blocks_for_pegout(&pegout_id);
            self.stop_pow_accum_for_pegout(&pegout_id);
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        if self.check_fork_accumulator.is_some() {
            warn!(
                "Active advance funds found on shutdown! {:?}",
                self.check_fork_accumulator
            );
        }
        self.check_fork_accumulator = None;
        self.request_events.clear();
        self.chain_view.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::REQUIRED_CONFIRMATIONS;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use crate::flows::advance_funds::tests::create_fake_block;
    use crate::types::EventWithBlock;
    use actors_mocking::fake_contracts::FakePegManager::{AdvanceFunds, RequestAdvanceFunds};
    use alloy_primitives::U256 as AlloyU256;
    use common::msg_broker::bitvmx_types::OutgoingBitVMXApiMessages;
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::test_utils::rsk_block_generator::create_block_from_template;
    use common::types::TxHash;
    use common::types::{BlockHash, RskBlock};
    use mockall::predicate::{eq, function};
    use primitive_types::{H256, U256};

    fn create_fake_request_event(pegout_id: &str) -> RequestAdvanceFunds {
        RequestAdvanceFunds {
            pegout_id: pegout_id.to_string(),
            amount: 1000,
        }
    }

    fn create_advance_funds_block(original_block: &RskBlock) -> RskBlock {
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

    fn create_fake_advance_funds_event(pegout_id: &str) -> AdvanceFunds {
        AdvanceFunds {
            pegout_id: pegout_id.to_string(),
            utxo_id: "utxo123".to_string(),
            operator_id: "op123".to_string(),
            required_effort: AlloyU256::from(1000),
            required_num_blocks: 4,
        }
    }

    #[test]
    fn test_new_processor_initial_state_is_clear() {
        let processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_process_new_event_request_advance_funds_keeps_events() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block = create_fake_block(100.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = EventWithBlock {
            inner: create_fake_request_event(pegout_id),
            block_number: request_block.number(),
            block_hash: BlockHash::from(H256::from_low_u64_be(123)),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(request_event))
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
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(EventWithBlock {
                inner: create_fake_request_event(pegout_id_2),
                block_number: request_block.number() + 1,
                block_hash: BlockHash::from(H256::from_low_u64_be(456)),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(456)),
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

        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
        assert!(processor.check_fork_accumulator.is_none());
    }

    #[test]
    fn test_process_new_event_advance_funds_creates_checker_when_one_request_exists() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let any_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            U256::from(105),
        ));
        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            any_block.number() + 1,
            U256::from(51),
        ));

        let pegout_id = "peg123";

        let request_event = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id),
            block_number: request_block.number(),
            block_hash: request_block.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event.clone(),
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should have processed request");
        processor
            .process_new_block(&any_block)
            .expect("Should have processed request");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(123)),
            }))
            .expect("Should have processed request");

        processor
            .process_new_block(&advance_funds_block.clone())
            .expect("Should have processed request");

        assert_eq!(processor.request_events.len(), 1);
        assert!(processor.request_events.contains_key(pegout_id));
        assert!(processor.check_fork_accumulator.is_some());

        let advance_funds = processor
            .check_fork_accumulator
            .as_ref()
            .expect("AdvanceFundsPowChecker should exist");
        assert_eq!(advance_funds.borrow().pegout_id(), pegout_id);
        assert_eq!(
            advance_funds.borrow().check_fork_args().pegout_id,
            pegout_id
        );

        assert_eq!(processor.chain_view.len(), 3);
        assert_eq!(
            processor
                .chain_view
                .get_at(&request_block.number())
                .expect("Should exist"),
            &request_block
        );
        assert_eq!(
            processor
                .chain_view
                .get_at(&any_block.number())
                .expect("Should exist"),
            &any_block
        );
        assert_eq!(
            processor
                .chain_view
                .get_at(&advance_funds_block.number())
                .expect("Should exist"),
            &advance_funds_block
        );
    }

    #[test]
    fn test_process_new_event_advance_funds_advance_funds_creates_advance_funds_when_two_requests_exist()
     {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let request_block_1 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let request_block_2 = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block_1.number() + 1,
            U256::from(52),
        ));
        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block_2.number() + 1,
            U256::from(51),
        ));

        let pegout_id_1 = "peg123";
        let pegout_id_2 = "peg456";

        let request_event_1 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_1),
            block_number: request_block_1.number(),
            block_hash: request_block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
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
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(456)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event_2.clone(),
            ))
            .expect("Should have processed request");
        processor
            .process_new_block(&request_block_2)
            .expect("Should have processed request");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id_1);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(789)),
            }))
            .expect("Should have processed request");

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should have processed request");

        assert_eq!(processor.request_events.len(), 2);
        assert!(processor.request_events.contains_key(pegout_id_1),);
        assert!(processor.request_events.contains_key(pegout_id_2));
        assert!(processor.check_fork_accumulator.is_some());

        let advance_funds = processor
            .check_fork_accumulator
            .as_ref()
            .expect("AdvanceFundsPowChecker should exist");
        assert_eq!(advance_funds.borrow().pegout_id(), pegout_id_1);
        assert_eq!(
            advance_funds.borrow().check_fork_args().pegout_id,
            pegout_id_1
        );

        assert_eq!(processor.chain_view.len(), 3);
        assert_eq!(
            processor
                .chain_view
                .get_at(&request_block_1.number())
                .expect("Should exist"),
            &request_block_1
        );
        assert_eq!(
            processor
                .chain_view
                .get_at(&request_block_2.number())
                .expect("Should exist"),
            &request_block_2
        );
        assert_eq!(
            processor
                .chain_view
                .get_at(&advance_funds_block.number())
                .expect("Should exist"),
            &advance_funds_block
        );
    }

    #[test]
    fn test_process_new_event_advance_funds_advance_funds_exits_when_no_requests() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let advance_funds_block = create_fake_block(110.into(), U256::from(51));
        let advance_funds_event = create_fake_advance_funds_event("peg123");

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.parent_hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(1234)),
            }))
            .expect("Should have processed request");

        assert!(processor.first_block_to_process.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_process_new_event_advance_funds_accumulator_exits_when_no_matching_request() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        let pegout_id_req = "peg123";
        let request_event = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_req),
            block_number: request_block.number(),
            block_hash: request_block.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                request_event.clone(),
            ))
            .expect("Should have processed request");
        processor
            .process_new_block(&request_block)
            .expect("Should have processed request");

        let pegout_id_kick = "peg456";
        let advance_funds_block = create_fake_block(110.into(), U256::from(51));
        let advance_funds_event = create_fake_advance_funds_event(pegout_id_kick);

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.parent_hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(456)),
            }))
            .expect("Should have processed request");

        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert!(processor.request_events.contains_key(pegout_id_req));
        assert!(processor.check_fork_accumulator.is_none());
        assert_eq!(processor.chain_view.len(), 1);
        assert_eq!(
            processor.chain_view.get_at(&request_block.number()),
            Some(&request_block)
        );
    }

    #[test]
    fn test_process_advance_funds_block_after_event_accumulates_effort_and_closes_advance_funds() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_zkp_bitvmx(&mut bitvmx_broker);

        let pegout_id = "peg123";
        let mut rsk_gateway = MockRskContractsGatewayApi::new();
        expect_notify_check_fork(&mut rsk_gateway, pegout_id);

        let mut processor =
            AdvanceFundsProcessor::new_for_test(Rc::new(rsk_gateway), Rc::new(bitvmx_broker));

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(100)),
                },
            ))
            .expect("Should have processed request");

        // we process the advance funds block
        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.request_events.contains_key(pegout_id));
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert_eq!(
            processor.chain_view.get_at(&request_block.number()),
            Some(&request_block)
        );

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);

        let required_blocks = advance_funds_event.required_num_blocks;
        let required_blocks_plus_confirmations = required_blocks + REQUIRED_CONFIRMATIONS;
        // -1 because the uncle we will create also counts for the pow
        // -1 to require one more block than required ones to achieve the pow
        let blocks_to_achieve_pow = required_blocks - 2;

        let block_effort = advance_funds_event
            .required_effort
            .checked_div(AlloyU256::from(blocks_to_achieve_pow))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

        // we process the kickoff block after the kickoff event
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(123)),
            }))
            .expect("Should have processed kickoff");
        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.chain_view.is_empty());
        assert!(processor.chain_view.is_observed());
        assert!(processor.chain_view.has_observer(pegout_id));

        let advance_funds_sibling = create_block_from_template(
            &advance_funds_block.block(),
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            advance_funds_block.parent(),
            vec![],
        );

        let block_with_uncle = RskBlockAndUncles::new(
            create_fake_block(advance_funds_block.number() + 1, block_effort),
            vec![advance_funds_sibling],
        );
        processor
            .process_new_block(&block_with_uncle)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );

        // starting in 2 because we already have: the one created after the kickoff event and the one before this loop
        // we stop at -2: range limit exclusive and leaving one confirmation pending
        for i in 2..=required_blocks_plus_confirmations - 2 {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                advance_funds_block.number() + i as u64,
                block_effort,
            ));
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        // confirmations -1, not ready

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.chain_view.is_empty());
        assert!(processor.chain_view.is_observed());
        assert!(processor.chain_view.has_observer(pegout_id));

        let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            advance_funds_block.number() + required_blocks_plus_confirmations as u64 - 1,
            block_effort,
        ));
        processor
            .process_new_block(&block)
            .expect("Should process block");

        // now we have enough confirmations

        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_process_advance_funds_block_before_event_accumulates_effort_and_closes_advance_funds() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_zkp_bitvmx(&mut bitvmx_broker);

        let pegout_id = "peg123";
        let mut rsk_gateway = MockRskContractsGatewayApi::new();
        expect_notify_check_fork(&mut rsk_gateway, pegout_id);

        let mut processor =
            AdvanceFundsProcessor::new_for_test(Rc::new(rsk_gateway), Rc::new(bitvmx_broker));

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(100)),
                },
            ))
            .expect("Should have processed request");

        // we process the advance funds block
        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.request_events.contains_key(pegout_id));
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block.number())
        );
        assert_eq!(
            processor.chain_view.get_at(&request_block.number()),
            Some(&request_block)
        );

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);

        let required_blocks = advance_funds_event.required_num_blocks;
        let required_blocks_plus_confirmations = required_blocks + REQUIRED_CONFIRMATIONS;
        // -1 because the uncle we will create also counts for the pow
        // -1 to require one more block than required ones to achieve the pow
        let blocks_to_achieve_pow = required_blocks - 2;

        let block_effort = advance_funds_event
            .required_effort
            .checked_div(AlloyU256::from(blocks_to_achieve_pow))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

        // we process the kickoff block before the kickoff event
        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process block");
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(256)),
            }))
            .expect("Should have processed kickoff");

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.chain_view.is_empty());
        assert!(processor.chain_view.is_observed());
        assert!(processor.chain_view.has_observer(pegout_id));

        let advance_funds_sibling = create_block_from_template(
            &advance_funds_block.block(),
            "0xa7b3f84f619c302a11892a379ac5a3a0bfbf8a3dce946a3db31cfb4c2f5cd909",
            advance_funds_block.parent(),
            vec![],
        );

        let block_with_uncle = RskBlockAndUncles::new(
            create_fake_block(advance_funds_block.number() + 1, block_effort),
            vec![advance_funds_sibling],
        );
        processor
            .process_new_block(&block_with_uncle)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.chain_view.is_empty());
        assert!(processor.chain_view.is_observed());
        assert!(processor.chain_view.has_observer(pegout_id));

        // starting in 2 because we already have: the one created before the kickoff event and the one before this loop
        // we stop at -2: range limit exclusive and leaving one confirmation pending
        for i in 2..=required_blocks_plus_confirmations - 2 {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                advance_funds_block.number() + i as u64,
                block_effort,
            ));
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        // confirmations -1, not ready

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.first_block_to_process.is_some());
        assert!(!processor.chain_view.is_empty());
        assert!(processor.chain_view.is_observed());
        assert!(processor.chain_view.has_observer(pegout_id));

        let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            advance_funds_block.number() + required_blocks_plus_confirmations as u64 - 1,
            block_effort,
        ));
        processor
            .process_new_block(&block)
            .expect("Should process block");

        // now we have enough confirmations

        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_process_advance_funds_event_without_blocks_early_exits() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block = create_fake_block(100.into(), U256::from(50));
        let advance_funds_block = create_fake_block(110.into(), U256::from(50));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(123)),
                },
            ))
            .expect("Should have processed request");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(456)),
            }))
            .expect("Should have processed kickoff");

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
        assert!(processor.check_fork_accumulator.is_none());
    }

    #[test]
    fn test_process_old_block_early_exits() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block = create_fake_block(100.into(), U256::from(50));

        let request_event = create_fake_request_event("peg123");
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(100)),
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
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            U256::from(100),
        ));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(110)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(789)),
            }))
            .expect("Should have processed kickoff");

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process block");

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.check_fork_accumulator.is_some());
        assert_eq!(processor.chain_view.len(), 2);

        processor.shutdown();

        assert!(processor.request_events.is_empty());
        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_remove_request_advance_funds_event_removes_it() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block_1 = create_fake_block(100.into(), U256::from(50));
        let request_block_2 = create_fake_block(101.into(), U256::from(51));

        let pegout_id_1 = "peg123";
        let pegout_id_2 = "peg456";

        let request_event_1 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_1),
            block_number: request_block_1.number(),
            block_hash: request_block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(123)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(request_event_1))
            .expect("Should have processed request");

        let request_event_2 = RequestAdvanceFundsEvent {
            inner: create_fake_request_event(pegout_id_2),
            block_number: request_block_2.number(),
            block_hash: request_block_2.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(456)),
        };
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
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
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: create_fake_request_event(pegout_id_1),
                    block_number: request_block_1.number(),
                    block_hash: request_block_1.hash(),
                    removed: true,
                    tx_hash: TxHash::from(H256::from_low_u64_be(789)),
                },
            ))
            .expect("Should have processed request");

        assert!(processor.check_fork_accumulator.is_none());
        assert_eq!(processor.request_events.len(), 1);
        assert_eq!(
            processor.first_block_to_process,
            Some(request_block_2.number())
        );
        assert!(processor.request_events.contains_key(pegout_id_2));
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    #[test]
    fn test_remove_request_advance_funds_block_removes_it() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );

        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let advance_funds_block =
            RskBlockAndUncles::new_no_uncles(create_advance_funds_block(&request_block.block()));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(123)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_none());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.chain_view.len(), 1);

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process block");

        assert!(processor.check_fork_accumulator.is_none());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.chain_view.len(), 1);
    }

    #[test]
    fn test_remove_advance_funds_advance_funds_block_removes_it() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let advance_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let advance_funds_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(50)));
        let advance_funds_block_2 = RskBlockAndUncles::new_no_uncles(create_advance_funds_block(
            &advance_funds_block.block(),
        ));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: advance_block.number(),
                    block_hash: advance_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(123)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process block");

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.check_fork_accumulator.is_none());
        assert_eq!(processor.chain_view.len(), 1);

        processor
            .process_new_block(&advance_funds_block_2)
            .expect("Should process block");

        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert!(processor.check_fork_accumulator.is_none());
        assert_eq!(processor.chain_view.len(), 1);
    }

    #[test]
    fn test_remove_advance_funds_advance_funds_event_stops_advance_funds() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let advance_funds_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(100)));

        let pegout_id = "peg123";

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(123)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process block");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(123)),
            }))
            .expect("Should have processed kickoff");

        assert!(processor.check_fork_accumulator.is_some());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.chain_view.len(), 1);

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: create_fake_advance_funds_event(pegout_id),
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: true,
                tx_hash: TxHash::from(H256::from_low_u64_be(456)),
            }))
            .expect("Should have processed removal");

        assert!(processor.check_fork_accumulator.is_none());
        assert!(
            processor
                .request_events
                .contains_key(&pegout_id.to_string())
        );
        assert_eq!(processor.chain_view.len(), 1);
    }

    #[test]
    fn test_process_blocks_handles_reorg_after_kickoff() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        expect_zkp_bitvmx(&mut bitvmx_broker);

        let pegout_id = "peg123";
        let mut rsk_gateway = MockRskContractsGatewayApi::new();
        expect_notify_check_fork(&mut rsk_gateway, pegout_id);

        let mut processor =
            AdvanceFundsProcessor::new_for_test(Rc::new(rsk_gateway), Rc::new(bitvmx_broker));

        // create initial request block
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        // process request event
        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(100)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process request block");

        // create and process kickoff event
        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        let required_blocks = advance_funds_event.required_num_blocks;
        let blocks_to_achieve_pow = required_blocks - 1; // Need one less to require more blocks

        let block_effort = advance_funds_event
            .required_effort
            .checked_div(AlloyU256::from(blocks_to_achieve_pow))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(101)),
            }))
            .expect("Should have processed kickoff");

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process kickoff block");

        // verify advance funds is active
        assert!(processor.check_fork_accumulator.is_some());
        assert_eq!(processor.chain_view.len(), 2);

        // build original chain - process several blocks after kickoff
        let mut original_blocks = Vec::new();
        for i in 1..=4 {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                advance_funds_block.number() + i,
                block_effort,
            ));
            original_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        // at this point we should have: request_block + advance_funds_block + 4 more blocks = 6 total
        assert_eq!(processor.chain_view.len(), 6);
        assert!(processor.check_fork_accumulator.is_some());

        // now simulate a reorg: create alternative chain from block after kickoff
        // the reorg starts at advance_funds_block.number() + 2 (replacing the 2nd block after kickoff)
        let reorg_point = advance_funds_block.number() + 2;

        // create alternative blocks with higher total difficulty to trigger reorg
        // but not too high to complete advance funds immediately
        let higher_effort = block_effort + U256::from(10); // Only slightly higher

        // first alternative block (replaces original_blocks[1])
        let alt_block_1 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(reorg_point, higher_effort));

        // process the alternative block - this should trigger reorg detection
        processor
            .process_new_block(&alt_block_1)
            .expect("Should handle reorg");

        // verify reorg was handled: chain should have been reorganized
        // we should have: request_block + advance_funds_block + original_blocks[0] + alt_block_1 = 4 blocks
        assert_eq!(processor.chain_view.len(), 4);

        // verify the actual blocks present after reorg
        let expected_blocks = vec![
            (&request_block, "Request block"),
            (&advance_funds_block, "Kickoff block"),
            (&original_blocks[0], "First original block after kickoff"),
            (&alt_block_1, "Alternative block"),
        ];

        for (expected_block, description) in expected_blocks {
            assert_eq!(
                processor.chain_view.get_at(&expected_block.number()),
                Some(expected_block),
                "{} should be present",
                description
            );
        }

        // verify reorged blocks - only blocks that get rolled back should be None
        // in this case, since alt_block_1 is at reorg_point, rollback_to will be called
        // and will remove blocks with number > reorg_point
        for (_, original_block) in original_blocks.iter().enumerate().skip(1) {
            if original_block.number() > reorg_point {
                // blocks after reorg point should be removed by rollback_to
                assert_eq!(processor.chain_view.get_at(&original_block.number()), None);
            } else {
                // blocks at or before reorg point should still be present
                // (either original or replaced)
                assert!(
                    processor
                        .chain_view
                        .get_at(&original_block.number())
                        .is_some()
                );
            }
        }

        assert!(processor.check_fork_accumulator.is_some());

        // continue building the alternative chain
        let mut alt_blocks = vec![alt_block_1];
        for i in 1..=5 {
            let block =
                RskBlockAndUncles::new_no_uncles(create_fake_block(reorg_point + i, higher_effort));
            alt_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should process alternative block");

            // check if advance funds completed during alternative chain building
            if processor.check_fork_accumulator.is_none() {
                break;
            }
        }

        // the advance funds might complete during alternative chain building
        // due to the accumulated effort, which is expected behavior
        if processor.check_fork_accumulator.is_none() {
            // advance funds completed during reorg - verify final state
            assert!(processor.request_events.is_empty());
            assert!(processor.first_block_to_process.is_none());
            assert!(processor.chain_view.is_empty());
            assert!(!processor.chain_view.is_observed());
        } else {
            // advance funds still active - continue with additional blocks until completion
            let mut additional_blocks_needed = 10; // arbitrary limit to prevent infinite loop

            for i in 6..=15 {
                let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                    reorg_point + i,
                    higher_effort,
                ));
                processor
                    .process_new_block(&block)
                    .expect("Should process additional block");

                // check if advance funds completed
                if processor.check_fork_accumulator.is_none() {
                    break;
                }

                additional_blocks_needed -= 1;
                if additional_blocks_needed == 0 {
                    panic!("Advance funds didn't complete after many blocks");
                }
            }

            // verify advance funds completed successfully after reorg
            assert!(processor.check_fork_accumulator.is_none());
            assert!(processor.request_events.is_empty());
            assert!(processor.first_block_to_process.is_none());
            assert!(processor.chain_view.is_empty());
            assert!(!processor.chain_view.is_observed());
        }
    }

    #[test]
    fn test_process_blocks_handles_deep_reorg_before_kickoff() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let pegout_id = "peg123";

        // create initial request block
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        // process request event
        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(105)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process request block");

        // build several blocks after request
        let mut original_blocks = Vec::new();
        for i in 1..=5 {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                request_block.number() + i,
                U256::from(50 + i),
            ));
            original_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        assert_eq!(processor.chain_view.len(), 6); // request + 5 blocks
        assert!(processor.check_fork_accumulator.is_none()); // No kickoff yet

        // now simulate a deep reorg that goes back to request block
        // create alternative chain with higher difficulty
        let reorg_point = request_block.number() + 1;
        let higher_difficulty = U256::from(200);

        // create alternative blocks
        let mut alternative_blocks = Vec::new();
        for i in 0..=7 {
            // more blocks than original to ensure higher total difficulty
            // use a different block number offset to make them truly different
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                reorg_point + i,
                higher_difficulty,
            ));
            alternative_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should handle deep reorg");
        }

        // verify deep reorg was handled properly
        // should have: request_block + 8 alternative blocks = 9 total
        assert_eq!(processor.chain_view.len(), 9);
        assert!(processor.check_fork_accumulator.is_none()); // Still no kickoff
        assert!(processor.request_events.contains_key(pegout_id));

        // verify blocks present after deep reorg
        let mut expected_blocks = vec![(&request_block, "Request block")];
        for alt_block in &alternative_blocks {
            expected_blocks.push((alt_block, "Alternative block"));
        }

        for (expected_block, description) in expected_blocks {
            assert_eq!(
                processor.chain_view.get_at(&expected_block.number()),
                Some(expected_block),
                "{} should be present",
                description
            );
        }

        // verify the chain structure after deep reorg
        // we started with 6 blocks (request + 5 original), processed 8 alternative blocks
        // the final state should have 9 blocks total (request + 8 alternatives)
        assert_eq!(processor.chain_view.len(), 9);

        // verify that we have blocks at the expected positions
        // request block should still be there
        assert!(
            processor
                .chain_view
                .get_at(&request_block.number())
                .is_some()
        );

        // alternative blocks should be present from reorg_point onwards
        for alt_block in &alternative_blocks {
            assert!(processor.chain_view.get_at(&alt_block.number()).is_some());
        }

        // now process kickoff event on the new chain
        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        let advance_funds_block_number = reorg_point + 3; // Pick a block in the middle of new chain

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event,
                block_number: advance_funds_block_number,
                block_hash: BlockHash::from(H256::from_low_u64_be(
                    advance_funds_block_number.value(),
                )),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(136)),
            }))
            .expect("Should have processed kickoff");

        // verify kickoff was processed on reorganized chain
        assert!(processor.check_fork_accumulator.is_some());
        assert!(!processor.request_events.is_empty());
        assert!(processor.first_block_to_process.is_some());
        assert_eq!(processor.chain_view.len(), 9);
        assert!(processor.chain_view.is_observed());
    }

    #[test]
    fn test_process_blocks_reorg_during_confirmations_period() {
        let mut bitvmx_broker = MockBrokerClientApi::new();
        bitvmx_broker
            .expect_send()
            .times(1)
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &IncomingBitVMXApiMessages| {
                    matches!(req, IncomingBitVMXApiMessages::GenerateZKP(_, _))
                }),
            )
            .returning(|_, _| Ok(true));

        let pegout_id = "peg123";
        let mut rsk_gateway = MockRskContractsGatewayApi::new();
        expect_notify_check_fork(&mut rsk_gateway, pegout_id);

        let mut processor =
            AdvanceFundsProcessor::new_for_test(Rc::new(rsk_gateway), Rc::new(bitvmx_broker));

        // set up request and kickoff like previous tests
        let request_block =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));

        let request_event = create_fake_request_event(pegout_id);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event,
                    block_number: request_block.number(),
                    block_hash: request_block.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(100)),
                },
            ))
            .expect("Should have processed request");

        processor
            .process_new_block(&request_block)
            .expect("Should process request block");

        let advance_funds_event = create_fake_advance_funds_event(pegout_id);
        let required_blocks = advance_funds_event.required_num_blocks;
        let required_blocks_plus_confirmations = required_blocks + REQUIRED_CONFIRMATIONS;

        let block_effort = advance_funds_event
            .required_effort
            .checked_div(AlloyU256::from(required_blocks))
            .expect("0 division");
        let block_effort = U256::from_big_endian(&block_effort.to_be_bytes_vec());

        let advance_funds_block = RskBlockAndUncles::new_no_uncles(create_fake_block(
            request_block.number() + 1,
            block_effort,
        ));

        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event.clone(),
                block_number: advance_funds_block.number(),
                block_hash: advance_funds_block.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(456)),
            }))
            .expect("Should have processed kickoff");

        processor
            .process_new_block(&advance_funds_block)
            .expect("Should process kickoff block");

        // build blocks until we have enough PoW but are still in confirmation period
        let mut pow_blocks = Vec::new();
        for i in 1..required_blocks {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                advance_funds_block.number() + i as u64,
                block_effort,
            ));
            pow_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should process block");
        }

        // add some confirmation blocks but not enough to complete
        let mut confirmation_blocks = Vec::new();
        let partial_confirmations = REQUIRED_CONFIRMATIONS / 2; // Only half the confirmations
        for i in 0..partial_confirmations {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                advance_funds_block.number() + required_blocks as u64 + i as u64,
                block_effort,
            ));
            confirmation_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should process confirmation block");
        }

        // we should be in confirmation period but not complete
        assert!(processor.check_fork_accumulator.is_some());
        let afc = processor.check_fork_accumulator.as_ref().unwrap();
        let has_enough_confirmations = afc.borrow().has_enough_confirmations();
        assert!(!has_enough_confirmations);

        // now simulate a reorg during the confirmation period
        // reorg from a point during the confirmation period
        let reorg_point = advance_funds_block.number() + required_blocks as u64 + 1;
        let higher_effort = block_effort * 2;

        // create alternative chain with higher effort that will complete the advance funds
        let mut alternative_blocks = Vec::new();
        for i in 0..required_blocks_plus_confirmations {
            let block = RskBlockAndUncles::new_no_uncles(create_fake_block(
                reorg_point + i as u64,
                higher_effort,
            ));
            alternative_blocks.push(block.clone());
            processor
                .process_new_block(&block)
                .expect("Should handle reorg during confirmation");
        }

        // verify advance funds completed after reorg (triggering the single ZKP call)
        assert!(processor.check_fork_accumulator.is_none());
        assert!(processor.request_events.is_empty());
        assert!(processor.first_block_to_process.is_none());
        assert!(processor.chain_view.is_empty());
        assert!(!processor.chain_view.is_observed());
    }

    fn expect_zkp_bitvmx(
        bitvmx_broker: &mut MockBrokerClientApi<
            IncomingBitVMXApiMessages,
            OutgoingBitVMXApiMessages,
        >,
    ) {
        bitvmx_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &IncomingBitVMXApiMessages| {
                    matches!(req, IncomingBitVMXApiMessages::GenerateZKP(_, _))
                }),
            )
            .return_once(|_, _| Ok(true));
    }

    #[test]
    fn test_advance_funds_with_active_accumulator_closes_existing_and_returns() {
        let mut processor = AdvanceFundsProcessor::new_for_test(
            Rc::new(MockRskContractsGatewayApi::new()),
            Rc::new(MockBrokerClientApi::new()),
        );
        let request_block_1 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(100.into(), U256::from(50)));
        let advance_funds_block_1 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(110.into(), U256::from(100)));

        let pegout_id_1 = "peg123";
        let pegout_id_2 = "peg456";

        let request_event_1 = create_fake_request_event(pegout_id_1);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event_1,
                    block_number: request_block_1.number(),
                    block_hash: request_block_1.hash(),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(888)),
                },
            ))
            .expect("Should have processed request");
        assert_eq!(processor.request_events.len(), 1);

        processor
            .process_new_block(&request_block_1)
            .expect("Should process block");

        let advance_funds_event_1 = create_fake_advance_funds_event(pegout_id_1);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event_1,
                block_number: advance_funds_block_1.number(),
                block_hash: advance_funds_block_1.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(963)),
            }))
            .expect("Should have processed kickoff");

        // verify first advance funds checker is active
        assert!(processor.check_fork_accumulator.is_some());
        let first_checker_pegout_id = processor
            .check_fork_accumulator
            .as_ref()
            .unwrap()
            .borrow()
            .pegout_id();
        assert_eq!(first_checker_pegout_id, pegout_id_1);
        assert_eq!(processor.request_events.len(), 1);

        let request_event_2 = create_fake_request_event(pegout_id_2);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::RequestAdvanceFunds(
                RequestAdvanceFundsEvent {
                    inner: request_event_2,
                    block_number: request_block_1.number() + 1,
                    block_hash: BlockHash::from(H256::from_low_u64_be(456)),
                    removed: false,
                    tx_hash: TxHash::from(H256::from_low_u64_be(456)),
                },
            ))
            .expect("Should have processed request");
        assert_eq!(processor.request_events.len(), 2);

        // attempt to kickoff second advance funds while first is active
        let advance_funds_block_2 =
            RskBlockAndUncles::new_no_uncles(create_fake_block(115.into(), U256::from(100)));
        let advance_funds_event_2 = create_fake_advance_funds_event(pegout_id_2);
        processor
            .process_new_rsk_event(&RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
                inner: advance_funds_event_2,
                block_number: advance_funds_block_2.number(),
                block_hash: advance_funds_block_2.hash(),
                removed: false,
                tx_hash: TxHash::from(H256::from_low_u64_be(115)),
            }))
            .expect("Should have processed kickoff");

        // verify that the first advance funds checker was closed and no new one was created
        assert!(processor.check_fork_accumulator.is_none());
        assert_eq!(processor.request_events.len(), 1);
        assert!(processor.request_events.contains_key(pegout_id_2));
    }

    fn expect_notify_check_fork(rsk_gateway: &mut MockRskContractsGatewayApi, pegout_id: &str) {
        let id = pegout_id.to_string();
        rsk_gateway
            .expect_notify_check_fork_completion()
            .with(eq(id))
            .times(1)
            .returning(|_| Ok(()));
    }
}
