# Pegin Recovery E2E

`scripts/test-pegin-recovery.sh` is a black-box recovery test for the pegin
processor state added by coordinator processor recovery. It verifies that an
in-flight pegin can survive a coordinator runtime restart without deleting
coordinator storage.

## What It Covers

- Optional setup and committee preparation.
- User pegin request creation.
- Coordinator restart after the pegin request is accepted and before a
  correlated pegin completion marker exists.
- Recovery from persisted pegin flow and processor state.
- Correlated pegin completion marker emission after restart.
- User RBTC balance increase and BTC balance decrease after restart.

The recovery cut is intentionally storage-preserving:

- Docker mode restarts only the `coordinator` service for each operator.
- Local mode detaches any supervising `run-clients` process, stops only local
  `coordinator` services, and relaunches only coordinators with
  `scripts/run-clients.sh --services coordinator`.

## How To Run

Start infra and clients first, then run:

```bash
bash scripts/test-pegin-recovery.sh --env local-anvil
```

For Docker operators:

```bash
bash scripts/test-pegin-recovery.sh --env docker-anvil
```

If setup and committee are already ready:

```bash
bash scripts/test-pegin-recovery.sh --env local-anvil --skip-prereqs
```

To move the restart cut later in the in-flight window:

```bash
bash scripts/test-pegin-recovery.sh --env local-anvil --settle-seconds 8
```

## Pass Criteria

The script passes only if all of these are true:

- The pegin request command returns a Bitcoin transaction id.
- No correlated pegin completion marker exists before the restart cut.
- Coordinators restart without clearing storage.
- The same pegin transaction id produces correlated completion markers for the
  expected operators after restart.
- User RBTC increases by the amount recorded in the pegin completion marker.
- User BTC decreases by at least the pegin value.

If the flow completes before restart, the script fails and asks for a smaller
`--settle-seconds` value. That protects the test from silently becoming a normal
happy-path run.

## Recovery Surface

This test exercises the pegin processor-owned state that is visible through the
end-to-end flow:

- buffered pegin events,
- BTC transaction status polling,
- native-bridge request or accept retry state when that retry window is reached,
- active BTC signature subflow state when the restart lands in that phase,
- persisted flow state plus processor snapshot restoration on startup.

The exact processor phase at the cut depends on local timing and mining cadence.
Use a larger `--settle-seconds` value to bias the cut toward later pegin phases,
and a smaller value if the flow completes before restart.
