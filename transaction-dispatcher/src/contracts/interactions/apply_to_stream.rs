use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::contracts::types::{Address, convert_to_member_registration_keys};
use crate::rsk_gateway::{BalanceProvider, DomainErrors};
use crate::types::{ApplyToStreamInput, ApplyToStreamOutput};

use anyhow::Result;
use log::{debug, error, info};
use union_contracts::bindings::committee_registry::CommitteeRegistry::StreamDenomination;

#[derive(Clone)]
pub(crate) struct ApplyToStreamInvoke<C: CommitteeRegistryContractApi, BP: BalanceProvider> {
    contract: C,
    gas_bumps: u8,
    balance_provider: BP,
    member_address: Address,
}

impl<C: CommitteeRegistryContractApi, BP: BalanceProvider> ApplyToStreamInvoke<C, BP> {
    pub(crate) fn new(
        contract: C,
        gas_bumps: u8,
        balance_provider: BP,
        member_address: Address,
    ) -> Self {
        ApplyToStreamInvoke {
            contract,
            gas_bumps,
            balance_provider,
            member_address,
        }
    }

    pub(crate) async fn run(
        &self,
        input: ApplyToStreamInput,
    ) -> Result<ApplyToStreamOutput, DomainErrors> {
        info!("Init ApplyToStream stream: {:?}", input);

        let member_balance = self
            .balance_provider
            .get_balance(self.member_address)
            .await
            .map_err(|e| DomainErrors::InternalServerError(e.to_string()))?;

        let min_deposit = self
            .contract
            .call_get_minimum_deposit(StreamDenomination::from(input.stream_id))
            .await?;

        if min_deposit > member_balance {
            return Err(DomainErrors::CommitteeError(
                "Member has not enough balance to apply to committee".to_string(),
            ));
        }

        let public_keys_regs = convert_to_member_registration_keys(
            &input.take_key,
            &input.dispute_key,
            &input.communication_key,
        )
        .map_err(|e| DomainErrors::InvalidPublicKey(format!("Invalid public key: {}", e)))?;

        debug!(
            "ApplyToStream with derived MemberRegistrationKeys {:?}",
            public_keys_regs
        );

        let receipt = self
            .contract
            .invoke_apply_to_stream(
                input.stream_id.into(),
                input.role.into(),
                public_keys_regs,
                input.funding_utxo,
                self.gas_bumps,
                min_deposit,
            )
            .await?;

        let result = match receipt.status() {
            true => {
                info!(
                    "ApplyToStream successful at tx {}",
                    receipt.transaction_hash
                );
                ApplyToStreamOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: true,
                }
            }
            false => {
                error!("ApplyToStream failed at tx {}", receipt.transaction_hash);
                ApplyToStreamOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                    success: false,
                }
            }
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::committee_registry::MockCommitteeRegistryContractApi;
    use crate::contracts::interactions::apply_to_stream::{
        ApplyToStreamInput, ApplyToStreamInvoke,
    };
    use crate::contracts::types::convert_to_member_registration_keys;
    use crate::rsk_gateway::{DomainErrors, MockBalanceProvider};
    use crate::types::{CommitteeECDSA, CommitteeRSA};
    use alloy_primitives::{Address, Bloom, TxHash, U256};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use mockall::predicate::eq;
    use std::str::FromStr;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        Role, StreamDenomination, UTXO,
    };

    impl ApplyToStreamInvoke<MockCommitteeRegistryContractApi, MockBalanceProvider> {
        pub(crate) fn new_for_tests(
            contract: MockCommitteeRegistryContractApi,
            balance_provider: MockBalanceProvider,
        ) -> Self {
            ApplyToStreamInvoke {
                contract,
                gas_bumps: 3,
                balance_provider,
                member_address: Address::from([0u8; 20]),
            }
        }
    }

    #[tokio::test]
    async fn test_apply_to_stream_success() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        let input = ApplyToStreamInput {
            stream_id: 123,
            role: 1,
            take_key: fake_take_key(),
            dispute_key: fake_dispute_key(),
            communication_key: fake_rsa_key(),
            funding_utxo: UTXO::default(),
        };

        // expect get_minimum_deposit to be called
        mock_instance
            .expect_call_get_minimum_deposit()
            .with(eq(StreamDenomination::from(input.stream_id)))
            .returning(|_| Ok(U256::from(100)))
            .times(1);

        // expect apply_to_stream_call to be called
        mock_instance
            .expect_invoke_apply_to_stream()
            .with(
                eq(StreamDenomination::from(input.stream_id)),
                eq(Role::from(input.role)),
                eq(convert_to_member_registration_keys(
                    &fake_take_key(),
                    &fake_dispute_key(),
                    &fake_rsa_key(),
                )
                .unwrap()),
                eq(UTXO::default()),
                eq(3u8),
                eq(U256::from(100)),
            )
            .returning(|_, _, _, _, _, _| {
                Ok(get_fake_receipt(
                    true,
                    "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                ))
            })
            .times(1);

        let mut mock_balance_provider = MockBalanceProvider::new();
        mock_balance_provider
            .expect_get_balance()
            .returning(|_| Ok(U256::from(1000))); // enough balance

        let interaction = ApplyToStreamInvoke::new_for_tests(mock_instance, mock_balance_provider);

        let result = interaction.run(input).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(
            output.transaction_hash,
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        );
        assert!(output.success);
    }

    #[tokio::test]
    async fn test_apply_to_stream_insufficient_balance() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        let input = ApplyToStreamInput {
            stream_id: 123,
            role: 1,
            take_key: fake_take_key(),
            dispute_key: fake_dispute_key(),
            communication_key: fake_rsa_key(),
            funding_utxo: UTXO::default(),
        };

        // expect get_minimum_deposit to be called
        mock_instance
            .expect_call_get_minimum_deposit()
            .with(eq(StreamDenomination::from(input.stream_id)))
            .returning(|_| Ok(U256::from(100)))
            .times(1);

        let mut mock_balance_provider = MockBalanceProvider::new();
        mock_balance_provider
            .expect_get_balance()
            .returning(|_| Ok(U256::from(50))); // low balance

        let interaction = ApplyToStreamInvoke {
            contract: mock_instance,
            gas_bumps: 3,
            balance_provider: mock_balance_provider,
            member_address: Address::from([0u8; 20]),
        };

        let result = interaction.run(input).await;

        assert!(result.is_err());
        if let Err(DomainErrors::CommitteeError(msg)) = result {
            assert_eq!(msg, "Member has not enough balance to apply to committee");
        } else {
            panic!("Expected DomainErrors::CommitteeError");
        }
    }

    fn get_fake_receipt(status: bool, hash: &str) -> TransactionReceipt<ReceiptEnvelope<Log>> {
        let receipt = Receipt {
            status: status.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        };

        let envelope = ReceiptEnvelope::Eip1559(ReceiptWithBloom {
            receipt,
            logs_bloom: Bloom::ZERO,
        });

        TransactionReceipt {
            inner: envelope,
            transaction_hash: TxHash::from_str(hash).expect("transaction hash is invalid"),
            transaction_index: Some(0),
            block_hash: None,
            block_number: None,
            gas_used: 21_000,
            effective_gas_price: 0,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::default(),
            to: Some(Address::default()),
            contract_address: None,
        }
    }

    fn fake_take_key() -> CommitteeECDSA {
        CommitteeECDSA {
            x: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
            y: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
            r: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
            s: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
            v: 27,
        }
    }

    fn fake_dispute_key() -> CommitteeECDSA {
        CommitteeECDSA {
            x: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
            y: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
            r: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
            s: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
            v: 28,
        }
    }

    fn fake_rsa_key() -> CommitteeRSA {
        "00c1e63f7b14e4e7a63b39f8445f9e30b4d6c92a08dc0240d49cf52c9a5d7f27f4b0a64226c04fbe3f63f6b0e9\
        a7050e4c7a16a8c929e04afefdf55b10903a0f8c15b6b04a78b1c255871a82ffbfe483dd2099f1b72013f5c6f66\
        f9e7d44c34c3b9f22b9bb09cc7a75e9eae1121f09e02b95ff9b12cfeb29f6f27bc2bcd43790c9c5896ac5947bb9\
        2c8b1587c4237edc42b8a0611ab6a2c62c44129c03b7b271e1a5c5e6b60c56c5f9308a5b4203d8f749fdb7c75e0\
        4b4dfd238a37e951bda7fa04b9e40f937cbfb72f83fc83a786c6d351b3a53d38fbdc721ff4dfc8a0a1a1143cf10\
        dfe8944acbb61d674370dd408e9189a9332d308f0c8438f1a94afcb92d"
            .to_string()
    }
}
