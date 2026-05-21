// default operator IDs for local deployments (overridable via NUM_OPERATORS env var)
pub(crate) const DEFAULT_NUM_OPERATORS: u8 = 4;
pub(crate) const MAX_OPERATORS: u8 = 10;

pub(crate) fn operator_ids() -> Vec<u8> {
    let count = std::env::var("NUM_OPERATORS")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(DEFAULT_NUM_OPERATORS)
        .min(MAX_OPERATORS);
    (1..=count).collect()
}

// local anvil default address
pub(crate) const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
