use super::btc_signature_flow::{BtcSignatureFlow, BtcSignatureFlowApi};
use crate::event_processor::EventProcessor;
use crate::types::{AllNoncesReadyEvent, BitVmxSigningInfo, RskPegManagerEvents};
use anyhow::{Result, anyhow, bail};
use common::msg_broker::bitvmx_types::{OutgoingBitVMXApiMessages, VariableTypes};
use common::runtime_sync::RuntimeSync;
use common::types::{Hash256, RskBlockAndUncles};
use std::collections::HashMap;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

pub(crate) const SIGNATURE_MESSAGE: &str = "signing_info";

#[cfg_attr(test, automock)]
pub trait BtcSignatureFlowFactoryApi<BSF: BtcSignatureFlowApi> {
    fn create_flow(&self, flow_id: Uuid) -> BSF;
}

pub struct BtcSignatureFlowFactory<CG: RskContractsGatewayApi> {
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
}

impl<CG: RskContractsGatewayApi> BtcSignatureFlowFactory<CG> {
    // TODO(signatures-1.3) use from pegin and pegout flows to instantiate BtcSignatureFlow
    pub fn new(contracts_gateway: Rc<CG>, rt_sync: RuntimeSync) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
        }
    }
}

impl<CG: RskContractsGatewayApi> BtcSignatureFlowFactoryApi<BtcSignatureFlow<CG>>
    for BtcSignatureFlowFactory<CG>
{
    fn create_flow(&self, flow_id: Uuid) -> BtcSignatureFlow<CG> {
        BtcSignatureFlow::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            flow_id,
        )
    }
}

pub struct BtcSignatureFlowProcessor<BSF, FactoryBSF>
where
    BSF: BtcSignatureFlowApi,
    FactoryBSF: BtcSignatureFlowFactoryApi<BSF>,
{
    flow_factory: FactoryBSF,
    flows: HashMap<Uuid, BSF>,
}

impl<BSF, FactoryBSF> BtcSignatureFlowProcessor<BSF, FactoryBSF>
where
    BSF: BtcSignatureFlowApi,
    FactoryBSF: BtcSignatureFlowFactoryApi<BSF>,
{
    // TODO(signatures-1.4) call from pegin and pegout flows
    pub fn new(flow_factory: FactoryBSF) -> Self {
        Self {
            flow_factory,
            flows: HashMap::new(),
        }
    }

    // TODO improve with a mapper tx_hash -> flow_id if this approach confirms valid
    fn get_flow_from_tx_hash(&self, event: &AllNoncesReadyEvent) -> Result<Uuid> {
        let event_hash_to_sign = Hash256::from(event.inner.value());

        for (flow_id, flow) in &self.flows {
            if let Some(hash_to_sign) = flow.get_hash_to_sign() {
                if hash_to_sign == event_hash_to_sign {
                    return Ok(*flow_id);
                }
            }
        }

        bail!("Flow with hash {} not found", event.inner.value())
    }

    fn close_flow(&mut self, flow_id: Uuid) {
        if let Some(flow) = self.flows.remove(&flow_id) {
            // clean blockchain_view if no more flows are active
            if self.flows.is_empty() {
                flow.blockchain_view().borrow_mut().clear();
            }
        }
    }
}

impl<BSF, FactoryBSF> EventProcessor for BtcSignatureFlowProcessor<BSF, FactoryBSF>
where
    BSF: BtcSignatureFlowApi,
    FactoryBSF: BtcSignatureFlowFactoryApi<BSF>,
{
    fn process_new_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), SIGNATURE_MESSAGE) =>
            {
                let signing_info = serde_json::from_str::<BitVmxSigningInfo>(data)
                    .map_err(|e| anyhow!("Failed to deserialize signature data: {e}"))?;

                if self.flows.contains_key(flow_id) {
                    bail!("Flow already exists with ID {flow_id}")
                }

                let new_flow = self.flow_factory.create_flow(*flow_id);
                self.flows.insert(*flow_id, new_flow);

                let flow = self.flows.get_mut(flow_id).unwrap();
                flow.send_nonce_to_contracts(&signing_info)?;
                Ok(())
            }
            _ => {
                bail!("Unexpected BitVMX event in BtcSignatureFlow: {event:?}")
            }
        }
    }

    fn process_new_rsk_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::AllNoncesReady(event) => {
                let flow_id = self.get_flow_from_tx_hash(&event)?;

                let flow = self.flows.get_mut(&flow_id).unwrap();
                if event.removed {
                    flow.unset_all_nonces_ready()?;
                } else {
                    flow.set_all_nonces_ready(event.block_number)?;
                }

                Ok(())
            }
            RskPegManagerEvents::AllSignaturesReady(event) => {
                let flow_id = self.get_flow_from_tx_hash(&event)?;

                let flow = self.flows.get_mut(&flow_id).unwrap();
                if event.removed {
                    flow.unset_all_signatures_ready()?;
                } else {
                    flow.set_all_signatures_ready(event.block_number)?;
                }

                Ok(())
            }
            _ => bail!("Unexpected RSK event in BtcSignatureFlow: {event:?}"),
        }
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        for flow in self.flows.values_mut() {
            flow.blockchain_view().borrow_mut().update(block.clone());
        }

        let mut confirmed_nonces = vec![];
        let mut confirmed_signatures = vec![];

        // collect flow IDs where nonces/signatures are ready
        for (flow_id, flow) in &self.flows {
            if flow.is_all_nonces_ready_confirmed()? {
                confirmed_nonces.push(*flow_id);
            }
            if flow.is_all_signagures_ready_confirmed()? {
                confirmed_signatures.push(*flow_id);
            }
        }

        // for ready nonces, send the signature to contracts
        for flow_id in confirmed_nonces {
            let flow = self.flows.get_mut(&flow_id).unwrap();
            flow.send_signature_to_contracts()?;
        }

        // for ready signatures, close the flow
        for flow_id in confirmed_signatures {
            self.close_flow(flow_id);
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        self.flows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain_tracker::BlockchainView;
    use crate::flows::btc_signature::btc_signature_flow::MockBtcSignatureFlowApi;
    use crate::types::{AllNoncesReadyEvent, AllSignaturesReadyEvent};
    use bitcoin::PublicKey;
    use common::test_utils::rsk_block_generator::create_block_and_uncles;
    use common::types::{BlockNumber, RskBlockAndUncles, TxHash};
    use mockall::predicate::*;
    use musig2::PubNonce;
    use primitive_types::H256;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::str::FromStr;

    #[test]
    fn test_process_new_bitvmx_event_creates_flow_and_sends_nonce() {
        // create signature data for the event
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let nonce = "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93".parse::<PubNonce>().unwrap();
        let signature = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249"
            .parse()
            .unwrap();
        let signing_info = BitVmxSigningInfo {
            protocol_name: "pegin".to_string(),
            take_aggr_key: PublicKey::from_str("04c4b0bbb339aa236bff38dbe6a451e111972a7909a126bc424013cba2ec33bc38e98ac269ffe028345c31ac8d0a365f29c8f7e7cfccac72f84e1acd02bc554f35").unwrap(),
            hash_to_sign,
            nonce: nonce.clone(),
            signature,
        };
        let signature_json = serde_json::to_string(&signing_info).unwrap();

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String(signature_json),
        );

        // setup mock flow to expect nonce being sent to contracts
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_send_nonce_to_contracts()
            .withf(move |arg: &BitVmxSigningInfo| {
                arg.hash_to_sign == hash_to_sign && arg.nonce == nonce && arg.signature == signature
            })
            .times(1)
            .returning(|_| Ok(()));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        // setup mock factory to create the flow
        let mut mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        mock_factory
            .expect_create_flow()
            .with(eq(flow_id))
            .times(1)
            .return_once(move |_| mock_flow);

        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_bitvmx_event_rejects_invalid_event() {
        let flow_id = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            "wrong-message".to_string(),
            VariableTypes::String("data".to_string()),
        );

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn test_process_new_rsk_event_all_nonces_ready() {
        // create all nonces ready event
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        // setup mock flow to handle the event
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_get_hash_to_sign()
            .times(1)
            .returning(move || Some(hash_to_sign));
        mock_flow
            .expect_set_all_nonces_ready()
            .with(eq(block_number))
            .times(1)
            .returning(|_| Ok(()));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_nonces_ready_removed() {
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: true,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_get_hash_to_sign()
            .times(1)
            .returning(move || Some(hash_to_sign));
        mock_flow
            .expect_unset_all_nonces_ready()
            .times(1)
            .returning(|| Ok(()));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_signatures_ready() {
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllSignaturesReady(AllSignaturesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_get_hash_to_sign()
            .times(1)
            .returning(move || Some(hash_to_sign));
        mock_flow
            .expect_set_all_signatures_ready()
            .with(eq(block_number))
            .times(1)
            .returning(|_| Ok(()));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_signatures_ready_removed() {
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllSignaturesReady(AllSignaturesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: true,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_get_hash_to_sign()
            .times(1)
            .returning(move || Some(hash_to_sign));
        mock_flow
            .expect_unset_all_signatures_ready()
            .times(1)
            .returning(|| Ok(()));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_rejects_invalid_event() {
        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let event = RskPegManagerEvents::UnknownEvent;

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn test_process_new_block_updates_blockchain_view() {
        let flow_id = Uuid::new_v4();

        // create blocks using the utility function
        let (block_1, _, _) = create_block_and_uncles();

        // use block_1 for this test
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        // setup mock flow to verify blockchain view is updated
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        // create a new blockchain view for each call
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_block(&block);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_block_with_confirmed_nonces_sends_signature() {
        let flow_id = Uuid::new_v4();

        let (_block_1, uncle_1, block_2) = create_block_and_uncles();

        let block = RskBlockAndUncles::new(block_2, vec![uncle_1]);

        // setup mock flow to simulate confirmed nonces
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_block(&block);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_block_with_confirmed_signatures_closes_flow() {
        let flow_id = Uuid::new_v4();

        // create blocks using the utility function
        let (block_1, uncle_1, _) = create_block_and_uncles();

        // use block_1 with uncle_1 for this test
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to simulate confirmed signatures
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(2) // one on process_new_block and one on close_flow
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        println!("{:?}", processor.flows.len());

        processor.flows.insert(flow_id, mock_flow);

        println!("{:?}", processor.flows.len());

        let result = processor.process_new_block(&block);

        assert!(result.is_ok());
        assert!(processor.flows.is_empty());
    }

    #[test]
    fn test_shutdown_clears_flows() {
        let flow_id = Uuid::new_v4();

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let mock_flow_for_insert = MockBtcSignatureFlowApi::new();
        processor.flows.insert(flow_id, mock_flow_for_insert);

        processor.shutdown();

        assert!(processor.flows.is_empty());
    }

    #[test]
    fn test_process_new_block_with_multiple_blocks() {
        // create test data
        let flow_id = Uuid::new_v4();

        // create blocks using the utility function
        let (block_1, uncle_1, block_2) = create_block_and_uncles();

        // process block_1 first
        let block_1_with_uncles = RskBlockAndUncles::new_no_uncles(block_1.clone());

        let mut mock_flow = MockBtcSignatureFlowApi::new();
        // set up expectations for the first block
        mock_flow
            .expect_blockchain_view()
            .times(2) // will be called twice, once for each block
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        // first block: no confirmations yet
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));

        // second block: nonces are confirmed
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        // process block_1
        let result_1 = processor.process_new_block(&block_1_with_uncles);
        assert!(result_1.is_ok());

        // process block_2, which should trigger the nonce confirmation
        let block_2_with_uncles = RskBlockAndUncles::new(block_2, vec![uncle_1]);
        let result_2 = processor.process_new_block(&block_2_with_uncles);
        assert!(result_2.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_flow_not_found() {
        // create all nonces ready event with a hash that doesn't match any flow
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Flow with hash"));
    }

    #[test]
    fn test_process_new_bitvmx_event_invalid_json() {
        let flow_id = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String("invalid json".to_string()),
        );

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to deserialize signature data")
        );
    }

    #[test]
    fn test_process_new_bitvmx_event_send_nonce_fails() {
        // create signature data for the event
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let nonce = "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93".parse::<PubNonce>().unwrap();
        let signature = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249"
            .parse()
            .unwrap();
        let signing_info = BitVmxSigningInfo {
            protocol_name: "pegin".to_string(),
            take_aggr_key: PublicKey::from_str("04c4b0bbb339aa236bff38dbe6a451e111972a7909a126bc424013cba2ec33bc38e98ac269ffe028345c31ac8d0a365f29c8f7e7cfccac72f84e1acd02bc554f35").unwrap(),
            hash_to_sign,
            nonce: nonce.clone(),
            signature
        };
        let signature_json = serde_json::to_string(&signing_info).unwrap();

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String(signature_json),
        );

        // setup mock flow to fail when sending nonce
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_send_nonce_to_contracts()
            .times(1)
            .returning(|_| Err(anyhow!("Contract call failed")));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        // setup mock factory to create the flow
        let mut mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        mock_factory
            .expect_create_flow()
            .with(eq(flow_id))
            .times(1)
            .return_once(move |_| mock_flow);

        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);

        let result = processor.process_new_bitvmx_event(&event);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Contract call failed")
        );
    }

    #[test]
    fn test_process_new_rsk_event_set_nonces_ready_fails() {
        // create all nonces ready event
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let block_number: BlockNumber = 100.into();
        let block_hash = Hash256::from(H256::random());

        let event = RskPegManagerEvents::AllNoncesReady(AllNoncesReadyEvent {
            inner: hash_to_sign,
            block_number: block_number.clone(),
            block_hash,
            removed: false,
            tx_hash: TxHash::from(H256::from_low_u64_be(122)),
        });

        // setup mock flow to fail when setting nonces ready
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_get_hash_to_sign()
            .times(1)
            .returning(move || Some(hash_to_sign));
        mock_flow
            .expect_set_all_nonces_ready()
            .with(eq(block_number))
            .times(1)
            .returning(|_| Err(anyhow!("Failed to set nonces ready")));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_rsk_event(&event);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to set nonces ready")
        );
    }

    #[test]
    fn test_process_new_block_with_multiple_flows() {
        let flow_id_1 = Uuid::new_v4();
        let flow_id_2 = Uuid::new_v4();

        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup first flow - nonces confirmed
        let mut mock_flow_1 = MockBtcSignatureFlowApi::new();
        mock_flow_1
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow_1
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow_1
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow_1
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        // setup second flow - signatures confirmed
        let mut mock_flow_2 = MockBtcSignatureFlowApi::new();
        mock_flow_2
            .expect_blockchain_view()
            .times(1) // only called during process_new_block, not during close_flow (since flow_1 still exists)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow_2
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow_2
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id_1, mock_flow_1);
        processor.flows.insert(flow_id_2, mock_flow_2);

        let result = processor.process_new_block(&block);

        assert!(result.is_ok());
        // flow_2 should be closed (signatures confirmed), flow_1 should remain
        assert_eq!(processor.flows.len(), 1);
        assert!(processor.flows.contains_key(&flow_id_1));
        assert!(!processor.flows.contains_key(&flow_id_2));
    }

    #[test]
    fn test_process_new_block_both_nonces_and_signatures_confirmed() {
        let flow_id = Uuid::new_v4();

        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow where both nonces and signatures are confirmed
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(2) // one on process_new_block and one on close_flow
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_block(&block);

        assert!(result.is_ok());
        // flow should be closed because signatures are confirmed
        assert!(processor.flows.is_empty());
    }

    #[test]
    fn test_process_new_bitvmx_event_flow_already_exists_fails() {
        // create signature data for the event
        let flow_id = Uuid::new_v4();
        let hash_to_sign = Hash256::from(H256::random());
        let nonce = "0279BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798032DE2662628C90B03F5E720284EB52FF7D71F4284F627B68A853D78C78E1FFE93".parse::<PubNonce>().unwrap();
        let signature = "44477400e59c41025e4e18c4de244b90b14554dcdcbfa396ead4659aa6343249"
            .parse()
            .unwrap();
        let signing_info = BitVmxSigningInfo {
            protocol_name: "pegin".to_string(),
            take_aggr_key: PublicKey::from_str("04c4b0bbb339aa236bff38dbe6a451e111972a7909a126bc424013cba2ec33bc38e98ac269ffe028345c31ac8d0a365f29c8f7e7cfccac72f84e1acd02bc554f35").unwrap(),
            hash_to_sign,
            nonce,
            signature,
        };
        let signature_json = serde_json::to_string(&signing_info).unwrap();

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String(signature_json),
        );

        // setup existing flow
        let existing_flow = MockBtcSignatureFlowApi::new();
        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, existing_flow);

        let result = processor.process_new_bitvmx_event(&event);

        // currently the code doesn't fail when flow already exists, it just uses the existing one
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Flow already exists with ID")
        );
    }

    #[test]
    fn test_close_flow_clears_blockchain_view_when_last_flow() {
        let flow_id_1 = Uuid::new_v4();
        let flow_id_2 = Uuid::new_v4();

        // setup two flows
        let mock_flow_1 = MockBtcSignatureFlowApi::new();
        // flow_1 doesn't expect blockchain_view to be called because it's not the last flow

        let mut mock_flow_2 = MockBtcSignatureFlowApi::new();
        mock_flow_2
            .expect_blockchain_view()
            .times(1) // called when it's the last flow to be removed
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id_1, mock_flow_1);
        processor.flows.insert(flow_id_2, mock_flow_2);

        // close first flow - blockchain view should not be cleared
        processor.close_flow(flow_id_1);
        assert_eq!(processor.flows.len(), 1);

        // close second flow - blockchain view should be cleared
        processor.close_flow(flow_id_2);
        assert!(processor.flows.is_empty());
    }

    #[test]
    fn test_process_new_block_send_signature_fails() {
        let flow_id = Uuid::new_v4();

        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to simulate confirmed nonces but signature sending fails
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signagures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Err(anyhow!("Contract call failed")));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_block(&block);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Contract call failed")
        );
    }

    #[test]
    fn test_process_new_block_confirmation_check_fails() {
        let flow_id = Uuid::new_v4();

        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to fail when checking nonce confirmation
        let mut mock_flow = MockBtcSignatureFlowApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Err(anyhow!("Failed to check nonce confirmation")));

        let mock_factory = MockBtcSignatureFlowFactoryApi::<MockBtcSignatureFlowApi>::new();
        let mut processor = BtcSignatureFlowProcessor::new(mock_factory);
        processor.flows.insert(flow_id, mock_flow);

        let result = processor.process_new_block(&block);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to check nonce confirmation")
        );
    }
}
