# QA Tools

A collection of tools for testing and validating the Union Bridge Monitor components. Each crate contains the validation tools for a component of the union bridge client.

## Crates

### Block Indexer Tools
- `block_indexer_runner`: Runs the block indexer with configurable parameters
- `block_indexer_validator`: Validates block indexer state after running
- Features: backward sync, checkpoints, different cache sizes, long runs

### Log Indexer Tools
- `log_indexer_runner`: Runs the log indexer with configurable parameters
- `log_indexer_validator`: Validates log indexer state after running
- Features: managed contracts monitoring, event tracking

### Check Fork Tools
- `check_fork_runner`: Runs and validates the check fork with configurable parameters
- Features: managed contracts monitoring, event tracking

### Transaction dispatcher Tools
WIP

### Utility Tools
- `archiver`: Archives execution results with timestamps
- `clear`: Cleans up temporary execution directories

## Usage
To find instructions, search for the comments under scenarios within `features/` folder of each crate.