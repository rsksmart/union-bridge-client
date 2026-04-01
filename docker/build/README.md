# Docker Build

This directory is only for building and publishing Union Client images. It does not own operator runtime
documentation.

## Related Docs

- [../README.md](../README.md): Docker doc index
- [../operator/README.md](../operator/README.md): run operators with the built images
- [../../CONTRIBUTING.md](../../CONTRIBUTING.md): shared configuration override rules

## Config

No local `.env` file is required for the supported flow in this directory.

The build compose uses the repository `config/` tree. Runtime overrides still follow the shared `UB__...` environment
variable mapping documented in [../../CONTRIBUTING.md](../../CONTRIBUTING.md#configuration-overrides).

## Build Builder Images

`d-build-builder.sh` builds the Union Client builder images.

```bash
bash d-build-builder.sh --help
```

## Build Service Images

`d-build-client.sh` builds the Union Client runtime images.

```bash
bash d-build-client.sh --help
```

If the images will connect to a local Anvil node, build with the `anvil` feature enabled:

```bash
bash d-build-client.sh --features=anvil
```

For public-repo Docker runtime usage:

1. build images from here with `d-build-client.sh`
2. run local operators from [../operator/README.md](../operator/README.md)

If you still run `docker compose` manually from this directory, export any required variables in your shell first, such
as `KEY_STORE_PASSWORD`.

## Registry Operations

Pull images:

```bash
bash d-ghcr-pull.sh
```

Push images:

```bash
bash d-ghcr-push.sh
```

Both require GHCR authentication first:

```bash
echo "$GITHUB_REGISTRY_TOKEN" | docker login ghcr.io -u <your_user> --password-stdin
```

## Next Step

After building or pulling images, switch to [../operator/README.md](../operator/README.md) for local runtime commands.
Remote deployments are maintained outside this repository.
