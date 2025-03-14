curl -X POST http://rskj-01.testnet.ub.iovlabs.net:4444 \
   -H "Content-Type: application/json" \
   -d '{
     "jsonrpc": "2.0",
     "method": "eth_blockNumber",
     "params": [],
     "id": 1
   }'
{"jsonrpc":"2.0","id":1,"result":"0x5dc24e"}

# in local:
RUST_BACKTRACE=1 RUST_LOG=info cargo run --bin block-indexer &
RUST_BACKTRACE=1 RUST_LOG=info cargo run --bin log-indexer &
tail  -100f ./logs/app.log | grep "subscribe_logs"
tail  -100f ./logs/app.log
ps aux | grep indexer

## to get the latest block height:
wscat -c ws://rskj-01.testnet.ub.iovlabs.net:4445/websocket
{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}
example response: {"jsonrpc":"2.0","id":1,"result":"0x5e156a"}
translate hex to dec, substract desired difference, convert to hex again. Fetch block by height, find block hash:
{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["0x1a3b8", false]}
adjust initial block hash in project config

# config Boton:

(in remote)
mkdir ~/.ssh

(in my local)
scp ~/.ssh_backup/id_ed25519 ubuntu@3.75.202.159:~/.ssh/
scp ~/.ssh_backup/id_ed25519.pub ubuntu@3.75.202.159:~/.ssh/
scp ~/.ssh_backup/authorized_keys ubuntu@3.75.202.159:~/.ssh/
scp ~/.ssh_backup/known_hosts ubuntu@3.75.202.159:~/.ssh/

(in remote)
ssh-add ~/.ssh/id_ed25519
mkdir UnionBridge
cd ~/UnionBridge
git clone git@github.com:rsksmart/union-bridge-monitor.git
cd union-bridge-monitor/
git pull
git checkout tests/boton-test-experimental 
cd ~/UnionBridge
git clone git@github.com:FairgateLabs/rust-bitvmx-workspace.git
git clone git@github.com:FairgateLabs/rust-bitvmx-storage-backend.git

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.bashrc

sudo apt update
sudo apt install build-essential
sudo apt install pkg-config libssl-dev
sudo apt install clang libclang-dev llvm
export LIBCLANG_PATH=$(llvm-config --libdir)

cd union-bridge-monitor/
cargo clean
cargo build

# Test scenarios

## Block indexer

### Happy path
Given the initial best block is B (B = node latest block height - 200)
And the indexer is started
And the indexer catches up with backward sync and is suscribed for a while
When the block indexer is shut down
Then the latest block in storage should be the latest from the node
And there should be no gaps in storage

### With shut down during backward sync (checkpoint)
Given the initial best block is B (B = node latest block height - 200)
And the indexer is started
And the indexer runs for 10 seconds (it initiates backward sync but does not complete it)
When the block indexer is shut down
Then the storage should have a checkpoint

### With shut down during backward sync and restart
Given the initial best block is B (B = node latest block height - 200)
And the indexer is started
And the indexer runs for 10 seconds (it initiates backward sync but does not complete it)
When the block indexer is shut down
And the indexer is started again for 5 minutes (it catches up with backward sync and is suscribed for a while)
Then the latest block in storage should be the latest from the node
And the storage should NOT have a checkpoint
And there should be no gaps in storage

### Long run in subscribe mode
Given the initial best block is B (B = node latest block height - 200)
And the indexer is started
And the indexer runs for 24 hours
When the indexer is shut down
Then the latest block in storage should be the latest from the node
And there should be no gaps in storage

### Long run in backward sync mode
Given the initial best block is the genesis block
And the indexer is started  
And the indexer runs until backward sync is completed
When the block indexer is shut down
Then the latest block in storage should be the latest from the node
And there should be no gaps in storage

### Small cache
Given the initial best block is B (B = node latest block height - 200)
And the cache size is 10
And the indexer is started
And the indexer runs for 15 minutes (it catches up with backward sync and is suscribed for a while, some minor reorgs might happen)
When the block indexer is shut down
Then the latest block in storage should be the latest from the node
And there should be no gaps in storage

### Large cache and long backward sync
Given the initial best block is B (B = node latest block height - 20000000)
And the cache size is 1000000
And the indexer is started
And the indexer runs until backward sync is completed
And the indexer still runs for a while in subscription mode
When the block indexer is shut down
Then the latest block in storage should be the latest from the node
And there should be no gaps in storage

## Log indexer

### Happy path
Given the log filter is set to track managed contracts from C.A...C.F
And the log filter is set starting from block B
And there are contract calls (L.A...L.D) before block B - contracts are C.A..C.F
And there are contract calls (L.E...L.H) after block B but before provider's best block - contracts are C.A..C.F
And there are contract calls (L.I...L.L) after block B but before provider's best block - contracts are C.E..C.K (outside the managed set)
When the log indexer is started
And the user calls (L.I...L.L) contracts C.A..C.F
And the user calls (L.M...L.P) contracts C.E..C.K
And the log indexer is shut down
Then the storage should contain only the logs corresponding to interactions with managed contracts C.A..C.F that occur after block B (L.E...L.H, L.I...L.L)

### Persistency after shut down
Given the log filter is set to track managed contracts from C.A...C.F
When the log indexer is started
And the user calls (L.A...L.D) contracts C.A..C.F
And the log indexer is shut down
And the log indexer is started again
And the user calls (L.E...L.H) contracts C.A..C.F
And the log indexer is shut down
Then the storage should contain the logs corresponding to calls L.A..L.H

### Long run 
Given the log filter is set to track managed contracts from C.A...C.F
When the log indexer is started
And the user calls (L.A...L.Z) contracts C.A..C.F over a period of 24 hours
And the indexer is shut down
Then the storage should contain the logs corresponding to calls L.A...L.Z

### DB stress test
Given the log filter is set to track managed contracts from C.A...C.F
When the log indexer is started
And the user generates an intense burst of calls (L.A...L.Z) to contracts C.A..C.F within a short timespan (10 seconds)
And the indexer is shut down
Then the storage should contain the logs corresponding to calls L.A...L.Z