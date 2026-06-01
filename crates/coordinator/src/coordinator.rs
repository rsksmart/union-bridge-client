use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bitcoin::Network;
use common::msg_broker::bitvmx_types::IncomingBitVMXApiMessages;
use common::msg_broker::broker::{
    BitVmxBrokerClientApi, UnionBrokerClientApi, is_recoverable_transport_error,
};
use common::msg_broker::types::ToServer;
use common::runtime_sync::RuntimeSync;
use common::shutdown_flag::ShutdownFlag;
use tracing::{error, info, instrument, trace, warn};
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;

use crate::RUNTIME_ENV_LOCAL_ANVIL;
use crate::event_processor::EventProcessor;
use crate::flows::committee::setup_committee_flow::SetupCommitteeFlowFactory;
use crate::flows::committee::setup_committee_processor::{
    SetupCommitteeProcessor, send_union_settings,
};
use crate::flows::common::native_bridge_verifier::NativeBridgeVerifier;
use crate::flows::common::{GlobalContext, Signaling};
use crate::flows::funding_info_flow::FundingInfoProcessor;
use crate::flows::operator_take::AdvanceFundsFlowProcessor;
use crate::flows::pegin::pegin_processor::{PeginFlowProcessor, subscribe_to_bitvmx_pegin_events};
use crate::flows::pegout::pegout_processor::PegoutFlowProcessor;
use crate::monitor::MonitorApi;
use crate::store::CoordinatorStoreApi;
pub struct Coordinator<
    M: MonitorApi,
    BC: BitVmxBrokerClientApi,
    UC: UnionBrokerClientApi,
    S: CoordinatorStoreApi,
> {
    monitor: M,
    bitvmx_broker: Rc<BC>,
    user_broker: UC,
    processors: Vec<Box<dyn EventProcessor>>,
    user_reply_rx: Receiver<ToServer>,
    check_period: Duration,
    bitvmx_not_responding_threshold: Duration,
    bitvmx_ping_after_silence: Duration,
    btc_confirmations: u32,
    runtime_broker_recovery: RuntimeBrokerRecovery,
    store: Rc<S>,
    global_context: GlobalContext,
    shutdown_flag: ShutdownFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BitvmxLiveness {
    Unknown,
    Healthy,
    NotResponding,
}

#[derive(Debug, Default)]
struct RuntimeBrokerRecovery {
    internal_subscriptions_pending: bool,
    bitvmx_session_pending: bool,
}

#[derive(Debug, Clone, Copy)]
enum RuntimeBrokerRecoveryScope {
    InternalSubscriptions,
    BitvmxSession,
}

impl RuntimeBrokerRecovery {
    fn mark_pending(&mut self, scope: RuntimeBrokerRecoveryScope) {
        match scope {
            RuntimeBrokerRecoveryScope::InternalSubscriptions => {
                self.internal_subscriptions_pending = true;
            }
            RuntimeBrokerRecoveryScope::BitvmxSession => self.bitvmx_session_pending = true,
        }
    }

    fn has_pending(&self) -> bool {
        self.internal_subscriptions_pending || self.bitvmx_session_pending
    }
}

fn uses_fake_native_bridge(runtime_environment: &str) -> bool {
    runtime_environment.eq_ignore_ascii_case(RUNTIME_ENV_LOCAL_ANVIL)
}

/// Log the active-flows summary once every N RSK blocks the coordinator ingests.
/// Tying cadence to block progress means silence in the log signals stalled ingestion.
/// At ~30s per RSK block, 10 blocks ≈ 5 minutes.
const ACTIVE_FLOWS_LOG_EVERY_N_BLOCKS: u32 = 10;

impl<
    M: MonitorApi,
    BC: BitVmxBrokerClientApi + 'static,
    UC: UnionBrokerClientApi,
    S: CoordinatorStoreApi + 'static,
> Coordinator<M, BC, UC, S>
{
    fn build_native_bridge_verifier<CG: RskContractsGatewayApi + 'static>(
        runtime_environment: &str,
        contracts_arc: &Rc<CG>,
        rt_sync: &RuntimeSync,
        btc_confirmations: u32,
        btc_confirmations_buffer: u32,
    ) -> NativeBridgeVerifier<CG> {
        if uses_fake_native_bridge(runtime_environment) {
            info!(
                "Environment: {runtime_environment} → Using Dummy Native Bridge Verifier (BitVMX confirmations only)"
            );
            NativeBridgeVerifier::Dummy
        } else {
            info!("Environment: {runtime_environment} → Using Real Native Bridge Verifier");
            let required_confirmations = btc_confirmations.saturating_add(btc_confirmations_buffer);
            NativeBridgeVerifier::real(
                contracts_arc.clone(),
                rt_sync.clone(),
                required_confirmations,
            )
        }
    }

    /// # Panics
    /// Panics if loading context from the database fails.
    #[allow(clippy::too_many_arguments)]
    pub fn new<CG: RskContractsGatewayApi + 'static>(
        rt_sync: &RuntimeSync,
        monitor: M,
        contracts_gateway: CG,
        bitvmx_broker: &Rc<BC>,
        user_broker: UC,
        store: S,
        shutdown_flag: ShutdownFlag,
        bitcoin_network: Network,
        runtime_environment: &str,
        check_period: Duration,
        bitvmx_not_responding_threshold: Duration,
        bitvmx_ping_after_silence: Duration,
        rsk_confirmations: u32,
        btc_confirmations: u32,
        btc_status_retry_blocks: u32,
        pegout_advance_funds_timeout_secs: u64,
        committee_drp_program_definition: String,
        native_bridge_btc_confirmations_buffer: u32,
        completion_marker_root: &str,
    ) -> Self {
        let contracts_arc = Rc::new(contracts_gateway);
        let store_rc = Rc::new(store);

        let global_context =
            store_rc.load_context().expect("Failed to load context from DB").unwrap_or_else(|| {
                warn!("No context found in DB, starting with empty one");
                GlobalContext::new()
            });
        let signaling = Rc::new(Signaling::new(completion_marker_root, runtime_environment));

        let setup_committee_flow_factory = SetupCommitteeFlowFactory::new(
            Rc::clone(&contracts_arc),
            rt_sync.clone(),
            bitvmx_broker.clone(),
            global_context.clone(),
            bitcoin_network,
            Rc::clone(&store_rc),
            committee_drp_program_definition,
            btc_confirmations,
            signaling.clone(),
        );

        let native_bridge_verifier = Self::build_native_bridge_verifier(
            runtime_environment,
            &contracts_arc,
            rt_sync,
            btc_confirmations,
            native_bridge_btc_confirmations_buffer,
        );
        let (user_reply_tx, user_reply_rx) = mpsc::channel();

        let processors: Vec<Box<dyn EventProcessor>> = vec![
            Box::new(PeginFlowProcessor::new(
                Rc::clone(&contracts_arc),
                rt_sync.clone(),
                bitvmx_broker.clone(),
                global_context.clone(),
                &store_rc,
                signaling.clone(),
                native_bridge_verifier.clone(),
                rsk_confirmations,
                btc_confirmations,
                btc_status_retry_blocks,
            )),
            Box::new(
                PegoutFlowProcessor::restore_or_new(
                    contracts_arc.clone(),
                    rt_sync.clone(),
                    bitvmx_broker.clone(),
                    global_context.clone(),
                    &store_rc,
                    signaling.clone(),
                    native_bridge_verifier.clone(),
                    pegout_advance_funds_timeout_secs,
                    rsk_confirmations,
                    btc_confirmations,
                    btc_status_retry_blocks,
                    Some(runtime_environment),
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
                signaling,
                rsk_confirmations,
                btc_status_retry_blocks,
                native_bridge_verifier.clone(),
            )),
            Box::new(SetupCommitteeProcessor::new(
                setup_committee_flow_factory,
                global_context.clone(),
                &store_rc,
                bitvmx_broker.as_ref(),
                rsk_confirmations,
            )),
            Box::new(FundingInfoProcessor::new(
                bitvmx_broker.clone(),
                contracts_arc.clone(),
                bitcoin_network,
                user_reply_tx,
            )),
        ];

        Self {
            monitor,
            bitvmx_broker: bitvmx_broker.clone(),
            user_broker,
            processors,
            user_reply_rx,
            check_period,
            bitvmx_not_responding_threshold,
            bitvmx_ping_after_silence,
            btc_confirmations,
            runtime_broker_recovery: RuntimeBrokerRecovery::default(),
            shutdown_flag,
            store: store_rc,
            global_context: global_context.clone(),
        }
    }

    pub fn new_for_tests(
        monitor: M,
        bitvmx_broker: BC,
        user_broker: UC,
        processors: Vec<Box<dyn EventProcessor>>,
        shutdown_flag: ShutdownFlag,
        store: S,
    ) -> Self {
        Self {
            monitor,
            bitvmx_broker: Rc::new(bitvmx_broker),
            user_broker,
            processors,
            user_reply_rx: mpsc::channel().1,
            check_period: Duration::from_millis(1),
            bitvmx_not_responding_threshold: Duration::from_secs(30),
            bitvmx_ping_after_silence: Duration::from_secs(15),
            btc_confirmations: 0,
            runtime_broker_recovery: RuntimeBrokerRecovery::default(),
            store: Rc::new(store),
            global_context: GlobalContext::new(),
            shutdown_flag,
        }
    }

    /// # Errors
    /// Returns an error if the coordinator run loop fails.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip_all)]
    pub fn run(&mut self) -> Result<()> {
        self.monitor.start_event_monitoring().context("Failed to start event monitoring")?;

        self.monitor.start_block_monitoring().context("Failed to start block monitoring")?;

        self.monitor
            .start_bitvmx_monitoring()
            .context("Failed to start BitVMX event monitoring")?;

        self.monitor.start_user_monitoring().context("Failed to start User request monitoring")?;

        let mut bitvmx_last_msg =
            Instant::now().checked_sub(self.bitvmx_ping_after_silence).unwrap_or_else(Instant::now);
        let mut bitvmx_ping: Option<Instant> = None;
        let mut bitvmx_liveness = BitvmxLiveness::Unknown;
        let mut blocks_since_active_flows_log: u32 = 0;

        let result = (|| -> Result<()> {
            loop {
                if !self.is_running() {
                    break;
                }

                self.check_bitvmx_liveness(&mut bitvmx_ping, &mut bitvmx_liveness, bitvmx_last_msg);
                self.recover_runtime_broker_state()
                    .context("Error recovering runtime broker state")?;

                let mut message_received = false;

                let user_request_result =
                    self.monitor.try_user_request().context("Error getting User request");
                if let Some(req) = self.handle_runtime_broker_result(
                    user_request_result,
                    None,
                    "getting User request",
                )? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_user_request(&req) {
                            error!("Error processing User request {req:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                // check if we have received a reply from the user
                while let Ok(reply) = self.user_reply_rx.try_recv() {
                    self.send_user_reply(reply).context("Error sending user reply")?;
                    message_received = true;
                }

                let bitvmx_event_result =
                    self.monitor.try_bitvmx_event().context("Error getting BitVMX event");
                if let Some(event) = self.handle_runtime_broker_result(
                    bitvmx_event_result,
                    Some(RuntimeBrokerRecoveryScope::BitvmxSession),
                    "getting BitVMX event",
                )? {
                    bitvmx_ping = None;
                    bitvmx_last_msg = Instant::now();
                    Self::record_bitvmx_activity(&mut bitvmx_liveness);

                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_bitvmx_event(&event) {
                            error!("Error processing BitVMX event {event:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                let rsk_event_result = self.monitor.try_rsk_event().context("Error getting event");
                if let Some(event) = self.handle_runtime_broker_result(
                    rsk_event_result,
                    Some(RuntimeBrokerRecoveryScope::InternalSubscriptions),
                    "getting RSK event",
                )? {
                    // each processor decides if the event is relevant
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_rsk_event(&event) {
                            error!("Error processing Union Bridge event {event:?}: {e:?}");
                        }
                    });

                    // no sleep, try to get new messages asap
                    message_received = true;
                }

                let block_result = self.monitor.try_block().context("Error getting block");
                if let Some(block) = self.handle_runtime_broker_result(
                    block_result,
                    Some(RuntimeBrokerRecoveryScope::InternalSubscriptions),
                    "getting block",
                )? {
                    self.processors.iter_mut().for_each(|p| {
                        if let Err(e) = p.process_new_block(&block) {
                            error!("Error processing block {block:?}: {e:?}");
                        }
                    });

                    blocks_since_active_flows_log += 1;
                    if blocks_since_active_flows_log >= ACTIVE_FLOWS_LOG_EVERY_N_BLOCKS {
                        self.log_active_flows();
                        blocks_since_active_flows_log = 0;
                    }

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

    fn handle_runtime_broker_result<T>(
        &mut self,
        result: Result<Option<T>>,
        recovery_scope: Option<RuntimeBrokerRecoveryScope>,
        operation: &'static str,
    ) -> Result<Option<T>> {
        match result {
            Ok(value) => Ok(value),
            Err(error) if Self::is_recoverable_broker_error(&error) => {
                if let Some(scope) = recovery_scope {
                    warn!(
                        ?error,
                        operation,
                        "Recoverable broker transport error; scheduling runtime broker recovery"
                    );
                    self.runtime_broker_recovery.mark_pending(scope);
                } else {
                    warn!(?error, operation, "Recoverable broker transport error; continuing");
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn recover_runtime_broker_state(&mut self) -> Result<()> {
        if !self.runtime_broker_recovery.has_pending() {
            return Ok(());
        }

        if self.runtime_broker_recovery.internal_subscriptions_pending {
            let result = self
                .monitor
                .recover_active_subscriptions()
                .context("Recovering active broker subscriptions");
            match result {
                Ok(()) => {
                    self.runtime_broker_recovery.internal_subscriptions_pending = false;
                }
                Err(error) if Self::is_recoverable_broker_error(&error) => {
                    warn!(
                        ?error,
                        "Recovering active broker subscriptions failed with a recoverable broker error"
                    );
                }
                Err(error) => return Err(error),
            }
        }

        if self.runtime_broker_recovery.bitvmx_session_pending {
            let result = self.recover_bitvmx_session().context("Recovering BitVMX session state");
            match result {
                Ok(()) => {
                    self.runtime_broker_recovery.bitvmx_session_pending = false;
                }
                Err(error) if Self::is_recoverable_broker_error(&error) => {
                    warn!(
                        ?error,
                        "Recovering BitVMX session state failed with a recoverable broker error"
                    );
                }
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }

    fn recover_bitvmx_session(&self) -> Result<()> {
        subscribe_to_bitvmx_pegin_events(self.bitvmx_broker.as_ref(), self.btc_confirmations)
            .context("Broker error on SubscribeToRskPegin recovery")?;

        send_union_settings(self.bitvmx_broker.as_ref())
            .context("Broker error on UnionSettings")?;

        Ok(())
    }

    fn is_recoverable_broker_error(error: &anyhow::Error) -> bool {
        error.chain().any(is_recoverable_transport_error)
    }

    fn check_bitvmx_liveness(
        &mut self,
        bitvmx_ping: &mut Option<Instant>,
        bitvmx_liveness: &mut BitvmxLiveness,
        bitvmx_last_msg: Instant,
    ) {
        if let Some(ping) = bitvmx_ping
            && ping.elapsed() > self.bitvmx_not_responding_threshold
        {
            if *bitvmx_liveness != BitvmxLiveness::NotResponding {
                error!("BitVMX is not responding: ping timed out after {:?}", ping.elapsed());
                *bitvmx_liveness = BitvmxLiveness::NotResponding;
                self.runtime_broker_recovery
                    .mark_pending(RuntimeBrokerRecoveryScope::BitvmxSession);
            }
            *bitvmx_ping = None;
        }

        // send ping if we have not received any message from BitVMX for a while and there is no pending ping
        if bitvmx_last_msg.elapsed() > self.bitvmx_ping_after_silence && bitvmx_ping.is_none() {
            self.send_bitvmx_ping();
            *bitvmx_ping = Some(Instant::now());
        }
    }

    fn send_bitvmx_ping(&mut self) {
        let ping_id = uuid::Uuid::new_v4();
        trace!("Sending Ping to BitVMX with uuid: {ping_id}");

        match self.bitvmx_broker.send(IncomingBitVMXApiMessages::Ping(ping_id)) {
            Ok(true) => {}
            Ok(false) => {
                warn!("Broker could not deliver BitVMX ping; scheduling BitVMX recovery");
                self.runtime_broker_recovery
                    .mark_pending(RuntimeBrokerRecoveryScope::BitvmxSession);
            }
            Err(error) if is_recoverable_transport_error(&error) => {
                warn!(
                    ?error,
                    "Recoverable broker error sending BitVMX ping; scheduling BitVMX recovery"
                );
                self.runtime_broker_recovery
                    .mark_pending(RuntimeBrokerRecoveryScope::BitvmxSession);
            }
            Err(error) => {
                error!("Failed to send Ping to BitVMX: {error:?}");
            }
        }
    }

    fn record_bitvmx_activity(bitvmx_liveness: &mut BitvmxLiveness) {
        match bitvmx_liveness {
            BitvmxLiveness::Unknown => {
                info!("BitVMX is responsive");
            }
            BitvmxLiveness::NotResponding => {
                info!("BitVMX is back online");
            }
            BitvmxLiveness::Healthy => {}
        }
        *bitvmx_liveness = BitvmxLiveness::Healthy;
    }

    fn is_running(&self) -> bool {
        !self.shutdown_flag.is_on()
    }

    fn send_user_reply(&mut self, reply: ToServer) -> Result<()> {
        // A transient user-api disconnect (or backpressure) should not take down
        // the coordinator. We log and drop the reply; the user-api side has its
        // own request timeout and will surface the failure to the caller.
        match self.user_broker.send(reply) {
            Ok(true) => {}
            Ok(false) => {
                warn!("Broker could not deliver user reply; dropping reply");
            }
            Err(error) if is_recoverable_transport_error(&error) => {
                warn!(?error, "Recoverable broker error sending user reply; dropping reply");
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn log_active_flows(&self) {
        let flows: Vec<_> = self.processors.iter().flat_map(|p| p.active_flows()).collect();
        let payload = serde_json::json!({ "count": flows.len(), "flows": flows });
        info!("active_flows {payload}");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::thread::{self, JoinHandle, sleep};
    use std::time::{Duration, Instant};

    use anyhow::anyhow;
    use common::msg_broker::bitvmx_types::{
        GLOBAL_SETTINGS_UUID, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages, UnionSettings,
        VariableTypes,
    };
    use common::msg_broker::broker::{BrokerError, MockBrokerClientApi};
    use common::msg_broker::types::{FromServer, ToServer};
    use common::shutdown_flag::ShutdownFlag;
    use common::test_utils::rsk_block_generator::{
        create_block_and_uncles, get_first_default_rsk_block, get_second_default_rsk_block,
    };
    use common::types;
    use common::types::RskBlockAndUncles;
    use mockall::mock;
    use mockall::predicate::{always, function};
    use transaction_dispatcher::rsk_gateway::{DomainErrors, RskContractsGatewayApi};
    use transaction_dispatcher::types::{
        AcceptPeginInput, AcceptPeginOutput, AddMemberNonceInput, AddMemberNonceOutput,
        AddMemberSignatureInput, AddMemberSignatureOutput, AddOperatorTakeTxHashInput,
        AddOperatorTakeTxHashOutput, ApplyToStreamInput, ApplyToStreamOutput,
        DepositAggregatedKeyInput, DepositAggregatedKeyOutput, DepositCommunicationDataInput,
        DepositCommunicationDataOutput, GetAcceptPeginTxidInput, GetAcceptPeginTxidOutput,
        GetBtcTransactionConfirmationsInput, GetBtcTransactionConfirmationsOutput,
        GetCommitteeInput, GetCommitteeOutput, GetCommunicationDataInput,
        GetCommunicationDataOutput, GetMemberPublicKeysInput, GetMemberPublicKeysOutput,
        PeginAddressInput, PeginAddressOutput, RegisterAdvanceFundsInput,
        RegisterAdvanceFundsOutput, RegisterChallengeInput, RegisterChallengeOutput,
        RegisterInputRevealedInput, RegisterInputRevealedOutput, RegisterOperatorTakeInput,
        RegisterOperatorTakeOutput, RegisterOperatorWonInput, RegisterOperatorWonOutput,
        RegisterPegoutInput, RegisterPegoutOutput, RegisterReimbursementKickoffInput,
        RegisterReimbursementKickoffOutput, RequestPeginInput, RequestPeginOutput,
        RequestPegoutInput, RequestPegoutOutput, TriggerOperatorTakeInput,
        TriggerOperatorTakeOutput,
    };

    use crate::coordinator::Coordinator;
    use crate::event_processor::{EventProcessor, MockEventProcessor};
    use crate::monitor::MockMonitorApi;
    use crate::store::MockCoordinatorStoreApi;
    use crate::types::RskPegManagerEvents;
    use crate::{RUNTIME_ENV_LOCAL_ANVIL, RUNTIME_ENV_LOCAL_RSKJ};

    type MockUnionBroker = MockBrokerClientApi<ToServer, FromServer>;
    type MockBitVmxBroker =
        MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>;

    #[test]
    fn test_uses_fake_native_bridge_only_for_anvil_tier() {
        assert!(super::uses_fake_native_bridge(RUNTIME_ENV_LOCAL_ANVIL));
        assert!(super::uses_fake_native_bridge("LOCAL-ANVIL")); // verifies case-insensitivity

        assert!(!super::uses_fake_native_bridge(RUNTIME_ENV_LOCAL_RSKJ));
        assert!(!super::uses_fake_native_bridge("regtest"));
        assert!(!super::uses_fake_native_bridge("alphanet"));
        assert!(!super::uses_fake_native_bridge("testnet"));
    }

    #[test]
    fn test_coordinator_run_handles_several_events() {
        let mut mock_monitor = MockMonitorApi::new();
        let (block_1, uncle_1, block_2) = create_block_and_uncles();

        let event_1 = RskPegManagerEvents::UnknownEvent;
        let event_2 = RskPegManagerEvents::IgnoredEvent;

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
        let mock_user_broker = MockBrokerClientApi::<ToServer, FromServer>::new();

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            mock_user_broker,
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

        let event_1 = RskPegManagerEvents::IgnoredEvent;
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
        let mock_user_broker = MockBrokerClientApi::<ToServer, FromServer>::new();

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            mock_user_broker,
            generate_ok_processors(),
            shutdown_flag,
            mock_store,
        );
        let result = coordinator.run();

        assert!(result.is_ok());
    }

    /// Feeds exactly `ACTIVE_FLOWS_LOG_EVERY_N_BLOCKS` blocks and verifies
    /// that `active_flows()` is invoked on each processor exactly once —
    /// guarding against off-by-one drift in the block-counter threshold and
    /// catching any future regression where the aggregation hook stops being
    /// called.
    #[test]
    fn test_coordinator_logs_active_flows_after_threshold_blocks() {
        let n_blocks: usize =
            super::ACTIVE_FLOWS_LOG_EVERY_N_BLOCKS.try_into().expect("threshold fits in usize");

        let mut mock_monitor = MockMonitorApi::new();

        mock_monitor.expect_start_event_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_start_bitvmx_monitoring().times(..).returning(|| Ok(()));
        mock_monitor.expect_start_block_monitoring().times(..).returning(|| Ok(()));
        mock_monitor.expect_start_user_monitoring().times(..).returning(|| Ok(()));
        mock_monitor.expect_cancel_event_monitoring().return_once(|| Ok(())).once();
        mock_monitor.expect_cancel_block_monitoring().return_once(|| Ok(())).once();
        mock_monitor.expect_cancel_bitvmx_monitoring().return_once(|| Ok(())).once();
        mock_monitor.expect_try_bitvmx_event().returning(|| Ok(None));
        mock_monitor.expect_try_rsk_event().returning(|| Ok(None));
        mock_monitor.expect_try_user_request().returning(|| Ok(None));

        let blocks: Vec<_> = (0..n_blocks)
            .map(|_| RskBlockAndUncles::new_no_uncles(get_first_default_rsk_block()))
            .collect();
        expect_try_block(blocks, &mut mock_monitor);

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker =
            MockBrokerClientApi::<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages>::new();
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::Ping(_))
            }))
            .returning(|_| Ok(true));

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_context().with(always()).returning(|_| Ok(()));
        let mock_user_broker = MockBrokerClientApi::<ToServer, FromServer>::new();

        let mut processor = MockEventProcessor::new();
        processor.expect_process_new_block().returning(|_| Ok(())).times(n_blocks);
        processor.expect_process_new_bitvmx_event().returning(|_| Ok(())).times(..);
        processor.expect_process_new_rsk_event().returning(|_| Ok(())).times(..);
        processor.expect_shutdown().return_once(|| ());
        // The key assertion: aggregation fires exactly once for `n_blocks`.
        processor.expect_active_flows().returning(Vec::new).times(1);

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            mock_user_broker,
            vec![Box::new(processor)],
            shutdown_flag,
            mock_store,
        );

        assert!(coordinator.run().is_ok());
    }

    #[test]
    fn test_coordinator_run_catches_recoverable_bitvmx_receive_error_and_recovers() {
        let mut mock_monitor = MockMonitorApi::new();
        expect_monitor_start_stop_ok(&mut mock_monitor);
        mock_monitor.expect_try_user_request().returning(|| Ok(None));
        mock_monitor.expect_try_bitvmx_event().returning_st({
            let mut calls = 0_u8;
            move || {
                calls = calls.saturating_add(1);
                if calls == 1 { Err(BrokerError::disconnected().into()) } else { Ok(None) }
            }
        });
        mock_monitor.expect_try_rsk_event().returning(|| Ok(None));
        mock_monitor.expect_try_block().returning(|| Ok(None));

        let shutdown_flag = ShutdownFlag::init();
        handle_shutdown(shutdown_flag.clone());

        let mut bitvmx_broker = MockBitVmxBroker::new();
        expect_bitvmx_ping(&mut bitvmx_broker, 1);
        expect_bitvmx_session_recovery(&mut bitvmx_broker, 7);

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_context().with(always()).returning(|_| Ok(()));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            MockUnionBroker::new(),
            generate_idle_processors(),
            shutdown_flag,
            mock_store,
        );
        coordinator.btc_confirmations = 7;

        assert!(coordinator.run().is_ok());
    }

    #[test]
    fn test_coordinator_run_propagates_non_recoverable_receive_errors() {
        let mut mock_monitor = MockMonitorApi::new();
        expect_monitor_start_stop_ok(&mut mock_monitor);
        mock_monitor.expect_try_user_request().returning(|| Ok(None));
        mock_monitor
            .expect_try_bitvmx_event()
            .return_once(|| Err(BrokerError::UnknownError(anyhow!("serialization failed")).into()));

        let mut bitvmx_broker = MockBitVmxBroker::new();
        expect_bitvmx_ping(&mut bitvmx_broker, 1);

        let mut mock_store = MockCoordinatorStoreApi::new();
        mock_store.expect_save_context().with(always()).returning(|_| Ok(()));

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            MockUnionBroker::new(),
            generate_idle_processors(),
            ShutdownFlag::init(),
            mock_store,
        );

        let result = coordinator.run();

        assert!(result.is_err());
    }

    #[test]
    fn test_bitvmx_liveness_recovery_replays_subscription_and_union_settings() {
        let mut bitvmx_broker = MockBitVmxBroker::new();
        expect_bitvmx_session_recovery(&mut bitvmx_broker, 42);

        let mut coordinator = Coordinator::new_for_tests(
            MockMonitorApi::new(),
            bitvmx_broker,
            MockUnionBroker::new(),
            vec![],
            ShutdownFlag::init(),
            MockCoordinatorStoreApi::new(),
        );
        coordinator.btc_confirmations = 42;

        let mut bitvmx_ping = Some(
            Instant::now()
                .checked_sub(coordinator.bitvmx_not_responding_threshold + Duration::from_secs(1))
                .expect("threshold fits"),
        );
        let mut bitvmx_liveness = super::BitvmxLiveness::Healthy;

        coordinator.check_bitvmx_liveness(&mut bitvmx_ping, &mut bitvmx_liveness, Instant::now());

        assert!(coordinator.runtime_broker_recovery.bitvmx_session_pending);
        assert!(coordinator.recover_runtime_broker_state().is_ok());
        assert!(!coordinator.runtime_broker_recovery.bitvmx_session_pending);
    }

    #[test]
    fn test_bitvmx_recovery_keeps_pending_on_recoverable_send_failure() {
        let mut bitvmx_broker = MockBitVmxBroker::new();
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::SubscribeToRskPegin(Some(3)))
            }))
            .return_once(|_| Err(BrokerError::disconnected()));

        let mut coordinator = Coordinator::new_for_tests(
            MockMonitorApi::new(),
            bitvmx_broker,
            MockUnionBroker::new(),
            vec![],
            ShutdownFlag::init(),
            MockCoordinatorStoreApi::new(),
        );
        coordinator.btc_confirmations = 3;
        coordinator
            .runtime_broker_recovery
            .mark_pending(super::RuntimeBrokerRecoveryScope::BitvmxSession);

        assert!(coordinator.recover_runtime_broker_state().is_ok());
        assert!(coordinator.runtime_broker_recovery.bitvmx_session_pending);
    }

    #[test]
    fn test_recovery_attempts_independent_pending_scopes_after_recoverable_error() {
        let mut mock_monitor = MockMonitorApi::new();
        mock_monitor
            .expect_recover_active_subscriptions()
            .return_once(|| Err(BrokerError::disconnected().into()));

        let mut bitvmx_broker = MockBitVmxBroker::new();
        expect_bitvmx_session_recovery(&mut bitvmx_broker, 5);

        let mut coordinator = Coordinator::new_for_tests(
            mock_monitor,
            bitvmx_broker,
            MockUnionBroker::new(),
            vec![],
            ShutdownFlag::init(),
            MockCoordinatorStoreApi::new(),
        );
        coordinator.btc_confirmations = 5;
        coordinator
            .runtime_broker_recovery
            .mark_pending(super::RuntimeBrokerRecoveryScope::InternalSubscriptions);
        coordinator
            .runtime_broker_recovery
            .mark_pending(super::RuntimeBrokerRecoveryScope::BitvmxSession);

        assert!(coordinator.recover_runtime_broker_state().is_ok());
        assert!(coordinator.runtime_broker_recovery.internal_subscriptions_pending);
        assert!(!coordinator.runtime_broker_recovery.bitvmx_session_pending);
    }

    #[test]
    fn test_bitvmx_recovery_propagates_non_recoverable_send_failure() {
        let mut bitvmx_broker = MockBitVmxBroker::new();
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::SubscribeToRskPegin(Some(3)))
            }))
            .return_once(|_| Err(BrokerError::UnknownError(anyhow!("bad message"))));

        let mut coordinator = Coordinator::new_for_tests(
            MockMonitorApi::new(),
            bitvmx_broker,
            MockUnionBroker::new(),
            vec![],
            ShutdownFlag::init(),
            MockCoordinatorStoreApi::new(),
        );
        coordinator.btc_confirmations = 3;
        coordinator
            .runtime_broker_recovery
            .mark_pending(super::RuntimeBrokerRecoveryScope::BitvmxSession);

        assert!(coordinator.recover_runtime_broker_state().is_err());
        assert!(coordinator.runtime_broker_recovery.bitvmx_session_pending);
    }

    #[test]
    fn test_send_user_reply_drops_recoverable_send_errors() {
        let mut user_broker = MockUnionBroker::new();
        user_broker.expect_send().return_once(|_| Err(BrokerError::disconnected()));

        let mut coordinator = Coordinator::new_for_tests(
            MockMonitorApi::new(),
            MockBitVmxBroker::new(),
            user_broker,
            vec![],
            ShutdownFlag::init(),
            MockCoordinatorStoreApi::new(),
        );

        assert!(coordinator.send_user_reply(ToServer::SubscribeBlocks).is_ok());
        assert!(!coordinator.runtime_broker_recovery.has_pending());
    }

    fn expect_monitor_start_stop_ok(mock_monitor: &mut MockMonitorApi) {
        mock_monitor.expect_start_event_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_start_block_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_start_bitvmx_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_start_user_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_cancel_event_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_cancel_block_monitoring().return_once(|| Ok(()));
        mock_monitor.expect_cancel_bitvmx_monitoring().return_once(|| Ok(()));
    }

    fn expect_bitvmx_ping(bitvmx_broker: &mut MockBitVmxBroker, times: usize) {
        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(req, IncomingBitVMXApiMessages::Ping(_))
            }))
            .times(times)
            .returning(|_| Ok(true));
    }

    fn expect_bitvmx_session_recovery(
        bitvmx_broker: &mut MockBitVmxBroker,
        btc_confirmations: u32,
    ) {
        bitvmx_broker
            .expect_send()
            .with(function(move |req: &IncomingBitVMXApiMessages| {
                matches!(
                    req,
                    IncomingBitVMXApiMessages::SubscribeToRskPegin(Some(confirmations))
                        if *confirmations == btc_confirmations
                )
            }))
            .return_once(|_| Ok(true));

        bitvmx_broker
            .expect_send()
            .with(function(|req: &IncomingBitVMXApiMessages| {
                matches!(
                    req,
                    IncomingBitVMXApiMessages::SetVar(
                        uuid,
                        name,
                        VariableTypes::String(payload)
                    ) if *uuid == GLOBAL_SETTINGS_UUID
                        && *name == UnionSettings::name()
                        && serde_json::from_str::<UnionSettings>(payload).is_ok()
                )
            }))
            .return_once(|_| Ok(true));
    }

    fn generate_idle_processors() -> Vec<Box<dyn EventProcessor>> {
        let mut processor = MockEventProcessor::new();
        processor.expect_shutdown().return_once(|| ());
        vec![Box::new(processor)]
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

            async fn get_btc_confirmations(
                &self,
                input: GetBtcTransactionConfirmationsInput,
            ) -> Result<GetBtcTransactionConfirmationsOutput, DomainErrors>;

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

            async fn register_advance_funds(
                &self,
                input: RegisterAdvanceFundsInput,
            ) -> Result<RegisterAdvanceFundsOutput, DomainErrors>;

            async fn get_accept_pegin_txid(
                &self,
                input: GetAcceptPeginTxidInput,
            ) -> Result<GetAcceptPeginTxidOutput, DomainErrors>;

            async fn register_reimbursement_kickoff(
                &self,
                input: RegisterReimbursementKickoffInput,
            ) -> Result<RegisterReimbursementKickoffOutput, DomainErrors>;

            async fn is_whitelisted(&self) -> Result<bool, DomainErrors>;

            async fn get_minimum_deposit(
                &self,
                stream_id: u8,
                role: u8,
            ) -> Result<alloy_primitives::Uint<256, 4>, DomainErrors>;
        }
    }
}
