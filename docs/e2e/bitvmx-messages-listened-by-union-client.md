# BitVMX Messages Listened to by Union Client

This document lists the BitVMX messages that the Union Bridge Client consumes during the five active runtime flows covered by the E2E documentation set. It focuses on incoming messages from BitVMX into the Union Client and explains where each message is used.

For the sequence context, see [Union Bridge Flows](flows.md).

| BitVMX message | Used in | Operational meaning |
| --- | --- | --- |
| `PeginTransactionFound(txid, status)` | request peg-in and accept peg-in | BitVMX discovered the Bitcoin pegin transaction and the Union Client can start the pegin flow |
| `SPVProof(txid, proof)` | request peg-in and accept peg-in, user take | BitVMX returned the Bitcoin SPV proof needed for a Rootstock registration |
| `CommInfo(...)` | committee and dispute setup, request peg-in and accept peg-in, user take, operator take | BitVMX returned the communication identity needed to build the next setup payload |
| `FundingBalance(...)` | committee and dispute setup | BitVMX returned the funding balance used to build dispute setup state |
| `SetupCompleted(program_id)` | committee and dispute setup, operator take | BitVMX accepted a setup request and the client can continue the flow |
| `Variable(flow_id, "pegin_accepted", ...)` | request peg-in and accept peg-in | BitVMX returned the accept-pegin payload, including the Bitcoin txids used by the next pegin steps |
| `Transaction(flow_id, tx_status, ...)` | request peg-in and accept peg-in, user take | BitVMX reported Bitcoin transaction status and confirmations for a dispatched transaction |
| `Variable(flow_id, "pegout_accepted", ...)` | user take, user take timeout and operator take | BitVMX returned the user-take payload that feeds the signature and dispatch path |
| `Variable(dispute_core_pid, "OP_COSIGN_UTXOS", ...)` | committee and dispute setup | BitVMX returned operator-side dispute inputs derived from dispute-core |
| `Variable(dispute_core_pid, "WT_INIT_CHALLENGE_UTXOS", ...)` | committee and dispute setup | BitVMX returned watchtower-side dispute inputs derived from dispute-core |
| `Variable(flow_id, "funds_advance_spv", ...)` | advance funds | BitVMX returned the advance-funds Bitcoin proof for Rootstock registration |
| `Variable(flow_id, "union_spv_notification", ...)` | advance funds | BitVMX returned the reimbursement kickoff or operator-take Bitcoin proof for the next Rootstock registration |

## Notes

The method or variable name string inside `Variable(program_id, name, …)` is matched **case-sensitively** in the client (for example, `"pegin_accepted"` and `"PeginAccepted"` are distinct), consistent with `SetVar` naming in [BitVMX actions triggered by Union Client](bitvmx-actions-triggered-by-union-client.md).

The Union Client does not treat these messages as isolated events. Each message is interpreted in the context of a current flow id, committee state, and Rootstock event history. In practice, the same message type may be valid in one flow stage and meaningless in another.
