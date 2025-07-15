use crate::blockchain_tracker::{BlockchainView, ConfirmableEvent};
use crate::config::REQUIRED_CONFIRMATIONS;
use crate::types::MemberSignature;
use anyhow::{Context, Result, bail};
use common::msg_broker::bitvmx_types::IncomingBitVMXApiMessages;
use common::msg_broker::broker::{BROKER_SERVER_ID, BitVmxBrokerClientApi};
use common::runtime_sync::RuntimeSync;
use common::types::BlockNumber;
use log::info;
use serde_json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use transaction_dispatcher::types::{AddMemberNonceInput, AddMemberSignatureInput};
use uuid::Uuid;

const SIGNATURE_VAR: &str = "my-signature";

pub trait BtcSignatureFlowApi {
    fn request_signature_to_bitvmx(&mut self, uuid: Uuid) -> Result<()>;

    fn send_signature_to_contracts(&mut self, uuid: Uuid, signature_data: String) -> Result<()>;

    fn set_all_signatures_ready(&mut self, id: Uuid, block_number: BlockNumber) -> Result<()>;

    fn unset_all_signatures_ready(&mut self, id: Uuid) -> Result<()>;

    fn is_all_signatures_ready_confirmed(&self, uuid: Uuid) -> Result<bool>;
}

pub struct BtcSignatureFlow<BC: BitVmxBrokerClientApi, CG: RskContractsGatewayApi> {
    bitvmx_broker: Arc<BC>,
    contracts: Arc<CG>,
    rt_sync: RuntimeSync,
    blockchain_view: Rc<RefCell<BlockchainView>>,
    state: HashMap<Uuid, Option<ConfirmableEvent>>,
}

impl<BC, CG> BtcSignatureFlow<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    pub fn new(
        bitvmx_broker: Arc<BC>,
        contracts_gateway: Arc<CG>,
        rt_sync: RuntimeSync,
        blockchain_view: Rc<RefCell<BlockchainView>>,
    ) -> Self {
        BtcSignatureFlow {
            bitvmx_broker,
            contracts: contracts_gateway,
            rt_sync,
            blockchain_view,
            state: HashMap::new(),
        }
    }
}

impl<BC, CG> BtcSignatureFlowApi for BtcSignatureFlow<BC, CG>
where
    BC: BitVmxBrokerClientApi,
    CG: RskContractsGatewayApi,
{
    fn request_signature_to_bitvmx(&mut self, uuid: Uuid) -> Result<()> {
        self.bitvmx_broker
            .send(
                BROKER_SERVER_ID,
                IncomingBitVMXApiMessages::GetVar(uuid, SIGNATURE_VAR.to_string()),
            )
            .with_context(|| format!("Failed to get var from protocol {uuid} on BitVMX"))?;

        self.state.insert(uuid, None);

        Ok(())
    }

    fn send_signature_to_contracts(&mut self, id: Uuid, signature_data: String) -> Result<()> {
        if self.state.get(&id).is_none() {
            bail!(
                "Protocol {} not found in BtcSignatureFlow state on send_signature_to_contracts",
                id
            );
        }

        info!("Sending signatures to contract for protocol {id}");

        let member_signature = serde_json::from_str::<MemberSignature>(&signature_data)
            .with_context(|| {
                format!("Failed to deserialize MemberSignature from JSON for protocol {id}")
            })?;

        // send the nonce

        let nonce = AddMemberNonceInput {
            hash_to_sign: member_signature.hash_to_sign.clone(),
            nonce: member_signature.btc_nonce,
        };

        let send_nonce_result = self
            .rt_sync
            .run(self.contracts.add_member_nonce(nonce.clone()))
            .with_context(|| format!("Failed to send nonce {:?} for protocol {}", nonce, id))?;

        if !send_nonce_result.success {
            bail!(
                "Tx {} rejected on send nonce {nonce:?} for protocol {id}",
                send_nonce_result.transaction_hash
            );
        }

        // send the signature

        let signature = AddMemberSignatureInput {
            hash_to_sign: member_signature.hash_to_sign,
            signature: member_signature.btc_signature,
        };

        let conf_sig_ev =
            ConfirmableEvent::new(id, REQUIRED_CONFIRMATIONS, self.blockchain_view.clone());

        let send_sig_result = self
            .rt_sync
            .run(self.contracts.add_member_signature(signature.clone()))
            .with_context(|| {
                format!(
                    "Failed to send signature {:?} for protocol {}",
                    signature, id
                )
            })?;

        if !send_sig_result.success {
            bail!(
                "Tx {} rejected on send signature {signature:?} for protocol {id}",
                send_sig_result.transaction_hash
            );
        }

        // for now (TBC in the future) we don't care about the AllNoncesReady event, and we wait only for the AllSignaturesReady event
        self.state.insert(id, Some(conf_sig_ev));

        Ok(())
    }

    fn set_all_signatures_ready(&mut self, id: Uuid, block_number: BlockNumber) -> Result<()> {
        let invoke = self
            .state
            .get_mut(&id)
            .context(format!(
                "Protocol {} not found in BtcSignatureFlow state on set_all_signatures_ready",
                id
            ))?
            .as_mut()
            .context(format!(
                "Invoke not found for protocol {} in BtcSignatureFlow state on set_all_signatures_ready",
                id
            ))?;

        invoke
            .start_confirming(block_number)
            .with_context(|| format!("Failed to start confirming signatures for protocol {}", id))
    }

    fn unset_all_signatures_ready(&mut self, id: Uuid) -> Result<()> {
        let invoke = self
            .state
            .get_mut(&id)
            .context(format!(
                "Protocol {} not found in BtcSignatureFlow state on unset_all_signatures_ready",
                id
            ))?
            .as_mut()
            .context(format!(
                "Invoke not found for protocol {} in BtcSignatureFlow state on unset_all_signatures_ready",
                id
            ))?;

        invoke
            .stop_confirming()
            .with_context(|| format!("Failed to stop confirming signatures for protocol {}", id))
    }

    fn is_all_signatures_ready_confirmed(&self, id: Uuid) -> Result<bool> {
        let invoke = self
            .state
            .get(&id)
            .context(format!(
                "Protocol {} not found in BtcSignatureFlow state",
                id
            ))?
            .as_ref()
            .context(format!(
                "Invoke not found for protocol {} in BtcSignatureFlow state",
                id
            ))?;

        Ok(invoke.is_confirmed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain_tracker::BlockchainView;
    use crate::coordinator::tests::MockRskContractsGatewayApi;
    use anyhow;
    use common::msg_broker::bitvmx_types::{IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages};
    use common::msg_broker::broker::{BROKER_SERVER_ID, MockBrokerClientApi};
    use common::runtime_sync::RuntimeSync;
    use common::test_utils::rsk_block_generator::create_block_and_uncles;
    use common::types::{BlockNumber, Hash256, RskBlock, RskBlockAndUncles};
    use mockall::predicate::{eq, function};
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use transaction_dispatcher::types::{AddMemberNonceInput, AddMemberSignatureInput};
    use uuid::Uuid;

    #[test]
    fn test_signature_flow_happy_path() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let signature_json = create_member_signature_json();
        let expected_hash = "0x1234567890abcdef1234567890abcdef12345678";
        let expected_nonce = "nonce_value_456";
        let expected_signature = "signature_value_123";

        // setup mock bitvmx broker
        let mock_bitvmx_broker = mock_bitvmx_get_var(protocol_id);

        // setup mock contracts gateway
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        setup_successful_contracts_mock(
            &mut mock_contracts,
            expected_hash,
            expected_nonce,
            expected_signature,
        );

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
        );

        // step 1: request signature from bitvmx
        let result = signature_flow.request_signature_to_bitvmx(protocol_id);
        assert!(result.is_ok(), "failed to request signature from bitvmx");

        // step 2: send signature to contracts
        let result = signature_flow.send_signature_to_contracts(protocol_id, signature_json);
        assert!(result.is_ok(), "failed to send signature to contracts");

        // step 3: set all signatures ready
        let start_block = BlockNumber::from(100);
        let result = signature_flow.set_all_signatures_ready(protocol_id, start_block);
        assert!(result.is_ok(), "failed to set all signatures ready");

        // step 4: verify not confirmed initially
        let is_confirmed = signature_flow.is_all_signatures_ready_confirmed(protocol_id);
        assert!(is_confirmed.is_ok(), "failed to check confirmation status");
        assert!(!is_confirmed.unwrap(), "should not be confirmed initially");

        // step 5: add enough blocks to reach confirmation
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // step 6: verify confirmed after enough blocks
        let is_confirmed = signature_flow.is_all_signatures_ready_confirmed(protocol_id);
        assert!(
            is_confirmed.is_ok(),
            "failed to check confirmation status after blocks"
        );
        assert!(
            is_confirmed.unwrap(),
            "should be confirmed after enough blocks"
        );
    }

    #[test]
    fn test_signature_flow_unset_signatures_ready() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let signature_json = create_member_signature_json();
        let expected_hash = "0x1234567890abcdef1234567890abcdef12345678";
        let expected_nonce = "nonce_value_456";
        let expected_signature = "signature_value_123";

        // setup mock bitvmx broker
        let mock_bitvmx_broker = mock_bitvmx_get_var(protocol_id);

        // setup mock contracts gateway
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        setup_successful_contracts_mock(
            &mut mock_contracts,
            expected_hash,
            expected_nonce,
            expected_signature,
        );

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
        );

        // step 1: request signature from bitvmx
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx");

        // step 2: send signature to contracts
        signature_flow
            .send_signature_to_contracts(protocol_id, signature_json)
            .expect("failed to send signature to contracts");

        // step 3: set all signatures ready
        let start_block = BlockNumber::from(100);
        signature_flow
            .set_all_signatures_ready(protocol_id, start_block)
            .expect("failed to set all signatures ready");

        // step 4: add enough blocks to reach confirmation
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // step 5: verify confirmed
        let is_confirmed = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id)
            .expect("failed to check confirmation status");
        assert!(is_confirmed, "should be confirmed after enough blocks");

        // step 6: unset signatures ready
        let result = signature_flow.unset_all_signatures_ready(protocol_id);
        assert!(result.is_ok(), "failed to unset all signatures ready");

        // step 7: verify not confirmed anymore
        let is_confirmed = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id)
            .expect("failed to check confirmation status after unset");
        assert!(!is_confirmed, "should not be confirmed after unset");
    }

    #[test]
    fn test_signature_flow_invalid_signature() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let invalid_signature_json = "{invalid_json}";

        // setup mock bitvmx broker
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker.expect_send().returning(|_, _| Ok(true));

        // setup mock contracts gateway (should not be called due to invalid json)
        let mock_contracts = MockRskContractsGatewayApi::new();

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view,
        );

        // step 1: request signature from bitvmx
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx");

        // step 2: try to send invalid signature to contracts
        let result = signature_flow
            .send_signature_to_contracts(protocol_id, invalid_signature_json.to_string());
        assert!(result.is_err(), "should fail with invalid signature json");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to deserialize MemberSignature"),
            "error should mention deserialization failure"
        );
    }

    #[test]
    fn test_signature_flow_protocol_not_found_errors() {
        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance with mock dependencies
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(MockBrokerClientApi::new()),
            Arc::new(MockRskContractsGatewayApi::new()),
            rt_sync,
            blockchain_view,
        );

        let non_existent_protocol = Uuid::new_v4();

        // test set_all_signatures_ready with non-existent protocol
        let result =
            signature_flow.set_all_signatures_ready(non_existent_protocol, BlockNumber::from(100));
        assert!(result.is_err(), "should fail with non-existent protocol");
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Protocol") && error_message.contains("not found"),
            "error should mention protocol not found"
        );

        // test unset_all_signatures_ready with non-existent protocol
        let result = signature_flow.unset_all_signatures_ready(non_existent_protocol);
        assert!(result.is_err(), "should fail with non-existent protocol");
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Protocol") && error_message.contains("not found"),
            "error should mention protocol not found"
        );

        // test is_all_signatures_ready_confirmed with non-existent protocol
        let result = signature_flow.is_all_signatures_ready_confirmed(non_existent_protocol);
        assert!(result.is_err(), "should fail with non-existent protocol");
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("Protocol") && error_message.contains("not found"),
            "error should mention protocol not found"
        );
    }

    #[test]
    fn test_signature_flow_contract_failures() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let signature_json = create_member_signature_json();

        // setup mock bitvmx broker
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker.expect_send().returning(|_, _| Ok(true));

        // setup mock contracts gateway to fail on add_member_nonce
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        mock_contracts.expect_add_member_nonce().returning(|_| {
            Err(
                transaction_dispatcher::rsk_gateway::DomainErrors::UnhandledContractError(
                    "Contract nonce call failed".to_string(),
                ),
            )
        });

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view,
        );

        // request signature from bitvmx
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx");

        // attempt to send signature to contracts should fail
        let result = signature_flow.send_signature_to_contracts(protocol_id, signature_json);
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
    fn test_signature_flow_bitvmx_broker_failure() {
        // setup test data
        let protocol_id = Uuid::new_v4();

        // setup mock bitvmx broker to fail
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker
            .expect_send()
            .returning(|_, _| Err(anyhow::Error::msg("Broker connection failed").into()));

        // setup mock contracts gateway
        let mock_contracts = MockRskContractsGatewayApi::new();

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view,
        );

        // request signature from bitvmx should fail
        let result = signature_flow.request_signature_to_bitvmx(protocol_id);
        assert!(result.is_err(), "should fail when broker fails");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to get var from protocol"),
            "error should mention protocol failure"
        );
    }

    #[test]
    fn test_signature_flow_out_of_order_calls() {
        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(MockBrokerClientApi::new()),
            Arc::new(MockRskContractsGatewayApi::new()),
            rt_sync,
            blockchain_view,
        );

        let protocol_id = Uuid::new_v4();

        // try to set signatures ready without prior setup
        let result = signature_flow.set_all_signatures_ready(protocol_id, BlockNumber::from(100));
        assert!(result.is_err(), "should fail when protocol not found");

        // try to check confirmation without prior setup
        let result = signature_flow.is_all_signatures_ready_confirmed(protocol_id);
        assert!(result.is_err(), "should fail when protocol not found");
    }

    #[test]
    fn test_signature_flow_multiple_protocols() {
        // setup test data for two protocols
        let protocol_id_1 = Uuid::new_v4();
        let protocol_id_2 = Uuid::new_v4();
        let signature_json = create_member_signature_json();
        let expected_hash = "0x1234567890abcdef1234567890abcdef12345678";
        let expected_nonce = "nonce_value_456";
        let expected_signature = "signature_value_123";

        // setup mock bitvmx broker
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker.expect_send().returning(|_, _| Ok(true));

        // setup mock contracts gateway
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        setup_successful_contracts_mock(
            &mut mock_contracts,
            expected_hash,
            expected_nonce,
            expected_signature,
        );

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
        );

        // setup both protocols
        signature_flow
            .request_signature_to_bitvmx(protocol_id_1)
            .expect("failed to request signature for protocol 1");
        signature_flow
            .request_signature_to_bitvmx(protocol_id_2)
            .expect("failed to request signature for protocol 2");

        signature_flow
            .send_signature_to_contracts(protocol_id_1, signature_json.clone())
            .expect("failed to send signature for protocol 1");
        signature_flow
            .send_signature_to_contracts(protocol_id_2, signature_json)
            .expect("failed to send signature for protocol 2");

        // set signatures ready for both protocols at different blocks
        let start_block_1 = BlockNumber::from(100);
        let start_block_2 = BlockNumber::from(100 + REQUIRED_CONFIRMATIONS as u64);

        signature_flow
            .set_all_signatures_ready(protocol_id_1, start_block_1)
            .expect("failed to set signatures ready for protocol 1");
        signature_flow
            .set_all_signatures_ready(protocol_id_2, start_block_2)
            .expect("failed to set signatures ready for protocol 2");

        // add blocks to confirm protocol 1 but not protocol 2
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block_1 + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // protocol 1 should be confirmed, protocol 2 should not
        let is_confirmed_1 = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id_1)
            .expect("failed to check confirmation for protocol 1");
        let is_confirmed_2 = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id_2)
            .expect("failed to check confirmation for protocol 2");

        assert!(is_confirmed_1, "protocol 1 should be confirmed");
        assert!(!is_confirmed_2, "protocol 2 should not be confirmed yet");

        // add more blocks to confirm protocol 2 (continuing from where we left off)
        for i in 0..REQUIRED_CONFIRMATIONS {
            let block = create_test_block(start_block_2 + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // now both protocols should be confirmed
        let is_confirmed_2 = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id_2)
            .expect("failed to check confirmation for protocol 2 after more blocks");
        assert!(
            is_confirmed_2,
            "protocol 2 should be confirmed after more blocks"
        );
    }

    #[test]
    fn test_signature_flow_confirmation_edge_cases() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let signature_json = create_member_signature_json();
        let expected_hash = "0x1234567890abcdef1234567890abcdef12345678";
        let expected_nonce = "nonce_value_456";
        let expected_signature = "signature_value_123";

        // setup mock bitvmx broker
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker.expect_send().returning(|_, _| Ok(true));

        // setup mock contracts gateway
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        setup_successful_contracts_mock(
            &mut mock_contracts,
            expected_hash,
            expected_nonce,
            expected_signature,
        );

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view.clone(),
        );

        // complete the setup
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx");
        signature_flow
            .send_signature_to_contracts(protocol_id, signature_json)
            .expect("failed to send signature to contracts");

        let start_block = BlockNumber::from(100);
        signature_flow
            .set_all_signatures_ready(protocol_id, start_block)
            .expect("failed to set all signatures ready");

        // add one less than required confirmations
        for i in 0..(REQUIRED_CONFIRMATIONS - 1) {
            let block = create_test_block(start_block + i.into());
            blockchain_view.borrow_mut().update(block);
        }

        // should not be confirmed yet
        let is_confirmed = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id)
            .expect("failed to check confirmation status");
        assert!(
            !is_confirmed,
            "should not be confirmed with one less than required confirmations"
        );

        // add exactly the required number of confirmations
        let block = create_test_block(start_block + (REQUIRED_CONFIRMATIONS - 1).into());
        blockchain_view.borrow_mut().update(block);

        // should now be confirmed
        let is_confirmed = signature_flow
            .is_all_signatures_ready_confirmed(protocol_id)
            .expect("failed to check confirmation status");
        assert!(
            is_confirmed,
            "should be confirmed with exactly required confirmations"
        );
    }

    #[test]
    fn test_signature_flow_duplicate_calls() {
        // setup test data
        let protocol_id = Uuid::new_v4();
        let signature_json = create_member_signature_json();

        // setup mock bitvmx broker - expecting two calls
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker
            .expect_send()
            .times(2)
            .returning(|_, _| Ok(true));

        // setup mock contracts gateway - expecting two sets of calls
        let mut mock_contracts = MockRskContractsGatewayApi::new();
        mock_contracts
            .expect_add_member_nonce()
            .times(2)
            .returning(|_| {
                let json_response = json!({
                    "transaction_hash": "0x1234567890abcdef1234567890abcdef12345678",
                    "success": true
                });
                Ok(serde_json::from_value(json_response).unwrap())
            });
        mock_contracts
            .expect_add_member_signature()
            .times(2)
            .returning(|_| {
                let json_response = json!({
                    "transaction_hash": "0x1234567890abcdef1234567890abcdef12345678",
                    "success": true
                });
                Ok(serde_json::from_value(json_response).unwrap())
            });

        // setup runtime sync and blockchain view
        let rt_sync = RuntimeSync::new().expect("failed to create runtime sync");
        let blockchain_view = Rc::new(RefCell::new(BlockchainView::new()));

        // create signature flow instance
        let mut signature_flow = BtcSignatureFlow::new(
            Arc::new(mock_bitvmx_broker),
            Arc::new(mock_contracts),
            rt_sync,
            blockchain_view,
        );

        // call request_signature_to_bitvmx twice
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx first time");
        signature_flow
            .request_signature_to_bitvmx(protocol_id)
            .expect("failed to request signature from bitvmx second time");

        // call send_signature_to_contracts twice
        signature_flow
            .send_signature_to_contracts(protocol_id, signature_json.clone())
            .expect("failed to send signature to contracts first time");
        signature_flow
            .send_signature_to_contracts(protocol_id, signature_json)
            .expect("failed to send signature to contracts second time");

        // both calls should succeed - this tests that the state management allows overwriting
    }

    fn mock_bitvmx_get_var(
        protocol_id: Uuid,
    ) -> MockBrokerClientApi<IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages> {
        let mut mock_bitvmx_broker = MockBrokerClientApi::new();
        mock_bitvmx_broker
            .expect_send()
            .with(
                eq(BROKER_SERVER_ID),
                function(move |msg: &IncomingBitVMXApiMessages| {
                    matches!(msg, IncomingBitVMXApiMessages::GetVar(id, var)
                        if *id == protocol_id && var == SIGNATURE_VAR)
                }),
            )
            .returning(|_, _| Ok(true));
        mock_bitvmx_broker
    }

    fn create_test_block(block_number: BlockNumber) -> RskBlockAndUncles {
        let (block, _uncle1, _uncle2) = create_block_and_uncles();

        // create a new block with the desired number and proper parent hash
        let block_hash =
            Hash256::from(primitive_types::H256::from_low_u64_be(block_number.value()));
        let parent_hash = if block_number.value() == 0 {
            block.parent_hash()
        } else {
            Hash256::from(primitive_types::H256::from_low_u64_be(
                block_number.value() - 1,
            ))
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

    fn setup_successful_contracts_mock(
        mock_contracts: &mut MockRskContractsGatewayApi,
        expected_hash: &str,
        expected_nonce: &str,
        expected_signature: &str,
    ) {
        let expected_hash = expected_hash.to_string();
        let expected_nonce = expected_nonce.to_string();
        let expected_signature = expected_signature.to_string();

        // expect add_member_nonce call
        mock_contracts
            .expect_add_member_nonce()
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

        // expect add_member_signature call
        mock_contracts
            .expect_add_member_signature()
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

    // helper to create a valid member signature json
    fn create_member_signature_json() -> String {
        json!({
            "hash_to_sign": "0x1234567890abcdef1234567890abcdef12345678",
            "btc_signature": "signature_value_123",
            "btc_nonce": "nonce_value_456"
        })
        .to_string()
    }
}
