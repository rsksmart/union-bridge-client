// default operator IDs for local deployments (overridable via NUM_OPERATORS env var)
pub const DEFAULT_NUM_OPERATORS: u8 = 4;
pub const MAX_OPERATORS: u8 = 10;

// Committee setup needs 32,000,000 sat of BitVMX outputs on regtest.
// Keep extra headroom for the SendFunds fee so setup does not fail on a
// marginally larger spend with "out of funds".
pub const DEFAULT_OPERATOR_FUND_AMOUNT: u64 = 32_100_000;

pub fn operator_ids() -> Vec<u8> {
    let count = std::env::var("NUM_OPERATORS")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(DEFAULT_NUM_OPERATORS)
        .min(MAX_OPERATORS);
    (1..=count).collect()
}

// project name for one-operator deployments
pub const ONE_OPERATOR_COMPOSE_PROJECT: &str = "union-operator";

// local anvil default address
pub const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
