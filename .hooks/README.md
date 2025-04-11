# Custom Git Hooks

These hooks are used within `rusty-hook.toml` file. If a change is required on them, you should modify the `.rs` file
and then build it again. For example:

```
rustc -O check_branch_name.rs -o check_branch_name
```