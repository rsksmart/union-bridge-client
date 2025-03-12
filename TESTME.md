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