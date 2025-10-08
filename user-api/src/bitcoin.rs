use anyhow::{Context, Result};
use bitcoin::{
    key::{CompressedPublicKey, Secp256k1},
    secp256k1::All,
    Address as BitcoinAddress, Network, PrivateKey, PublicKey,
};
use common::types::Address;
use log::info;

#[derive(Clone)]
pub struct User {
    pub public_key: PublicKey,
    pub bitcoin_address: BitcoinAddress,
    pub network: Network,
    pub secp: Secp256k1<All>,
    pub rsk_address: Address,
}

impl User {
    pub fn new(
        rsk_address: Address,
        wallet_private_key_wif: &str,
        network: Network,
    ) -> Result<Self> {
        let secp = Secp256k1::new();

        // Parse the WIF private key
        let private_key =
            PrivateKey::from_wif(wallet_private_key_wif).context("Invalid WIF private key")?;

        // Ensure the key is for the correct network
        if private_key.network != network.into() {
            anyhow::bail!(
                "Private key network mismatch: key is for {:?}, but network is {:?}",
                private_key.network,
                network
            );
        }

        let public_key = private_key.public_key(&secp);

        // Generate the Bitcoin address from the public key
        let compressed_pubkey = CompressedPublicKey::try_from(public_key)
            .context("Failed to get compressed public key")?;
        let bitcoin_address = BitcoinAddress::p2wpkh(&compressed_pubkey, network);

        info!("User Bitcoin Address: {:?}", bitcoin_address);
        info!("User RSK Address: {:?}", rsk_address);

        Ok(Self {
            public_key,
            bitcoin_address,
            network,
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
}
