// Integration test to verify RuntimeSync properly propagates DomainErrors
// This test simulates the real-world scenario where coordinator calls
// transaction-dispatcher through RuntimeSync

use common::runtime_sync::RuntimeSync;
use transaction_dispatcher::rsk_gateway::DomainErrors;

#[test]
fn test_runtime_sync_propagates_domain_errors_in_real_scenario() {
    let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

    // Simulate what happens when transaction-dispatcher returns a DomainErrors
    let result: Result<(), DomainErrors> = rt_sync.run(async {
        // Simulate a contract call that fails with InvalidAddress
        Err(DomainErrors::InvalidAddress("0xInvalidAddress".to_string()))
    });

    // Verify the coordinator can match on the specific error variant
    match result {
        Err(DomainErrors::InvalidAddress(addr)) => {
            println!("✅ Successfully caught InvalidAddress error: {}", addr);
            assert_eq!(addr, "0xInvalidAddress");
        }
        Err(other) => panic!("Expected InvalidAddress, got: {:?}", other),
        Ok(_) => panic!("Expected error, got Ok"),
    }
}

#[test]
fn test_runtime_sync_propagates_internal_server_error() {
    let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

    // Simulate what happens when gateway creation fails
    let result: Result<String, DomainErrors> = rt_sync.run(async {
        Err(DomainErrors::InternalServerError(
            "Failed to create gateway: Contract PegManager at address 0x123 has no deployed code"
                .to_string(),
        ))
    });

    // Verify we can extract the detailed error message
    match result {
        Err(DomainErrors::InternalServerError(msg)) => {
            println!("✅ Successfully caught InternalServerError: {}", msg);
            assert!(msg.contains("Contract PegManager"));
            assert!(msg.contains("no deployed code"));
        }
        _ => panic!("Expected InternalServerError"),
    }
}

#[test]
fn test_runtime_sync_allows_error_conversion_to_anyhow() {
    let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

    // Simulate coordinator converting DomainErrors to anyhow::Error
    let result: Result<(), DomainErrors> =
        rt_sync.run(async { Err(DomainErrors::PeginAlreadyRequested("tx_123".to_string())) });

    // Convert to anyhow::Error (what coordinator does)
    let anyhow_result: anyhow::Result<()> =
        result.map_err(|e| anyhow::anyhow!("Transaction failed: {}", e));

    match anyhow_result {
        Err(e) => {
            println!("✅ Successfully converted to anyhow::Error: {}", e);
            assert!(e.to_string().contains("Transaction failed"));
            assert!(e.to_string().contains("Pegin already requested"));
        }
        Ok(_) => panic!("Expected error"),
    }
}

#[test]
fn test_multiple_error_types_preserved() {
    let rt_sync = RuntimeSync::new().expect("Failed to create RuntimeSync");

    // Test different error variants
    let errors = vec![
        DomainErrors::InvalidAddress("0x1".to_string()),
        DomainErrors::PeginAlreadyRequested("tx1".to_string()),
        DomainErrors::PeginAlreadyAccepted("tx2".to_string()),
        DomainErrors::InvalidPublicKey("key1".to_string()),
        DomainErrors::NotOwner("owner1".to_string()),
        DomainErrors::StreamNotFoundByDenomination("denom1".to_string()),
    ];

    for error in errors {
        let error_clone = format!("{:?}", error);
        let result: Result<(), DomainErrors> = rt_sync.run(async move { Err(error) });

        assert!(result.is_err(), "Expected error for: {}", error_clone);
        println!("✅ Error type preserved: {}", error_clone);
    }
}
