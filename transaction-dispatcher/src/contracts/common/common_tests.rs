use crate::contracts::common::{bumped_gas, likely_oog};
use alloy_primitives::{Bloom, TxHash};
use alloy_rpc_types::{Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};

// Helper function to create a fake receipt for testing
fn create_fake_receipt(status: bool, gas_used: u64, _gas_limit: u64) -> TransactionReceipt {
    let receipt = Receipt {
        status: status.into(),
        cumulative_gas_used: gas_used,
        logs: vec![],
    };
    let envelope = ReceiptEnvelope::Eip1559(ReceiptWithBloom {
        receipt,
        logs_bloom: Bloom::ZERO,
    });
    TransactionReceipt {
        inner: envelope,
        transaction_hash: TxHash::from([1u8; 32]),
        transaction_index: Some(0),
        block_hash: None,
        block_number: None,
        gas_used,
        effective_gas_price: 1000000000,
        blob_gas_used: None,
        blob_gas_price: None,
        from: alloy_primitives::Address::from([3u8; 20]),
        to: Some(alloy_primitives::Address::from([4u8; 20])),
        contract_address: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bumped_gas_base_case() {
        // Test base case with no attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 0);

        // Should be 120% of estimated (base headroom)
        assert_eq!(result, 120000);
    }

    #[test]
    fn test_bumped_gas_with_attempts() {
        // Test with 1 attempt (10% bump on top of 20% base)
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 1);

        // Base: 100000 * 1.20 = 120000
        // Attempt 1: 120000 * 1.10 = 132000
        assert_eq!(result, 132000);
    }

    #[test]
    fn test_bumped_gas_multiple_attempts() {
        // Test with 3 attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, 3);

        // Base: 100000 * 1.20 = 120000
        // Attempt 1: 120000 * 1.10 = 132000
        // Attempt 2: 132000 * 1.10 = 145200
        // Attempt 3: 145200 * 1.10 = 159720
        assert_eq!(result, 159720);
    }

    #[test]
    fn test_bumped_gas_never_below_estimated() {
        // Test edge case where calculation might go below estimated
        let estimated = 1000u64;
        let result = bumped_gas(estimated, 0);

        // Should never be below estimated
        assert!(result >= estimated);
    }

    #[test]
    fn test_bumped_gas_overflow_protection() {
        // Test with very large numbers to ensure no overflow
        let estimated = u64::MAX;
        let result = bumped_gas(estimated, 10);

        // Should not panic and should handle overflow gracefully
        assert!(result >= estimated);
    }

    #[test]
    fn test_likely_oog_success() {
        let gas_limit = 200000u64;
        let gas_used = 150000u64; // Well below limit
        let receipt = create_fake_receipt(true, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    #[test]
    fn test_likely_oog_failure() {
        let gas_limit = 200000u64;
        let gas_used = 195000u64; // Close to limit (within 5% margin)
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(result); // Should be OOG
    }

    #[test]
    fn test_likely_oog_margin_calculation() {
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000
        let gas_used = gas_limit - oog_margin + 1; // Just above margin
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(result); // Should be OOG
    }

    #[test]
    fn test_likely_oog_below_margin() {
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000
        let gas_used = gas_limit - oog_margin - 1; // Just below margin
        let receipt = create_fake_receipt(false, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    #[test]
    fn test_likely_oog_successful_transaction() {
        // Successful transactions should never be considered OOG
        let gas_limit = 200000u64;
        let gas_used = 200000u64; // Used all gas but succeeded
        let receipt = create_fake_receipt(true, gas_used, gas_limit);

        let result = likely_oog(&receipt, gas_limit);
        assert!(!result); // Should not be OOG
    }

    // Integration tests for send_tx_with_gas_bump
    // Note: These are more complex and would require a full mock implementation
    // For now, we'll focus on unit tests for the helper functions

    #[test]
    fn test_gas_bump_math_consistency() {
        // Test that gas bumping math is consistent across different scenarios
        let test_cases = vec![
            (1000u64, 0u8, 1200u64),
            (1000u64, 1u8, 1320u64),
            (1000u64, 2u8, 1452u64),
            (1000u64, 3u8, 1597u64),
            (50000u64, 0u8, 60000u64),
            (50000u64, 1u8, 66000u64),
        ];

        for (estimated, attempts, expected) in test_cases {
            let result = bumped_gas(estimated, attempts);
            assert_eq!(
                result, expected,
                "Failed for estimated={}, attempts={}",
                estimated, attempts
            );
        }
    }

    #[test]
    fn test_edge_case_zero_estimated() {
        // Test edge case with zero estimated gas
        let result = bumped_gas(0, 0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_edge_case_max_attempts() {
        // Test with maximum u8 attempts
        let estimated = 100000u64;
        let result = bumped_gas(estimated, u8::MAX);

        // Should not panic and should be >= estimated
        assert!(result >= estimated);
    }

    #[test]
    fn test_oog_detection_edge_cases() {
        // Test OOG detection with edge cases
        let gas_limit = 100000u64;

        // Exactly at the margin
        let gas_used = gas_limit - (gas_limit / 20);
        let receipt = create_fake_receipt(false, gas_used, gas_limit);
        assert!(likely_oog(&receipt, gas_limit));

        // Just below the margin
        let gas_used = gas_limit - (gas_limit / 20) - 1;
        let receipt = create_fake_receipt(false, gas_used, gas_limit);
        assert!(!likely_oog(&receipt, gas_limit));
    }

    // Integration tests for send_tx_with_gas_bump
    // Note: These tests focus on the mathematical correctness of the gas bumping logic
    // Full integration tests would require complex mocking of the Provider and SolCallBuilder

    #[test]
    fn test_gas_bump_algorithm_correctness() {
        // Test that the gas bumping algorithm produces expected results
        let test_cases = vec![
            // (estimated, attempt, expected)
            (100000u64, 0u8, 120000u64), // Base case: 20% headroom
            (100000u64, 1u8, 132000u64), // First retry: 20% + 10% = 32%
            (100000u64, 2u8, 145200u64), // Second retry: 20% + 10% + 10% = 45.2%
            (100000u64, 3u8, 159720u64), // Third retry: 20% + 10% + 10% + 10% = 59.72%
            (50000u64, 0u8, 60000u64),   // Different base amount
            (50000u64, 1u8, 66000u64),   // Different base amount with retry
        ];

        for (estimated, attempt, expected) in test_cases {
            let result = bumped_gas(estimated, attempt);
            assert_eq!(
                result, expected,
                "Gas bump failed for estimated={}, attempt={}, expected={}, got={}",
                estimated, attempt, expected, result
            );
        }
    }

    #[test]
    fn test_gas_bump_never_below_estimated() {
        // Ensure gas bump never goes below estimated, even with edge cases
        let test_cases = vec![
            (0u64, 0u8),
            (1u64, 0u8),
            (1u64, 1u8),
            (1u64, 10u8),
            (u64::MAX, 0u8),
            (u64::MAX, 1u8),
        ];

        for (estimated, attempt) in test_cases {
            let result = bumped_gas(estimated, attempt);
            assert!(
                result >= estimated,
                "Gas bump went below estimated: estimated={}, attempt={}, result={}",
                estimated,
                attempt,
                result
            );
        }
    }

    #[test]
    fn test_oog_detection_accuracy() {
        // Test OOG detection accuracy with various scenarios
        let gas_limit = 200000u64;
        let oog_margin = gas_limit / 20; // 5% margin = 10000

        // Test cases: (gas_used, status, expected_oog)
        let test_cases = vec![
            // Successful transactions should never be OOG
            (gas_limit, true, false),
            (gas_limit - 1, true, false),
            (gas_limit - oog_margin, true, false),
            // Failed transactions near the limit should be OOG
            (gas_limit, false, true),
            (gas_limit - 1, false, true),
            (gas_limit - oog_margin + 1, false, true),
            // Failed transactions well below limit should not be OOG
            (gas_limit - oog_margin - 1, false, false),
            (gas_limit / 2, false, false),
        ];

        for (gas_used, status, expected_oog) in test_cases {
            let receipt = create_fake_receipt(status, gas_used, gas_limit);
            let result = likely_oog(&receipt, gas_limit);
            assert_eq!(
                result, expected_oog,
                "OOG detection failed: gas_used={}, status={}, expected={}, got={}",
                gas_used, status, expected_oog, result
            );
        }
    }

    #[test]
    fn test_gas_bump_overflow_protection() {
        // Test that gas bumping handles overflow gracefully
        let large_estimated = u64::MAX / 2; // Large but not max to allow for multiplication

        // Test with various attempt counts
        for attempt in 0..=10 {
            let result = bumped_gas(large_estimated, attempt);
            // Should not panic and should be >= estimated
            assert!(
                result >= large_estimated,
                "Gas bump overflow protection failed: estimated={}, attempt={}, result={}",
                large_estimated,
                attempt,
                result
            );
        }
    }

    #[test]
    fn test_gas_bump_mathematical_properties() {
        // Test mathematical properties of gas bumping
        let base_estimated = 100000u64;

        // Property 1: Each attempt should increase gas (when not at overflow)
        let mut prev_gas = bumped_gas(base_estimated, 0);
        for attempt in 1..=5 {
            let current_gas = bumped_gas(base_estimated, attempt);
            assert!(
                current_gas >= prev_gas,
                "Gas should not decrease between attempts: attempt={}, prev={}, current={}",
                attempt,
                prev_gas,
                current_gas
            );
            prev_gas = current_gas;
        }

        // Property 2: Gas bump should be monotonically increasing with attempts
        for attempt in 0..=10 {
            let gas = bumped_gas(base_estimated, attempt);
            assert!(
                gas >= base_estimated,
                "Gas should never be below estimated: attempt={}, estimated={}, gas={}",
                attempt,
                base_estimated,
                gas
            );
        }
    }
}
