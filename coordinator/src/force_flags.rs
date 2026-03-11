//! Force flags for testing purposes only.
//!
//! These flags allow QA and developers to trigger advance funds and dispute
//! mechanisms without waiting for normal timeouts or conditions.
//!
//! **IMPORTANT**: These flags are ONLY active in non-production environments
//! (Local, `LocalDocker`, Regtest). They are automatically disabled in
//! Alphanet and Testnet.
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

/// Environments where force flags are BLOCKED (production-like)
const BLOCKED_ENVIRONMENTS: [&str; 2] = ["alphanet", "testnet"];

/// File path for hot-reloadable `FORCE_ADVANCE` flag
const FORCE_ADVANCE_FILE: &str = "/tmp/FORCE_ADVANCE";

/// File path for hot-reloadable `FORCE_DISPUTE` flag
const FORCE_DISPUTE_FILE: &str = "/tmp/FORCE_DISPUTE";

/// Checks if the current environment allows force flags.
///
/// Returns `true` for Local, `LocalDocker`, and Regtest environments.
/// Returns `false` for Alphanet and Testnet (production-like environments).
fn is_force_flags_allowed(env_name: Option<&str>) -> bool {
    match env_name {
        None => true, // Local development (no --env flag)
        Some(env) => {
            let env_lower = env.to_lowercase();
            !BLOCKED_ENVIRONMENTS.iter().any(|&blocked| env_lower.contains(blocked))
        }
    }
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
/// Only works in non-production environments (Local, `LocalDocker`, Regtest).
#[must_use]
pub fn get_force_advance_address(env_name: Option<&str>) -> Option<String> {
    if !is_force_flags_allowed(env_name) {
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
            "[FORCE_ADVANCE] Force advance funds targeting address {addr} in environment: {env_name:?}"
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
/// Only works in non-production environments (Local, `LocalDocker`, Regtest).
#[must_use]
pub fn is_force_dispute_enabled(env_name: Option<&str>) -> bool {
    if !is_force_flags_allowed(env_name) {
        return false;
    }

    // Check file first (hot-reloadable), then env var
    let enabled = Path::new(FORCE_DISPUTE_FILE).exists()
        || std::env::var("FORCE_DISPUTE")
            .ok()
            .is_some_and(|v| v.to_lowercase() == "true" || v == "1");

    if enabled {
        warn!("[FORCE_DISPUTE] Force dispute is ENABLED for environment: {env_name:?}");
    }
    enabled
}

#[cfg(test)]
// Allow unsafe blocks for env var manipulation in tests.
// SAFETY: Tests using set_var/remove_var must run with --test-threads=1
// to avoid race conditions. This is documented in the test comments.
#[allow(unsafe_code)]
mod tests {
    use super::*;

    #[test]
    fn test_force_flags_allowed_local_environments() {
        // Local (no env) should allow force flags
        assert!(is_force_flags_allowed(None));

        // Local environments should allow force flags
        assert!(is_force_flags_allowed(Some("local")));
        assert!(is_force_flags_allowed(Some("local-docker")));
        assert!(is_force_flags_allowed(Some("docker-local")));
        assert!(is_force_flags_allowed(Some("LOCAL"))); // case insensitive

        // Regtest should allow force flags
        assert!(is_force_flags_allowed(Some("regtest")));
        assert!(is_force_flags_allowed(Some("REGTEST")));
    }

    #[test]
    fn test_force_flags_blocked_production_environments() {
        // Alphanet should block force flags
        assert!(!is_force_flags_allowed(Some("alphanet")));
        assert!(!is_force_flags_allowed(Some("ALPHANET")));
        assert!(!is_force_flags_allowed(Some("docker-alphanet")));

        // Testnet should block force flags
        assert!(!is_force_flags_allowed(Some("testnet")));
        assert!(!is_force_flags_allowed(Some("TESTNET")));
    }

    // All FORCE_ADVANCE env-var tests are combined into a single test to prevent
    // race conditions -- parallel tests sharing a process-global env var will conflict.
    #[test]
    fn test_force_advance_env_var() {
        // SAFETY: set_var/remove_var are unsafe in multi-threaded contexts.
        // Consolidating into one test avoids the race without requiring --test-threads=1.
        unsafe {
            std::env::remove_var("FORCE_ADVANCE");

            // Without env var set, should return None
            assert!(get_force_advance_address(None).is_none());
            assert!(get_force_advance_address(Some("local")).is_none());

            // With env var set to an address in local, should return Some(address)
            std::env::set_var("FORCE_ADVANCE", "0xABCDEF1234567890");
            assert_eq!(get_force_advance_address(None).as_deref(), Some("0xABCDEF1234567890"));
            assert_eq!(
                get_force_advance_address(Some("local")).as_deref(),
                Some("0xABCDEF1234567890")
            );
            assert_eq!(
                get_force_advance_address(Some("regtest")).as_deref(),
                Some("0xABCDEF1234567890")
            );

            // In production, should return None regardless
            assert!(get_force_advance_address(Some("alphanet")).is_none());
            assert!(get_force_advance_address(Some("testnet")).is_none());

            // Specific address value
            std::env::set_var("FORCE_ADVANCE", "0xDEADBEEF");
            assert_eq!(get_force_advance_address(Some("local")).as_deref(), Some("0xDEADBEEF"));

            // Empty string should return None
            std::env::set_var("FORCE_ADVANCE", "");
            assert!(get_force_advance_address(Some("local")).is_none());

            // Whitespace-only should return None
            std::env::set_var("FORCE_ADVANCE", "  ");
            assert!(get_force_advance_address(Some("local")).is_none());

            // Address with whitespace should be trimmed
            std::env::set_var("FORCE_ADVANCE", "  0xABC123  ");
            assert_eq!(get_force_advance_address(Some("local")).as_deref(), Some("0xABC123"));

            std::env::remove_var("FORCE_ADVANCE");
        }
    }

    #[test]
    fn test_force_dispute_env_var() {
        // SAFETY: set_var/remove_var are unsafe in multi-threaded contexts.
        // Consolidating into one test avoids the race without requiring --test-threads=1.
        unsafe {
            std::env::remove_var("FORCE_DISPUTE");

            // Without env var set, should return false
            assert!(!is_force_dispute_enabled(None));
            assert!(!is_force_dispute_enabled(Some("local")));

            // With env var set to true in local, should return true
            std::env::set_var("FORCE_DISPUTE", "true");
            assert!(is_force_dispute_enabled(None));
            assert!(is_force_dispute_enabled(Some("local")));
            assert!(is_force_dispute_enabled(Some("regtest")));

            // With env var set to true in production, should return false
            assert!(!is_force_dispute_enabled(Some("alphanet")));
            assert!(!is_force_dispute_enabled(Some("testnet")));

            std::env::remove_var("FORCE_DISPUTE");
        }
    }
}
