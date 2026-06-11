# Pegout Recovery E2E

`scripts/test-pegout-recovery.sh` is a black-box recovery test for pegout
processor state. It verifies that an in-flight pegout can survive a coordinator
runtime restart without deleting coordinator storage.

## What It Covers

- Optional setup, committee preparation, and a normal pegin prerequisite.
- User pegout request creation.
- Coordinator restart after the pegout request is accepted and before a
  correlated pegout completion marker exists.
- Recovery from persisted pegout flow and processor state.
- Correlated pegout completion marker emission after restart.
- User BTC balance increase and RBTC balance decrease after restart.

The recovery cut is intentionally storage-preserving:

- Docker mode restarts only the `coordinator` service for each operator.
- Local mode detaches any supervising `run-clients` process, stops only local
  `coordinator` services, and relaunches only coordinators with
  `scripts/run-clients.sh --services coordinator`.

## How To Run

Start infra and clients first, then run:

```bash
bash scripts/test-pegout-recovery.sh --env local-anvil
```

For Docker operators:

```bash
bash scripts/test-pegout-recovery.sh --env docker-anvil
```

If setup, committee, and a user pegin are already ready:

```bash
bash scripts/test-pegout-recovery.sh --env local-anvil --skip-prereqs
```

To move the restart cut later in the in-flight window:

```bash
bash scripts/test-pegout-recovery.sh --env local-anvil --settle-seconds 8
```

## Pass Criteria

The script passes only if all of these are true:

- The pegout request command returns a Rootstock transaction hash.
- No correlated pegout completion marker exists before the restart cut.
- Coordinators restart without clearing storage.
- The same pegout request transaction hash produces correlated completion
  markers for the expected operators after restart.
- User BTC increases by at least the pegout value minus the existing fee margin.
- User RBTC decreases by at least the pegout value.

If the flow completes before restart, the script fails and asks for a smaller
`--settle-seconds` value. That protects the test from silently becoming a normal
happy-path run.

## Recovery Surface

This test exercises the pegout processor-owned state that is visible through the
end-to-end flow:

- retry schedulers,
- buffered pegout events,
- BTC transaction status polling,
- active BTC signature subflow state when the restart lands in that phase,
- `registerPegout` native-bridge retry state when that retry window is reached,
- persisted flow state plus processor snapshot restoration on startup.

The exact processor phase at the cut depends on local timing and mining cadence.
Use a larger `--settle-seconds` value to bias the cut toward later pegout phases,
and a smaller value if the flow completes before restart.
