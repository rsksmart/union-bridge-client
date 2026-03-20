# Parameter Sources and Mappings

This document summarizes the main identifiers and value mappings that the Union Bridge Client carries across Rootstock, its own internal state, and BitVMX during the five active runtime flows covered by the E2E documentation set.

For the sequence context, see [Union Bridge Flows](flows.md).

## Main Identifiers

| Value | Source | Used in |
| --- | --- | --- |
| `temp_flow_id` | derived from the Bitcoin pegin `txid` | request peg-in and accept peg-in before `PeginRequested` creates the official identity |
| `official_flow_id` | derived from `committee_id + slot_id` | request peg-in and accept peg-in after `PeginRequested` |
| `committee_id` | Rootstock event fields | all currently documented flows |
| `slot_id` / `slot_index` | Rootstock event fields | request peg-in and accept peg-in, user take, user take timeout and operator take, and advance funds routing |
| `pegout_id` | `PegoutRequested` or `OperatorTakeTriggered` fields | user take, user take timeout and operator take, and advance funds |
| `accept_pegin_txid` | `PeginAcceptedMessage` or `get_accept_pegin_txid` | request peg-in and accept peg-in and advance funds |
| `pubkey_hash` | `CommsAddress` returned by BitVMX | committee and dispute setup and BitVMX communication identity |

## Rootstock to BitVMX Mappings

| Value published to BitVMX | Derived from |
| --- | --- |
| `rootstock_address` in `PeginRequestMessage` | `PeginRequested.requestPeginInfo.rskDestinationAddress` |
| `reimbursement_pubkey` in `PeginRequestMessage` | `PeginRequested.requestPeginInfo.btcReimbursementPubKey` |
| `take_aggregated_key` in pegout setup | `CommitteeRegistry.get_committee().committee.aggregatedKey` |
| committee communication endpoints | `get_committee_communication_data` |
| committee communication keys | `get_member_public_keys` |
| selected operator pubkey | `OperatorTakeTriggered.operatorTakePubKey` |
| `AdvanceFundsRequest.committee_id` | `OperatorTakeTriggered.committeeId` |
| `AdvanceFundsRequest.slot_index` | `OperatorTakeTriggered.slotId` |
| `AdvanceFundsRequest.pegout_id` | `OperatorTakeTriggered.pegoutId` |
| `AdvanceFundsRequest.user_pubkey` | `OperatorTakeTriggered.userPubKey` |
| `AdvanceFundsRequest.operator_take_pubkey` | `OperatorTakeTriggered.operatorTakePubKey` |

## BitVMX to Rootstock Mappings

| Value consumed from BitVMX | Used by Union Client as |
| --- | --- |
| `SPVProof` for the pegin transaction | input to `request_pegin` |
| `PeginAcceptedMessage.accept_pegin_txid` | Bitcoin transaction id later proven in `accept_pegin` |
| `PeginAcceptedMessage.operator_take_txid` | operator-side tx hash stored through `add_operator_take_tx_hash` |
| `PegOutAccepted.user_take_txid` | Bitcoin transaction id later proven in `register_pegout` |
| `FundsAdvanceSPV` | Bitcoin proof later passed to `register_advance_funds` |
| `union_spv_notification` for reimbursement kickoff | Bitcoin proof later passed to `register_reimbursement_kickoff` |
| `union_spv_notification` for operator take | Bitcoin proof later passed to `register_operator_take` |

## Notes

The key operational risk in these mappings is not a single malformed field in isolation. It is drift between identifiers that should describe the same flow branch across Rootstock, Union Client state, and BitVMX state. That is why the client keeps translating between event fields, flow ids, txids, and committee routing data at every stage.
