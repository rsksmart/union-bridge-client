use alloy_sol_types::sol;

sol!(
    // solc v0.8.26; solc FakePegManager.sol --via-ir --optimize --bin
    #[sol(
        rpc,
        bytecode = "6080806040523460155761028e908161001a8239f35b5f80fdfe60806040526004361015610011575f80fd5b5f3560e01c806399ebd28d1461012a5763c13959081461002f575f80fd5b346101265760c03660031901126101265760243567ffffffffffffffff8111610126576100609036906004016101b8565b60443567ffffffffffffffff8111610126576100809036906004016101b8565b9060643567ffffffffffffffff8111610126576100a19036906004016101b8565b9060a4359263ffffffff8416809403610126577f5eaad33b212195ded2a453a276ee5fb782b8303e6ecf39df7e7059c09bfc6c779261010061010e926100f26040519560a0875260a0870190610234565b908582036020870152610234565b908382036040850152610234565b926084356060830152608082015280600435930390a2005b5f80fd5b346101265760603660031901126101265760043567ffffffffffffffff81116101265761015b9036906004016101b8565b6044359067ffffffffffffffff8216809203610126577f4a2dea8b27f26fca7d713a9fdc4434df46d7a236a28d23e6e242c86631db5425906101a860405191604083526040830190610234565b92602082015280602435930390a2005b81601f820112156101265780359067ffffffffffffffff82116102205760405192601f8301601f19908116603f0116840167ffffffffffffffff811185821017610220576040528284526020838301011161012657815f926020809301838601378301015290565b634e487b7160e01b5f52604160045260245ffd5b805180835260209291819084018484015e5f828201840152601f01601f191601019056fea26469706673582212202d592284e826b53ebb89683f53f320e59520f2235e8c223245fa2a8d78f1d7b064736f6c634300081e0033"
    )]
    #[derive(Eq, PartialEq, Debug)]
    contract FakePegManager {
        event RequestAdvanceFunds(
            bytes32 indexed block_hash,
            string peg_out_id,
            uint64 amount
        );

        function requestAdvanceFunds(
            string memory peg_out_id,
            bytes32 block_hash,
            uint64 amount
        ) public {
            emit RequestAdvanceFunds(block_hash, peg_out_id, amount);
        }

        event KickoffAdvanceFunds(
            bytes32 indexed block_hash,
            string peg_out_id,
            string utxo_id,
            string operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        );

        function kickoffAdvanceFunds(
            bytes32 block_hash,
            string memory peg_out_id,
            string memory utxo_id,
            string memory operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        ) public {
            emit KickoffAdvanceFunds(block_hash, peg_out_id, utxo_id, operator_id, required_effort, required_num_blocks);
        }
    }
);
