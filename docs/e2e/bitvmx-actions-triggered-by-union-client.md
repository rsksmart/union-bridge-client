# BitVMX Actions Triggered by Union Client

This document lists the actions that the Union Bridge Client actively sends into BitVMX during the five active runtime flows covered by the E2E documentation set. It focuses on the client-to-BitVMX direction, including the `SetVar` values used to move a flow forward.

For the sequence context, see [Union Bridge Flows](flows.md).

## Main BitVMX Actions

| Action | Used in | Operational role |
| --- | --- | --- |
| `SubscribeToRskPegin()` | request peg-in and accept peg-in | subscribes to Bitcoin pegin discovery routed through BitVMX |
| `GetSPVProof(txid)` | request peg-in and accept peg-in, user take | asks BitVMX for the SPV proof needed for the next Rootstock registration |
| `GetTransaction(flow_id, txid)` | request peg-in and accept peg-in, user take | asks BitVMX for transaction state when the client needs to recheck maturity |
| `GetCommInfo()` | committee and dispute setup, request peg-in and accept peg-in, user take, operator take | retrieves the communication identity used to configure the next BitVMX setup |
| `GetFundingBalance(req_id)` | committee and dispute setup | retrieves the BitVMX-side funding balance used by dispute setup |
| `SetFundingUtxo(utxo)` | committee and dispute setup | publishes the funding UTXO used by dispute setup |
| `GetVar(program_id, variable_name)` | committee and dispute setup | retrieves dispute-core outputs such as operator and watchtower UTXOs |
| `Setup(program_id, flow_name, participants, leader)` | committee and dispute setup, request peg-in and accept peg-in, user take, operator take | starts the BitVMX program associated with the next flow stage |
| `DispatchTransactionName(flow_id, tx_name)` | request peg-in and accept peg-in, user take | asks BitVMX to broadcast a named Bitcoin transaction |

## `SetVar` Values Published by Union Client

BitVMX variable names passed to `SetVar` are **case-sensitive**. The name string must match exactly what the program and the Union Client expect (for example, `PeginAccepted` and `pegin_accepted` are different variables). Typos or different casing will not resolve to the same slot in BitVMX.

| `SetVar` name | Used in | Purpose |
| --- | --- | --- |
| `union_settings` | committee and dispute setup | publishes the shared runtime settings used by dispute setup |
| `ADVANCE_FUNDS_INPUT` | committee and dispute setup | publishes the funding UTXO and dispute input state |
| `committee` | committee and dispute setup | publishes committee composition into BitVMX |
| `dispute_core_data` | committee and dispute setup | publishes dispute-core input data |
| `pegin_request` | request peg-in and accept peg-in | publishes the Rootstock-derived pegin request into BitVMX |
| `PeginAccepted` | request peg-in and accept peg-in | publishes the final Rootstock pegin acceptance back into BitVMX |
| `pegout_request` | user take | publishes the Rootstock-derived pegout request into BitVMX |
| `PEG_OUT_COMPLETED` | user take | publishes the final Rootstock pegout registration back into BitVMX |
| `SELECTED_OPERATOR_PUBKEY_<slot>` | advance funds | publishes the operator selected to continue the timed-out pegout |
| `advance_funds_request` | advance funds | publishes the operator-side Bitcoin request data for the advance-funds path |

## Notes

The Union Client uses BitVMX as a stateful execution environment. These actions are not arbitrary RPC calls. Each one is part of a specific flow transition and usually depends on confirmed Rootstock events or previously received BitVMX outputs.
