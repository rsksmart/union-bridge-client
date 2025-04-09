## 🛠 ABI Binding with `RUN_MODE` (Build-Time Configuration)

This project uses [`alloy-sol-types`](https://docs.rs/alloy-sol-types) and a `build.rs` script to automatically generate
contract bindings using the `sol!` macro **at compile time**, depending on the environment.

### ✅ How It Works

- The `build.rs` script reads the `RUN_MODE` environment variable.
- It selects the corresponding ABI files at:
  ```
  config/{RUN_MODE}/abi/{ABI_NAME}.json
  ```

- It generates a Rust file `abi.rs` containing a `sol!` macro for each ABI, for example:
  ```rust
  sol!(
      #[sol(rpc)]
      SolPegManager,
      "config/local/abi/PegManager.json"
  );
  ```
- This file is included into the crate at compile time using:

  ```rust
  include!(concat!(env!("OUT_DIR"), "/abi.rs"));
  ```

### 📦 Directory Structure

```
├── local/abi
│ ├── PegManager.json
│ └── BitcoinManager.json
│
└── stage/abi
│ ├── PegManager.json
│ └── BitcoinManager.json
│
```

Each environment has its own ABI version, allowing for environment-specific contracts.

---

### 🚀 Usage

By default, `RUN_MODE=local`.

You can override this when building or running:

```bash
RUN_MODE=stage cargo build
RUN_MODE=prod cargo run
```

> ⚠️ Changing `RUN_MODE` requires a rebuild to regenerate the correct ABI binding.

---

### 💡 Why This Approach?

- Keeps ABI management **compile-time safe** via `sol!`
- Allows **single binary per environment**
- Avoids Cargo features / conditional compilation clutter
- Simple to swap environments by setting `RUN_MODE`

---

### 💪 Example

Import it with `include!` and use it as you would if you were directly using `sol!` (check Alloy doc for more details)

```rust
include!(concat!(env!("OUT_DIR"), "/abi.rs"));
```

---

### 📎 Troubleshooting

- If you see a build error like **“ABI file not found”**, make sure the file exists at:
  ```
  config/{RUN_MODE}/abi/PegManager.json
  ```
- The build script will fail early if the ABI file is missing.

---

### 🧰 Customization

If needed, you can support additional contracts by having `build.rs` generate more `sol!` blocks.
