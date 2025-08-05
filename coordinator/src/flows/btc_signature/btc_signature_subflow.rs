use super::btc_signature_lifecycle::{BtcSignatureLifeCycle, BtcSignatureLifecycleApi};
use crate::types::RskPegManagerEvents;
use anyhow::{Result, anyhow, bail};
use common::msg_broker::bitvmx_types::{
    BitVmxSigningInfo, OutgoingBitVMXApiMessages, VariableTypes,
};
use common::types::RskBlockAndUncles;

use common::runtime_sync::RuntimeSync;
use log::info;
use std::rc::Rc;
use transaction_dispatcher::rsk_gateway::RskContractsGatewayApi;
use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

pub(crate) const SIGNATURE_MESSAGE: &str = "signing_info";

#[cfg_attr(test, automock)]
pub(crate) trait BtcSignatureSubFlowApi {
    fn delegate_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()>;
    fn delegate_rsk_event(&mut self, flow_id: Uuid, event: &RskPegManagerEvents) -> Result<()>;
    fn delegate_block(&mut self, block: &RskBlockAndUncles) -> Result<()>;
    fn is_done(&self) -> bool;
}

#[cfg_attr(test, automock)]
pub(crate) trait BtcSignatureSubFlowFactoryApi<BSF: BtcSignatureSubFlowApi> {
    fn create_flow(&self, flow_id: Uuid) -> BSF;
}

/// ergonomic type alias for tests usage
#[cfg(test)]
pub(crate) type MockBtcSigSubFlowFactory =
    MockBtcSignatureSubFlowFactoryApi<MockBtcSignatureSubFlowApi>;

pub(crate) struct BaseBtcSignatureSubFlow<BSF: BtcSignatureLifecycleApi> {
    lifecycle: BSF,
    is_done: bool,
}

impl<CG> BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>
where
    CG: RskContractsGatewayApi,
{
    pub(crate) fn new(contracts_gateway: Rc<CG>, rt_sync: RuntimeSync, flow_id: Uuid) -> Self {
        let lifecycle =
            BtcSignatureLifeCycle::new(contracts_gateway.clone(), rt_sync.clone(), flow_id);

        Self {
            lifecycle,
            is_done: false,
        }
    }
}

impl<BSF> BtcSignatureSubFlowApi for BaseBtcSignatureSubFlow<BSF>
where
    BSF: BtcSignatureLifecycleApi,
{
    fn delegate_bitvmx_event(&mut self, event: &OutgoingBitVMXApiMessages) -> Result<()> {
        match event {
            OutgoingBitVMXApiMessages::Variable(flow_id, method, VariableTypes::String(data))
                if matches!(method.as_str(), SIGNATURE_MESSAGE) =>
            {
                if self.lifecycle.flow_id() != *flow_id {
                    return Ok(()); // not mine
                }

                let signing_info = serde_json::from_str::<BitVmxSigningInfo>(data)
                    .map_err(|e| anyhow!("Failed to deserialize signature data: {e}"))?;

                self.lifecycle.send_nonce_to_contracts(&signing_info)?;

                Ok(())
            }
            _ => {
                bail!("Unexpected BitVMX event in BtcSignatureSubFlow: {event:?}")
            }
        }
    }

    fn delegate_rsk_event(&mut self, flow_id: Uuid, event: &RskPegManagerEvents) -> Result<()> {
        if self.lifecycle.flow_id() != flow_id {
            return Ok(()); // not mine
        }

        info!("Handling delegated event {event:?} for flow {flow_id} and event");

        match event {
            RskPegManagerEvents::AllNoncesReady(event) => {
                if let Some(hash) = self.lifecycle.get_hash_to_sign() {
                    //the hash is used to check if the event is for the current flow if not, it is ignored
                    if hash == event.inner {
                        if event.removed {
                            self.lifecycle.unset_all_nonces_ready()?;
                        } else {
                            self.lifecycle.set_all_nonces_ready(event.block_number)?;
                        }
                    }
                }
                Ok(())
            }
            RskPegManagerEvents::AllSignaturesReady(event) => {
                if let Some(hash) = self.lifecycle.get_hash_to_sign() {
                    //the hash is used to check if the event is for the current flow if not, it is ignored
                    if hash == event.inner {
                        if event.removed {
                            self.lifecycle.unset_all_signatures_ready()?;
                        } else {
                            self.lifecycle
                                .set_all_signatures_ready(event.block_number)?;
                        }
                    }
                }
                Ok(())
            }
            _ => bail!("Unexpected RSK event in BtcSignatureSubFlow: {event:?}"),
        }
    }

    fn delegate_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        // update blockchain view
        self.lifecycle
            .blockchain_view()
            .borrow_mut()
            .update(block.clone());

        // check if nonces are ready and send signature
        if self.lifecycle.is_all_nonces_ready_confirmed()? {
            self.lifecycle.send_signature_to_contracts()?;
        }

        // check if signatures are ready and close flow
        if self.lifecycle.is_all_signatures_ready_confirmed()? {
            self.is_done = true;
            self.lifecycle.blockchain_view().borrow_mut().clear();
        }

        Ok(())
    }

    fn is_done(&self) -> bool {
        self.is_done
    }
}

pub(crate) struct BtcSignatureSubFlowFactory<CG: RskContractsGatewayApi> {
    contracts_gateway: Rc<CG>,
    rt_sync: RuntimeSync,
}

impl<CG: RskContractsGatewayApi> BtcSignatureSubFlowFactory<CG> {
    pub(crate) fn new(contracts_gateway: Rc<CG>, rt_sync: RuntimeSync) -> Self {
        Self {
            contracts_gateway,
            rt_sync,
        }
    }
}

impl<CG: RskContractsGatewayApi>
    BtcSignatureSubFlowFactoryApi<BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>>>
    for BtcSignatureSubFlowFactory<CG>
{
    fn create_flow(&self, flow_id: Uuid) -> BaseBtcSignatureSubFlow<BtcSignatureLifeCycle<CG>> {
        BaseBtcSignatureSubFlow::<BtcSignatureLifeCycle<CG>>::new(
            self.contracts_gateway.clone(),
            self.rt_sync.clone(),
            flow_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain_tracker::BlockchainView;
    use crate::flows::btc_signature::btc_signature_lifecycle::MockBtcSignatureLifecycleApi;
    use crate::types::{AllNoncesReadyEvent, AllSignaturesReadyEvent};
    use bitcoin::PublicKey;
    use common::test_utils::rsk_block_generator::create_block_and_uncles;
    use common::types::{BlockNumber, Hash256, RskBlockAndUncles, TxHash};
    use mockall::predicate::*;
    use musig2::PubNonce;
    use primitive_types::H256;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::str::FromStr;

    type MockBtcSignatureSubFlow = BaseBtcSignatureSubFlow<MockBtcSignatureLifecycleApi>;

    impl BaseBtcSignatureSubFlow<MockBtcSignatureLifecycleApi> {
        pub(crate) fn new(mock: MockBtcSignatureLifecycleApi) -> Self {
            Self {
                lifecycle: mock,
                is_done: false,
            }
        }
    }

    #[test]
    fn test_process_new_bitvmx_event() {
        // create signature data for the event
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

        let flow_id = Uuid::new_v4();

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String(signature_json),
        );

        // setup mock flow to expect nonce being sent to contracts
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        mock_flow.expect_flow_id().returning(move || flow_id);

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_bitvmx_event(&event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_bitvmx_event_rejects_invalid_event() {
        let event = OutgoingBitVMXApiMessages::Variable(
            Uuid::new_v4(),
            "wrong-message".to_string(),
            VariableTypes::String("data".to_string()),
        );

        let mut sub_flow = MockBtcSignatureSubFlow::new(MockBtcSignatureLifecycleApi::new());

        let result = sub_flow.delegate_bitvmx_event(&event);

        assert!(result.is_err());
    }

    #[test]
    fn test_process_new_rsk_event_all_nonces_ready() {
        // create all nonces ready event
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
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_nonces_ready_removed() {
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

        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_signatures_ready() {
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

        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_all_signatures_ready_removed() {
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

        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_rsk_event_rejects_invalid_rsk_event() {
        let mut sub_flow = MockBtcSignatureSubFlow::new(MockBtcSignatureLifecycleApi::new());

        let event = RskPegManagerEvents::UnknownEvent;

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_err());
    }

    #[test]
    fn test_process_new_block_updates_blockchain_view() {
        // create blocks using the utility function
        let (block_1, _, _) = create_block_and_uncles();

        // use block_1 for this test
        let block = RskBlockAndUncles::new_no_uncles(block_1);

        // setup mock flow to verify blockchain view is updated
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_block_with_confirmed_nonces_sends_signature() {
        let (_block_1, uncle_1, block_2) = create_block_and_uncles();

        let block = RskBlockAndUncles::new(block_2, vec![uncle_1]);

        // setup mock flow to simulate confirmed nonces
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_new_block_with_confirmed_signatures_closes_flow() {
        // create blocks using the utility function
        let (block_1, uncle_1, _) = create_block_and_uncles();

        // use block_1 with uncle_1 for this test
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to simulate confirmed signatures
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(3) // one on process_new_block, one on close_flow and one to check if empty in this test
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

        assert!(result.is_ok());
        assert!(sub_flow.is_done);
        assert!(sub_flow.lifecycle.blockchain_view().borrow().is_empty());
    }

    #[test]
    fn test_process_new_block_with_multiple_blocks() {
        // create test data
        // create blocks using the utility function
        let (block_1, uncle_1, block_2) = create_block_and_uncles();

        // process block_1 first
        let block_1_with_uncles = RskBlockAndUncles::new_no_uncles(block_1.clone());

        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));

        // second block: nonces are confirmed
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(false));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        // process block_1
        let result_1 = sub_flow.delegate_block(&block_1_with_uncles);
        assert!(result_1.is_ok());

        // process block_2, which should trigger the nonce confirmation
        let block_2_with_uncles = RskBlockAndUncles::new(block_2, vec![uncle_1]);
        let result_2 = sub_flow.delegate_block(&block_2_with_uncles);
        assert!(result_2.is_ok());
    }

    #[test]
    fn test_process_new_bitvmx_event_invalid_json() {
        let flow_id = Uuid::new_v4();

        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String("invalid json".to_string()),
        );

        let mut sub_flow = MockBtcSignatureSubFlow::new(MockBtcSignatureLifecycleApi::new());
        sub_flow
            .lifecycle
            .expect_flow_id()
            .times(1)
            .returning(move || flow_id);

        let result = sub_flow.delegate_bitvmx_event(&event);

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

        let flow_id = Uuid::new_v4();
        let event = OutgoingBitVMXApiMessages::Variable(
            flow_id,
            SIGNATURE_MESSAGE.to_string(),
            VariableTypes::String(signature_json),
        );

        // setup mock flow to fail when sending nonce
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_send_nonce_to_contracts()
            .times(1)
            .returning(|_| Err(anyhow!("Contract call failed")));
        mock_flow
            .expect_blockchain_view()
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);
        sub_flow
            .lifecycle
            .expect_flow_id()
            .times(1)
            .returning(move || flow_id);

        let result = sub_flow.delegate_bitvmx_event(&event);

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
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
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

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let flow_id = Uuid::new_v4();
        sub_flow
            .lifecycle
            .expect_flow_id()
            .returning(move || flow_id);

        let result = sub_flow.delegate_rsk_event(flow_id, &event);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to set nonces ready")
        );
    }

    #[test]
    fn test_process_new_block_both_nonces_and_signatures_confirmed() {
        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow where both nonces and signatures are confirmed
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(3) // one on process_new_block, one on close_flow and one to check if empty in this test
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_is_all_signatures_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Ok(()));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

        assert!(result.is_ok());
        // flow should be closed because signatures are confirmed
        assert!(sub_flow.lifecycle.blockchain_view().borrow().is_empty());
    }

    #[test]
    fn test_process_new_block_send_signature_fails() {
        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to simulate confirmed nonces but signature sending fails
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Ok(true));
        mock_flow
            .expect_send_signature_to_contracts()
            .times(1)
            .returning(|| Err(anyhow!("Contract call failed")));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

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
        let (block_1, uncle_1, _) = create_block_and_uncles();
        let block = RskBlockAndUncles::new(block_1, vec![uncle_1]);

        // setup mock flow to fail when checking nonce confirmation
        let mut mock_flow = MockBtcSignatureLifecycleApi::new();
        mock_flow
            .expect_blockchain_view()
            .times(1)
            .returning(|| Rc::new(RefCell::new(BlockchainView::new())));
        mock_flow
            .expect_is_all_nonces_ready_confirmed()
            .times(1)
            .returning(|| Err(anyhow!("Failed to check nonce confirmation")));

        let mut sub_flow = MockBtcSignatureSubFlow::new(mock_flow);

        let result = sub_flow.delegate_block(&block);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to check nonce confirmation")
        );
    }
}
