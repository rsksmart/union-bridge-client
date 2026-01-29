//! Force flags for testing purposes only.
//!
//! These flags allow QA and developers to trigger advance funds and dispute
//! mechanisms without waiting for normal timeouts or conditions.
//!
//! **IMPORTANT**: These flags are ONLY active in non-production environments
//! (Local, LocalDocker, Regtest). They are automatically disabled in
//! Alphanet and Testnet.
//!
//! ## Environment Variables
//!
//! - `FORCE_ADVANCE=true` - Immediately trigger operator take for pegouts
//!   in DispatchTransaction step (skips the normal timeout)
//! - `FORCE_DISPUTE=true` - Override ReimbursementResult to OperatorWon
//!   (simulates a successful dispute)

use log::warn;

/// Environments where force flags are BLOCKED (production-like)
const BLOCKED_ENVIRONMENTS: [&str; 2] = ["alphanet", "testnet"];

/// Checks if the current environment allows force flags.
///
/// Returns `true` for Local, LocalDocker, and Regtest environments.
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

/// Checks if FORCE_ADVANCE is enabled.
///
/// When enabled, pegouts in `DispatchTransaction` step will immediately
/// trigger operator take instead of waiting for the normal timeout.
///
/// Only works in non-production environments (Local, LocalDocker, Regtest).
pub fn is_force_advance_enabled(env_name: Option<&str>) -> bool {
    if !is_force_flags_allowed(env_name) {
        return false;
    }
    let enabled = std::env::var("FORCE_ADVANCE")
        .ok()
        .map(|v| v.to_lowercase() == "true" || v == "1")
        .unwrap_or(false);

    if enabled {
        warn!("[FORCE_ADVANCE] Force advance funds is ENABLED for environment: {:?}", env_name);
    }
    enabled
}

/// Checks if FORCE_DISPUTE is enabled.
///
/// When enabled, all ReimbursementResult challenge results will be overridden
/// to `OperatorWon`, simulating a successful dispute.
///
/// Only works in non-production environments (Local, LocalDocker, Regtest).
pub fn is_force_dispute_enabled(env_name: Option<&str>) -> bool {
    if !is_force_flags_allowed(env_name) {
        return false;
    }
    let enabled = std::env::var("FORCE_DISPUTE")
        .ok()
        .map(|v| v.to_lowercase() == "true" || v == "1")
        .unwrap_or(false);

    if enabled {
        warn!("[FORCE_DISPUTE] Force dispute is ENABLED for environment: {:?}", env_name);
    }
    enabled
}

#[cfg(test)]
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

    #[test]
    fn test_force_advance_respects_env_safety() {
        // SAFETY: These tests must run with --test-threads=1 to avoid race conditions
        // with environment variables. set_var/remove_var are unsafe in multi-threaded contexts.
        unsafe {
            // Clear any existing env var
            std::env::remove_var("FORCE_ADVANCE");

            // Without env var set, should return false
            assert!(!is_force_advance_enabled(None));
            assert!(!is_force_advance_enabled(Some("local")));

            // With env var set to true in local, should return true
            std::env::set_var("FORCE_ADVANCE", "true");
            assert!(is_force_advance_enabled(None));
            assert!(is_force_advance_enabled(Some("local")));
            assert!(is_force_advance_enabled(Some("regtest")));

            // With env var set to true in production, should return false
            assert!(!is_force_advance_enabled(Some("alphanet")));
            assert!(!is_force_advance_enabled(Some("testnet")));

            // Clean up
            std::env::remove_var("FORCE_ADVANCE");
        }
    }

    #[test]
    fn test_force_dispute_respects_env_safety() {
        // SAFETY: These tests must run with --test-threads=1 to avoid race conditions
        // with environment variables. set_var/remove_var are unsafe in multi-threaded contexts.
        unsafe {
            // Clear any existing env var
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

            // Clean up
            std::env::remove_var("FORCE_DISPUTE");
        }
    }

    #[test]
    fn test_force_flags_accept_various_true_values() {
        // SAFETY: These tests must run with --test-threads=1 to avoid race conditions
        // with environment variables. set_var/remove_var are unsafe in multi-threaded contexts.
        unsafe {
            std::env::set_var("FORCE_ADVANCE", "true");
            assert!(is_force_advance_enabled(Some("local")));

            std::env::set_var("FORCE_ADVANCE", "TRUE");
            assert!(is_force_advance_enabled(Some("local")));

            std::env::set_var("FORCE_ADVANCE", "1");
            assert!(is_force_advance_enabled(Some("local")));

            std::env::set_var("FORCE_ADVANCE", "false");
            assert!(!is_force_advance_enabled(Some("local")));

            std::env::set_var("FORCE_ADVANCE", "0");
            assert!(!is_force_advance_enabled(Some("local")));

            // Clean up
            std::env::remove_var("FORCE_ADVANCE");
        }
    }
}
