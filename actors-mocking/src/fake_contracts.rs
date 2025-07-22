use alloy_sol_types::sol;

sol!(
    // solc v0.8.26; solc FakePegManager.sol --via-ir --optimize --bin
    #[sol(
        rpc,
        bytecode = "60808060405234601557610300908161001a8239f35b5f80fdfe60806040526004361015610011575f80fd5b5f3560e01c80631500d10d1461013757806368a5a3e3146100ac5763c0e254b31461003a575f80fd5b346100a85760203660031901126100a85760043567ffffffffffffffff81116100a8576100a361008f7fb1f222903c5f107a05ac83d2c9adc8a29ef824132cbf3a7c8549edde4cc4784b92369060040161022a565b6040519182916020835260208301906102a6565b0390a1005b5f80fd5b346100a85760403660031901126100a85760043567ffffffffffffffff81116100a8576100dd90369060040161022a565b60243567ffffffffffffffff81168091036100a8577f0a9fb39bfe27b96188cc03a418b8ba609cee80431e89a30497d55fb6e07492b59161012c916040519283926040845260408401906102a6565b9060208301520390a1005b346100a85760a03660031901126100a85760043567ffffffffffffffff81116100a85761016890369060040161022a565b60243567ffffffffffffffff81116100a85761018890369060040161022a565b9060443567ffffffffffffffff81116100a8576101a990369060040161022a565b9060843563ffffffff81168091036100a8576102176101fb936102097f02926e2d41e145ca1df4aea6bc003da88793f052271f659ae4c9597aac34c2079660405196879660a0885260a08801906102a6565b9086820360208801526102a6565b9084820360408601526102a6565b90606435606084015260808301520390a1005b81601f820112156100a85780359067ffffffffffffffff82116102925760405192601f8301601f19908116603f0116840167ffffffffffffffff81118582101761029257604052828452602083830101116100a857815f926020809301838601378301015290565b634e487b7160e01b5f52604160045260245ffd5b805180835260209291819084018484015e5f828201840152601f01601f191601019056fea264697066735822122055b5fadff19ce34aeb21bb91d9e38585a8c4a7dd8eb106696658f11bc2c371fe64736f6c634300081e0033"
    )]
    #[derive(Eq, PartialEq, Debug)]
    contract FakePegManager {
        event RequestAdvanceFunds(
            string pegout_id,
            uint64 amount
        );

        function requestAdvanceFunds(
            string memory pegout_id,
            uint64 amount
        ) public {
            emit RequestAdvanceFunds(pegout_id, amount);
        }

        event AdvanceFunds(
            string pegout_id,
            string utxo_id,
            string operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        );

        function advanceFunds(
            string memory pegout_id,
            string memory utxo_id,
            string memory operator_id,
            uint256 required_effort,
            uint32 required_num_blocks
        ) public {
            emit AdvanceFunds(pegout_id, utxo_id, operator_id, required_effort, required_num_blocks);
        }

        event CheckForkComplete(
            string pegout_id
        );

        function checkForkComplete(
            string memory pegout_id
        ) public {
            emit CheckForkComplete(pegout_id);
        }
    }
);
