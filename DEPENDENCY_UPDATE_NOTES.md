# Dependency Update: rust-bitvmx-storage-backend

## Summary
Updated `rust-bitvmx-storage-backend` dependency from version `v0.1.0` (rev `7f40d056f86295e3fd2a4f268f923ca3b4e70fbf`) to version `v0.1.1` (rev `16a01bfe316266bdac080828e260cd191fcb3cde`).

## Breaking Changes

### API Changes

#### 1. Storage Initialization
The `Storage` API has been refactored to use a `StorageConfig` structure instead of direct path parameters:

**Before:**
```rust
let db = Storage::new_with_path(&PathBuf::from(path))?;
```

**After:**
```rust
use storage_backend::storage_config::StorageConfig;

let config = StorageConfig::new(path.to_string(), None);
let db = Storage::new(&config)?;  // For new databases (creates if missing)
// or
let db = Storage::open(&config)?; // For existing databases only
```

#### 2. Iteration API
The `get_all()` method has been removed. Use `partial_compare()` or `partial_compare_keys()` instead:

**Before:**
```rust
let entries: HashMap<String, T> = db.get_all()?;
```

**After:**
```rust
// For key-value pairs
let entries: Vec<(String, String)> = db.partial_compare("prefix/")?;
for (key, value_str) in entries {
    let value: T = serde_json::from_str(&value_str)?;
    // process...
}

// For keys only
let keys: Vec<String> = db.partial_compare_keys("prefix/")?;
```

### New Dependencies
The update introduces several new encryption-related dependencies:
- `aead`, `aes-gcm`, `chacha20`, `chacha20poly1305`
- `cocoon`, `pbkdf2`, `sha2`
- Various cryptographic primitives

This suggests the new version may include encryption capabilities (via the optional `password` parameter in `StorageConfig`).

## Files Modified

### Dependency Configuration
1. **Cargo.toml** (workspace root)
   - Updated `storage-backend` dependency rev to `16a01bfe316266bdac080828e260cd191fcb3cde`

2. **bitcoin-wallet/Cargo.toml**
   - Updated `storage-backend` dependency rev to `16a01bfe316266bdac080828e260cd191fcb3cde`

### Code Changes
All files using `Storage::new_with_path()` have been updated:

1. **log-indexer/src/store.rs**
   - Added import: `use storage_backend::storage_config::StorageConfig;`
   - Changed `Storage::new_with_path()` to `Storage::new(&StorageConfig::new(...))`
   - Removed unused `PathBuf` import
   - Replaced `get_all()` with `partial_compare("")` in `get_all_logs()` test utility
   - Added manual JSON deserialization for values returned from `partial_compare()`

2. **block-indexer/src/store.rs**
   - Added import: `use storage_backend::storage_config::StorageConfig;`
   - Changed `Storage::new_with_path()` to `Storage::new(&StorageConfig::new(...))`
   - Removed unused `PathBuf` import

3. **coordinator/src/store.rs**
   - Added import: `use storage_backend::storage_config::StorageConfig;`
   - Changed `Storage::new_with_path()` to `Storage::new(&StorageConfig::new(...))`
   - Removed unused `PathBuf` import

4. **bitcoin-wallet/src/utxo_store.rs**
   - Added import: `use storage_backend::storage_config::StorageConfig;`
   - Changed `Storage::new_with_path()` to `Storage::new(&StorageConfig::new(...))`
   - Used `Storage::new()` to preserve the original semantics (create database if it doesn't exist)
   - Replaced `get_all()` calls with `partial_compare("utxo/")` for iteration
   - Replaced `get_all()` in `clear()` with `partial_compare_keys("utxo/")` for key-only iteration
   - Added manual JSON deserialization for values returned from `partial_compare()`

## Testing Status

### ✅ Passing Tests
- All workspace modules compile successfully
- `block-indexer` tests: 22 passed
- `coordinator` tests: 101 passed
- `log-indexer` store tests: 3 passed
- All storage-related functionality verified

## Verification

To verify the update:
```bash
# Update dependencies
cargo update -p rust-bitvmx-storage-backend

# Build workspace
cargo build --workspace

# Run tests
cargo test --workspace --lib
```

## Migration Notes

If you need to add password protection to storage in the future, you can now use:
```rust
let config = StorageConfig::new(path.to_string(), Some(password_string));
```

The password parameter is currently set to `None` for all usages to maintain backward compatibility.

