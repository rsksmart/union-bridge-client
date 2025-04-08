# As a monitor component,
# I want to continuously track the status of the Rootstock chain
# so that I have an index of the logs I am interested in
# and I robustly handle network issues, shutdowns, different cache sizes and long runs

Feature: Rootstock log monitoring and tracking

Scenario: happy path
Given the initial best block is B (B = node best block height - 100)
And the log subscription filter is configured to capture logs from the managed contracts (C.A..C.D)
When the log indexer is started
And the log indexer is suscribed for a while
And the log indexer is shut down
Then the logs stored should originate exclusively from the contracts (C.A..C.D)
And the logs stored should belong to blocks with height greater than B
And the logs stored should belong to blocks with height greater than L (L = node best block height - 10)
And the logs stored should appear in the order they were emitted without any missing events within that block range

# tmux new-session -d -s log-happy-path 'cargo run --bin block-indexer-runner -- -f 100 -t log-happy-path'
# tmux attach-session -t log-happy-path
# detach: CTRL+b, d
# tail  -1000f /tmp/monitor-executions/log-happy-path/app.log
# wait for a while... -> shut down
# cargo run --bin log-indexer-validator -- -t log-happy-path
# cargo run --bin archiver -- -t log-happy-path
