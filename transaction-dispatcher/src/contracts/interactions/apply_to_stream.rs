use crate::contracts::committee_registry::CommitteeRegistryContractApi;
use crate::contracts::types::Address;
use crate::rsk_gateway::{BalanceProvider, DomainErrors};
use crate::types::{ApplyToStreamInput, ApplyToStreamOutput};

use anyhow::Result;
use log::{debug, error, info};
use union_contracts::bindings::committee_registry::CommitteeRegistry::{
    PublicKeyRegistration, StreamDenomination,
};

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

        let public_keys_regs = input
            .public_keys
            .iter()
            .cloned()
            .map(|key| {
                PublicKeyRegistration::try_from(key).map_err(|e| {
                    DomainErrors::InvalidPublicKey(format!("Invalid public key: {}", e))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        debug!(
            "ApplyToStream with derived PublicKeyRegistrations {:?}",
            public_keys_regs
        );

        let receipt = self
            .contract
            .invoke_apply_to_stream(
                input.stream_id,
                input.role,
                public_keys_regs,
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
    use crate::rsk_gateway::{DomainErrors, MockBalanceProvider};
    use crate::types::CommitteePublicKey;
    use alloy_primitives::{Address, Bloom, TxHash, U256};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use mockall::predicate::eq;
    use std::str::FromStr;
    use union_contracts::bindings::committee_registry::CommitteeRegistry::{
        PublicKeyRegistration, StreamDenomination,
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
            public_keys: fake_pub_keys(),
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
                eq(input.stream_id),
                eq(input.role),
                eq(fake_pub_keys()
                    .iter()
                    .cloned()
                    .map(PublicKeyRegistration::try_from)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()),
                eq(3u8),
                eq(U256::from(100)),
            )
            .returning(|_, _, _, _, _| {
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
            public_keys: fake_pub_keys(),
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

    fn fake_pub_keys() -> Vec<CommitteePublicKey> {
        [
            CommitteePublicKey {
                x: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
                y: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
                r: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
                s: "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".to_string(),
                v: 27,
            },
            CommitteePublicKey {
                x: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
                y: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
                r: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
                s: "0x0f0e0d0c0b0a09080706050403020100fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0".to_string(),
                v: 28,
            },
            CommitteePublicKey {
                x: "0xff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00".to_string(),
                y: "0xff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00".to_string(),
                r: "0xff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00".to_string(),
                s: "0xff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00".to_string(),
                v: 28,
            },
        ]
        .to_vec()
    }
}
