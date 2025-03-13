use alloy_sol_types::sol;

sol!(
    #[sol(rpc)]
    BitcoinManager,
    "../config/dev/abi/BitcoinManager.json"
);
