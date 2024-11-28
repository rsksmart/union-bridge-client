# Union Bridge - Monitor

# What is the Monitor?

The `monitor` checks the state of Rootstock and feeds the `zkp` with `CheckForkArgs` for zkp validation.

TODO: Add more details about the Monitor when clear.

# Setup

In order to be able to run the monitor in your local, follow the steps below:

1. Create a `zkp` symlink to the `FairgateLabs/rust-bitvmx-zk-proof` crate in your local. It will be used as the ZKVM
   for the Monitor. Note that this is a temporary approach until the projects structure and collaboration is defined.