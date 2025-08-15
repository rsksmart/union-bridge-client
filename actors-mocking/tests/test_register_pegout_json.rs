#[cfg(test)]
mod tests {
    use bitcoin::Transaction;
    use serde::{Deserialize, Serialize};
    use std::fs;

    // Structure for RegisterPegout parameters from JSON file
    #[derive(Debug, Deserialize, Serialize)]
    struct RegisterPegoutParams {
        block_hash: String,
        btc_tx: Transaction,
        merkle_branch_path: String,
        merkle_branch_hashes: Vec<String>,
    }

    #[test]
    fn test_deserialize_register_pegout_params() {
        // Test that we can deserialize the example JSON file
        let json_str = fs::read_to_string("tests/resources/register_pegout_params_example.json")
            .expect("Failed to read example JSON file");

        let params: RegisterPegoutParams =
            serde_json::from_str(&json_str).expect("Failed to deserialize RegisterPegoutParams");

        // Verify the deserialized data
        assert_eq!(
            params.block_hash,
            "000000000000000000031234567890abcdef1234567890abcdef1234567890"
        );
        assert_eq!(params.merkle_branch_path, "0x1234");
        assert_eq!(params.merkle_branch_hashes.len(), 3);
        assert_eq!(
            params.merkle_branch_hashes[0],
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        );

        // Verify transaction structure
        assert_eq!(params.btc_tx.version.0, 2);
        assert_eq!(params.btc_tx.input.len(), 1);
        assert_eq!(params.btc_tx.output.len(), 2);
        assert_eq!(params.btc_tx.output[0].value.to_sat(), 99000);
        assert_eq!(params.btc_tx.output[1].value.to_sat(), 300);
    }

    #[test]
    fn test_serialize_register_pegout_params() {
        use bitcoin::absolute::LockTime;
        use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness};
        use std::str::FromStr;

        // Create a sample transaction
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::from_str(
                    "73d69e28cbe4ffc75b786b5dae8086a8112f6eb793d6891f2f900aac968a78ea:0",
                )
                .expect("Failed to parse outpoint"),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::from_consensus(4294967293),
                witness: Witness::new(),
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(99000),
                    script_pubkey: ScriptBuf::from_hex(
                        "00143fd2e14f4b448a071e074e1e1879318447f2a266",
                    )
                    .expect("Failed to parse script"),
                },
                TxOut {
                    value: Amount::from_sat(300),
                    script_pubkey: ScriptBuf::from_hex(
                        "0014298a0fe992f755152a81ee64bdc4cc96d3bb8969",
                    )
                    .expect("Failed to parse script"),
                },
            ],
        };

        let params = RegisterPegoutParams {
            block_hash: "000000000000000000031234567890abcdef1234567890abcdef1234567890"
                .to_string(),
            btc_tx: tx,
            merkle_branch_path: "0x1234".to_string(),
            merkle_branch_hashes: vec![
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
            ],
        };

        // Serialize to JSON
        let json_str = serde_json::to_string_pretty(&params)
            .expect("Failed to serialize RegisterPegoutParams");

        // Deserialize back and verify
        let deserialized: RegisterPegoutParams =
            serde_json::from_str(&json_str).expect("Failed to deserialize back");

        assert_eq!(deserialized.block_hash, params.block_hash);
        assert_eq!(deserialized.merkle_branch_path, params.merkle_branch_path);
        assert_eq!(
            deserialized.merkle_branch_hashes.len(),
            params.merkle_branch_hashes.len()
        );
    }
}
