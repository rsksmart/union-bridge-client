use alloy_sol_types::sol;

sol!(
    // solc v0.8.26; solc FakePegManager.sol --via-ir --optimize --bin
    #[sol(
        rpc,
        bytecode = "60808060405234601557610287908161001a8239f35b5f80fdfe60806040526004361015610011575f80fd5b5f3560e01c806317f9dfcf146100be576368a5a3e31461002f575f80fd5b346100ba5760403660031901126100ba5760043567ffffffffffffffff81116100ba576100609036906004016101b1565b60243567ffffffffffffffff81168091036100ba577f0a9fb39bfe27b96188cc03a418b8ba609cee80431e89a30497d55fb6e07492b5916100af9160405192839260408452604084019061022d565b9060208301520390a1005b5f80fd5b346100ba5760a03660031901126100ba5760043567ffffffffffffffff81116100ba576100ef9036906004016101b1565b60243567ffffffffffffffff81116100ba5761010f9036906004016101b1565b9060443567ffffffffffffffff81116100ba576101309036906004016101b1565b9060843563ffffffff81168091036100ba5761019e610182936101907f67b49ba174446084853057b1c2a5b55048ca4917e26d46fba6d7cf9bc2b1472a9660405196879660a0885260a088019061022d565b90868203602088015261022d565b90848203604086015261022d565b90606435606084015260808301520390a1005b81601f820112156100ba5780359067ffffffffffffffff82116102195760405192601f8301601f19908116603f0116840167ffffffffffffffff81118582101761021957604052828452602083830101116100ba57815f926020809301838601378301015290565b634e487b7160e01b5f52604160045260245ffd5b805180835260209291819084018484015e5f828201840152601f01601f191601019056fea2646970667358221220248151846548451796738baf5c93aa6cdf3e004397378af953d68fbd0648666764736f6c634300081e0033"
    )]
    #[derive(Eq, PartialEq, Debug)]
    contract FakePegManager {
        event RequestAdvanceFunds(
            string peg_out_id,
            uint64 amount
        );

        function requestAdvanceFunds(
            string memory peg_out_id,
            uint64 amount
        ) public {
            emit RequestAdvanceFunds(peg_out_id, amount);
        }

        event KickoffAdvanceFunds(
            string peg_out_id,
            string utxo_id,
            string operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        );

        function kickoffAdvanceFunds(
            string memory peg_out_id,
            string memory utxo_id,
            string memory operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        ) public {
            emit KickoffAdvanceFunds(peg_out_id, utxo_id, operator_id, required_effort, required_num_blocks);
        }
    }
);
