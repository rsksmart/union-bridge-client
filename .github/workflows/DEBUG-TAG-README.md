# Debug Tag for Act/GitHub Actions Troubleshooting

## Overview

The `v0.2.0-debug-act` tag includes extensive logging to help debug issues when running workflows in `act` (local GitHub Actions) or GitHub Actions.

## What's Included

### Enhanced Logging in `rsk_wallet.rs`

The `run_cast_send_local` function now includes:

1. **Pre-execution checks:**
   - Cast version verification
   - RPC connectivity test (`eth_blockNumber`)
   - Full command logging

2. **Execution timing:**
   - Start time tracking
   - Duration measurement
   - Exit code logging

3. **Complete output capture:**
   - Full stdout capture and logging
   - Full stderr capture and logging
   - Byte counts for both streams

4. **Enhanced error messages:**
   - Includes stdout, stderr, RPC URL, and duration in error messages
   - Helps identify connection issues, timeouts, or cast errors

## Creating the Debug Tag

From the `union-bridge-client` repository root:

```bash
# Make the script executable
chmod +x .github/workflows/create-debug-tag.sh

# Run the script
.github/workflows/create-debug-tag.sh

# Push the tag to remote
git push origin v0.2.0-debug-act
```

## Using the Debug Tag

### In Act (Local Testing)

```bash
# From union_bridge_e2e_framework directory
./scripts/test-act-sandbox.sh v0.2.0-debug-act
```

### In GitHub Actions

Set the `client_ref` input when manually triggering the workflow:

```yaml
workflow_dispatch:
  inputs:
    client_ref:
      value: 'v0.2.0-debug-act'
```

Or modify `.github/workflows/versions.yml`:

```yaml
client_ref: v0.2.0-debug-act
```

## Debugging Cast Send Failures

When `cast send` fails, the enhanced logging will show:

1. **Cast version** - Verifies cast is installed correctly
2. **RPC connectivity** - Tests if Anvil is accessible before attempting send
3. **Command details** - Full command being executed
4. **Timing** - How long the command took
5. **Exit code** - Process exit status
6. **stdout/stderr** - Complete output from cast command

### Example Debug Output

```
[DEBUG] =========================================
[DEBUG] run_cast_send_local: Starting funding
[DEBUG] Address: 0x98b43217B8a0edd7BD942144e0c185247477c4B6
[DEBUG] RPC URL: http://127.0.0.1:8545
[DEBUG] From address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
[DEBUG] cast version: cast 0.2.0
[DEBUG] Checking RPC connectivity...
[DEBUG] RPC connectivity OK: 0x123
[DEBUG] Executing: cast send --rpc-url http://127.0.0.1:8545 ...
[DEBUG] cast send completed in 2.5s
[DEBUG] Exit code: Some(0)
[DEBUG] stdout (45 bytes): ...
[DEBUG] stderr (0 bytes): 
[DEBUG] cast send SUCCESS
[DEBUG] =========================================
```

## Understanding Bitcoin-Wallet Slowness

The bitcoin-wallet commands are executed via `BaseCliAdapter` in the e2e framework, which already logs:
- Command execution time
- Real-time stdout/stderr
- Timeout warnings (at 80% of timeout)

If commands are slow, check:
1. **Rust compilation time** - First run compiles the binary
2. **Network latency** - RPC calls to bitcoind
3. **Database operations** - UTXO database reads/writes
4. **Docker overhead** - If running in containers

## Reverting to Production Version

To switch back to the production version:

```bash
# In act
./scripts/test-act-sandbox.sh v0.2.0

# Or remove client_ref input to use default
./scripts/test-act-sandbox.sh
```

## Notes

- The debug tag is based on `v0.2.0`
- All logging goes to stderr (eprintln!) so it appears in CI logs
- The tag can be updated by deleting and recreating it
- Production code should not include this extensive logging
