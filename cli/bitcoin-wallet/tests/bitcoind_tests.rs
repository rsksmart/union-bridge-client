use bitcoin::Network;
use bollard::errors::Error;
use ub_wallet::bitcoin::bitcoind::{Bitcoind, BitcoindFlags, RpcConfig};

#[test]
#[ignore = "requires Docker daemon"]
fn test_start_stop_bitcoind() -> Result<(), Error> {
    let rpc_config = RpcConfig {
        username: "foo".to_string(),
        password: secrecy::SecretString::from("rpcpassword"),
        url: "http://localhost:18443".to_string(),
        wallet: "mywallet".to_string(),
        network: Network::Regtest,
    };

    let bitcoind = Bitcoind::new("bitcoin-regtest", "ruimarinho/bitcoin-core", rpc_config);

    bitcoind.start()?;
    bitcoind.stop()?;

    Ok(())
}

#[test]
#[ignore = "requires Docker daemon"]
fn test_start_stop_bitcoind_with_flags() -> Result<(), Error> {
    let rpc_config = RpcConfig {
        username: "foo".to_string(),
        password: secrecy::SecretString::from("rpcpassword"),
        url: "http://localhost:18443".to_string(),
        wallet: "mywallet".to_string(),
        network: Network::Regtest,
    };

    let flags = BitcoindFlags {
        min_relay_tx_fee: 0.00001,
        block_min_tx_fee: 0.00001,
        debug: 1,
        fallback_fee: 0.0002,
    };

    let bitcoind =
        Bitcoind::new_with_flags("bitcoin-regtest", "ruimarinho/bitcoin-core", rpc_config, flags);

    bitcoind.start()?;
    bitcoind.stop()?;

    Ok(())
}
