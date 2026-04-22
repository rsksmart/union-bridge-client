// default operator IDs for local deployments (overridable via NUM_OPERATORS env var)
pub const DEFAULT_NUM_OPERATORS: u8 = 4;
pub const MAX_OPERATORS: u8 = 10;

pub fn operator_ids() -> Vec<u8> {
    let count = std::env::var("NUM_OPERATORS")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(DEFAULT_NUM_OPERATORS)
        .min(MAX_OPERATORS);
    (1..=count).collect()
}

pub fn operator_and_prover_counts() -> (u64, u64) {
    let operator_count = u64::try_from(operator_ids().len()).expect("operator count fits in u64");
    let prover_count = operator_count.div_ceil(2);
    (operator_count, prover_count)
}

// local anvil default address
pub const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

// Must stay aligned with the packet size used by the committee setup flow.
pub const COMMITTEE_PACKET_SIZE: u64 = 100; // TODO this should come from config or contracts settings
pub const UNION_BRIDGE_DIR: &str = ".union_bridge";
