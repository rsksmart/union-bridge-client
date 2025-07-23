# QA Tools

A collection of tools for testing and validating the Union Bridge Monitor components. Each crate contains the validation tools for a component of the union bridge client.

## Initial setup

### BitVMX Union Bridge contracts

Before executing any tests, you will need to set up the BitVMX Union Bridge contracts in your local environment.
In order to do that, follow these instructions:

```bash
# Move to the directory right above union-bridge-client, so the contracts repo will be a sibling.
cd ..
git clone git@github.com:temp-rsk/bitvmx-union-bridge-contracts.git 
cd bitvmx-union-bridge-contracts   
git checkout v0.0.1-alpha.2_tweaks
git submodule update --init --recursive
```

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


### Transaction dispatcher Tools (manual)

To find instructions on how to execute tests, search for the comments under scenarios within `features/` folder,
in .feature.manual files. Keep in mind that these scenarios are not actively maintained anymore since the manual tests 
are being replaced with automated tests.

### Transaction dispatcher Tools (automated)

#### Execute automated tests in CI/CD environments
There is a pipeline `wf_qa_tests.yml` that is executed on every PR to main branch and every merge to main. 
Currently the pipeline includes steps to execute the steps and upload the reports to Testomat.

#### Execute automated tests locally
```bash
cd qa-tools
export KEY_STORE_PASSWORD="=== REPLACE_WITH_PASSWORD ==="
export KEY_STORE_ADDRESS="=== REPLACE_WITH_ADDRESS ==="
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
You will need to have docker installed and running to execute the pipeline locally.
Once docker is running, you can execute the pipeline with the command below.
```bash
export KEY_STORE_PATH="replace/with/path/to/your/keystore"
export KEY_STORE_FILE="$(cat "$KEY_STORE_PATH")"
export FAIRGATE_GITHUB_TOKEN="=== REPLACE_WITH_TOKEN ==="
export UNION_CONTRACTS_GITHUB_TOKEN="=== REPLACE_WITH_TOKEN ==="
export KEY_STORE_PASSWORD="=== REPLACE_WITH_PASSWORD ==="
export KEY_STORE_ADDRESS="=== REPLACE_WITH_ADDRESS ==="
act -j qa-tests \
--secret FAIRGATE_GITHUB_TOKEN=$FAIRGATE_GITHUB_TOKEN \
--secret UNION_CONTRACTS_GITHUB_TOKEN=$UNION_CONTRACTS_GITHUB_TOKEN \
--secret KEY_STORE_PASSWORD=$KEY_STORE_PASSWORD \
--secret KEY_STORE_FILE=$KEY_STORE_FILE
```
Note: PROJECT_REPORTING_API_KEY for testomat is not provided when running tests locally as in this case no reports
should be uploaded to testomat unless we explicitly want to test this part of the pipeline.

Optional: add `reuse` flag to speed up the pipeline setup.
```bash
... same setup as above ...
act -j qa-tests --reuse \
... same setup as above ...
```
Currently the pipeline execution prints the JUnit XML report to the console.

#### Upload test reports to Testomat
As mentioned above, the pipeline `wf_qa_tests.yml` uploads the test reports to Testomat.io automatically.
It is discouraged to upload test reports manually, as it can pollute the testomat.io project with meaningless test runs.
In the qa pipeline, test report uploading to Testomat is disabled when the pipeline is executed locally with `act` command.
If for whatever reason you need to upload test reports manually (e.g. for testing purposes), you can do it with the 
command below. Remember to Delete the uploaded report if it does not provide any value to the project.
```bash
TESTOMATIO="=== REPLACE_WITH_PROJECT_REPORTING_API_KEY ===" \
TESTOMATIO_TITLE="Manual upload - tx dispatcher | $(date +'%Y-%m-%d %H:%M')" \
npx report-xml reports/tx_dispatcher.xml
```
# Automated tests execution and reporting strategy

## On Pull Request to main
- Why: as early feedback for the PR author
- How: with the wf_qa_tests.yml pipeline
- Generating report: always
- Uploading report to testomat: always
-
## On Push to main
- Why: to know that the main branch is stable and all tests are passing
- How: TBD
- Generating report: always
- Uploading report to testomat: always

## Run ad-hoc from testomat
- Why: to detect issues under specific conditions, e.g. check for network issues, after enviroment changes, etc.
- How: trigger from testomat - TBD
- Generating report: always
- Uploading report to testomat: always

## Run in nightly pipeline
- Why: to catch flaky tests, intermittent issues, etc.
- How: with the wf_qa_tests.yml pipeline - TBD
- Generating report: always
- Uploading report to testomat: always

## Run locally and manually
- Why:
  - Test WIP tests
  - Test WIP features
  - Test WIP pipelines
- How: 
  - With "raw" commands
  - With act pipeline
- Generating report: as needed, with `JUNIT_REPORT` env variable
- Uploading report to testomat: never, unless explicitly needed for testing purposes

# Testomat.io and repo feature synchronization

## How to import features to Testomat.io
It is important to keep Testomat.io in sync with the features we have in our repository.

### Let the pipeline handle it for you
Update the wf_qa_sync_testomat.yml pipeline if have to import new features into Testomat.io.
The actual import in testomat will be done when main branch is pushed, so you don't have to worry about it.

If you want to test your pipeline locally you can use the command below. But be careful! It will impact the existing features in Testomat.io.
```bash
act push --secret PROJECT_REPORTING_API_KEY="PROJECT_REPORTING_API_KEY"
```
If you accidentally messed features in Testomat, undo your changes in the repo and run the command again to restore the original state in Testomat.

### Do it manually (with reasons)
If for whatever reason you need to import features manually (e.g. you need the results in testomat before your automated
tests are pushed to main), you can do it with the help of check-cucumber package.
Navigate just above the "features" directory you want to import and run the check-cucumber command with the proper API
key and the wrapper folder name you want to see for those features in Testomat.
```bash
cd qa-tools/path/just/above/features
TESTOMATIO=PROJECT_REPORTING_API_KEY TESTOMATIO_PREPEND_DIR="FOLDER_NAME_YOU_WANT_IN_TESTOMAT" npx check-cucumber@latest "**/*.feature" --dir features
```
As mentioned above, be careful with this command, as it will impact the existing features in Testomat.

#### How to get project Reporting API key:
Go to your Testomat.io project, navigate to Settings -> Reporting API and copy the key.