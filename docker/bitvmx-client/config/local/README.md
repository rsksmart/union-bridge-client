# BitVMX client config (local / Anvil)

These `op_*.yaml` files must match the **bitvmx-client Docker image** declared in
`docker/bitvmx-client/docker-compose.yml` (currently **v0.5.3-alpha**).

That version uses the schema from [rust-bitvmx-client `config/op_1.yaml`](https://github.com/FairgateLabs/rust-bitvmx-client/blob/main/config/op_1.yaml):

- `comms` (listen address + P2P key + comms DB) — **not** top-level `p2p`
- `broker` (nested `port`, `ip`, `storage.path`, `allow_list`, …) — **not** `broker_port` / `broker_storage`

If the image is upgraded again, sync these files with the upstream `config/op_*.yaml` examples.

Docker-specific values here:

- Bitcoin RPC: `host.docker.internal:18443`, wallet `mainwallet` (see `docker/local-infra`)
- `comms.address` / static IPs: `172.20.0.1x` on `bitvmx-shared-network` (see `docker/operator/docker-compose.all.yml` + `start_operators.sh`)
- `broker.ip: 0.0.0.0` so the coordinator can reach the broker via the `bitvmx-client` service name on the compose network
