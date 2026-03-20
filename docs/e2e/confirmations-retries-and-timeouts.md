# Confirmations, Retry Delays, and Timeouts

This document summarizes the confirmation rules, retry delays, and timeout values that gate the five active runtime flows covered by the E2E documentation set.

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

## Notes

The important operational detail is that an SPV proof from BitVMX is not always enough to trigger an immediate Rootstock write. The Union Client can still postpone the write if the Native Bridge confirmation checks do not yet consider the Bitcoin transaction mature enough.
