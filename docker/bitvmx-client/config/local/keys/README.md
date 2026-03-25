This directory is part of the tracked BitVMX config template.

Runtime private keys are not stored in the repo anymore. During operator setup,
`<project_root>/cli-setup-operators.sh` generates the referenced key files under:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_N/bitvmx/keys/`

For the local template, the generated files are:

- `op_N.key`
- `services.key`
- `l2.key`
- `emulator.key`
- `prover.key`

So, for example, `op_1` ends up with:

- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/op_1.key`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/services.key`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/l2.key`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/emulator.key`
- `${BASE_STORAGE_PATH:-$HOME}/.union_bridge/op_1/bitvmx/keys/prover.key`
