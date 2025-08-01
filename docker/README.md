# Setup
Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that will be automatically used by the Docker Compose setup.

# Compose CLI

`./compose-cli.sh` provides an unified CLI tool for managing Docker Compose operations for the Union client.

## Setup

Make the compose script executable:
```
chmod +x compose-cli.sh
```

## Usage

The CLI help menu should provide all the information you need on how to build and run the services, including features and mocking management, and some examples.
```bash
./compose-cli.sh --help
```

# actors-mocking

If you want to use the `actors-mocking` CLI, run (you may need to double-enter):

```bash
./actors-mocking-cli.sh
```

## Troubleshooting

If you see an error like _Failed to get initial block by hash_, you may need to either:

1) reconfigure the `indexer.initial_block_hash` if running without features
2) rebuild the compose with the `anvil` feature enabled, which will skip this check: `./compose-cli.sh build --feature-anvil`