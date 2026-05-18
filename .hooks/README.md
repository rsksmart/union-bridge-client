# Hook helpers

This directory holds the shell scripts invoked by git hooks. The hooks
themselves are installed under `.git/hooks/` by [cargo-husky](https://github.com/rhysd/cargo-husky)
from the entrypoints in [`.cargo-husky/hooks/`](../.cargo-husky/hooks/); those
entrypoints are kept thin so the real logic here can be edited without
re-running `cargo build`.

| Script | Used by |
| --- | --- |
| `format-code.sh` | `pre-commit` hook (write mode) and CI (`--check` mode) |
| `check-lints.sh` | `pre-push` hook and CI |
| `check-branch-name.sh` | `pre-push` hook |
| `check-commit-message.sh` | `commit-msg` hook |

Each helper:

- runs in workspace-aware fashion across `.`, `cli/`, and `check-fork/zkp/guest`;
- can be invoked manually at any time (e.g. `bash .hooks/format-code.sh --check`).

Editing any of these takes effect on the next hook fire. The cargo-husky entrypoints
are only re-installed when its version in `Cargo.toml` changes.
