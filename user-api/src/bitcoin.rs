use anyhow::{Context, Result};
use bitcoin::key::rand::rngs::OsRng;
use bitcoin::key::Parity;
use bitcoin::{
    absolute,
    key::Secp256k1,
    secp256k1::{self, All, Message, PublicKey as SecpPublicKey, SecretKey},
    sighash::SighashCache,
    transaction, Address as BitcoinAddress, Amount, Network, OutPoint, PrivateKey, PublicKey,
    ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
pub use bitvmx_bitcoin_rpc::bitcoin_client::{BitcoinClient, BitcoinClientApi};
use bitvmx_bitcoin_rpc::rpc_config::RpcConfig;
use common::msg_broker::bitvmx_types::PartialUtxo;
use common::types::Address;
use log::info;
use std::sync::Arc;

const REGTEST: Network = Network::Regtest;
const SPEEDUP_VALUE: u64 = 540;

#[derive(Clone)]
pub struct User {
    pub public_key: PublicKey,
    pub bitcoin_client: Arc<BitcoinClient>,
    pub bitcoin_address: BitcoinAddress,
    secret_key: SecretKey,
    pub network: Network,
    pub secp: Secp256k1<All>,
    pub rsk_address: Address,
}

impl User {
    pub fn new(rsk_address: Address, bitcoin_client: BitcoinClient) -> Result<Self> {
        let secp = Secp256k1::new();

        // For simplicity, we generate a new keypair for the user every time since this is a temporary approach
        let (user_address, user_pubkey, user_sk) =
            Self::emulated_user_keypair(&secp, &bitcoin_client, REGTEST)?;

        Ok(Self {
            bitcoin_client: Arc::new(bitcoin_client),
            public_key: user_pubkey,
            bitcoin_address: user_address,
            secret_key: user_sk,
            network: REGTEST,
            secp,
            rsk_address,
        })
    }

    pub fn get_rsk_address(&self) -> Address {
        self.rsk_address
    }


    pub fn public_key(&self) -> Result<PublicKey> {
        Ok(self.public_key)
    }


    fn dispatch_tx(&self, tx: Transaction) -> Result<Txid> {
        let txid = self
            .bitcoin_client
            .send_transaction(&tx)
            .map_err(|e| anyhow::anyhow!("Failed to dispatch transaction: {}", e))?;

        Ok(txid)
    }

    fn get_funding_utxo(&self, amount: u64) -> Result<PartialUtxo> {
        let (funding_tx, vout) = self
            .bitcoin_client
            .fund_address(&self.bitcoin_address, Amount::from_sat(amount))?;
        Ok((funding_tx.compute_txid(), vout, Some(amount), None))
    }


    pub fn create_and_dispatch_speedup(&self, tx_output: OutPoint, fee: u64) -> Result<()> {
        let speedup_tx = self.create_speedup_tx(tx_output, fee)?;

        let speedup_txid = self.dispatch_tx(speedup_tx)?;
        info!("Speedup transaction dispatched: {}", speedup_txid);
        Ok(())
    }

    pub fn create_speedup_tx(&self, tx_output: OutPoint, fee: u64) -> Result<Transaction> {
        let funding_utxo = self.get_funding_utxo(10_000_000)?;

        // Create two inputs: one from the funding utxo, one from the output to speed up
        let input_funding = TxIn {
            previous_output: OutPoint::new(funding_utxo.0, funding_utxo.1),
            script_sig: ScriptBuf::default(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(),
        };

        let input_speedup = TxIn {
            previous_output: tx_output,
            script_sig: ScriptBuf::default(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(),
        };

        // Output: all funds (minus fee) to user address
        let total_in = funding_utxo.2.context("Funding UTXO missing amount")?;
        let output = TxOut {
            value: Amount::from_sat(total_in - fee),
            script_pubkey: self.bitcoin_address.script_pubkey(),
        };

        let mut transaction = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![input_speedup, input_funding],
            output: vec![output],
        };

        // Sign the transaction (this may need to be adapted to sign both inputs)
        let signed_transaction = self.sign_p2wpkh_transaction(
            &mut transaction,
            [(0, SPEEDUP_VALUE), (1, total_in)].to_vec(),
        )?;
        info!(
            "Speeding up txid: {}. Speedup txid: {}",
            tx_output.txid,
            signed_transaction.compute_txid()
        );

        Ok(signed_transaction)
    }



    fn sign_p2wpkh_transaction(
        &self,
        transaction: &mut Transaction,
        index_amount: Vec<(usize, u64)>,
    ) -> Result<Transaction> {
        let user_bitcoin_privkey = PrivateKey {
            compressed: true,
            network: self.network.into(),
            inner: self.secret_key,
        };

        let user_comp_pubkey =
            bitcoin::CompressedPublicKey::from_private_key(&self.secp, &user_bitcoin_privkey)?;
        let uncompressed_pk = secp256k1::PublicKey::from_slice(&user_comp_pubkey.to_bytes())?;

        // Sign the transactions inputs
        let wpkh = self
            .public_key
            .wpubkey_hash()
            .context("key is compressed")?;
        let script_pubkey = ScriptBuf::new_p2wpkh(&wpkh);
        let mut sighasher = SighashCache::new(transaction);

        let sighash_type = bitcoin::EcdsaSighashType::All;
        for (input_index, value) in index_amount {
            let sighash = sighasher
                .p2wpkh_signature_hash(
                    input_index,
                    &script_pubkey,
                    Amount::from_sat(value),
                    sighash_type,
                )
                .context("failed to create rsk request pegin input sighash")?;

            let signature = bitcoin::ecdsa::Signature {
                signature: self
                    .secp
                    .sign_ecdsa(&Message::from(sighash), &self.secret_key),
                sighash_type,
            };

            *sighasher.witness_mut(input_index).context("No witness")? =
                Witness::p2wpkh(&signature, &uncompressed_pk);
        }

        // Now the transaction is signed
        let signed_transaction = sighasher.into_transaction().to_owned();
        Ok(signed_transaction)
    }

    // This method changes the parity of a keypair to be even, this is needed for Taproot.
    fn adjust_parity(
        secp: &Secp256k1<All>,
        pubkey: SecpPublicKey,
        seckey: SecretKey,
    ) -> (SecpPublicKey, SecretKey) {
        let (_, parity) = pubkey.x_only_public_key();

        if parity == Parity::Odd {
            (pubkey.negate(secp), seckey.negate())
        } else {
            (pubkey, seckey)
        }
    }

    pub fn emulated_user_keypair(
        secp: &Secp256k1<All>,
        bitcoin_client: &BitcoinClient,
        network: Network,
    ) -> Result<(bitcoin::Address, PublicKey, SecretKey)> {
        let mut rng = OsRng;

        // emulate the user keypair
        let user_sk = SecretKey::new(&mut rng);
        let user_pk = SecpPublicKey::from_secret_key(secp, &user_sk);
        let (user_pk, user_sk) = Self::adjust_parity(secp, user_pk, user_sk);
        let user_pubkey = PublicKey {
            compressed: true,
            inner: user_pk,
        };

        let user_address: bitcoin::Address = bitcoin_client.get_new_address(user_pubkey, network);
        info!(
            "User Address({}): {:?}",
            user_address.address_type().context("No address type")?,
            user_address
        );
        Ok((user_address, user_pubkey, user_sk))
    }
}

// this is temporary for Regtest stage, required for utxo generation
pub fn build_bitcoin_client_regtest() -> BitcoinClient {
    let config_bitcoin_client = RpcConfig::new(
        REGTEST,
        "http://127.0.0.1:18443".to_string(),
        "foo".to_string(),
        "rpcpassword".to_string(),
        "test_wallet".to_string(),
    );

    let bitcoin_client = BitcoinClient::new_from_config(&config_bitcoin_client)
        .expect("Cannot create Setup Committee Flow without a Bitcoin Client");

    bitcoin_client
}




