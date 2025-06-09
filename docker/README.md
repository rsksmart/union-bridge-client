# Setup
Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that will be automatically used by the Docker Compose setup.

```bash

# Build

To build the whole project's docker-compose, run:

## Whole Project

```bash
./docker/build.sh
```

In anvil mode;

```bash
./docker/build.sh anvil
```

## Just a Service
To build just one service, run:

```bash
./docker/build.sh service="<service_name>"
```

This mode works also with `anvil`:

## Notes

_Note: if you are not in a NIX system, you can check the commands within `docker/build.sh` and run them manually as a temporary approach._

# Run

To run the project's docker-compose, run:

```bash
docker compose up
```

_Note: take into account that the Cargo features are specified at build time, so if you want to change them, you need to rebuild the compose._

# sc-event-mocking

If you want to use the `sc-event-mocking` CLI, run (you may need to double-enter):

```bash
./cli-event-mocking.sh
```

# Troubleshooting

If you see an error like _Failed to get initial block by hash_, you may need to either:
1) reconfigure the `indexer.initial_block_hash` if running without features
2) rebuild the compose with the `anvil` feature enabled, which will skip this check: `./docker/build.sh anvil`