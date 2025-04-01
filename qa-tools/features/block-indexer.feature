# As a monitor component,
# I want to continuously track the status of the Rootstock chain
# so that I have the latest canonical chain information
# and I robustly handle network issues, shutdowns, different cache sizes and long runs

Feature: Rootstock block monitoring and tracking

Scenario: happy path
Given the initial best block is B (B = node latest block height - 100)
And the indexer is started
And the indexer catches up with backward sync and is suscribed for a while
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 100 -t happy_path
# tail  -1000f /tmp/monitor-executions/happy_path/app.log
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t happy_path
# cargo run --bin archiver -- -t happy_path

Scenario: shut down during backward sync (checkpoint)
Given the initial best block is B (B = node latest block height - 5000)
And the indexer is started
And the indexer runs in backward sync but it does not complete it
When the block indexer is shut down
Then the storage should have a checkpoint
Then the latest block in storage should be B, not the best one from the provider

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown_sync
# tail  -1000f /tmp/monitor-executions/shutdown_sync/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown_sync
# cargo run --bin archiver -- -t shutdown_sync

Scenario: shut down during backward sync and restart
Given the initial best block is B (B = node latest block height - 5000)
And the indexer is started
And the indexer runs in backward sync but it does not complete it
When the block indexer is shut down
And the indexer is started again
And the indexer catches up with backward sync and is suscribed for a while
Then the best block in storage should be the best from the node
And the storage should not have a checkpoint
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown_n_restart
# tail  -1000f /tmp/monitor-executions/shutdown_n_restart/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-runner -- -t shutdown_n_restart -c false
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown_n_restart
# cargo run --bin archiver -- -t shutdown_n_restart

Scenario: shut down during subscription and restart with a more recent initial_block_hash
Given the initial best block is B (B = node latest block height - 5000)
And the indexer is started
And the indexer runs in backward sync but it does not complete it
When the block indexer is shut down
And the indexer is started again after with initial block L (L = node latest block height - 100)
Then the indexer should reach synced status again
And the latest block in storage should be the latest from the node
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown_n_restart_more_recent
# tail  -1000f /tmp/monitor-executions/shutdown_n_restart_more_recent/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-runner -- -f 100 -t shutdown_n_restart_more_recent
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown_n_restart_more_recent
# cargo run --bin archiver -- -t shutdown_n_restart_more_recent

Scenario: long run in subscribe mode
Given the initial best block is B (B = node latest block height - 100)
And the indexer is started
And the indexer catches up with backward sync and is suscribed for a very long while
When the indexer is shut down
Then the best block in storage should be the best from the node
And the storage should not have a checkpoint
And there should be no gaps in storage

# tmux new-session -d -s long_run_subs 'cargo run --bin block-indexer-runner -- -f 100 -t long_run_subs'
# tmux attach-session -t long_run_subs
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long_run_subs/app.log
# cargo run --bin block-indexer-validator -- -t long_run_subs
# cargo run --bin archiver -- -t long_run_subs

Scenario: long run in backward sync mode
Given the initial best block is the genesis block
And the indexer is started  
And the indexer runs until backward sync is completed
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s long_run_sync 'cargo run --bin block-indexer-runner -- -b 0 -t long_run_sync'
# tmux attach-session -t long_run_sync
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long_run_sync/app.log
# cargo run --bin block-indexer-validator -- -t long_run_sync
# cargo run --bin archiver -- -t long_run_sync

Scenario: small cache
Given the initial best block is B (B = node latest block height - 100)
And the cache size is 5
And the indexer is started
And the indexer catches up with backward sync and is suscribed for a while
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s small_cache 'cargo run --bin block-indexer-runner -- -f 100 -a 5 -t small_cache'
# tmux attach-session -t small_cache
# detach: CTRL+b, d
# ... until subscribed, wait a bit longer (15 min) -> shut down
# tail  -1000f /tmp/monitor-executions/small_cache/app.log
# cargo run --bin block-indexer-validator -- -t small_cache
# cargo run --bin archiver -- -t small_cache

Scenario: large cache and long backward sync
Given the initial best block is the genesis block
And the cache size is 1000000
And the indexer is started
And the indexer runs until backward sync is completed
And the indexer still runs for a while in subscription mode
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s long_run_sync_large_cache 'cargo run --bin block-indexer-runner -- -b 0 -t long_run_sync_large_cache'
# tmux attach-session -t long_run_sync_large_cache
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long_run_sync_large_cache/app.log
# cargo run --bin block-indexer-validator -- -t long_run_sync_large_cache
# cargo run --bin archiver -- -t long_run_sync_large_cache