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

If you want to use the `sc-event-mocking` CLI, run (and then double-enter):

```bash
docker attach $(docker compose ps -q sc-event-mocking)
```