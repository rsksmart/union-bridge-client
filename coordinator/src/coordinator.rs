use crate::flows::advance_funds::advance_funds_processor::AdvanceFundsProcessor;
use crate::{
    event_processor::{EventProcessor, PeginProcessor},
    monitor::MonitorApi,
};
use anyhow::{Context, Result};
use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use common::shutdown_flag::ShutdownFlag;
use log::{error, info, warn};
use std::ops::Sub;
use std::rc::Rc;
use std::time::Instant;
use std::{thread, time::Duration};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

const CHECK_PERIOD: Duration = Duration::from_secs(1);
const BITVMX_NOT_RESPONDING_THRESHOLD: Duration = Duration::from_secs(30);
const BITVMX_PING_AFTER_SILENCE: Duration = Duration::from_secs(15);

pub struct Coordinator<M: MonitorApi, BC: BitVmxBrokerClientApi> {
    monitor: M,
    bitvmx_broker: Rc<BC>,
    processors: Vec<Box<dyn EventProcessor>>,
    check_period: Duration,
    shutdown_flag: ShutdownFlag,
}

impl<M: MonitorApi, BC: BitVmxBrokerClientApi + 'static> Coordinator<M, BC> {
    pub fn new<CG: RskContractsGatewayApi + 'static>(
        rt_sync: RuntimeSync,
        monitor: M,
        contracts_gateway: CG,
        bitvmx_broker: Rc<BC>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        let contracts_arc = Rc::new(contracts_gateway);
        Self {
            monitor,
            bitvmx_broker: bitvmx_broker.clone(),
            processors: vec![
                Box::new(AdvanceFundsProcessor::new(
                    rt_sync.clone(),
                    contracts_arc.clone(),
                    bitvmx_broker.clone(),
                )),
                Box::new(PeginProcessor::new(
                    rt_sync.clone(),
                    contracts_arc,
                    bitvmx_broker.clone(),
                )),
            ],
            check_period: CHECK_PERIOD,
            shutdown_flag,
        }
    }

    pub fn new_for_tests(
        monitor: M,
        bitvmx_broker: BC,
        processors: Vec<Box<dyn EventProcessor>>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            monitor,
            bitvmx_broker: Rc::new(bitvmx_broker),
            processors,
            check_period: Duration::from_millis(1),
            shutdown_flag,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        self.monitor
            .start_event_monitoring()
            .context("Failed to start event monitoring")?;

        self.monitor
            .start_block_monitoring()
            .context("Failed to start block monitoring")?;

        self.monitor
            .start_bitvmx_monitoring()
            .context("Failed to start BitVMX event monitoring")?;

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

                if let Some(event) = self
                    .monitor
                    .try_bitvmx_event()
                    .context("Error getting BitVMX event")?
                {
                    self.check_bitvmx_pong(&event).then(|| bitvmx_ping = None);
                    bitvmx_last_msg = Instant::now();

                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_bitvmx_event(&event) {
                            error!("Error processing BitVMX event {:?}: {:?}", event, e);
                        }
                    });
                }

                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
                //  if block monitor restarted, this is not realising and keeps waiting logs forever
                //  maybe using persistent storage instead of memory fixes it?
                if let Some(event) = self.monitor.try_event().context("Error getting event")? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_rsk_event(&event) {
                            error!("Error processing Union Bridge event {:?}: {:?}", event, e);
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
                            error!("Error processing block {:?}: {:?}", block, e);
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                if !message_received {
                    thread::sleep(self.check_period);
                }
            }
            Ok(())
        })();

        self.processors.iter_mut().for_each(|p| p.shutdown());

        self.monitor
            .cancel_bitvmx_monitoring()
            .context("Failed to cancel BitVMX event monitoring")?;

        self.monitor
            .cancel_block_monitoring()
            .context("Failed to cancel block monitoring")?;

        self.monitor
            .cancel_event_monitoring()
            .context("Failed to cancel event monitoring")?;

        result
    }

    fn check_bitvmx_liveness(&self, bitvmx_ping: &mut Option<Instant>, bitvmx_last_msg: Instant) {
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
        info!("Sending Ping to BitVMX");

        let result = self
            .bitvmx_broker
            .send(BROKER_SERVER_ID, IncomingBitVMXApiMessages::Ping());

        if result.is_err() {
            // TODO we need to handle this situation properly
            error!("Failed to send Ping to BitVMX: {:?}", result);
        }
    }

    fn check_bitvmx_pong(&mut self, event: &OutgoingBitVMXApiMessages) -> bool {
        match event {
            OutgoingBitVMXApiMessages::Pong() => {
                info!("Received Pong from BitVMX");
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
    use crate::event_processor::{EventProcessor, MockEventProcessor};
    use crate::{
        coordinator::Coordinator,
        monitor::MockMonitorApi,
        types::{AdvanceFundsEvent, RequestAdvanceFundsEvent, RskPegManagerEvents},
    };
    use actors_mocking::fake_contracts::FakePegManager::{AdvanceFunds, RequestAdvanceFunds};
    use alloy_primitives::U256;
    use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
    use common::msg_broker::broker::{BROKER_SERVER_ID, MockBrokerClientApi};
    use common::types::TxHash;
    use common::{
        shutdown_flag::ShutdownFlag,
        test_utils::rsk_block_generator::{
            create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
        },
        types::RskBlockAndUncles,
    };
    use mockall::mock;
    use mockall::predicate::{eq, function};
    use primitive_types::H256;
    use std::{
        thread::{self, JoinHandle, sleep},
        time::Duration,
    };
    use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
    use transaction_dispatcher::types::{
        AcceptPegInInput, AcceptPegInOutput, AddMemberNonceInput, AddMemberNonceOutput,
        AddMemberSignatureInput, AddMemberSignatureOutput, PegInAddressInput, PegInAddressOutput,
        RegisterPegInInput, RegisterPegInOutput, RegisterPegOutInput, RegisterPegOutOutput,
    };

    fn create_fake_request_event(peg_out_id: &str) -> RequestAdvanceFunds {
        RequestAdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
            amount: 1000,
        }
    }

    fn create_fake_advance_funds_event(peg_out_id: &str) -> AdvanceFunds {
        AdvanceFunds {
            peg_out_id: peg_out_id.to_string(),
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
            inner: create_fake_request_event("peg_out_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash().into(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_1.number().value())),
        });

        let event_2: RskPegManagerEvents = RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
            inner: create_fake_advance_funds_event("peg_out_id_1"),
            block_number: block_2.number(),
            block_hash: block_2.hash().into(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_2.number().value())),
        });

        let bitvmx_event = OutgoingBitVMXApiMessages::Pong();

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_bitvmx_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_bitvmx_monitoring()
            .return_once(|| Ok(()))
            .once();

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new(block_2, vec![uncle_1]),
            ],
            &mut mock_monitor,
        );

        mock_monitor
            .expect_try_bitvmx_event()
            .returning(move || Ok(Some(bitvmx_event.clone())));

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker = MockBrokerClientApi::new();
        bitvmx_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &IncomingBitVMXApiMessages| {
                    matches!(req, IncomingBitVMXApiMessages::Ping())
                }),
            )
            .return_once(|_, _| Ok(true));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            generate_ok_processors(),
            shutdown_flag,
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
            inner: create_fake_request_event("peg_out_id_1"),
            block_number: block_1.number(),
            block_hash: block_1.hash().into(),
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(block_1.number().value())),
        });

        let event_2 = RskPegManagerEvents::UnknownEvent;

        mock_monitor
            .expect_start_event_monitoring()
            .return_once(|| Ok(()));

        mock_monitor
            .expect_start_block_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_start_bitvmx_monitoring()
            .times(..)
            .returning(|| Ok(()));

        mock_monitor
            .expect_cancel_event_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_block_monitoring()
            .return_once(|| Ok(()))
            .once();

        mock_monitor
            .expect_cancel_bitvmx_monitoring()
            .return_once(|| Ok(()))
            .once();

        expect_try_event(vec![event_1, event_2], &mut mock_monitor);

        expect_try_block(
            vec![
                RskBlockAndUncles::new_no_uncles(block_1),
                RskBlockAndUncles::new_no_uncles(block_2),
            ],
            &mut mock_monitor,
        );

        let bitvmx_event = OutgoingBitVMXApiMessages::Pong();

        mock_monitor
            .expect_try_bitvmx_event()
            .returning(move || Ok(Some(bitvmx_event.clone())));

        mock_monitor
            .expect_try_bitvmx_event()
            .returning(move || Ok(None));

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker = MockBrokerClientApi::new();
        bitvmx_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(|req: &IncomingBitVMXApiMessages| {
                    matches!(req, IncomingBitVMXApiMessages::Ping())
                }),
            )
            .return_once(|_, _| Ok(true));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            generate_ok_processors(),
            shutdown_flag,
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

    fn expect_try_event(client_requests: Vec<RskPegManagerEvents>, monitor: &mut MockMonitorApi) {
        use std::collections::VecDeque;

        monitor.expect_try_event().returning_st({
            let mut responses = client_requests
                .into_iter()
                .map(|e| Ok(Some(e)))
                .collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn expect_try_block(blocks: Vec<RskBlockAndUncles>, monitor: &mut MockMonitorApi) {
        use std::collections::VecDeque;

        monitor.expect_try_block().returning_st({
            let mut responses = blocks
                .into_iter()
                .map(|b| Ok(Some(b)))
                .collect::<VecDeque<_>>();

            move || responses.pop_front().unwrap_or(Ok(None))
        });
    }

    fn generate_ok_processors() -> Vec<Box<dyn EventProcessor>> {
        let mut mock_pegout_processor = MockEventProcessor::new();
        let mut mock_get_temp_addr_processor = MockEventProcessor::new();

        expect_processors_ok(&mut mock_pegout_processor);
        expect_processors_ok(&mut mock_get_temp_addr_processor);

        vec![
            Box::new(mock_pegout_processor),
            Box::new(mock_get_temp_addr_processor),
        ]
    }

    fn expect_processors_ok(mock_pegout_processor: &mut MockEventProcessor) {
        mock_pegout_processor
            .expect_process_new_bitvmx_event()
            .returning(|_| Ok(()))
            .times(1..);
        mock_pegout_processor
            .expect_process_new_block()
            .returning(|_| Ok(()))
            .times(1..);
        mock_pegout_processor
            .expect_process_new_rsk_event()
            .returning(|_| Ok(()))
            .times(1..);
        mock_pegout_processor.expect_shutdown().return_once(|| ());
    }

    mock! {
        #[derive(Clone)]
        pub RskContractsGatewayApi {}

        impl RskContractsGatewayApi for RskContractsGatewayApi {
            async fn get_temporary_peg_in_address(
                &self,
                input: PegInAddressInput,
            ) -> Result<PegInAddressOutput, DomainErrors>;

            async fn register_peg_in_request(
                &self,
                input: RegisterPegInInput,
            ) -> Result<RegisterPegInOutput, DomainErrors>;

            async fn accept_peg_in_request(
                &self,
                input: AcceptPegInInput,
            ) -> Result<AcceptPegInOutput, DomainErrors>;

            async fn register_peg_out_request(
                &self,
                input: RegisterPegOutInput,
            ) -> Result<RegisterPegOutOutput, DomainErrors>;

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
        }
    }
}
