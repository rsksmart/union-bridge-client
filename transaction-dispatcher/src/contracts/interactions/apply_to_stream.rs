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

        let stream_denomination: u8 = input
            .stream_id
            .as_u8()
            .map_err(|_| DomainErrors::InvalidValue("Invalid Stream denomination".to_string()))?;

        let min_deposit = self
            .contract
            .call_get_minimum_deposit(StreamDenomination::from(stream_denomination))
            .await?;

        if min_deposit > member_balance {
            return Err(DomainErrors::CommitteeError(
                "Member has not enough balance to apply to committee".to_string(),
            ));
        }

        let public_keys_regs = convert_to_member_registration_keys(
            &input.take_key,
            &input.dispute_key,
            &input.peer_id,
        )
        .map_err(|e| DomainErrors::InvalidPublicKey(format!("Invalid public key: {}", e)))?;

        debug!(
            "ApplyToStream with derived MemberRegistrationKeys {:?}",
            public_keys_regs
        );

        let receipt = self
            .contract
            .invoke_apply_to_stream(
                stream_denomination.into(),
                input.role.into(),
                public_keys_regs,
                input.funding_utxo,
                self.gas_bumps,
                min_deposit,
            )
            .await?;

        match receipt.status() {
            true => {
                info!(
                    "ApplyToStream successful at tx {}",
                    receipt.transaction_hash
                );
                Ok(ApplyToStreamOutput {
                    transaction_hash: receipt.transaction_hash.to_string(),
                })
            }
            false => {
                error!("ApplyToStream failed at tx {}", receipt.transaction_hash);
                Err(DomainErrors::TransactionFailed(format!(
                    "ApplyToStream transaction failed with receipt status false at tx {}",
                    receipt.transaction_hash
                )))
            }
        }
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
    use crate::types::CommitteeECDSA;
    use alloy_primitives::{Address, Bloom, TxHash, U256};
    use alloy_rpc_types::{Log, Receipt, ReceiptEnvelope, ReceiptWithBloom, TransactionReceipt};
    use common::msg_broker::bitvmx_types::PeerId;
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
            stream_id: 123.into(),
            role: 1,
            take_key: fake_take_key(),
            dispute_key: fake_dispute_key(),
            peer_id: fake_peer_id(),
            funding_utxo: UTXO::default(),
        };

        let stream_denomination = input.stream_id.as_u8().unwrap();

        // expect get_minimum_deposit to be called
        mock_instance
            .expect_call_get_minimum_deposit()
            .with(eq(StreamDenomination::from(stream_denomination)))
            .returning(|_| Ok(U256::from(100)))
            .times(1);

        let stream_denomination = input.stream_id.as_u8().unwrap();

        // expect apply_to_stream_call to be called
        mock_instance
            .expect_invoke_apply_to_stream()
            .with(
                eq(StreamDenomination::from(stream_denomination)),
                eq(Role::from(input.role)),
                eq(convert_to_member_registration_keys(
                    &fake_take_key(),
                    &fake_dispute_key(),
                    &fake_peer_id(),
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
    }

    #[tokio::test]
    async fn test_apply_to_stream_insufficient_balance() {
        let mut mock_instance = MockCommitteeRegistryContractApi::new();

        let input = ApplyToStreamInput {
            stream_id: 123.into(),
            role: 1,
            take_key: fake_take_key(),
            dispute_key: fake_dispute_key(),
            peer_id: fake_peer_id(),
            funding_utxo: UTXO::default(),
        };

        let stream_denomination = input.stream_id.clone().as_u8().unwrap();

        // contracts stores streamId as u64, but only accept u8 on StreamDenomination struct
        mock_instance
            .expect_call_get_minimum_deposit()
            .with(eq(StreamDenomination::from(stream_denomination)))
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

    fn fake_peer_id() -> PeerId {
        let x = "30820122300d06092a864886f70d01010105000382010f003082010a0282010100b0595a239c455f955ac2617061fadc0f3c532056da4a4ab4111b6581a62143e6c00b3041a00c290232fa65794ea0a55ca5f2ed3310ecbcab06a721d66e99a27e0d1b8a6afd8e395b741fbcf6cb73294eaeff43118f828f0118a4b5fdc95d472bcadaf2bc4d665e535ccd70b8ee5b82624794351a82c9f819d9a53638122228d1800d7d6561ae98183ae53c6cf23964c7eceeae95807db49a164cfbbc1ddc87a975fbe3d43545e8ce1bad2043cfe6a9aa3a7538ebdab8e6b900c94a691c1321d7c2d7f1a1beb3c3ef03686f7805ce938c92c8d5057cb5101cd51c1d97d7d3d4b9f13b7cb28bc5c4c5c9983a3062efc606b9c440021e1d5257d88d9c3ced0ac38f0203010001";
        PeerId(x.to_string())
    }
}
