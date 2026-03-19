use anyhow::{Context, Result, bail};
use bitcoin::PublicKey;
use bitcoin::key::Parity::Even;
use bitcoin::secp256k1::XOnlyPublicKey;

pub struct ContractPubKeyParser;

impl ContractPubKeyParser {
    pub fn from_bytes(bytes: &[u8]) -> Result<PublicKey> {
        match bytes.len() {
            32 => {
                let x_only_key = XOnlyPublicKey::from_slice(bytes)
                    .context("Failed to parse x-only public key bytes")?;
                Ok(PublicKey::new(x_only_key.public_key(Even)))
            }
            33 => {
                PublicKey::from_slice(bytes).context("Failed to parse compressed public key bytes")
            }
            len => bail!("Unsupported bitcoin public key length: {len}"),
        }
    }

    pub fn from_hex(raw_hex: &str) -> Result<PublicKey> {
        let normalized = raw_hex.trim();
        let normalized = normalized
            .strip_prefix("0x")
            .or_else(|| normalized.strip_prefix("0X"))
            .unwrap_or(normalized);
        let key_bytes = hex::decode(normalized).context("Invalid bitcoin public key hex")?;

        Self::from_bytes(&key_bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::PublicKey;
    use bitcoin::secp256k1::{PublicKey as SecpPublicKey, Secp256k1, SecretKey};

    use super::ContractPubKeyParser;

    #[test]
    fn parses_xonly_pubkey_bytes() {
        let key = ContractPubKeyParser::from_bytes(
            &hex::decode(
                "79be667ef9dcbbac55a06295ce870b07029bfcd b2dce28d959f2815b16f81798"
                    .replace(' ', ""),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            key,
            PublicKey::from_str(
                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            )
            .unwrap()
        );
    }

    #[test]
    fn parses_compressed_pubkey_bytes() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[2_u8; 32]).unwrap();
        let expected = PublicKey::new(SecpPublicKey::from_secret_key(&secp, &secret_key));

        let parsed = ContractPubKeyParser::from_bytes(&expected.to_bytes()).unwrap();

        assert_eq!(parsed, expected);
    }

    #[test]
    fn parses_hex_with_prefix() {
        let parsed = ContractPubKeyParser::from_hex(
            "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .unwrap();

        assert_eq!(
            parsed,
            PublicKey::from_str(
                "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            )
            .unwrap()
        );
    }
}
