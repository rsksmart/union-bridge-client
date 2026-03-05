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
            Environment::Alphanet => ALPHANET_HOSTS.iter().map(|&s| s.to_string()).collect(),
            Environment::Testnet => TESTNET_HOSTS.iter().map(|&s| s.to_string()).collect(),
            Environment::Regtest => {
                // same host for each member (each user api port)
                operator_ids().iter().map(|_| REGTEST_HOST.to_string()).collect()
            }
            Environment::Local | Environment::LocalDocker => {
                unreachable!("hosts() only called for remote environments")
            }
        }
    }

    /// returns the RPC URL for this environment
    pub fn rpc_url(&self) -> String {
        match self {
            Environment::Local | Environment::LocalDocker => "http://localhost:8545".to_string(),
            Environment::Regtest => "http://node-use2-1.regtest.rskcomputing.net".to_string(),
            Environment::Alphanet => {
                "http://node-use1-1.alphanet.rskcomputing.net:4444".to_string()
            }
            Environment::Testnet => "http://rskj-01.testnet.ub.iovlabs.net:4444".to_string(),
        }
    }

    /// returns the StreamManager contract address for this environment
    pub fn stream_manager_address(&self) -> &str {
        match self {
            Environment::Local | Environment::LocalDocker => {
                "0x610178dA211FEF7D417bC0e6FeD39F05609AD788"
            }
            Environment::Regtest => "0xC689FB08B10BE1080c87dC6f8EdcA7Cd20471218",
            Environment::Alphanet => "0x71260e163CE80566e82B26934Ea98A1C194340d3",
            Environment::Testnet => "0x38C5d2e8a3fB87B851ab7b501a317E48fc5bFa7F",
        }
    }

    /// returns the bitvmx endpoints for this environment
    pub fn user_api_endpoints(&self) -> Vec<String> {
        let ports = user_api_ports();
        match self {
            Environment::Local | Environment::LocalDocker => ports
                .iter()
                .map(|port| format!("{}:{}", LOCAL_HOST, port))
                .collect(),

            Environment::Regtest => ports
                .iter()
                .map(|port| format!("{}:{}", REGTEST_HOST, port))
                .collect(),
            Environment::Alphanet => ALPHANET_HOSTS
                .iter()
                .map(|host| format!("{}:{}", host, BASE_USER_API_PORT))
                .collect(),

            Environment::Testnet => TESTNET_HOSTS
                .iter()
                .map(|host| format!("{}:{}", host, BASE_USER_API_PORT))
                .collect(),
        }
    }
}

const BASE_USER_API_PORT: u16 = 40001;

fn user_api_ports() -> Vec<u16> {
    operator_ids()
        .iter()
        .map(|&id| BASE_USER_API_PORT + (id as u16) - 1)
        .collect()
}

const LOCAL_HOST: &str = "localhost";

const REGTEST_HOST: &str = "union-bridge-use2-1.regtest.rskcomputing.net";

const ALPHANET_HOSTS: [&str; 10] = [
    "union-bridge-use1-1.alphanet.rskcomputing.net",
    "union-bridge-use1-2.alphanet.rskcomputing.net",
    "union-bridge-use1-3.alphanet.rskcomputing.net",
    "union-bridge-use1-4.alphanet.rskcomputing.net",
    "union-bridge-use1-5.alphanet.rskcomputing.net",
    "union-bridge-use1-6.alphanet.rskcomputing.net",
    "union-bridge-use1-7.alphanet.rskcomputing.net",
    "union-bridge-use1-8.alphanet.rskcomputing.net",
    "union-bridge-use1-9.alphanet.rskcomputing.net",
    "union-bridge-use1-10.alphanet.rskcomputing.net",
];

const TESTNET_HOSTS: [&str; 4] = [
    "operator-01.testnet.ub.iovlabs.net",
    "operator-02.testnet.ub.iovlabs.net",
    "operator-03.testnet.ub.iovlabs.net",
    "operator-04.testnet.ub.iovlabs.net",
];
