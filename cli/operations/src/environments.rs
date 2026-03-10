use crate::constants::operator_ids;
use clap::ValueEnum;

/// unified environment enum for all cli commands
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum Environment {
    /// local cargo-run services (no docker)
    Local,
    /// local docker compose services
    LocalDocker,
    /// regtest environment (closer to alphanet in protection measurements)
    Regtest,
    /// remote alphanet deployment
    Alphanet,
    /// remote testnet deployment
    Testnet,
}

impl Default for Environment {
    fn default() -> Self {
        Environment::Local
    }
}

impl Environment {
    /// returns the name of the environment as a string
    pub fn get_name(&self) -> String {
        match self {
            Environment::Local => "local".to_string(),
            Environment::LocalDocker => "local-docker".to_string(),
            Environment::Regtest => "regtest".to_string(),
            Environment::Alphanet => "alphanet".to_string(),
            Environment::Testnet => "testnet".to_string(),
        }
    }

    /// returns true if this is a remote environment (alphanet, testnet, or regtest)
    pub fn is_remote(&self) -> bool {
        matches!(self, Environment::Alphanet | Environment::Testnet | Environment::Regtest)
    }

    /// returns the remote hosts for remote environments (Alphanet, Testnet, Regtest)
    pub fn hosts(&self) -> Vec<String> {
        match self {
            Environment::Alphanet => alphanet_hosts(),
            Environment::Testnet => testnet_hosts(),
            Environment::Regtest => {
                let host = regtest_host();
                operator_ids().iter().map(|_| host.clone()).collect()
            }
            Environment::Local | Environment::LocalDocker => {
                unreachable!("hosts() only called for remote environments")
            }
        }
    }

    /// returns the RPC URL for this environment
    pub fn rpc_url(&self) -> String {
        match self {
            Environment::Local | Environment::LocalDocker => {
                env_or("UC_LOCAL_RPC_URL", "http://localhost:8545")
            }
            Environment::Regtest => env_or("UC_REGTEST_RPC_URL", "<REGTEST_RPC_URL>"),
            Environment::Alphanet => env_or("UC_ALPHANET_RPC_URL", "<ALPHANET_RPC_URL>"),
            Environment::Testnet => env_or("UC_TESTNET_RPC_URL", "<TESTNET_RPC_URL>"),
        }
    }

    /// returns the `StreamManager` contract address for this environment
    pub fn stream_manager_address(&self) -> String {
        match self {
            Environment::Local | Environment::LocalDocker => env_or(
                "UC_LOCAL_STREAM_MANAGER",
                "0x610178dA211FEF7D417bC0e6FeD39F05609AD788",
            ),
            Environment::Regtest => env_or("UC_REGTEST_STREAM_MANAGER", "<STREAM_MANAGER_ADDRESS>"),
            Environment::Alphanet => {
                env_or("UC_ALPHANET_STREAM_MANAGER", "<STREAM_MANAGER_ADDRESS>")
            }
            Environment::Testnet => {
                env_or("UC_TESTNET_STREAM_MANAGER", "<STREAM_MANAGER_ADDRESS>")
            }
        }
    }

    /// returns the user-api endpoints for this environment
    pub fn user_api_endpoints(&self) -> Vec<String> {
        let ports = user_api_ports();
        match self {
            Environment::Local | Environment::LocalDocker => ports
                .iter()
                .map(|port| format!("{}:{}", LOCAL_HOST, port))
                .collect(),

            Environment::Regtest => {
                let host = regtest_host();
                ports
                    .iter()
                    .map(|port| format!("{host}:{port}"))
                    .collect()
            }
            Environment::Alphanet => alphanet_hosts()
                .into_iter()
                .map(|host| format!("{host}:{BASE_USER_API_PORT}"))
                .collect(),

            Environment::Testnet => testnet_hosts()
                .into_iter()
                .map(|host| format!("{host}:{BASE_USER_API_PORT}"))
                .collect(),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn csv_env_or(key: &str, defaults: &[&str]) -> Vec<String> {
    std::env::var(key).map_or_else(
        |_| defaults.iter().map(|&s| s.to_string()).collect(),
        |val| val.split(',').map(|s| s.trim().to_string()).collect(),
    )
}

const BASE_USER_API_PORT: u16 = 40001;

fn user_api_ports() -> Vec<u16> {
    operator_ids()
        .iter()
        .map(|&id| BASE_USER_API_PORT + (id as u16) - 1)
        .collect()
}

const LOCAL_HOST: &str = "localhost";

fn regtest_host() -> String {
    env_or("UC_REGTEST_HOST", "<REGTEST_HOST>")
}

fn alphanet_hosts() -> Vec<String> {
    csv_env_or("UC_ALPHANET_HOSTS", &[
        "<ALPHANET_HOST_1>",
        "<ALPHANET_HOST_2>",
        "<ALPHANET_HOST_3>",
        "<ALPHANET_HOST_4>",
        "<ALPHANET_HOST_5>",
        "<ALPHANET_HOST_6>",
        "<ALPHANET_HOST_7>",
        "<ALPHANET_HOST_8>",
        "<ALPHANET_HOST_9>",
        "<ALPHANET_HOST_10>",
    ])
}

fn testnet_hosts() -> Vec<String> {
    csv_env_or("UC_TESTNET_HOSTS", &[
        "<TESTNET_HOST_1>",
        "<TESTNET_HOST_2>",
        "<TESTNET_HOST_3>",
        "<TESTNET_HOST_4>",
    ])
}
