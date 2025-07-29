use crate::blockchain_tracker::{BlockchainView, ConfirmableEvent};
use crate::config::REQUIRED_CONFIRMATIONS;
use crate::types::BitVmxSigningInfo;
use anyhow::{Context, Result, anyhow, bail};
use common::runtime_sync::RuntimeSync;
use common::types::{BlockNumber, Hash256};
use log::info;
use std::cell::RefCell;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{AddMemberNonceInput, AddMemberSignatureInput};
use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub(crate) trait BtcSignatureLifecycleApi {
    fn flow_id(&self) -> Uuid;

    fn send_nonce_to_contracts(&mut self, data: &BitVmxSigningInfo) -> Result<()>;

    fn set_all_nonces_ready(&mut self, block_number: BlockNumber) -> Result<()>;

    fn unset_all_nonces_ready(&mut self) -> Result<()>;

    fn is_all_nonces_ready_confirmed(&self) -> Result<bool>;

    fn send_signature_to_contracts(&mut self) -> Result<()>;

    fn set_all_signatures_ready(&mut self, block_number: BlockNumber) -> Result<()>;

    fn unset_all_signatures_ready(&mut self) -> Result<()>;

    fn is_all_signatures_ready_confirmed(&self) -> Result<bool>;

    fn blockchain_view(&self) -> Rc<RefCell<BlockchainView>>;

    // TODO implement auto-clean after inactivity to cover cases where .close_flow() is not called
}

pub(crate) struct State {
    pub(crate) flow_id: Uuid,
    pub(crate) data: Option<BitVmxSigningInfo>,
    pub(crate) nonce_step: Option<ConfirmableEvent>,
    pub(crate) signature_step: Option<ConfirmableEvent>,
}

pub(crate) struct BtcSignatureLifeCycle<CG: RskContractsGatewayApi> {
    contracts: Rc<CG>,
    rt_sync: RuntimeSync,
    blockchain_view: Rc<RefCell<BlockchainView>>,
    state: State,
}

impl<CG> BtcSignatureLifeCycle<CG>
where
    CG: RskContractsGatewayApi,
{
    pub(in crate::flows::btc_signature) fn new(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        flow_id: Uuid,
    ) -> Self {
        BtcSignatureLifeCycle {
            contracts: contracts_gateway,
            rt_sync,
            blockchain_view: Rc::new(RefCell::new(BlockchainView::new())),
            state: State {
                flow_id,
                data: None,
                nonce_step: None,
                signature_step: None,
            },
        }
    }

    #[cfg(test)]
    pub(in crate::flows::btc_signature) fn new_for_tests(
        contracts_gateway: Rc<CG>,
        rt_sync: RuntimeSync,
        blockchain_view: Rc<RefCell<BlockchainView>>,
        flow_id: Uuid,
    ) -> Self {
        BtcSignatureLifeCycle {
            contracts: contracts_gateway,
            rt_sync,
            blockchain_view,
            state: State {
                flow_id,
                data: None,
                nonce_step: None,
                signature_step: None,
            },
        }
    }

    /// Modify the confirmation status of the Nonces step.
    /// - Some(block_number) => start confirming at block_number
    /// - None => stop confirming
    fn modify_nonces_confirmation_status(
        &mut self,
        block_number: Option<BlockNumber>,
    ) -> Result<()> {
        let flow_id = self.state.flow_id;
        if block_number.is_some() {
            info!("Start confirming Nonces {flow_id}");
        } else {
            info!("Stop confirming Nonces {flow_id}");
        }

        if self.state.signature_step.is_some() {
            bail!("Nonces unset received for flow {flow_id} in Signatures step");
        }

        let nonce_event = self
            .state
            .nonce_step
            .as_mut()
            .ok_or_else(|| anyhow!("flow {flow_id} is not at Nonces step"))?;

        Self::modify_event_confirmation_status(flow_id, block_number, nonce_event)
    }

    /// Modify the confirmation status of the Signatures step.
    /// - Some(block_number) => start confirming at block_number
    /// - None => stop confirming
    fn modify_signatures_confirmation_status(
        &mut self,
        block_number: Option<BlockNumber>,
    ) -> Result<()> {
        let flow_id = self.state.flow_id;
        if block_number.is_some() {
            info!("Start confirming Signatures {flow_id}");
        } else {
            info!("Stop confirming Signatures {flow_id}");
        }

        let signature_event = self
            .state
            .signature_step
            .as_mut()
            .ok_or_else(|| anyhow!("flow {flow_id} is not at Signatures step"))?;

        Self::modify_event_confirmation_status(flow_id, block_number, signature_event)
    }

    /// Modify the confirmation status of the received ConfirmableEvent.
    /// - Some(block_number) => start confirming at block_number
    /// - None => stop confirming
    fn modify_event_confirmation_status(
        flow_id: Uuid,
        block_number: Option<BlockNumber>,
        confirmable_event: &mut ConfirmableEvent,
    ) -> Result<()> {
        match block_number {
            Some(block) => confirmable_event
                .start_confirming(block)
                .with_context(|| format!("Failed to start confirming event for flow {flow_id}")),
            None => confirmable_event
                .stop_confirming()
                .with_context(|| format!("Failed to stop confirming event for flow {flow_id}")),
        }
    }
}

impl<CG> BtcSignatureLifecycleApi for BtcSignatureLifeCycle<CG>
where
    CG: RskContractsGatewayApi,
{
    fn flow_id(&self) -> Uuid {
        self.state.flow_id
    }

    fn send_nonce_to_contracts(&mut self, data: &BitVmxSigningInfo) -> Result<()> {
        info!("Sending nonce to contract for flow {}", self.state.flow_id);

        if self.state.nonce_step.is_some() || self.state.data.is_some() {
            bail!("flow {} is already in Nonces step", self.state.flow_id);
        }

        // keep track of the signature data in state
        self.state.data = Some(data.clone());

        // send the nonce

        let nonce = AddMemberNonceInput {
            hash_to_sign: data.hash_to_sign,
            nonce: data.nonce.clone(),
        };

        let send_nonce_result = self
            .rt_sync
            .run(self.contracts.add_member_nonce(nonce.clone()))
            .with_context(|| {
                format!(
                    "Failed to send nonce {nonce:?} for flow {}",
                    self.state.flow_id
                )
            })?;

        if !send_nonce_result.success {
            bail!(
                "Tx {} rejected on send nonce {nonce:?} for flow {}",
                send_nonce_result.transaction_hash,
                self.state.flow_id
            );
        }

        // move to the Nonces step
        let confirmable = ConfirmableEvent::new(
            self.state.flow_id,
            REQUIRED_CONFIRMATIONS,
            self.blockchain_view.clone(),
        );
        self.state.nonce_step = Some(confirmable);

        Ok(())
    }

    fn set_all_nonces_ready(&mut self, block_number: BlockNumber) -> Result<()> {
        self.modify_nonces_confirmation_status(Some(block_number))
    }

    fn unset_all_nonces_ready(&mut self) -> Result<()> {
        self.modify_nonces_confirmation_status(None)
    }

    fn is_all_nonces_ready_confirmed(&self) -> Result<bool> {
        let nonce_step = self
            .state
            .nonce_step
            .as_ref()
            .ok_or_else(|| anyhow!("flow {} is not at Nonces step", self.state.flow_id))?;

        Ok(nonce_step.is_confirmed())
    }

    fn send_signature_to_contracts(&mut self) -> Result<()> {
        info!(
            "Sending signatures to contract for flow {}",
            self.state.flow_id
        );

        if !self.is_all_nonces_ready_confirmed()? {
            bail!(
                "flow {} has not completed the Nonces step yet",
                self.state.flow_id
            );
        };

        if self.state.signature_step.is_some() {
            bail!("flow {} already in Signatures step", self.state.flow_id);
        }

        let member_signature = self
            .state
            .data
            .as_ref()
            .ok_or_else(|| anyhow!("Signature data missing on flow {}", self.state.flow_id))?;

        // send the signature

        let signature_input = AddMemberSignatureInput {
            hash_to_sign: member_signature.hash_to_sign,
            signature: member_signature.signature,
        };

        let send_sig_result = self
            .rt_sync
            .run(self.contracts.add_member_signature(signature_input.clone()))
            .with_context(|| {
                format!(
                    "Failed to send signature {signature_input:?} for flow {}",
                    self.state.flow_id
                )
            })?;

        if !send_sig_result.success {
            bail!(
                "Tx {} rejected on send signature {signature_input:?} for flow {}",
                send_sig_result.transaction_hash,
                self.state.flow_id
            );
        }

        // move to the Signatures step
        let confirmable = ConfirmableEvent::new(
            self.state.flow_id,
            REQUIRED_CONFIRMATIONS,
            self.blockchain_view.clone(),
        );
        self.state.signature_step = Some(confirmable);

        Ok(())
    }

    fn set_all_signatures_ready(&mut self, block_number: BlockNumber) -> Result<()> {
        self.modify_signatures_confirmation_status(Some(block_number))
    }

    fn unset_all_signatures_ready(&mut self) -> Result<()> {
        self.modify_signatures_confirmation_status(None)
    }

    fn is_all_signatures_ready_confirmed(&self) -> Result<bool> {
        let signature_step = self
            .state
            .signature_step
            .as_ref()
            .ok_or_else(|| anyhow!("flow {} is not at Signatures step", self.state.flow_id))?;

        Ok(signature_step.is_confirmed())
    }

    fn blockchain_view(&self) -> Rc<RefCell<BlockchainView>> {
        self.blockchain_view.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain_tracker::BlockchainView;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use bitcoin::PublicKey;
    use common::runtime_sync::RuntimeSync;
    use common::test_utils::rsk_block_generator::create_block_and_uncles;
    use common::types::{BlockNumber, Hash256, RskBlock, RskBlockAndUncles};
    use mockall::predicate::function;
    use musig2::{PartialSignature, PubNonce};
    use primitive_types::H256;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::str::FromStr;
    use transaction_dispatcher::types::{AddMemberNonceInput, AddMemberSignatureInput};
    use uuid::Uuid;

    #[test]
    fn test_nonce_step_with_unset() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), None); // only expect nonce calls

        // step 1: send nonce (but NOT signature)
        flow.send_nonce_to_contracts(&bitvmx_signature)
            .expect("failed to send nonce to contracts");

        // step 2: set nonces ready
        let start_block = BlockNumber::from(100);
        flow.set_all_nonces_ready(start_block)
            .expect("failed to set nonces ready");

        // verify not confirmed initially
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 3: add blocks but not enough for confirmation
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify not yet confirmed
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 4: unset nonces ready
        let result = flow.unset_all_nonces_ready();
        assert!(result.is_ok(), "failed to unset nonces ready");

        // step 5: add enough blocks to confirm
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // check that confirmations don't accumulate after unsetting nonces
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(
            !is_confirmed,
            "confirmations should not accumulate after unsetting nonces"
        );

        // step 6: set nonces ready again
        let result = flow.set_all_nonces_ready(start_block);
        assert!(result.is_ok(), "failed to set nonces ready again");

        // add enough blocks to reach confirmation for nonces
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // step 7: verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(is_confirmed, "should be confirmed after enough blocks");
    }

    #[test]
    fn test_nonce_step_happy_path() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), None); // only expect nonce calls

        // step 1: send nonce (but NOT signature)
        flow.send_nonce_to_contracts(&bitvmx_signature)
            .expect("failed to send nonce to contracts");

        // step 2: set nonces ready
        let start_block = BlockNumber::from(100);
        flow.set_all_nonces_ready(start_block)
            .expect("failed to set nonces ready");

        // verify not confirmed initially
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 3: add blocks but not enough for confirmation
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify not yet confirmed
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 4: add missing block to confirm
        let block = create_test_block(start_block + (REQUIRED_CONFIRMATIONS - 1).into());
        blockchain_view.borrow_mut().update(block);

        // verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(is_confirmed, "should be confirmed after enough blocks");
    }

    #[test]
    fn test_signature_step_without_nonce() {
        let (_signature_json, mut flow, _) = setup_test_flow_with_options(None, None); // no contract calls expected

        // step 2: send signatures to contract
        let result = flow.send_signature_to_contracts();

        // verify the Nonces step is not complete yet
        assert!(
            result.is_err(),
            "should fail when nonces step is not complete"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("is not at Nonces step"),
            "error should mention flow is not at Nonces step"
        );
    }

    #[test]
    fn test_signature_step_happy_path() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), Some(1)); // expect both nonce and signature calls

        // step 2: complete Nonces step first
        let start_block = BlockNumber::from(100);
        complete_nonce_step(&mut flow, &bitvmx_signature, start_block, &blockchain_view)
            .expect("failed to complete Nonces step");

        // step 3: send signatures to contract
        flow.send_signature_to_contracts()
            .expect("failed to send signature to contracts");

        // step 4: set signatures ready
        let signature_start_block = BlockNumber::from(start_block + REQUIRED_CONFIRMATIONS.into());
        flow.set_all_signatures_ready(signature_start_block)
            .expect("failed to set signatures ready");

        // verify not confirmed initially
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed initially");

        // step 5: add blocks but not enough for confirmation
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(signature_start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify not yet confirmed
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 6: add missing block to confirm
        let block = create_test_block(signature_start_block + (REQUIRED_CONFIRMATIONS - 1).into());
        blockchain_view.borrow_mut().update(block);

        // verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(is_confirmed, "should be confirmed after enough blocks");
    }

    #[test]
    fn test_signature_step_with_signature_unset() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), Some(1)); // expect both nonce and signature calls

        // step 1: complete Nonces step
        let start_block = BlockNumber::from(100);
        complete_nonce_step(&mut flow, &bitvmx_signature, start_block, &blockchain_view)
            .expect("failed to complete Nonces step");

        // step 2: send signatures to contract
        flow.send_signature_to_contracts()
            .expect("failed to send signature to contracts");

        // step 3: set signatures ready
        flow.set_all_signatures_ready(start_block)
            .expect("failed to set signatures ready");

        // verify not confirmed initially
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed initially");

        // step 4: add blocks but not enough for confirmation
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify not yet confirmed
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 5: unset signatures ready
        let result = flow.unset_all_signatures_ready();
        assert!(result.is_ok(), "failed to unset signatures ready");

        // step 6: add enough blocks to confirm
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // check that confirmations don't accumulate after unsetting signatures
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(
            !is_confirmed,
            "confirmations should not accumulate after unsetting signatures"
        );

        // step 7: set signatures ready again
        let result = flow.set_all_signatures_ready(start_block);
        assert!(result.is_ok(), "failed to set signatures ready again");

        // add enough blocks to reach confirmation for signatures
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // step 8: verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(is_confirmed, "should be confirmed after enough blocks");
    }

    #[test]
    fn test_signature_step_with_nonce_unset() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), Some(1)); // expect both nonce and signature calls

        // step 1: complete Nonces step
        let start_block = BlockNumber::from(100);
        complete_nonce_step(&mut flow, &bitvmx_signature, start_block, &blockchain_view)
            .expect("failed to complete Nonces step");

        // step 2: send signatures to contract
        flow.send_signature_to_contracts()
            .expect("failed to send signature to contracts");

        // step 3: set signatures ready
        flow.set_all_signatures_ready(start_block)
            .expect("failed to set signatures ready");

        // step 4: add blocks but not enough for confirmation
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify not yet confirmed
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(!is_confirmed, "should not be confirmed yet");

        // step 5: unset nonces ready while in signature step
        let result = flow.unset_all_nonces_ready();
        assert!(
            result.is_err(),
            "should fail when unsetting nonces during signature step"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Nonces unset received for flow"),
            "error should mention nonces unset during signature step"
        );
    }

    #[test]
    fn test_full_flow_happy_path() {
        let (bitvmx_signature, mut flow, blockchain_view) =
            setup_test_flow_with_options(Some(1), Some(1)); // expect both nonce and signature calls

        // step 2: complete Nonces step
        let start_block = BlockNumber::from(100);
        complete_nonce_step(&mut flow, &bitvmx_signature, start_block, &blockchain_view)
            .expect("failed to complete Nonces step");

        // step 3: complete Signatures step
        let signature_start_block = BlockNumber::from(start_block + REQUIRED_CONFIRMATIONS.into());
        complete_signature_step(&mut flow, signature_start_block, &blockchain_view)
            .expect("failed to complete Signatures step");

        assert!(
            flow.is_all_signatures_ready_confirmed().unwrap(),
            "Signatures should be confirmed"
        );
    }

    #[test]
    fn test_contract_failures() {
        // setup test data
        let flow_id = Uuid::new_v4();

        // setup mock contracts gateway to fail on add_member_nonce
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        mock_contracts.expect_add_member_nonce().returning(|_| {
            Err(
                transaction_dispatcher::rsk_gateway::DomainErrors::UnhandledContractError(
                    "Contract nonce call failed".to_string(),
                ),
            )
        });

        // set up runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create a signature flow instance
        let mut flow = BtcSignatureLifeCycle::new_for_tests(
            Rc::new(mock_contracts),
            rt_sync,
            blockchain_view,
            flow_id,
        );

        // attempt to send nonce to contracts should fail
        let result = flow.send_nonce_to_contracts(&fake_signature_bitvmx("pegin"));
        assert!(
            result.is_err(),
            "should fail when contract nonce call fails"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to send nonce"),
            "error should mention nonce failure"
        );
    }

    #[test]
    fn test_out_of_order_calls() {
        // setup test data
        let flow_id = Uuid::new_v4();

        // setup mock contracts gateway (minimal expectations)
        let mock_contracts = MockRskContractsGatewayApi::new();

        // set up runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create a signature flow instance
        let mut flow = BtcSignatureLifeCycle::new_for_tests(
            Rc::new(mock_contracts),
            rt_sync,
            blockchain_view,
            flow_id,
        );

        // test 1: try to skip Nonces step and go directly to signature
        let result = flow.send_signature_to_contracts();
        assert!(
            result.is_err(),
            "should fail when trying to send signature before nonce"
        );

        // test 2: try to set nonces ready before sending nonce
        let result = flow.set_all_nonces_ready(BlockNumber::from(100));
        assert!(
            result.is_err(),
            "should fail when trying to set nonces ready before sending"
        );

        // test 3: try to set signatures ready before any signature work
        let result = flow.set_all_signatures_ready(BlockNumber::from(100));
        assert!(
            result.is_err(),
            "should fail when trying to set signatures ready too early"
        );

        // test 4: try to check confirmations before setting ready
        let result = flow.is_all_nonces_ready_confirmed();
        assert!(
            result.is_err(),
            "should fail when checking nonce confirmation before setting ready"
        );

        let result = flow.is_all_signatures_ready_confirmed();
        assert!(
            result.is_err(),
            "should fail when checking signature confirmation before setting ready"
        );
    }

    #[test]
    fn test_duplicate_calls_for_steps() {
        // setup test data
        let flow_id = Uuid::new_v4();
        let bitvmx_signature = fake_signature_bitvmx("pegin");

        // setup mock contracts gateway - expecting only one call each (duplicates should be prevented)
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        setup_nonce_mock(
            &mut mock_contracts,
            bitvmx_signature.hash_to_sign,
            &bitvmx_signature.nonce,
            1,
        );
        setup_signature_mock(
            &mut mock_contracts,
            bitvmx_signature.hash_to_sign,
            &bitvmx_signature.signature,
            1,
        );

        // set up runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create a signature flow instance
        let mut flow = BtcSignatureLifeCycle::new_for_tests(
            Rc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
            flow_id,
        );

        // step 1: send nonce to contracts
        let start_block = BlockNumber::from(100);
        complete_nonce_step(&mut flow, &bitvmx_signature, start_block, &blockchain_view)
            .expect("failed to complete Nonces step");

        // a second request to send_nonce_to_contracts for the same flow ID should fail
        let result = flow.send_nonce_to_contracts(&bitvmx_signature);
        assert!(
            result.is_err(),
            "should fail when calling send_nonce_to_contracts twice"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already in Nonces step"),
            "error should mention already in Nonces step"
        );

        // step 2: send signatures to contracts
        let signature_start_block = BlockNumber::from(start_block + REQUIRED_CONFIRMATIONS.into());
        complete_signature_step(&mut flow, signature_start_block, &blockchain_view)
            .expect("failed to complete Signatures step");

        // a second request to send_signature_to_contracts for the same flow ID should fail
        let result = flow.send_signature_to_contracts();
        assert!(
            result.is_err(),
            "should fail when calling send_signature_to_contracts twice"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already in Signatures step"),
            "error should mention already in Signatures step"
        );
    }

    pub(in crate::flows::btc_signature) fn create_test_block(
        block_number: BlockNumber,
    ) -> RskBlockAndUncles {
        let (block, _uncle1, _uncle2) = create_block_and_uncles();

        // create a new block with the desired number and proper parent hash
        let block_hash = Hash256::from(H256::from_low_u64_be(block_number.value()));
        let parent_hash = if block_number.value() == 0 {
            block.parent_hash()
        } else {
            Hash256::from(H256::from_low_u64_be(block_number.value() - 1))
        };

        let modified_block = RskBlock::new(
            block_number,
            block_hash,
            parent_hash,
            block.timestamp(),
            block.difficulty(),
            block.total_difficulty(),
            block.pow(),
            block.uncles(),
        );

        RskBlockAndUncles::new_no_uncles(modified_block)
    }

    pub(in crate::flows::btc_signature) fn setup_nonce_mock(
        mock_contracts: &mut MockRskContractsGatewayApi,
        expected_hash: Hash256,
        expected_nonce: &PubNonce,
        times: usize,
    ) {
        mock_contracts
            .expect_add_member_nonce()
            .times(times)
            .with(function({
                let expected_hash = expected_hash.clone();
                let expected_nonce = expected_nonce.clone();
                move |input: &AddMemberNonceInput| {
                    input.hash_to_sign == expected_hash && input.nonce == expected_nonce
                }
            }))
            .returning(|_| {
                let json_response = json!({
                    "transaction_hash": "0x1234567890abcdef1234567890abcdef12345678",
                    "success": true
                });
                Ok(serde_json::from_value(json_response).unwrap())
            });
    }

    pub(in crate::flows::btc_signature) fn setup_signature_mock(
        mock_contracts: &mut MockRskContractsGatewayApi,
        expected_hash: Hash256,
        expected_signature: &PartialSignature,
        times: usize,
    ) {
        mock_contracts
            .expect_add_member_signature()
            .times(times)
            .with(function({
                let expected_hash = expected_hash.clone();
                let expected_signature = expected_signature.clone();
                move |input: &AddMemberSignatureInput| {
                    input.hash_to_sign == expected_hash && input.signature == expected_signature
                }
            }))
            .returning(|_| {
                let json_response = json!({
                    "transaction_hash": "0x1234567890abcdef1234567890abcdef12345678",
                    "success": true
                });
                Ok(serde_json::from_value(json_response).unwrap())
            });
    }

    pub(in crate::flows::btc_signature) fn setup_test_flow_with_options(
        nonce_contract_calls: Option<usize>,
        signature_contract_calls: Option<usize>,
    ) -> (
        BitVmxSigningInfo,
        BtcSignatureLifeCycle<MockRskContractsGatewayApi>,
        Rc<RefCell<BlockchainView>>,
    ) {
        // setup test data
        let flow_id = Uuid::new_v4();
        let bitvmx_signature = fake_signature_bitvmx("pegin");

        // setup mock contracts gateway based on expectations
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        if let Some(times) = nonce_contract_calls {
            setup_nonce_mock(
                &mut mock_contracts,
                bitvmx_signature.hash_to_sign,
                &bitvmx_signature.nonce,
                times,
            );
        }
        if let Some(times) = signature_contract_calls {
            setup_signature_mock(
                &mut mock_contracts,
                bitvmx_signature.hash_to_sign,
                &bitvmx_signature.signature,
                times,
            );
        }

        // set up runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create a signature flow instance
        let flow = BtcSignatureLifeCycle::new_for_tests(
            Rc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
            flow_id,
        );

        (bitvmx_signature, flow, blockchain_view)
    }

    pub(in crate::flows::btc_signature) fn complete_nonce_step<CG: RskContractsGatewayApi>(
        flow: &mut BtcSignatureLifeCycle<CG>,
        bitvmx_signature: &BitVmxSigningInfo,
        start_block: BlockNumber,
        blockchain_view: &Rc<RefCell<BlockchainView>>,
    ) -> Result<()> {
        flow.send_nonce_to_contracts(bitvmx_signature)?;
        flow.set_all_nonces_ready(start_block)?;

        // add enough blocks for nonce confirmation
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_nonces_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(
            is_confirmed,
            "nonces should be confirmed after enough blocks"
        );

        Ok(())
    }

    pub(in crate::flows::btc_signature) fn complete_signature_step<CG: RskContractsGatewayApi>(
        flow: &mut BtcSignatureLifeCycle<CG>,
        start_block: BlockNumber,
        blockchain_view: &Rc<RefCell<BlockchainView>>,
    ) -> Result<()> {
        flow.send_signature_to_contracts()?;
        flow.set_all_signatures_ready(start_block)?;

        // add enough blocks for signature confirmation
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // verify confirmed after enough blocks
        let is_confirmed = flow
            .is_all_signatures_ready_confirmed()
            .expect("failed to check confirmation status");
        assert!(
            is_confirmed,
            "signatures should be confirmed after enough blocks"
        );

        Ok(())
    }

    pub(in crate::flows::btc_signature) fn fake_signature_bitvmx(
        protocol_name: &str,
    ) -> BitVmxSigningInfo {
        let hash_to_sign = "a1b2c3d4e5f60123456789abcdef0123456789abcdef0123456789abcdef0123"
            .try_into()
            .unwrap();
        let nonce = "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93".parse::<PubNonce>().unwrap();
        let signature = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249"
            .parse()
            .unwrap();
        let aggr_key = PublicKey::from_str("04c4b0bbb339aa236bff38dbe6a451e111972a7909a126bc424013cba2ec33bc38e98ac269ffe028345c31ac8d0a365f29c8f7e7cfccac72f84e1acd02bc554f35").unwrap();

        BitVmxSigningInfo {
            protocol_name: protocol_name.to_string(),
            take_aggr_key: aggr_key,
            hash_to_sign,
            nonce: nonce.clone(),
            signature,
        }
    }
}
