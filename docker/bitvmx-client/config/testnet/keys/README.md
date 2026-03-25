This directory is part of the tracked BitVMX config template.

Runtime private keys are not stored in the repo anymore. During operator setup,
`<project_root>/cli-setup-operators.sh` generates the referenced key files under:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/keys/`

For the testnet template, the generated files are the operator keys referenced
by the `testnet_op_N.yaml` files, for example:

- `op_1.key`
- `op_2.key`
- `...`

So, for example, `op_1` ends up with:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/op_1.key`
