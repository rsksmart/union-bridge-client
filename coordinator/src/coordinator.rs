use crate::event_processor::BitVmxPingPongProcessor;
use crate::{
    event_processor::{AdvanceFundsProcessor, EventProcessor},
    monitor::MonitorApi,
};
use anyhow::{Context, Result};
use bitvmx_client::types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
use common::runtime_sync::RuntimeSync;
use common::types::RskBlockAndUncles;
use common::{msg_broker::broker::BrokerClientApi, shutdown_flag::ShutdownFlag};
use log::error;
use std::{thread, time::Duration};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

const CHECK_PERIOD: Duration = Duration::from_secs(1);

pub struct Coordinator<M: MonitorApi> {
    monitor: M,
    bitvmx_ping_pong_processor: Box<dyn EventProcessor>,
    processors: Vec<Box<dyn EventProcessor>>,
    check_period: Duration,
    shutdown_flag: ShutdownFlag,
}

impl<M: MonitorApi> Coordinator<M> {
    pub fn new<
        BC: BrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> + Clone + 'static,
        CG: RskContractsGatewayApi + Clone + 'static,
    >(
        rt_sync: RuntimeSync,
        monitor: M,
        contracts_gateway: CG,
        bitvmx_broker: BC,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            monitor,
            bitvmx_ping_pong_processor: Box::new(BitVmxPingPongProcessor::new(
                bitvmx_broker.clone(),
            )),
            processors: vec![Box::new(AdvanceFundsProcessor::new(
                rt_sync,
                contracts_gateway.clone(),
                bitvmx_broker.clone(),
            ))],
            check_period: CHECK_PERIOD,
            shutdown_flag,
        }
    }

    pub fn new_for_tests(
        monitor: M,
        bitvmx_ping_pong_processor: Box<dyn EventProcessor>,
        processors: Vec<Box<dyn EventProcessor>>,
        shutdown_flag: ShutdownFlag,
    ) -> Self {
        Self {
            monitor,
            bitvmx_ping_pong_processor,
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

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                let mut message_received = false;

                if let Some(event) = self
                    .monitor
                    .try_bitvmx_event()
                    .context("Error getting BitVMX event")?
                {
                    self.check_bitvmx_pong(&event);

                    // each processor decides if the event is relevant
                    self.processors
                        .iter_mut()
                        .try_for_each(|p| p.process_new_bitvmx_event(&event))?;
                }

                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132
                //  if block monitor restarted, this is not realising and keeps waiting logs forever
                //  maybe using persistent storage instead of memory fixes it?
                if let Some(event) = self.monitor.try_event().context("Error getting event")? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_event(&event) {
                            error!("Error processing event {:?}: {:?}", event, e);
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - if block monitor restarted, this is not realising and keeps waiting blocks forever
                //  if block monitor restarted, this is not realising and keeps waiting logs forever
                //  maybe using persistent storage instead of memory fixes it?
                if let Some(block) = self.monitor.try_block().context("Error getting block")? {
                    self.send_bitvmx_ping(&block);

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

    fn send_bitvmx_ping(&mut self, block: &RskBlockAndUncles) {
        let result = self.bitvmx_ping_pong_processor.process_new_block(&block);
        if result.is_err() {
            // TODO for now just log it, but we should handle it properly
            error!("Error checking BitVMX status : {:?}", result);
        }
    }

    fn check_bitvmx_pong(&mut self, event: &OutgoingBitVMXApiMessages) {
        let result = self
            .bitvmx_ping_pong_processor
            .process_new_bitvmx_event(event);
        if result.is_err() {
            // TODO for now just log it, but we should handle it properly
            error!("Error checking BitVMX Pong : {:?}", result);
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
    use bitvmx_client::types::OutgoingBitVMXApiMessages;
    use common::{
        shutdown_flag::ShutdownFlag,
        test_utils::rsk_block_generator::{
            create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
        },
        types::RskBlockAndUncles,
    };
    use mockall::mock;
    use std::{
        thread::{self, JoinHandle, sleep},
        time::Duration,
    };
    use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
    use transaction_dispatcher::types::{
        AcceptPegInInput, AcceptPegInOutput, PegInAddressInput, PegInAddressOutput,
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
        });

        let event_2: RskPegManagerEvents = RskPegManagerEvents::AdvanceFunds(AdvanceFundsEvent {
            inner: create_fake_advance_funds_event("peg_out_id_1"),
            block_number: block_2.number(),
            block_hash: block_2.hash().into(),
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

        let mut ping_pong_processor = MockEventProcessor::new();

        ping_pong_processor
            .expect_process_new_bitvmx_event()
            .returning(|_| Ok(()))
            .times(1..);

        ping_pong_processor
            .expect_process_new_block()
            .returning(|_| Ok(()))
            .times(1..);

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            Box::new(ping_pong_processor),
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

        let mut ping_pong_processor = MockEventProcessor::new();

        ping_pong_processor
            .expect_process_new_bitvmx_event()
            .returning(|_| Ok(()))
            .times(1..);

        ping_pong_processor
            .expect_process_new_block()
            .returning(|_| Ok(()))
            .times(1..);

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            Box::new(ping_pong_processor),
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
            .expect_process_new_event()
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

            async fn notify_check_fork_completion(
                &self,
                input: &str,
            ) -> Result<(), DomainErrors>;
        }
    }
}
