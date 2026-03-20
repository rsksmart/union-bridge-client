# E2E Documentation

This documentation set explains how the Union Bridge Client interacts end-to-end between Rootstock, Bitcoin, and BitVMX. The goal is operational clarity.

The current scope is limited to the currently documented flows in this E2E set: committee and dispute setup, request peg-in and accept peg-in, user take, user take timeout and operator take, and advance funds.

## Documents

- [Union Bridge Flows](flows.md)
- [BitVMX Messages Listened to by Union Client](bitvmx-messages-listened-by-union-client.md)
- [Rootstock Contract Functions Called by Union Client](rootstock-contract-functions-called-by-union-client.md)
- [Rootstock Contract Events Listened to by Union Client](rootstock-contract-events-listened-by-union-client.md)
- [Parameter Sources and Mappings](parameter-sources-and-mappings.md)
- [BitVMX Actions Triggered by Union Client](bitvmx-actions-triggered-by-union-client.md)
- [Confirmations, Retry Delays, and Timeouts](confirmations-retries-and-timeouts.md)

## How to Read These Docs

Start with [Union Bridge Flows](flows.md) if you want the end-to-end story of each flow and the Mermaid sequence diagrams. Use the topic documents when you need a direct answer to one operational question, such as which BitVMX messages are consumed, which Rootstock events are listened to, or which confirmations gate a given step.
