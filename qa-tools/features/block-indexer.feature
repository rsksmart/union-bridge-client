# As a monitor component,
# I want to continuously track the status of the Rootstock chain
# so that I have the latest canonical chain information
# and I robustly handle network issues, shutdowns, different cache sizes and long runs

Feature: Rootstock block monitoring and tracking

Scenario: happy path
Given the initial best block is B (B = node best block height - 100)
And the block indexer is started
And the block indexer catches up with backward sync and is suscribed for a while
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 100 -t happy-path-blk-indxr
# tail  -1000f /tmp/monitor-executions/happy-path-blk-indxr/app.log
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t happy-path-blk-indxr
# cargo run --bin archiver -- -t happy-path-blk-indxr

Scenario: shut down during backward sync (checkpoint)
Given the initial best block is B (B = node latest block height - 5000)
And the block indexer is started
And the block indexer runs in backward sync but it does not complete it
When the block indexer is shut down
Then the storage should have a checkpoint
Then the latest block in storage should be B, not the best one from the provider

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown-sync-blk-indxr
# tail  -1000f /tmp/monitor-executions/shutdown-sync-blk-indxr/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown-sync-blk-indxr
# cargo run --bin archiver -- -t shutdown-sync-blk-indxr

Scenario: shut down during backward sync and restart
Given the initial best block is B (B = node latest block height - 5000)
And the block indexer is started
And the block indexer runs in backward sync but it does not complete it
When the block indexer is shut down
And the block indexer is started again
And the block indexer catches up with backward sync and is suscribed for a while
Then the best block in storage should be the best from the node
And the storage should not have a checkpoint
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown-n-restart-blk-indxr
# tail  -1000f /tmp/monitor-executions/shutdown-n-restart-blk-indxr/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-runner -- -t shutdown-n-restart-blk-indxr -c false
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown-n-restart-blk-indxr
# cargo run --bin archiver -- -t shutdown-n-restart-blk-indxr

Scenario: shut down during subscription and restart with a more recent initial block
Given the initial best block is B (B = node latest block height - 5000)
And the block indexer is started
And the block indexer runs in backward sync but it does not complete it
When the block indexer is shut down
And the block indexer is started again after with initial block L (L = node latest block height - 100)
Then the block indexer should reach synced status again
And the latest block in storage should be the latest from the node
And there should be no gaps in storage

# cargo run --bin block-indexer-runner -- -f 5000 -t shutdown-n-restart-more-recent-blk-indxr
# tail  -1000f /tmp/monitor-executions/shutdown-n-restart-more-recent-blk-indxr/app.log
# ... before finishes backward sync -> shut down
# cargo run --bin block-indexer-runner -- -f 100 -t shutdown-n-restart-more-recent-blk-indxr
# ... until subscribed -> shut down
# cargo run --bin block-indexer-validator -- -t shutdown-n-restart-more-recent-blk-indxr
# cargo run --bin archiver -- -t shutdown-n-restart-more-recent-blk-indxr

Scenario: long run in subscribe mode
Given the initial best block is B (B = node latest block height - 100)
And the block indexer is started
And the block indexer catches up with backward sync and is suscribed for a very long while
When the block indexer is shut down
Then the best block in storage should be the best from the node
And the storage should not have a checkpoint
And there should be no gaps in storage

# tmux new-session -d -s long-run-subs-blk-indxr 'cargo run --bin block-indexer-runner -- -f 100 -t long-run-subs-blk-indxr'
# tmux attach-session -t long-run-subs-blk-indxr
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long-run-subs-blk-indxr/app.log
# cargo run --bin block-indexer-validator -- -t long-run-subs-blk-indxr
# cargo run --bin archiver -- -t long-run-subs-blk-indxr

Scenario: long run in backward sync mode
Given the initial best block is the genesis block
And the block indexer is started  
And the block indexer runs until backward sync is completed
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s long-run-sync-blk-indxr 'cargo run --bin block-indexer-runner -- -b 0 -t long-run-sync-blk-indxr'
# tmux attach-session -t long-run-sync-blk-indxr
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long-run-sync-blk-indxr/app.log
# cargo run --bin block-indexer-validator -- -t long-run-sync-blk-indxr
# cargo run --bin archiver -- -t long-run-sync-blk-indxr

Scenario: small cache
Given the initial best block is B (B = node latest block height - 100)
And the cache size is 5
And the block indexer is started
And the block indexer catches up with backward sync and is suscribed for a while
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s small-cache-blk-indxr 'cargo run --bin block-indexer-runner -- -f 100 -a 5 -t small-cache-blk-indxr'
# tmux attach-session -t small-cache-blk-indxr
# detach: CTRL+b, d
# ... until subscribed, wait a bit longer (15 min) -> shut down
# tail  -1000f /tmp/monitor-executions/small-cache-blk-indxr/app.log
# cargo run --bin block-indexer-validator -- -t small-cache-blk-indxr
# cargo run --bin archiver -- -t small-cache-blk-indxr

Scenario: large cache and long backward sync
Given the initial best block is the genesis block
And the cache size is 1000000
And the block indexer is started
And the block indexer runs until backward sync is completed
And the block indexer still runs for a while in subscription mode
When the block indexer is shut down
Then the best block in storage should be the best from the node
And there should be no gaps in storage

# tmux new-session -d -s long-run-sync-large-cache-blk-indxr 'cargo run --bin block-indexer-runner -- -b 0 -t long-run-sync-large-cache-blk-indxr'
# tmux attach-session -t long-run-sync-large-cache-blk-indxr
# detach: CTRL+b, d
# ... 24 hours -> shut down
# tail  -1000f /tmp/monitor-executions/long-run-sync-large-cache-blk-indxr/app.log
# cargo run --bin block-indexer-validator -- -t long-run-sync-large-cache-blk-indxr
# cargo run --bin archiver -- -t long-run-sync-large-cache-blk-indxr