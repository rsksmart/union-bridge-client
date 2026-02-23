use std::ops::Sub;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bitcoin::Network;
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::BitVmxBrokerClientApi;
use common::runtime_sync::RuntimeSync;
use common::shutdown_flag::ShutdownFlag;
use log::{debug, error, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

use crate::event_processor::EventProcessor;
use crate::flows::advance_funds::advance_funds_processor::AdvanceFundsProcessor;
use crate::flows::committee::setup_committee_flow::{
    SetupCommitteeFlowFactory, SetupCommitteeProcessor,
};
use crate::flows::common::GlobalContext;
use crate::flows::fund_bitvmx_flow::FundBitvmxProcessor;
use crate::flows::operator_take::AdvanceFundsFlowProcessor;
use crate::flows::pegin::pegin_processor::PeginFlowProcessor;
use crate::flows::pegout::pegout_processor::PegoutFlowProcessor;
use crate::monitor::MonitorApi;
use crate::store::CoordinatorStoreApi;

const CHECK_PERIOD: Duration = Duration::from_secs(1);
const BITVMX_NOT_RESPONDING_THRESHOLD: Duration = Duration::from_secs(30);
const BITVMX_PING_AFTER_SILENCE: Duration = Duration::from_secs(15);

pub struct Coordinator<M: MonitorApi, BC: BitVmxBrokerClientApi, S: CoordinatorStoreApi> {
    monitor: M,
    bitvmx_broker: Rc<BC>,
    processors: Vec<Box<dyn EventProcessor>>,
    check_period: Duration,
    store: Rc<S>,
    global_context: GlobalContext,
    shutdown_flag: ShutdownFlag,
}

impl<M: MonitorApi, BC: BitVmxBrokerClientApi + 'static, S: CoordinatorStoreApi + 'static>
    Coordinator<M, BC, S>
{
    // todo(fede) new methods should be transaparent and any other extra logic should be in a
    // separate build/factory method
    /// # Panics
    /// Panics if loading context from the database fails.
    pub fn new<CG: RskContractsGatewayApi + 'static>(
        rt_sync: &RuntimeSync,
        monitor: M,
        contracts_gateway: CG,
        bitvmx_broker: &Rc<BC>,
        store: S,
        shutdown_flag: ShutdownFlag,
        bitcoin_network: Network,
    ) -> Self {
        let contracts_arc = Rc::new(contracts_gateway);
        let store_rc = Rc::new(store);

        let global_context =
            store_rc.load_context().expect("Failed to load context from DB").unwrap_or_else(|| {
                warn!("No context found in DB, starting with empty one");
                GlobalContext::new()
            });

        let setup_committee_flow_factory = SetupCommitteeFlowFactory::new(
            Rc::clone(&contracts_arc),
            rt_sync.clone(),
            bitvmx_broker.clone(),
            global_context.clone(),
            bitcoin_network,
            Rc::clone(&store_rc),
        );

        Self {
            monitor,
            bitvmx_broker: bitvmx_broker.clone(),
            processors: vec![
                Box::new(AdvanceFundsProcessor::new(
                    rt_sync.clone(),
                    Rc::clone(&contracts_arc),
                    bitvmx_broker.clone(),
                )),
                Box::new(PeginFlowProcessor::new(
                    Rc::clone(&contracts_arc),
                    rt_sync.clone(),
                    bitvmx_broker.clone(),
                    global_context.clone(),
                    Rc::clone(&store_rc),
                )),
                Box::new(
                    PegoutFlowProcessor::restore_or_new(
                        contracts_arc.clone(),
                        rt_sync.clone(),
                        bitvmx_broker.clone(),
                        global_context.clone(),
                        store_rc.clone(),
                    )
                    // todo(fede) ideally this method should return a result
                    .expect("couldn't restore or create pegout flow processor"),
                ),
                //Operator_take_flow
                Box::new(AdvanceFundsFlowProcessor::new(
                    contracts_arc.clone(),
                    rt_sync.clone(),
                    bitvmx_broker.clone(),
                    global_context.clone(),
                )),
                Box::new(SetupCommitteeProcessor::new(
                    setup_committee_flow_factory,
                    global_context.clone(),
                    Rc::clone(&store_rc),
                    bitvmx_broker.as_ref(),
                )),
                Box::new(FundBitvmxProcessor::new(bitvmx_broker.clone(), bitcoin_network)),
            ],
            check_period: CHECK_PERIOD,
            shutdown_flag,
            store: store_rc,
            global_context: global_context.clone(),
        }
    }

    pub fn new_for_tests(
        monitor: M,
        bitvmx_broker: BC,
        processors: Vec<Box<dyn EventProcessor>>,
        shutdown_flag: ShutdownFlag,
        store: S,
    ) -> Self {
        Self {
            monitor,
            bitvmx_broker: Rc::new(bitvmx_broker),
            processors,
            check_period: Duration::from_millis(1),
            store: Rc::new(store),
            global_context: GlobalContext::new(),
            shutdown_flag,
        }
    }

    /// # Errors
    /// Returns an error if the coordinator run loop fails.
    pub fn run(&mut self) -> Result<()> {
        self.monitor.start_event_monitoring().context("Failed to start event monitoring")?;

        self.monitor.start_block_monitoring().context("Failed to start block monitoring")?;

        self.monitor
            .start_bitvmx_monitoring()
            .context("Failed to start BitVMX event monitoring")?;

        self.monitor.start_user_monitoring().context("Failed to start User request monitoring")?;

        let mut bitvmx_last_msg = Instant::now().sub(BITVMX_PING_AFTER_SILENCE);
        let mut bitvmx_ping: Option<Instant> = None;

        // TODO we will need to think what happens if we accumulate messages of a certain type

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                self.check_bitvmx_liveness(&mut bitvmx_ping, bitvmx_last_msg);

                let mut message_received = false;

                if let Some(req) =
                    self.monitor.try_user_request().context("Error getting User request")?
                {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_user_request(&req) {
                            error!("Error processing User request {req:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                if let Some(event) =
                    self.monitor.try_bitvmx_event().context("Error getting BitVMX event")?
                {
                    Self::check_bitvmx_pong(&event).then(|| bitvmx_ping = None);
                    bitvmx_last_msg = Instant::now();

                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_bitvmx_event(&event) {
                            error!("Error processing BitVMX event {event:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
                //  if block monitor restarted, this is not realising and keeps waiting logs forever
                //  maybe using persistent storage instead of memory fixes it?
                if let Some(event) = self.monitor.try_rsk_event().context("Error getting event")? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_rsk_event(&event) {
                            error!("Error processing Union Bridge event {event:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - if block monitor restarted, this is not realising and keeps waiting blocks forever
                //  if block monitor restarted, this is not realising and keeps waiting logs forever
                //  maybe using persistent storage instead of memory fixes it?
                if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_block(&block) {
                            error!("Error processing block {block:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                self.store.save_context(&self.global_context).context("Storing context in DB")?;

                if !message_received {
                    thread::sleep(self.check_period);
                }
            }
            Ok(())
        })();

        self.processors.iter_mut().for_each(|p| p.shutdown());

        // Final persistence save during shutdown to ensure no state is lost
        self.store
            .save_context(&self.global_context)
            .context("Final context save during shutdown")?;

        self.monitor
            .cancel_bitvmx_monitoring()
            .context("Failed to cancel BitVMX event monitoring")?;

        self.monitor.cancel_block_monitoring().context("Failed to cancel block monitoring")?;

        self.monitor.cancel_event_monitoring().context("Failed to cancel event monitoring")?;

        result
    }

    fn check_bitvmx_liveness(&self, bitvmx_ping: &mut Option<Instant>, bitvmx_last_msg: Instant) {
        #[allow(clippy::collapsible_if)]
        if let Some(ping) = bitvmx_ping {
            if ping.elapsed() > BITVMX_NOT_RESPONDING_THRESHOLD {
                // TODO in the future we have to properly handle this situation
                warn!("BitVMX is not responding");
                *bitvmx_ping = None;
            }
        }

        // send ping if we have not received any message from BitVMX for a while and there is no pending ping
        if bitvmx_last_msg.elapsed() > BITVMX_PING_AFTER_SILENCE && bitvmx_ping.is_none() {
            self.send_bitvmx_ping();
            *bitvmx_ping = Some(Instant::now());
        }
    }

    fn send_bitvmx_ping(&self) {
        let ping_id = uuid::Uuid::new_v4();
        debug!("Sending Ping to BitVMX with uuid: {ping_id}");

        let result = self.bitvmx_broker.send(IncomingBitVMXApiMessages::Ping(ping_id));

        if result.is_err() {
            // TODO we need to handle this situation properly
            error!("Failed to send Ping to BitVMX: {result:?}");
        }
    }

    fn check_bitvmx_pong(event: &OutgoingBitVMXApiMessages) -> bool {
        match event {
            OutgoingBitVMXApiMessages::Pong(uuid) => {
                debug!("Received Pong from BitVMX with uuid: {uuid}");
                true
            }
            _ => false,
        }
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::thread::{self, JoinHandle, sleep};
    use std::time::Duration;

    use alloy_primitives::U256;
    use common::mocks::fake_contracts::FakePegManager::{AdvanceFunds, RequestAdvanceFunds};
    use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
    use common::msg_broker::broker::MockBrokerClientApi;
    use common::shutdown_flag::ShutdownFlag;
    use common::test_utils::rsk_block_generator::{
        create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
    };
    use common::types;
    use common::types::{RskBlockAndUncles, TxHash};
    use mockall::mock;
    use mockall::predicate::{always, function};
    use primitive_types::H256;
    use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
    use transaction_dispatcher::types::{
        AcceptPeginInput, AcceptPeginOutput, AddMemberNonceInput, AddMemberNonceOutput,
        AddMemberSignatureInput, AddMemberSignatureOutput, AddOperatorTakeTxHashInput,
        AddOperatorTakeTxHashOutput, ApplyToStreamInput, ApplyToStreamOutput,
        DepositAggregatedKeyInput, DepositAggregatedKeyOutput, DepositCommunicationDataInput,
        DepositCommunicationDataOutput, GetCommitteeInput, GetCommitteeOutput,
        GetCommunicationDataInput, GetCommunicationDataOutput, GetMemberPublicKeysInput,
        GetMemberPublicKeysOutput, PeginAddressInput, PeginAddressOutput, RegisterChallengeInput,
        RegisterChallengeOutput, RegisterInputRevealedInput, RegisterInputRevealedOutput,
        RegisterOperatorTakeInput, RegisterOperatorTakeOutput, RegisterOperatorWonInput,
        RegisterOperatorWonOutput, RegisterPegoutInput, RegisterPegoutOutput, RequestPeginInput,
        RequestPeginOutput, RequestPegoutInput, RequestPegoutOutput, TriggerOperatorTakeInput,
        TriggerOperatorTakeOutput,
    };

    use crate::coordinator::Coordinator;
    use crate::event_processor::{EventProcessor, MockEventProcessor};
    use crate::monitor::MockMonitorApi;
    use crate::store::MockCoordinatorStoreApi;
    use crate::types::{AdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents};

    fn create_fake_request_event(pegout_id: &str) -> RequestAdvanceFunds {
        RequestAdvanceFunds { pegout_id: pegout_id.to_string(), amount: 1000 }
    }

    fn create_fake_advance_funds_event(pegout_id: &str) -> AdvanceFunds {
        AdvanceFunds {
            pegout_id: pegout_id.to_string(),
            utxo_id: "utxo123".to_string(),
            operator_id: "op123".to_string(),
            required_effort: U256::from(1000),
            required_num_blocks: 4,
        }
    }

    #[test]
    fn test_coordinator_run_handles_several_events() {
        let mut mock_monitor = MockMonitorApi::new();
        let (block_1, uncle_1, block_2) = create_block_and_uncles();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
            inner: create_fake_request_event("pegout_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_1.number().value())),
        });

        let event_2: RskPegManagerEvents = RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
            inner: create_fake_advance_funds_event("pegout_id_1"),
            block_number: block_2.number(),
            block_hash: block_2.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_2.number().value())),
        });

        let bitvmx_event = OutgoingBitVMXApiMessages::Pong(uuid::Uuid::new_v4());

        mock_monitor.expect_start_event_monitoring().return_once(|| Ok(()));

        mock_monitor.expect_start_bitvmx_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_start_block_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_start_user_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_cancel_event_monitoring().return_once(|| Ok(())).once();

        mock_monitor.expect_cancel_block_monitoring().return_once(|| Ok(())).once();

        mock_monitor.expect_cancel_bitvmx_monitoring().return_once(|| Ok(())).once();

        expect_try_rsk_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new(block_2, vec![uncle_1]),
            ],
            &mut mock_monitor,
        );

        mock_monitor.expect_try_bitvmx_event().returning(move || Ok(Some(bitvmx_event.clone())));

        mock_monitor.expect_try_user_request().returning(|| Ok(None)).times(1..);

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::Ping(_))
            }))
            .return_once(|_| Ok(true));

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_context().with(always()).returning(|_| Ok(()));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            generate_ok_processors(),
            shutdown_flag,
            mock_store,
        );
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    #[test]
    fn test_coordinator_run_handles_unknown_event() {
        let mut mock_monitor = MockMonitorApi::new();

        let block_1 = get_first_default_rsk_block();
        let block_2 = get_second_default_rsk_block();

        let event_1 = RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFundsEvent {
            inner: create_fake_request_event("pegout_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_1.number().value())),
        });

        let event_2 = RskPegManagerEvents::UnknownEvent;

        mock_monitor.expect_start_event_monitoring().return_once(|| Ok(()));

        mock_monitor.expect_start_block_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_start_user_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_start_bitvmx_monitoring().times(..).returning(|| Ok(()));

        mock_monitor.expect_cancel_event_monitoring().return_once(|| Ok(())).once();

        mock_monitor.expect_cancel_block_monitoring().return_once(|| Ok(())).once();

        mock_monitor.expect_cancel_bitvmx_monitoring().return_once(|| Ok(())).once();

        expect_try_rsk_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new_no_uncles(block_2),
            ],
            &mut mock_monitor,
        );

        let bitvmx_event = OutgoingBitVMXApiMessages::Pong(uuid::Uuid::new_v4());

        mock_monitor.expect_try_bitvmx_event().returning(move || Ok(Some(bitvmx_event.clone())));

        mock_monitor.expect_try_bitvmx_event().returning(move || Ok(None));

        mock_monitor.expect_try_user_request().returning(|| Ok(None)).times(1..);

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::Ping(_))
            }))
            .return_once(|_| Ok(true));

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_context().with(always()).returning(|_| Ok(()));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            generate_ok_processors(),
            shutdown_flag,
            mock_store,
        );
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    fn handle_shutdown(shutdown_flag: ShutdownFlag) -> JoinHandle<()> {
        thread::spawn(move || {
            // give time for logic to proceed
            sleep(Duration::from_millis(100));
            shutdown_flag.set();
        })
    }

    fn expect_try_rsk_event(
        client_requests: Vec<RskPegManagerEvents>,
        monitor: &mut MockMonitorApi,
    ) {
        use std::collections::VecDeque;

        monitor.expect_try_rsk_event().returning_st({
            let mut responses =
                client_requests.into_iter().map(|e| Ok(Some(e))).collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn expect_try_block(blocks: Vec<RskBlockAndUncles>, monitor: &mut MockMonitorApi) {
        use std::collections::VecDeque;

        monitor.expect_try_block().returning_st({
            let mut responses = blocks.into_iter().map(|b| Ok(Some(b))).collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn generate_ok_processors() -> Vec<Box<dyn EventProcessor>> {
        let mut mock_pegout_processor = MockEventProcessor::new();
        let mut mock_get_temp_addr_processor = MockEventProcessor::new();

        expect_processors_ok(&mut mock_pegout_processor);
        expect_processors_ok(&mut mock_get_temp_addr_processor);

        vec![Box::new(mock_pegout_processor), Box::new(mock_get_temp_addr_processor)]
    }

    fn expect_processors_ok(mock_pegout_processor: &mut MockEventProcessor) {
        mock_pegout_processor.expect_process_new_bitvmx_event().returning(|_| Ok(())).times(1..);
        mock_pegout_processor.expect_process_new_block().returning(|_| Ok(())).times(1..);
        mock_pegout_processor.expect_process_new_rsk_event().returning(|_| Ok(())).times(1..);
        mock_pegout_processor.expect_shutdown().return_once(|| ());
    }

    mock! {
        #[derive(Clone)]
        pub RskContractsGatewayApi {}

        impl RskContractsGatewayApi for RskContractsGatewayApi {
            fn my_address(&self) -> types::Address;

            async fn get_balance(&self) -> Result<alloy_primitives::Uint<256, 4>, DomainErrors>;

            async fn get_temporary_pegin_address(
                &self,
                input: PeginAddressInput,
            ) -> Result<PeginAddressOutput, DomainErrors>;

            async fn request_pegin(
                &self,
                input: RequestPeginInput,
            ) -> Result<RequestPeginOutput, DomainErrors>;

            async fn accept_pegin(
                &self,
                input: AcceptPeginInput,
            ) -> Result<AcceptPeginOutput, DomainErrors>;

            async fn request_pegout(
                &self,
                input: RequestPegoutInput,
            ) -> Result<RequestPegoutOutput, DomainErrors>;

            async fn register_pegout(
                &self,
                input: RegisterPegoutInput,
            ) -> Result<RegisterPegoutOutput, DomainErrors>;

            async fn register_operator_take(
                &self,
                input: RegisterOperatorTakeInput,
            ) -> Result<RegisterOperatorTakeOutput, DomainErrors>;

            async fn trigger_operator_take(
                &self,
                input: TriggerOperatorTakeInput,
            ) -> Result<TriggerOperatorTakeOutput, DomainErrors>;

            async fn add_member_nonce(
                &self,
                input: AddMemberNonceInput,
            ) -> Result<AddMemberNonceOutput, DomainErrors>;

            async fn add_member_signature(
                &self,
                input: AddMemberSignatureInput,
            ) -> Result<AddMemberSignatureOutput, DomainErrors>;

            async fn notify_check_fork_completion(
                &self,
                input: &str,
            ) -> Result<(), DomainErrors>;

            async fn get_member_public_keys(
                &self, input: GetMemberPublicKeysInput
            ) -> Result<GetMemberPublicKeysOutput, DomainErrors>;

            async fn apply_to_stream(
                &self,
                input: ApplyToStreamInput,
            ) -> Result<ApplyToStreamOutput, DomainErrors>;

            async fn get_committee(
                &self,
                input: GetCommitteeInput,
            ) -> Result<GetCommitteeOutput, DomainErrors>;

            async fn get_committee_communication_data(
                &self,
                input: GetCommunicationDataInput,
            ) -> Result<GetCommunicationDataOutput, DomainErrors>;

            async fn deposit_communication_data(
                &self,
                input: DepositCommunicationDataInput
            ) -> Result<DepositCommunicationDataOutput, DomainErrors>;

            async fn deposit_aggregated_key(
                &self,
                input: DepositAggregatedKeyInput
            ) -> Result<DepositAggregatedKeyOutput, DomainErrors>;

            async fn add_operator_take_tx_hash(
                &self,
                input: AddOperatorTakeTxHashInput,
            ) -> Result<AddOperatorTakeTxHashOutput, DomainErrors>;

            async fn register_challenge(
                &self,
                input: RegisterChallengeInput,
            ) -> Result<RegisterChallengeOutput, DomainErrors>;

            async fn register_input_revealed(
                &self,
                input: RegisterInputRevealedInput,
            ) -> Result<RegisterInputRevealedOutput, DomainErrors>;

            async fn register_operator_won(
                &self,
                input: RegisterOperatorWonInput,
            ) -> Result<RegisterOperatorWonOutput, DomainErrors>;
        }
    }
}
