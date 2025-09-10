#!/bin/bash

set -euo pipefail

# Constants
ANVIL_ADDRESS="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

cast send --unlocked --from "$ANVIL_ADDRESS" <OPERATOR_1_ADDRESS> --value 1000000000000000000 --rpc-url http://127.0.0.1:8545
sleep 2
cast send --unlocked --from "$ANVIL_ADDRESS" <OPERATOR_2_ADDRESS> --value 1000000000000000000 --rpc-url http://127.0.0.1:8545
sleep 2
cast send --unlocked --from "$ANVIL_ADDRESS" <OPERATOR_3_ADDRESS> --value 1000000000000000000 --rpc-url http://127.0.0.1:8545
sleep 2
cast send --unlocked --from "$ANVIL_ADDRESS" <OPERATOR_4_ADDRESS> --value 1000000000000000000 --rpc-url http://127.0.0.1:8545