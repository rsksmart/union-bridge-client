# v0.3.1 to v0.4.x DB Migration

## Overview

The on-disk format of the coordinator database did not change between `v0.3.1` and `v0.4.x` (same `rust-bitvmx-storage-backend` rev, same key layout, same JSON encoding). What changed is the shape of the Rust structs that the coordinator deserializes. Two new required fields and one config rename break startup for any operator that ran `v0.3.1` against a populated database.

The relevant code path is `coordinator/src/store.rs::load_all_flows`. Restoring flows from disk is fail-fast at the prefix level: if a single row under `setup_committee_flows/`, `pegout_flows/` or `pegin_flows/` fails to deserialize, `restore_flows` returns `Err` and the coordinator refuses to start.

The migration is performed by a stand-alone tool, `tools/migrate-v031/`, that the operator runs once before deploying `v0.4.x`. The tool mutates legacy rows in place. The mutations are idempotent (gated by `is_none()` checks) and additive (no field is removed or rewritten), so reruns are no-ops and a downgrade back to `v0.3.1` still works.

The coordinator binary contains no migration code and is byte-identical to upstream `chore/release/v0.4.x` at the same merge base. The legacy `[bridge.*]` config detection and the post-migration schema verification both live inside the tool itself: pass `--config <toml-path>` to refuse migration if the TOML still has the legacy section, and the tool always probes the DB at the end to confirm the v0.4.x schema. If the operator forgets to run the tool, the v0.4.x coordinator's existing `restore_flows` will fail at startup with a deserialization error pointing at the missing field.

Stable across the upgrade: `global_context` and the `PersistentGlobalContext` struct (committees, take key, dispute key, comm key), `StoreKey` and `StorePrefix`, every `Steps` enum variant from `v0.3.1` (new variants were added but none removed), and the storage backend itself. Operator keys and committee membership are preserved without any DB action.

## Incompatibilities

Audited against `v0.3.1` at `4c286d981ebda52cbb353427ccfccc33dc4bfe0b` and `v0.4.x` at the head of `chore/release/v0.4.x`. Only on-disk persisted state is listed; runtime-only and wire-only changes are out of scope.

| Row prefix or file | What changed | Migration action |
| --- | --- | --- |
| `setup_committee_flows/*` | `ctx.setup_full_penalization_req: SetupChannelReq` is new and required, no default | Inject `[]` if missing |
| `pegout_flows/*` | `ctx.request_pegout_tx_hash: String` is new and required, no default | Inject `""` if missing; warn for in-flight rows |
| `pegin_flows/*` | `ctx.operator_take_txid: Option<Txid>` and `ctx.operator_won_txid: Option<Txid>` are new (Option, so missing rows still deserialize), but the flow expects them populated by step `AddOperatorTakeHash` | If missing, copy from `ctx.bitvmx_pegin_accepted.{operator_take_txid, operator_won_txid}` |
| `config/*.toml` | The `[bridge.*]` section was renamed and split across `[flows.*]` and `[coordinator]` | Manual edit before deploy. `migrate-v031 --config <toml-path>` refuses to migrate while the legacy section is still present |

If after the migration any row still fails to deserialize as a `v0.4.x` `State`, the next call to `restore_flows` returns `Err` and the binary exits with a non-zero code. systemd or docker do not advance, the operator sees the error in logs, and they can restore from snapshot.

### Why These Defaults

- `setup_committee_flows` → `[]`: not a fallback. The field is only populated during the new `FullPenalizationSetup` step, which did not exist in `v0.3.1`, so an empty list is the actual representative value for any pre-existing row.
- `pegin_flows` → lift from `bitvmx_pegin_accepted`: this is the calculated value, not a default. When the source field is also missing, the flow is at an early step where the txids did not exist anywhere in `v0.3.1` either; `v0.4.x` populates them naturally when it executes the new `RequestOperatorTakeTransactionInfo` / `RequestOperatorWonTransactionInfo` steps.
- `pegout_flows` → `""`: the only real default-vs-reconstruct decision. The original tx_hash comes from the `EventWithBlock` wrapper, which only the log-indexer persists in a separate DB. Reconstructing it would require a cross-DB lookup against the log-indexer keys. Left as future work; revisit if in-flight pegout warnings show up in production.

## Config Rename

| `v0.3.1` | `v0.4.x` |
| --- | --- |
| `[bridge.coordinator] required_confirmations` | `[flows.common] rsk_confirmations` |
| `[bridge.coordinator] check_period_secs` | `[coordinator] check_period_secs` |
| `[bridge.coordinator] bitvmx_not_responding_threshold_secs` | `[coordinator] bitvmx_not_responding_threshold_secs` |
| `[bridge.coordinator] bitvmx_ping_after_silence_secs` | `[coordinator] bitvmx_ping_after_silence_secs` |
| `[bridge.pegin] min_tx_confirmations` | `[flows.common] btc_confirmations` |
| `[bridge.pegin] blocks_delay_for_tx_check` | `[flows.common] btc_status_retry_blocks` |
| `[bridge.pegout] blocks_delay_for_tx_check` | `[flows.common] btc_status_retry_blocks` |
| `[bridge.pegout] advance_funds_timeout_secs` | `[flows.pegout] advance_funds_timeout_secs` |
| `[bridge.committee] drp_program_definition` | `[flows.committee] drp_program_definition` |
| `[bridge.native_bridge] min_tx_confirmations` | `[flows.native_bridge] btc_confirmations_buffer` |

The legacy `[bridge.*]` section can be removed entirely once the new sections are in place; there is no overlap.

## Operator Runbook

### Prerequisites

- An operator host running `v0.3.1` with `~/.union_bridge/op_NN/` populated.
- A `v0.4.x` deployment artefact (binary, image, or compose file).
- A `migrate-v031` binary, either built from this repo (`cargo build --release -p migrate-v031`) or distributed alongside the `v0.4.x` artefact.
- Disk space for one snapshot of the operator state.

### Steps

1. Stop `v0.3.1`.

   ```bash
   docker compose -f docker/operator/docker-compose.yml down
   ```

2. Snapshot the operator state. Store it somewhere safe.

   ```bash
   tar czf op_NN_pre_v04x_$(date +%s).tar.gz -C ~ .union_bridge/op_NN
   ```

3. Update the operator config first if needed (rename the `[bridge.*]` keys per the table in [Config Rename](#config-rename)). Then run the migration tool, passing the config so it can refuse to migrate while the legacy section is still present:

   ```bash
   cargo run --release -p migrate-v031 -- \
       ~/.union_bridge/op_NN/local_database/coordinator \
       --config /path/to/operator/config.toml
   # Or, with a prebuilt binary:
   # ./migrate-v031 <db-path> --config <toml-path>
   ```

   Expected output:

   ```
   INFO  Config at <path> has no legacy [bridge.*] section
   INFO  v0.3.1 → v0.4.x migration: N rows mutated
   INFO  Post-migration schema verification passed
   ```

   Or if the DB is already at v0.4.x shape (rerun, partial run, or a fresh DB):

   ```
   INFO  Nothing to migrate; DB already at v0.4.x shape
   INFO  Post-migration schema verification passed
   ```

   The `--config` flag is optional; omit it if the operator config has already been renamed and you only need to migrate the DB. Without it the tool only mutates the DB and runs the schema verification.

4. *(Optional)* Dry-run against a copy. There is no `--dry-run` flag; the recommended preview is to run the tool plus `v0.4.x` against a copy of the operator directory.

   ```bash
   cp -r ~/.union_bridge/op_NN ~/.union_bridge/op_NN_dryrun
   cargo run --release -p migrate-v031 -- \
       ~/.union_bridge/op_NN_dryrun/local_database/coordinator
   BASE_STORAGE_PATH=$HOME/.union_bridge/op_NN_dryrun \
       docker compose -f docker/operator/docker-compose.yml up
   ```

   Confirm the coordinator reaches its normal flow loop, then stop and clean up:

   ```bash
   docker compose -f docker/operator/docker-compose.yml down
   rm -rf ~/.union_bridge/op_NN_dryrun
   ```

5. Deploy `v0.4.x`.

   ```bash
   docker compose -f docker/operator/docker-compose.yml up -d
   ```

6. Verify. Look for these lines on coordinator startup:

   ```
   INFO  Restored M SetupCommitteeFlow flows from persistence
   INFO  Restored M PeginFlow flows from persistence
   INFO  Restored M PegoutFlow flows from persistence
   ```

   If startup fails with a `serde_json::from_str` deserialization error inside `restore_flows`, the migration was skipped or did not complete; restore the snapshot, rerun step 3, and redeploy. For any other non-zero exit, restore the snapshot and contact the dev team with the log.

### Warnings

The migration tool logs a warning per in-flight pegout where `request_pegout_tx_hash` is missing:

```
WARN  Migrating in-flight pegout pegout_flows/<uuid> (step=DispatchTransaction) ...
```

The flow continues under `v0.4.x` with an empty `request_pegout_tx_hash`; its final completion marker may carry that empty value. To avoid it, drain the pegout under `v0.3.1` before upgrading.

## Rollback

Restore the snapshot:

```bash
docker compose -f docker/operator/docker-compose.yml down
rm -rf ~/.union_bridge/op_NN
tar xzf op_NN_pre_v04x_<ts>.tar.gz -C ~
# redeploy v0.3.1
```

Because the migration only adds fields and `v0.3.1` ignores unknown ones, downgrading without restoring also works, but restoring the snapshot is more deterministic.

## Removal Plan

The migration is intentionally self-contained so it can be deleted in a future release once all known operators are on `v0.4.x` or later. To remove:

- Delete the `tools/migrate-v031/` directory.
- Remove the `tools/migrate-v031` entry from the workspace `Cargo.toml` `members = [...]` array.
- Delete this document.

That is the entire diff. No production crate (`coordinator/`, `common/`, etc.) is touched by this migration in the first place, so there is nothing to clean up on the production side.

There are no on-disk artefacts to clean up either.
