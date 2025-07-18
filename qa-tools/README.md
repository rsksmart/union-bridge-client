# QA Tools

A collection of tools for testing and validating the Union Bridge Monitor components. Each crate contains the validation tools for a component of the union bridge client.

## Crates

### Block Indexer Tools (manual execution)
- `block_indexer_runner`: Runs the block indexer with configurable parameters
- `block_indexer_validator`: Validates block indexer state after running
- Features: backward sync, checkpoints, different cache sizes, long runs

To find instructions on how to execute tests, search for the comments under scenarios within `features/` folder.

### Log Indexer Tools (manual execution)
- `log_indexer_runner`: Runs the log indexer with configurable parameters
- `log_indexer_validator`: Validates log indexer state after running
- Features: managed contracts monitoring, event tracking

To find instructions on how to execute tests, search for the comments under scenarios within `features/` folder.

### Utility Tools for block indexer and log indexer
- `archiver`: Archives `/tmp/monitor-executions` with timestamps
- `clear`: Cleans up `/tmp/monitor-executions` directories

### Check Fork Tools (manual execution)
- `check_fork_runner`: Runs and validates the check fork with configurable parameters
- Features: managed contracts monitoring, event tracking

To find instructions on how to execute tests, search for the comments under scenarios within `features/` folder.

### Coordinator Tools (manual execution)
- Coordinator manual testing relies on actors-mocking crate to simulate the deployment of the Union Bridge contracts,
the BitVMX messages, and the emission of the events that are expected to be processed by the coordinator.
- In particular, actors-mocking provides a CLI tool to emit the mocked events:
  - RequestAdvanceFunds (raf): returns a pegout_id
  - RemoveRequestAdvanceFunds (reraf pegout_id)
  - AdvanceFunds (kaf pegout_id)
  - RemoveAdvanceFunds (reaf pegout_id)
- The feature file includes useful comments with the necessary commands to run the tests.
- Remember to adjust the .env and config files accordingly (instructions are provided in the background section of the feature file).
- The crate includes also a script to load useful commands for executing some test steps. Find the details in the feature file.

### Transaction dispatcher Tools (automated)

#### Execute automated tests locally
```bash
cd qa-tools
export KEY_STORE_PASSWORD="=== REPLACE_WITH_PASSWORD ==="
export KEY_STORE_PATH="replace/with/path/to/your/keystore"
KEY_STORE_FILE="$(cat "$KEY_STORE_PATH")"
echo "${KEY_STORE_FILE}" > test_keystore/keyfile
cargo run --bin qa-tools-transaction-dispatcher -- --tags @transaction-dispatcher
```
Optional: add `JUNIT_REPORT` env variable to generate JUnit XML reports under `qa-tools/reports/` directory.
```bash
... same setup as above ...
JUNIT_REPORT="reports/tx_dispatcher.xml" cargo run --bin qa-tools-transaction-dispatcher -- --tags @transaction-dispatcher
```
#### Execute automated tests via ACT pipeline (local)
```bash
export KEY_STORE_PATH="replace/with/path/to/your/keystore"
export KEY_STORE_FILE="$(cat "$KEY_STORE_PATH")"
export FAIRGATE_GITHUB_TOKEN="=== REPLACE_WITH_TOKEN ==="
export UNION_CONTRACTS_GITHUB_TOKEN="=== REPLACE_WITH_TOKEN ==="
export KEY_STORE_PASSWORD="=== REPLACE_WITH_PASSWORD ==="
act -j test \
--secret FAIRGATE_GITHUB_TOKEN=$FAIRGATE_GITHUB_TOKEN \
--secret UNION_CONTRACTS_GITHUB_TOKEN=&UNION_CONTRACTS_GITHUB_TOKEN \
--secret KEY_STORE_PASSWORD=$KEY_STORE_PASSWORD \
--secret KEY_STORE_FILE=$KEY_STORE_FILE
```

Optional: add `reuse` flag to speed up the pipeline setup.
```bash
... same setup as above ...
act -j test --reuse \
... same setup as above ...
```

Currently the pipeline execution prints the JUnit XML report to the console. Pushing the report to Testomat is WIP.



