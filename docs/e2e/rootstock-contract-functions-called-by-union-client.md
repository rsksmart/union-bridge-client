# Rootstock Contract Functions Called by Union Client

This document lists the Rootstock contract functions that the Union Bridge Client calls across the five active runtime flows covered by the E2E documentation set. It focuses on the contract surface the client uses to advance a flow after listening to BitVMX messages or confirmed Rootstock events.

For the sequence context, see [Union Bridge Flows](flows.md).

| Function | Contract | Used in | Operational role |
| --- | --- | --- | --- |
| `get_temporary_pegin_address` | `PeginManager` / `StreamManager` | pegin entrypoint | resolves the temporary pegin address used before BitVMX later detects the Bitcoin transaction |
| `request_pegin` | `PeginManager` | request peg-in and accept peg-in | registers the initial Bitcoin pegin proof on Rootstock |
| `accept_pegin` | `PeginManager` | request peg-in and accept peg-in | registers the accepted pegin once the accept-pegin Bitcoin transaction is proven |
| `request_pegout` | `PegoutManager` | pegout entrypoint | creates the Rootstock-side pegout request that later starts the user-take flow |
| `register_pegout` | `PegoutManager` | user take | registers the user-take Bitcoin proof on Rootstock |
| `trigger_operator_take` | `PegoutManager` | user take timeout and operator take | escalates a stalled user-take flow into the operator-take branch |
| `register_advance_funds` | `PegoutManager` | advance funds | registers the advance-funds Bitcoin proof on Rootstock |
| `get_accept_pegin_txid` | `PegoutManager` | advance funds | resolves the accept-pegin linkage needed for later operator-side registrations |
| `register_reimbursement_kickoff` | `PegoutManager` | advance funds | registers the reimbursement kickoff Bitcoin proof on Rootstock |
| `register_operator_take` | `PegoutManager` | advance funds | registers the operator-take Bitcoin proof and closes the operator branch |
| `add_member_nonce` | `SignatureManager` | user take | persists the member nonce used by the Bitcoin signature subflow |
| `add_member_signature` | `SignatureManager` | user take | persists the member signature used by the Bitcoin signature subflow |
| `add_operator_take_tx_hash` | `SignatureManager` | request peg-in and accept peg-in | stores the operator transaction hashes associated with the accepted pegin |
| `is_whitelisted` | `CommitteeRegistry` | committee and dispute setup | checks whether the local member is allowed to participate |
| `apply_to_stream` | `CommitteeRegistry` | committee and dispute setup | enrolls the local member in the committee stream |
| `get_committee` | `CommitteeRegistry` | committee and dispute setup, request peg-in and accept peg-in, user take | retrieves committee composition and aggregated key state |
| `get_committee_communication_data` | `CommitteeRegistry` | committee and dispute setup, request peg-in and accept peg-in, user take | retrieves committee communication endpoints |
| `deposit_communication_data` | `CommitteeRegistry` | committee and dispute setup | publishes communication data used by the committee |
| `deposit_aggregated_key` | `CommitteeRegistry` | committee and dispute setup | publishes the aggregated key used by later flows |
| `get_member_public_keys` | `MemberRegistry` | committee and dispute setup, request peg-in and accept peg-in, user take | retrieves per-member public keys and role-specific routing data |
| `get_btc_confirmations` | `NativeBridge` | request peg-in and accept peg-in, user take, operator take | checks Bitcoin maturity before the client proceeds with the Rootstock registration |

## Notes

These functions are the ones the client actively drives in the current five-flow scope. The larger gateway surface may expose more calls, but they are outside the scope of this E2E set if they do not participate in the active flow documentation documented here.
