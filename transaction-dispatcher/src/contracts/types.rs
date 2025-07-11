use alloy_rpc_types::TransactionReceipt;
pub type FixedBytes32 = alloy_primitives::FixedBytes<32>;
pub type Bytes = alloy_sol_types::private::Bytes;
pub type TransactionReceiptResult = alloy_contract::Result<TransactionReceipt>;
pub type Address = alloy_primitives::Address;
