use crate::types::{Address, BlockNumber, RskBlock, RskLog, Selector};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum BrokerRequests {
    SubscribeBlocks,
    UnsubscribeBlocks,
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - implement block retrieval, it will be needed to get the first blocks that might be missing after the trigger event (event received after block)
    GetBlock(BlockNumber),
    SubscribeLogs(Address),
    UnsubscribeLogs(Address),
    // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-132 - add a limit time for receiving a response?
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BrokerResponses {
    Block(RskBlock),
    Log(RskLog),
}

// TODO(Jira) https://rsklabs.atlassian.net/browse/UB-3 - build on boot using generated alloy event types and a configured PegManager address:
pub struct FakePegManagerConfig {}

impl FakePegManagerConfig {
    pub fn get_peg_manager_address() -> Address {
        Address::try_from("0x663B50C9DA9Bd586f855aF13e91EF2f0954c9761").expect("Invalid address")
    }

    pub fn get_request_advance_funds_selector() -> Selector {
        Selector {
            sig_hash: "0x00000000".to_string(),
            address: Self::get_peg_manager_address(),
        }
    }

    pub fn get_kickoff_advance_funds_selector() -> Selector {
        Selector {
            sig_hash: "0x00000000".to_string(),
            address: Self::get_peg_manager_address(),
        }
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
        // TODO(Jira) https://rsklabs.atlassian.net/browse/UB-134 -
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
