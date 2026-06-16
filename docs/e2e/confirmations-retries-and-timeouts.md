# Confirmations, Retry Delays, and Timeouts

This document summarizes the confirmation rules, retry delays, timeout values, and restart recovery state that gate the five active runtime flows covered by the E2E documentation set.

For the sequence context, see [Union Bridge Flows](flows.md).

| Rule | Value | Used in | Meaning |
| --- | --- | --- | --- |
| Rootstock event confirmations | `5` | all five flows | Rootstock events are processed only after five Rootstock confirmations |
| Pegin Bitcoin minimum confirmations | `1` | request peg-in and accept peg-in | minimum Bitcoin maturity before the initial pegin proof is considered ready for the next step |
| Pegin transaction recheck delay | `20` blocks | request peg-in and accept peg-in | delay before asking BitVMX for transaction status again |
| Pegout SPV minimum confirmations | `1` | user take | minimum Bitcoin maturity before the user-take proof is considered ready for Rootstock registration |
| Pegout transaction recheck delay | `20` blocks | user take | delay before asking BitVMX for transaction status again |
| Advance-funds timeout | `600` seconds | user take timeout and operator take | time allowed for the user-take dispatch path before triggering operator take |
| Local advance-funds timeout override | `30` seconds | user take timeout and operator take | local-only faster timeout for development |
| Advance-funds SPV minimum confirmations | `1` | advance funds | minimum Bitcoin maturity before advance-funds or operator-side proofs are considered ready |
| Advance-funds recheck delay | `20` blocks | advance funds | delay before retrying a Native Bridge-gated registration |
| Native Bridge minimum confirmations | `2` | request peg-in and accept peg-in, user take, and advance funds | extra guard used before the corresponding Rootstock registration succeeds |

## Restart Recovery State

Flow state machines persist their durable `State` or `FlowContext`, but the coordinator processors also own runtime state for confirmation checks, retry delays, polling, buffered event replay, and BTC signature subflows. On restart, processors restore that state before handling new broker messages, Rootstock events, blocks, or user requests.

Persisted or per-flow durable state:

- `PeginFlowProcessor`: Rootstock confirmations in progress, BTC transaction polling, `requestPegin` and `acceptPegin` Native Bridge retry state, buffered `PeginRequested`, `AllOperatorTakeTxidsAdded`, and `PeginAccepted` events, and active BTC signature subflows.
- `PegoutFlowProcessor`: Rootstock confirmations in progress, BTC transaction polling, advance-funds timeout scheduling, `registerPegout` Native Bridge retry state, and active BTC signature subflows.
- `OperatorTakeFlow(Uuid)`: each advance-funds flow context. `AdvanceFundsProcessorState` only stores processor runtime state: Rootstock confirmations in progress, Native Bridge retries, and cached `(committee_id, slot_id) -> PegoutRequested tx hash` data.
- `SetupCommitteeProcessor`: setup-committee flow state is persisted independently. Its processor-owned confirmation view is not required to resume pegin, pegout, operator-take, or BTC signature recovery.

If a pegin or pegout processor snapshot is absent, only deterministic scheduler state is reconstructed from restored flow state: pegin request SPV tracking, pegin and pegout BTC transaction polling, pegin `acceptPegin` retries, pegout advance-funds timeout scheduling, and pegout `registerPegout` retries. Active BTC signature subflows and buffered events require the processor snapshot; they are not derivable from flow state alone without replaying external events or re-sending contract writes.

Rootstock confirmation snapshots store the event id, the block number that started confirmation, and the required confirmation count. The restored `BlockchainView` starts fresh and continues confirmation evaluation as new blocks arrive; already indexed blocks are not replayed from the snapshot.

The intentionally volatile processor-owned state is limited to user reply correlation in `FundingInfoProcessor.pending_requests`, dependency handles rebuilt from coordinator configuration, runtime `BlockchainView` block caches, and terminal in-memory flow entries that cleanup removes from persisted flow stores.

## Notes

The important operational detail is that an SPV proof from BitVMX is not always enough to trigger an immediate Rootstock write. The Union Client can still postpone the write if the Native Bridge confirmation checks do not yet consider the Bitcoin transaction mature enough.

The Native Bridge confirmation gate is environment-dependent. In `alphanet`, `regtest`, and `testnet`, the Union Client uses the real Native Bridge verifier and checks `get_btc_confirmations` before the corresponding Rootstock write. In other environments, it uses a dummy verifier that skips that Native Bridge check, so this extra gate does not apply there.
