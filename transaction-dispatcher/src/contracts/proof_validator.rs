use alloy_sol_types::sol;

sol!(
    #[sol(rpc)]
    SolProofValidator,
    "../config/dev/abi/ProofValidator.json" // TODO we could also use bytecode here, automate deploys for testing, etc.
);
