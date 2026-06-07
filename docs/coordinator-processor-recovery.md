# Coordinator Processor Recovery

Coordinator flow state machines persist their durable `State`, but processors
also own runtime structures that are needed to resume work at async wait
boundaries. On startup, processors now restore those structures before broker
messages, Rootstock events, blocks, or user requests are processed.

## Persisted Processor State

`PeginFlowProcessor` persists:

- Rootstock event confirmations in progress.
- BTC transaction status polling scheduler.
- `requestPegin` SPV tracking and native-bridge retry state.
- Buffered `PeginRequested`, `AllOperatorTakeTxidsAdded`, and `PeginAccepted`
  events.
- `acceptPegin` native-bridge retry state.
- Active BTC signature subflows, including BitVMX signature data, nonce and
  signature confirmation checkpoints, `is_nonces_step_done`, and `is_done`.

`PegoutFlowProcessor` persists:

- Rootstock event confirmations in progress.
- BTC transaction status polling scheduler.
- Advance-funds timeout scheduler and flows waiting for timeout scheduling.
- `registerPegout` native-bridge retry state.
- Active BTC signature subflows, including BitVMX signature data, nonce and
  signature confirmation checkpoints, `is_nonces_step_done`, and `is_done`.

`AdvanceFundsFlowProcessor` persists:

- Active advance-funds flow contexts.
- Rootstock event confirmations in progress.
- Native-bridge retry tracker.
- Cached `(committee_id, slot_id) -> PegoutRequested tx hash` data needed to
  construct deterministic operator-take trigger context.

`SetupCommitteeProcessor` already persists setup-committee flow state. Its
processor-owned confirmation view remains outside the pegin/pegout recovery
surface covered by this feature.

## Reconstructed From Flow State

If a pegin or pegout processor snapshot is absent, the processor reconstructs
only deterministic scheduler state from restored flow state:

- Pegin `RequestPeginSpvProof` restores request SPV tracking.
- Pegin `ConfirmAcceptPeginTransaction` restores BTC status polling.
- Pegin `AcceptPegin` restores `acceptPegin` retry scheduling.
- Pegout `WaitUserTakeSignaturesReady` restores pending timeout scheduling.
- Pegout `ConfirmUserTakeTransaction` restores BTC status polling.
- Pegout `RegisterPegout` restores `registerPegout` retry scheduling.

Active BTC signature subflows and buffered events require the processor snapshot;
they are not derivable from flow state alone without replaying external events
or re-sending contract writes.

## Confirmation Snapshots

Rootstock confirmation tracking stores the event id, the block number that
started confirmation, and the required confirmation count. The restored
`BlockchainView` is rebuilt empty and attached to new observers, so confirmation
evaluation continues when new blocks arrive. Already indexed blocks are not
replayed from the snapshot.

## Intentionally Volatile State

The following processor-owned state remains volatile:

- `FundingInfoProcessor.pending_requests`: user reply correlation with a short
  TTL. Dropping it on restart is safe because callers can repeat the request.
- `SetupCommitteeProcessor.events_confirming`: not required to resume pegin,
  pegout, operator-take, or BTC signature flow recovery. Setup-committee flow
  state remains persisted independently.
- Processor dependency handles such as contract gateways, brokers, runtime
  sync, native-bridge verifier, signaling, and static configuration. These are
  rebuilt from coordinator configuration on startup.
- `BlockchainView` block caches. They are runtime views only; confirmation
  checkpoints are persisted separately and reattached to a fresh view.
- Terminal in-memory flow entries. Terminal pegin and pegout flows are removed
  from their persisted flow stores by existing cleanup. Terminal advance-funds
  flows are omitted from the next processor snapshot after cleanup.
