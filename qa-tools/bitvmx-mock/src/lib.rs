pub mod automated_mock;

pub use automated_mock::AutomatedBitVmxMock;

pub use bitcoin::Transaction;
pub use common::msg_broker::bitvmx_types::{
    BtcTxSPVProof, IncomingBitVMXApiMessages, OutgoingBitVMXApiMessages,
    TransactionBlockchainStatus, TransactionStatus, VariableTypes,
};
pub use common::msg_broker::broker::{BitVmxBrokerServer, BITVMX_L2_BROKER_CLIENT_ID};

pub use anyhow::Result;
