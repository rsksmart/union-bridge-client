#[cfg(test)]
mod tests {
    use crate::flows::committee::setup_committee_flow::{StepData, Steps};
    use crate::user_requests::ApplyToStream;
    use bitcoin::PublicKey;
    use common::msg_broker::bitvmx_types::{P2PAddress, PeerId, SignedPublicKey};
    use common::types::StreamId;
    use std::str::FromStr;
    use uuid::Uuid;

    // Test helper functions
    fn create_test_p2p_address() -> P2PAddress {
        P2PAddress {
            address: "127.0.0.1:8080".to_string(),
            peer_id: PeerId("test_peer_id".to_string()),
        }
    }

    fn create_test_public_key() -> PublicKey {
        // Create a test public key using a known valid key
        PublicKey::from_str("02a1633cafcc01ebfb6d78e39f687a1f0995c62fc95f51ead10a02ee0be551b5dc")
            .unwrap()
    }

    fn create_test_signed_public_key() -> SignedPublicKey {
        SignedPublicKey {
            public_key: create_test_public_key(),
            signature_r: [1u8; 32],
            signature_s: [2u8; 32],
            recovery_id: 27,
        }
    }

    fn create_test_apply_to_stream() -> ApplyToStream {
        use crate::types::{Role, Utxo};
        ApplyToStream {
            stream_id: StreamId::from(1),
            role: Role::Prover,
            funding_utxo: Utxo { value: 100000 },
            speed_up_utxo: Utxo { value: 50000 },
        }
    }

    // Test StepData conversions
    #[test]
    fn test_step_data_into_user_input() {
        let apply_to_stream = create_test_apply_to_stream();
        let step_data = StepData::UserRequest(apply_to_stream.clone());

        let result = step_data.into_user_input().unwrap();
        assert_eq!(result.stream_id, apply_to_stream.stream_id);
        assert_eq!(result.role, apply_to_stream.role);
    }

    #[test]
    fn test_step_data_into_user_input_wrong_type() {
        let p2p_addr = create_test_p2p_address();
        let step_data = StepData::CommInfo(p2p_addr);

        let result = step_data.into_user_input();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Expected UserRequest data")
        );
    }

    #[test]
    fn test_step_data_into_p2p_address() {
        let p2p_addr = create_test_p2p_address();
        let step_data = StepData::CommInfo(p2p_addr.clone());

        let result = step_data.into_p2p_address().unwrap();
        assert_eq!(result.address, p2p_addr.address);
        assert_eq!(result.peer_id, p2p_addr.peer_id);
    }

    #[test]
    fn test_step_data_into_pubkey() {
        let pubkey = create_test_public_key();
        let step_data = StepData::PublicKey(pubkey);

        let result = step_data.into_pubkey().unwrap();
        assert_eq!(result, pubkey);
    }

    #[test]
    fn test_step_data_into_signed_payload() {
        let signature_r = [1u8; 32];
        let signature_s = [2u8; 32];
        let recovery_id = 27;
        let step_data = StepData::SignedMessage(signature_r, signature_s, recovery_id);

        let (r, s, rec_id) = step_data.into_signed_payload().unwrap();
        assert_eq!(r, signature_r);
        assert_eq!(s, signature_s);
        assert_eq!(rec_id, recovery_id);
    }

    #[test]
    fn test_step_data_into_signed_payload_wrong_type() {
        let pubkey = create_test_public_key();
        let step_data = StepData::PublicKey(pubkey);

        let result = step_data.into_signed_payload();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Expected SignedMessage data")
        );
    }

    // Test Steps enum
    #[test]
    fn test_steps_enum_values() {
        // Test that all expected steps are present
        assert_eq!(Steps::Init as u8, 0);
        assert_eq!(Steps::GetMyCommInfo as u8, 1);
        assert_eq!(Steps::GetMyTakeKey as u8, 2);
        assert_eq!(Steps::SignMyTakeKey as u8, 3);
        assert_eq!(Steps::GetMyDisputeKey as u8, 4);
        assert_eq!(Steps::SignMyDisputeKey as u8, 5);
        assert_eq!(Steps::GetMyCommKey as u8, 6);
        assert_eq!(Steps::SignMyCommKey as u8, 7);
        assert_eq!(Steps::ApplyToStream as u8, 8);
        assert_eq!(Steps::DepositP2PData as u8, 9);
        assert_eq!(Steps::SetupTakeAggregatedKey as u8, 10);
        assert_eq!(Steps::SetupDisputeAggregatedKey as u8, 11);
        assert_eq!(Steps::DepositAggregatedKey as u8, 12);
        assert_eq!(Steps::SetupDisputeCore as u8, 13);
        assert_eq!(Steps::Done as u8, 14);
    }

    #[test]
    fn test_steps_equality() {
        assert_eq!(Steps::Init, Steps::Init);
        assert_ne!(Steps::Init, Steps::Done);
    }

    #[test]
    fn test_steps_enum_ordering() {
        // Test that steps are in logical order
        assert!((Steps::Init as u8) < (Steps::GetMyCommInfo as u8));
        assert!((Steps::GetMyCommInfo as u8) < (Steps::GetMyTakeKey as u8));
        assert!((Steps::GetMyTakeKey as u8) < (Steps::SignMyTakeKey as u8));
        assert!((Steps::SignMyTakeKey as u8) < (Steps::GetMyDisputeKey as u8));
        assert!((Steps::GetMyDisputeKey as u8) < (Steps::SignMyDisputeKey as u8));
        assert!((Steps::SignMyDisputeKey as u8) < (Steps::GetMyCommKey as u8));
        assert!((Steps::GetMyCommKey as u8) < (Steps::SignMyCommKey as u8));
        assert!((Steps::SignMyCommKey as u8) < (Steps::ApplyToStream as u8));
        assert!((Steps::ApplyToStream as u8) < (Steps::DepositP2PData as u8));
        assert!((Steps::DepositP2PData as u8) < (Steps::SetupTakeAggregatedKey as u8));
        assert!((Steps::SetupTakeAggregatedKey as u8) < (Steps::SetupDisputeAggregatedKey as u8));
        assert!((Steps::SetupDisputeAggregatedKey as u8) < (Steps::DepositAggregatedKey as u8));
        assert!((Steps::DepositAggregatedKey as u8) < (Steps::SetupDisputeCore as u8));
        assert!((Steps::SetupDisputeCore as u8) < (Steps::Done as u8));
    }

    // Test StepData debug formatting
    #[test]
    fn test_step_data_debug_formatting() {
        let apply_to_stream = create_test_apply_to_stream();
        let step_data = StepData::UserRequest(apply_to_stream);
        let debug_str = format!("{:?}", step_data);
        assert!(debug_str.contains("UserRequest"));
    }

    // Test StepData clone
    #[test]
    fn test_step_data_clone() {
        let apply_to_stream = create_test_apply_to_stream();
        let step_data = StepData::UserRequest(apply_to_stream);
        let cloned = step_data.clone();

        // Test that clone works and produces equivalent data
        assert_eq!(
            step_data.into_user_input().unwrap().stream_id,
            cloned.into_user_input().unwrap().stream_id
        );
    }

    // Test helper functions
    #[test]
    fn test_create_pubkey_hash() {
        let pubkey = create_test_public_key();
        let hash = create_pubkey_hash(&pubkey).unwrap();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_construct_signed_pubkey() {
        let pubkey = create_test_public_key();
        let signature_r = [1u8; 32];
        let signature_s = [2u8; 32];
        let recovery_id = 27;

        let signed_pubkey = construct_signed_pubkey(pubkey, signature_r, signature_s, recovery_id);

        assert_eq!(signed_pubkey.public_key, pubkey);
        assert_eq!(signed_pubkey.signature_r, signature_r);
        assert_eq!(signed_pubkey.signature_s, signature_s);
        assert_eq!(signed_pubkey.recovery_id, recovery_id);
    }

    #[test]
    fn test_signed_to_committee_public_key() {
        let signed_pubkey = create_test_signed_public_key();
        let result = signed_to_committee_public_key(signed_pubkey).unwrap();

        assert_eq!(result.v, 27);
        assert!(!result.x.is_empty());
        assert!(!result.y.is_empty());
        assert!(!result.r.is_empty());
        assert!(!result.s.is_empty());
    }

    #[test]
    fn test_signed_to_committee_public_key_invalid_recovery_id() {
        let mut signed_pubkey = create_test_signed_public_key();
        signed_pubkey.recovery_id = 99; // Invalid recovery ID

        let result = signed_to_committee_public_key(signed_pubkey);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid recovery_id")
        );
    }

    // Test DisputeCoreSetup
    #[test]
    fn test_get_dispute_core_pid() {
        let committee_id = Uuid::new_v4();
        let pubkey = create_test_public_key();

        let protocol_id = get_dispute_core_pid(committee_id, &pubkey).unwrap();
        assert_ne!(protocol_id, committee_id);

        // Test that same inputs produce same output
        let protocol_id2 = get_dispute_core_pid(committee_id, &pubkey).unwrap();
        assert_eq!(protocol_id, protocol_id2);
    }

    #[test]
    fn test_get_dispute_core_pid_different_inputs() {
        let committee_id1 = Uuid::new_v4();
        let committee_id2 = Uuid::new_v4();
        let pubkey1 = create_test_public_key();
        let pubkey2 = create_test_public_key();

        let protocol_id1 = get_dispute_core_pid(committee_id1, &pubkey1).unwrap();
        let protocol_id2 = get_dispute_core_pid(committee_id2, &pubkey2).unwrap();

        assert_ne!(protocol_id1, protocol_id2);
    }

    // Test error handling
    #[test]
    fn test_step_data_conversion_errors() {
        let p2p_addr = create_test_p2p_address();

        // Test wrong type conversions - each test uses a fresh StepData
        let step_data1 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data1.into_user_input().is_err());

        let step_data2 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data2.into_pubkey().is_err());

        let step_data3 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data3.into_signed_payload().is_err());

        let step_data4 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data4.into_committee_pending().is_err());

        let step_data5 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data5.into_all_comm_data_ready().is_err());

        let step_data6 = StepData::CommInfo(p2p_addr.clone());
        assert!(step_data6.into_committee_ready().is_err());

        let step_data7 = StepData::CommInfo(p2p_addr);
        assert!(step_data7.into_setup_completed().is_err());
    }

    // Import existing helper functions instead of duplicating them
    use crate::flows::committee::dispute_core_setup::get_dispute_core_pid;
    use crate::flows::committee::setup_committee_flow::{
        construct_signed_pubkey, create_pubkey_hash, signed_to_committee_public_key,
    };
}
