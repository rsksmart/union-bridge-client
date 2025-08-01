# Setup
Copy the `.env.example` file to `.env` and adjust the values as needed. This file contains environment variables that will be automatically used by the Docker Compose setup.

# Build

_Note: All the commands mentioned below should be run from the `docker` directory._

## Whole Project

**Only the first time, or when you want to rebuild the builder image** (containing apt packages, etc.), you should run the following command:

```bash
./compose_build_builder.sh
```

Usually you will only need to build the project itself, which is done by running the following command:

```bash
./compose_build.sh
```

In anvil mode;

```bash
./compose_build.sh anvil
```

With mocking (actors-mocking):

```bash
./compose_build.sh mocking
```

## Just a Service

To build just one service, run:

```bash
./compose_build.sh service="<service_name>"
```

This mode works also with `anvil`.

## Notes

_Note: if you are not in a NIX system, you can check the commands within `docker/compose_build.sh` and run them manually as a temporary approach._

# Run

To run the project's docker-compose, run:

```bash
./compose_up.sh
```

If you want to run with mocks, you first need to build with mocking (see above) and then run:

```bash
./compose_up.sh mocking
```

_Note: take into account that mocking and the Cargo features are specified at build time, so if you want to change them, you need to rebuild the compose._

# actors-mocking

If you want to use the `actors-mocking` CLI, run (you may need to double-enter):

```bash
./cli-event-mocking.sh
```

## Troubleshooting

If you see an error like _Failed to get initial block by hash_, you may need to either:

1) reconfigure the `indexer.initial_block_hash` if running without features
2) rebuild the compose with the `anvil` feature enabled, which will skip this check: `./docker_build.sh anvil`