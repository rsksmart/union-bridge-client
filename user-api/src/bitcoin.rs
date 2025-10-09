use anyhow::{Context, Result};
use bitcoin::{
    key::{CompressedPublicKey, Secp256k1},
    Address as BitcoinAddress, Network, PrivateKey, PublicKey,
};
use common::types::Address;
use log::info;

#[derive(Clone)]
pub struct User {
    pub bitcoin_public_key: PublicKey,
    pub bitcoin_address: BitcoinAddress,
    pub rsk_address: Address,
    pub bitcoin_network: Network,
}

impl User {
    pub fn new(
        rsk_address: Address,
        bitcoin_wallet_private_wif: &str,
        bitcoin_network: Network,
    ) -> Result<Self> {
        let secp = Secp256k1::new();

        // Parse the WIF private key
        let private_key =
            PrivateKey::from_wif(bitcoin_wallet_private_wif).context("Invalid WIF private key")?;

        // Ensure the key is for the correct network
        if private_key.network != bitcoin_network.into() {
            anyhow::bail!(
                "Private key network mismatch: key is for {:?}, but network is {:?}",
                private_key.network,
                bitcoin_network
            );
        }

        let bitcoin_public_key = private_key.public_key(&secp);

        // Generate the Bitcoin address from the public key
        let compressed_pubkey = CompressedPublicKey::try_from(bitcoin_public_key)
            .context("Failed to get compressed public key")?;
        let bitcoin_address = BitcoinAddress::p2wpkh(&compressed_pubkey, bitcoin_network);

        info!("User Bitcoin Address: {:?}", bitcoin_address);
        info!("User RSK Address: {:?}", rsk_address);

        Ok(Self {
            bitcoin_public_key,
            bitcoin_address,
            bitcoin_network,
            rsk_address,
        })
    }
}
