# QA Tools

A collection of tools for testing and validating the Union Bridge Monitor components.

## Tools

### Block Indexer Tools
- `block-indexer-runner`: Runs the block indexer with configurable parameters
- `block-indexer-validator`: Validates block indexer state after running
- Features: backward sync, checkpoints, different cache sizes, long runs

### Log Indexer Tools
- `log-indexer-runner`: Runs the log indexer with configurable parameters
- `log-indexer-validator`: Validates log indexer state after running
- Features: managed contracts monitoring, event tracking

### Utility Tools
- `archiver`: Archives execution results with timestamps
- `clear`: Cleans up temporary execution directories

## Usage

All tools use the `/tmp/monitor-executions` directory for storing data and require a tag parameter (`-t`).

Common parameters:
- `-t <tag>`: Required tag for the execution (e.g., "happy_path")
- `-e <env>`: Environment (default: "stage")
- `-f <finality>`: Block finality for initial block selection
- `-b <height>`: Specific block height for initial block
- `-a <size>`: Cache size override
- `-c <bool>`: Use default config (true) or existing config (false)

### Example Scenarios

See `features/block-indexer.feature` and `features/log-indexer.feature` for detailed test scenarios and commands.

Basic usage:
```bash
# Run block indexer
cargo run --bin block-indexer-runner -- -f 100 -t happy_path

# Monitor logs
tail -1000f /tmp/monitor-executions/happy_path/app.log

# Validate results
cargo run --bin block-indexer-validator -- -t happy_path

# Archive results
cargo run --bin archiver -- -t happy_path
```

For long-running tests, use tmux:
```bash
tmux new-session -d -s test_session 'cargo run --bin block-indexer-runner -- -f 100 -t test_tag'
tmux attach-session -t test_session
# Use Ctrl+b, d to detach
```