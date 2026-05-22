//! Force flags for testing purposes only.
//!
//! These flags allow QA and developers to trigger advance funds and dispute
//! mechanisms without waiting for normal timeouts or conditions.
//!
//! **IMPORTANT**: These flags are enabled only when the loaded runtime
//! environment is `local-anvil` or `local-rskj`. The runtime string is the
//! tier (backend axis), so both cargo and docker operator modes are covered.
//!
//! ## Activation Methods
//!
//! Flags can be enabled via **file** (hot-reloadable) or **environment variable**:
//!
//! ### File-based (recommended for testing - hot-reloadable)
//!
//! - `echo "0xOPERATOR_ADDRESS" > /tmp/FORCE_ADVANCE` - Target operator skips signatures
//! - `touch /tmp/FORCE_DISPUTE` - Enable dispute override
//! - `rm /tmp/FORCE_ADVANCE` - Disable (delete the file)
//!
//! ### Environment Variables (set at startup)
//!
//! - `FORCE_ADVANCE=0xOPERATOR_ADDRESS` - Target operator skips signatures
//! - `FORCE_DISPUTE=true` - Enable dispute override
//!
//! ## Flag Behavior
//!
//! - `FORCE_ADVANCE` - Contains a Rootstock address. The targeted operator skips the
//!   signature sub-flow, simulating operator misbehavior. Since signatures never complete,
//!   the advance funds timeout triggers naturally.
//! - `FORCE_DISPUTE` - Overrides `ReimbursementResult` to `OperatorWon`, simulating
//!   a successful dispute.

use std::path::Path;

use log::warn;

use crate::{RUNTIME_ENV_LOCAL_ANVIL, RUNTIME_ENV_LOCAL_RSKJ};

/// File path for hot-reloadable `FORCE_ADVANCE` flag
const FORCE_ADVANCE_FILE: &str = "/tmp/FORCE_ADVANCE";

/// File path for hot-reloadable `FORCE_DISPUTE` flag
const FORCE_DISPUTE_FILE: &str = "/tmp/FORCE_DISPUTE";

/// Checks if the current environment allows force flags.
///
/// Returns `true` only for local test runtime environments.
fn is_force_flags_allowed(runtime_environment: Option<&str>) -> bool {
    runtime_environment.is_some_and(|env| {
        env.eq_ignore_ascii_case(RUNTIME_ENV_LOCAL_ANVIL)
            || env.eq_ignore_ascii_case(RUNTIME_ENV_LOCAL_RSKJ)
    })
}

/// Returns the Rootstock address targeted by `FORCE_ADVANCE`, if set.
///
/// When set, the targeted operator skips the signature sub-flow, simulating
/// operator misbehavior. Since signatures never complete, the advance funds
/// timeout triggers naturally.
///
/// The value should be a Rootstock address (e.g. `0x1234...`).
///
/// Checks file first (hot-reloadable), then falls back to environment variable.
///
/// Only works in local test runtime environments.
#[must_use]
pub fn get_force_advance_address(runtime_environment: Option<&str>) -> Option<String> {
    if !is_force_flags_allowed(runtime_environment) {
        return None;
    }

    // Check file first (hot-reloadable), then env var
    let address = std::fs::read_to_string(FORCE_ADVANCE_FILE)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("FORCE_ADVANCE")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    if let Some(ref addr) = address {
        warn!(
            "[FORCE_ADVANCE] Force advance funds targeting address {addr} in environment: {runtime_environment:?}"
        );
    }
    address
}

/// Checks if `FORCE_DISPUTE` is enabled.
///
/// When enabled, all `ReimbursementResult` challenge results will be overridden
/// to `OperatorWon`, simulating a successful dispute.
///
/// Checks file first (hot-reloadable), then falls back to environment variable.
///
/// Only works in local test runtime environments.
#[must_use]
pub fn is_force_dispute_enabled(runtime_environment: Option<&str>) -> bool {
    if !is_force_flags_allowed(runtime_environment) {
        return false;
    }

    // Check file first (hot-reloadable), then env var
    let enabled = Path::new(FORCE_DISPUTE_FILE).exists()
        || std::env::var("FORCE_DISPUTE")
            .ok()
            .is_some_and(|v| v.to_lowercase() == "true" || v == "1");

    if enabled {
        warn!("[FORCE_DISPUTE] Force dispute is ENABLED for environment: {runtime_environment:?}");
    }
    enabled
}

#[cfg(test)]
// Allow unsafe blocks for env var manipulation in tests.
// SAFETY: Tests serialize process-global env/file mutations with a mutex.
#[allow(unsafe_code)]
mod tests {
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use super::*;

    static FORCE_FLAGS_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock_force_flags_test_state() -> MutexGuard<'static, ()> {
        FORCE_FLAGS_TEST_LOCK.lock().expect("force flags test lock poisoned")
    }

    fn clear_force_flags_state() {
        // SAFETY: Test state is serialized through FORCE_FLAGS_TEST_LOCK,
        // so mutating process environment here is safe.
        unsafe {
            std::env::remove_var("FORCE_ADVANCE");
            std::env::remove_var("FORCE_DISPUTE");
        }
        let _ = std::fs::remove_file(FORCE_ADVANCE_FILE);
        let _ = std::fs::remove_file(FORCE_DISPUTE_FILE);
    }

    #[test]
    fn test_force_flags_allowed_local_environments() {
        let _guard = lock_force_flags_test_state();
        clear_force_flags_state();

        // Both dev-tier runtimes (anvil and rskj) should allow force flags.
        assert!(is_force_flags_allowed(Some(RUNTIME_ENV_LOCAL_ANVIL)));
        assert!(is_force_flags_allowed(Some("LOCAL-ANVIL"))); // verifies case-insensitivity
        assert!(is_force_flags_allowed(Some(RUNTIME_ENV_LOCAL_RSKJ)));

        // Production-like and unknown runtime tiers should not allow force flags.
        assert!(!is_force_flags_allowed(None));
        assert!(!is_force_flags_allowed(Some("")));
        assert!(!is_force_flags_allowed(Some("regtest")));
        assert!(!is_force_flags_allowed(Some("REGTEST")));
        assert!(!is_force_flags_allowed(Some("stage")));
    }

    #[test]
    fn test_force_flags_blocked_non_local_environments() {
        let _guard = lock_force_flags_test_state();
        clear_force_flags_state();

        // Alphanet should block force flags
        assert!(!is_force_flags_allowed(Some("alphanet")));
        assert!(!is_force_flags_allowed(Some("ALPHANET")));

        // Testnet should block force flags
        assert!(!is_force_flags_allowed(Some("testnet")));
        assert!(!is_force_flags_allowed(Some("TESTNET")));
    }

    #[test]
    fn test_force_advance_respects_env_safety() {
        let _guard = lock_force_flags_test_state();

        // SAFETY: Access to process-global env vars is serialized via FORCE_FLAGS_TEST_LOCK.
        unsafe {
            clear_force_flags_state();

            // Without env var set, should return None
            assert!(get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).is_none());

            // With env var set to an address in local-anvil, should return Some(address)
            std::env::set_var("FORCE_ADVANCE", "0xABCDEF1234567890");
            assert_eq!(
                get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).as_deref(),
                Some("0xABCDEF1234567890")
            );
            // rskj dev tier is also enabled (covers both cargo and docker operator modes).
            assert_eq!(
                get_force_advance_address(Some(RUNTIME_ENV_LOCAL_RSKJ)).as_deref(),
                Some("0xABCDEF1234567890")
            );
            // In non-local/non-test environments, should return None regardless.
            assert!(get_force_advance_address(Some("regtest")).is_none());
            assert!(get_force_advance_address(Some("alphanet")).is_none());
            assert!(get_force_advance_address(Some("testnet")).is_none());

            // Clean up
            clear_force_flags_state();
        }
    }

    #[test]
    fn test_force_dispute_respects_env_safety() {
        let _guard = lock_force_flags_test_state();

        // SAFETY: Access to process-global env vars is serialized via FORCE_FLAGS_TEST_LOCK.
        unsafe {
            clear_force_flags_state();

            // Without env var set, should return false
            assert!(!is_force_dispute_enabled(Some(RUNTIME_ENV_LOCAL_ANVIL)));

            // With env var set to true in local-anvil, should return true
            std::env::set_var("FORCE_DISPUTE", "true");
            assert!(is_force_dispute_enabled(Some(RUNTIME_ENV_LOCAL_ANVIL)));
            // rskj dev tier is also enabled (covers both cargo and docker operator modes).
            assert!(is_force_dispute_enabled(Some(RUNTIME_ENV_LOCAL_RSKJ)));
            // In non-local/non-test environments, should return false.
            assert!(!is_force_dispute_enabled(Some("regtest")));
            assert!(!is_force_dispute_enabled(Some("alphanet")));
            assert!(!is_force_dispute_enabled(Some("testnet")));

            // Clean up
            clear_force_flags_state();
        }
    }

    #[test]
    fn test_force_advance_returns_address_value() {
        let _guard = lock_force_flags_test_state();

        // SAFETY: Access to process-global env vars is serialized via FORCE_FLAGS_TEST_LOCK.
        unsafe {
            clear_force_flags_state();

            std::env::set_var("FORCE_ADVANCE", "0xDEADBEEF");
            assert_eq!(
                get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).as_deref(),
                Some("0xDEADBEEF")
            );

            // Empty string should return None
            std::env::set_var("FORCE_ADVANCE", "");
            assert!(get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).is_none());

            // Whitespace-only should return None
            std::env::set_var("FORCE_ADVANCE", "  ");
            assert!(get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).is_none());

            // Address with whitespace should be trimmed
            std::env::set_var("FORCE_ADVANCE", "  0xABC123  ");
            assert_eq!(
                get_force_advance_address(Some(RUNTIME_ENV_LOCAL_ANVIL)).as_deref(),
                Some("0xABC123")
            );

            // Clean up
            clear_force_flags_state();
        }
    }
}
