# Regtest Instance Runbook

This runbook is for the shared regtest environment centered on:

- `union-bridge-use2-1.regtest.rskcomputing.net`
- `node-use2-1.regtest.rskcomputing.net`
- `powpeg-use2-1.regtest.rskcomputing.net`
- `powpeg-use2-2.regtest.rskcomputing.net`

## Source-of-truth branch sync

If you pushed new code to the branch that the regtest instance should use, update `~/union-bridge-client` on `union-bridge-use2-1` before testing.

Use `git pull` when the remote checkout can fetch from origin cleanly. If the remote host cannot fetch from GitHub directly, sync the branch state with the approved bundle workflow instead.

## When a Docker rebuild is mandatory

Rebuild `latest-regtest` before using the instance when either of these is true:

- the checkout on `union-bridge-use2-1` changed to a different branch or commit that should be tested
- the new branch state includes Rust source changes that affect the Union client binaries or the BitVMX client image contents

Command:

```bash
cd ~/union-bridge-client/docker/build
bash d-compose-cli.sh build --tag=latest-regtest --no-cache
```

Shell-only changes such as test scripts or host-side orchestration scripts do not require this rebuild unless they also change what gets baked into the images.

## Fresh regtest cycle

After syncing branch state, and after rebuilding `latest-regtest` when required, run:

```bash
cd ~/union-bridge-client
./cli-infra.sh --start-regtest --fresh
```

This is the supported way to refresh the regtest environment. It redeploys contracts, rewrites regtest config, and restarts the operator stack on `union-bridge-use2-1`.

## Happy-path validation

After the fresh cycle completes, validate the instance with:

```bash
cd ~/union-bridge-client
bash tests/run-happy-path-regtest.sh
```

Only treat the instance update as successful if this script exits `0`.

## Current operational notes

- `node-use2-1` must preserve its special gas/consensus config. Do not overwrite `/etc/rsk/node.conf` casually.
- `powpeg-use2-1` is the active PowPeg runtime used by the happy-path flow.
- `powpeg-use2-2` may be updated on disk without being brought online, depending on the maintenance task.
- The repo on `union-bridge-use2-1` usually has runtime-modified tracked files under `config/environment/regtest.toml` and `docker/bitvmx-client/config/regtest/client/config/op_*.yaml`. Do not reset them blindly during branch sync.
