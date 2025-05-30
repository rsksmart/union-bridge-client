# Build

To build the whole project's docker-compose, run:
```bash
docker compose build
```

To build just one crate contaier, run:
```bash
docker compose build --build-arg JUST_CRATE=<crate-name>
```

# Run
To run the whole project's docker-compose, run:
```bash
docker compose up -d
```

# sc-event-mocking
If you want to use the `sc-event-mocking` CLI, run (and double-enter):
```bash
docker attach $(docker compose ps -q sc-event-mocking)
```

# key store management
TODO(iago) explain