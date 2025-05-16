use crate::types::RskPegManagerEvents::UnknownEvent;
use alloy_primitives::{Address, B256, Bytes, FixedBytes, LogData, U256};
use alloy_sol_types::SolEvent;
use common::types::{BlockNumber, RskLog};
use log::{error, warn};
use std::ops::Add;
use std::str::FromStr;
use union_contracts::bindings::pegmanager::PegManager::RegisteredPegInRequest;

pub(crate) type PegOutId = String;

#[derive(Eq, PartialEq, Debug)]
pub enum RskPegManagerEvents {
    RequestAdvanceFunds(RequestAdvanceFunds),
    RemoveRequestAdvanceFunds {
        peg_out_id: PegOutId,
    },
    KickoffAdvanceFunds {
        // TODO add other fields
        peg_out_id: PegOutId,
        block_num: BlockNumber,
    },
    RemoveKickoffAdvanceFunds {
        peg_out_id: PegOutId,
    },
    RegisteredPegInRequest(RegisteredPegInRequest),
    UnknownEvent,
}

// TODO(Jira-ContractImplemented) temporary approach until required contract gets implemented
#[derive(Eq, PartialEq, Debug)]
pub struct RequestAdvanceFunds {
    pub peg_out_id: PegOutId,
    pub block_num: BlockNumber,
    pub amount: u64,
}

pub fn decode_rsk_log_to_peg_manager_event(log: RskLog) -> RskPegManagerEvents {
    let parsed_topics: Vec<B256> = log
        .event()
        .topics()
        .iter()
        .filter_map(|topic| topic.parse::<B256>().ok())
        .collect();

    let topic0 = parsed_topics.get(0).cloned();

    let hex_data = match alloy_primitives::hex::decode(&log.event().data()) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to decode RSK log {:?}: {}", log, e);
            return RskPegManagerEvents::UnknownEvent;
        }
    };

    let log_data = LogData::new(parsed_topics, hex_data.into());
    if log_data.is_none() {
        error!("Failed to create Alloy LogData from rsk_log");
        return UnknownEvent;
    }

    let log_data = log_data.unwrap();

    // TODO(Jira-ContractImplemented) temporary approach until required contract gets implemented
    if log.info().address()
        == common::types::Address::try_from("0x9d4b2c05818a0086e641437fcb64ab6098c7bbec")
            .expect("Invalid address")
    {
        return RskPegManagerEvents::RequestAdvanceFunds(RequestAdvanceFunds {
            peg_out_id: "fake_pegout_1".to_string(),
            block_num: BlockNumber::from(1),
            amount: 1000,
        });
    }

    match topic0 {
        Some(ev) if *ev == RegisteredPegInRequest::SIGNATURE_HASH => {
            decode_register_pegin_event(&log_data)
        }
        // TODO add other types here in the future
        _ => {
            warn!("Unknown event type in log {:?}", log_data);
            UnknownEvent
        }
    }
}

fn decode_register_pegin_event(log_data: &LogData) -> RskPegManagerEvents {
    match RegisteredPegInRequest::decode_log_data(&log_data, true) {
        Ok(ev) => RskPegManagerEvents::RegisteredPegInRequest(ev),
        Err(_) => UnknownEvent,
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Dispute {
    pub peg_out_id: PegOutId,
    req_adv_block: BlockNumber,
    req_adv_confirmations: u32,
    kickoff_adv_block: Option<BlockNumber>,
    kickoff_adv_confirmations: u32,
}

const REQ_ADV_CONFIRMATIONS_TOLERANCE_THRESHOLD: f64 = 1.10;

impl Dispute {
    pub fn new(
        peg_out_id: PegOutId,
        req_adv_block: BlockNumber,
        req_adv_confirmations: u32,
        kickoff_adv_confirmations: u32,
    ) -> Self {
        Self {
            peg_out_id,
            req_adv_block,
            req_adv_confirmations,
            kickoff_adv_block: None,
            kickoff_adv_confirmations,
        }
    }

    pub fn set_kickoff(&mut self, block_num: BlockNumber) {
        self.kickoff_adv_block = Some(block_num);
    }

    pub fn unset_kickoff(&mut self) {
        self.kickoff_adv_block = None;
    }

    pub fn is_complete_on(&self, last_block: &BlockNumber) -> bool {
        match self.kickoff_adv_block {
            Some(b) => last_block >= &b.add(self.kickoff_adv_confirmations as u64),
            None => {
                self.log_delayed_kickoff(last_block);
                false
            }
        }
    }

    fn log_delayed_kickoff(&self, last_block: &BlockNumber) {
        let tolerance =
            self.req_adv_confirmations as f64 * REQ_ADV_CONFIRMATIONS_TOLERANCE_THRESHOLD;
        if last_block <= &self.req_adv_block.add(tolerance as u64) {
            warn!("KickoffAdvanceFunds not received yet, but we are past the tolerance threshold");
        }
    }
}

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - build on boot via config
pub struct FakePegManagerConfig {}

impl FakePegManagerConfig {
    pub fn get_peg_manager_address() -> common::types::Address {
        // TODO(iago) from config!!!
        common::types::Address::try_from("0x9d4b2c05818a0086e641437fcb64ab6098c7bbec")
            .expect("Invalid address")
    }

    pub fn get_req_adv_confirmations_for_amount(amount: u64) -> u32 {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 - get threshold from config
        if amount < 1000 {
            10
        } else if amount < 10000 {
            20
        } else if amount < 100000 {
            30
        } else {
            40
        }
    }

    pub fn get_kickoff_adv_confirmations_for_amount(amount: u64) -> u32 {
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 - get threshold from config
        // get threshold from config
        if amount < 1000 {
            5
        } else if amount < 10000 {
            10
        } else if amount < 100000 {
            15
        } else {
            20
        }
    }
}

pub struct FakeEventGenerator {}

impl FakeEventGenerator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn register_pegin_request() -> RegisteredPegInRequest {
        RegisteredPegInRequest {
            blockHash: FixedBytes::<32>::from_str(
                "0x0000000000000000000282fa21665766e58eb6cb94e458c3ef6d4af1121e38d9",
            )
            .expect("Invalid blockHash"),
            txHash: FixedBytes::<32>::from_str(
                "0x8264f7a960bc2f030c740ff08089b202adb73b820a3d7e174edc7626806905bf",
            )
            .expect("Invalid txHash"),
            vout: 0,
            value: 100000,
            packetNumber: U256::from(10),
            rskDestinationAddress: Address::from_str("0x9d4b2c05818a0086e641437fcb64ab6098c7bbec")
                .unwrap(),
            btcReimbursementPubKey: FixedBytes::<32>::from_str(
                "0x7d235c24420b2f55450c8414725aa74e6db01035245efdab0e1cfa7ab29aca0f",
            )
            .expect("Invalid btcReimbursementPubKey"),
            utxoScriptPubKey: Bytes::from_str(
                "0x5120228f281f297fd01cd363b9c93f742ba2976c1ec5a6083d9f754cb61e505356c3",
            )
            .expect("Invalid utxoScriptPubKey"),
        }
    }
}
