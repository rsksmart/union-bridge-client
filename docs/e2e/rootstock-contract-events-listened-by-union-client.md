# Rootstock Contract Events Listened to by Union Client

This document lists the Rootstock contract events that the Union Bridge Client listens to across the five active runtime flows covered by the E2E documentation set. It focuses on the events that advance or gate a flow after the coordinator waits for the configured Rootstock confirmation threshold.

For the sequence context, see [Union Bridge Flows](flows.md).

| Rootstock event | Contract | Used in | Operational meaning |
| --- | --- | --- | --- |
| `NewPendingCommittee` | `CommitteeRegistry` | committee and dispute setup | committee formation has progressed enough for the client to track the pending committee state |
| `NewCommittee` | `CommitteeRegistry` | committee and dispute setup | a new committee is available and downstream setup can continue |
| `AllCommunicationDataReady` | `CommitteeRegistry` | committee and dispute setup | communication data is complete enough to continue committee-level setup |
| `MemberInfoDeposited` | `CommitteeRegistry` | committee and dispute setup | member-level setup data was deposited and the next setup step can continue |
| `PeginRequested` | `PeginManager` | request peg-in and accept peg-in | the initial pegin proof was registered and the client can transition to the official flow id |
| `PeginAccepted` | `PeginManager` | request peg-in and accept peg-in | the accepted pegin was registered and the pegin flow can close |
| `AllOperatorTakeTxidsAdded` | `SignatureManager` | request peg-in and accept peg-in | the operator-side transaction hashes were persisted and the corresponding signature path can continue |
| `AllNoncesReady` | `SignatureManager` | user take | all required nonces are available and the signature subflow can advance |
| `AllSignaturesReady` | `SignatureManager` | user take | all required signatures are available and the Bitcoin transaction can be dispatched |
| `PegoutRequested` | `PegoutManager` | user take | the Rootstock pegout request exists and the BitVMX user-take branch can be prepared |
| `PegoutRegistered` | `PegoutManager` | user take, advance funds | a Bitcoin-side take path was registered on Rootstock and the current pegout branch can close |
| `OperatorTakeTriggered` | `PegoutManager` | user take timeout and operator take, advance funds | the user-take path timed out and the operator branch is now active |
| `AdvanceFundsRegistered` | `PegoutManager` | advance funds | the advance-funds Bitcoin proof was accepted on Rootstock |
| `ReimbursementKickoffRegistered` | `PegoutManager` | advance funds | the reimbursement kickoff Bitcoin proof was accepted on Rootstock |

## Notes

The coordinator does not act on these events immediately after they appear in a log. They first wait for the configured Rootstock confirmation threshold, which is documented in [Confirmations, Retry Delays, and Timeouts](confirmations-retries-and-timeouts.md).
