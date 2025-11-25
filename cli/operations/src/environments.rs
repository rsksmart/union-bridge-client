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
                USER_API_PORTS.iter().map(|_| REGTEST_HOST.to_string()).collect()
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
            Environment::Alphanet => {
                "http://node-use1-1.alphanet.rskcomputing.net:4444".to_string()
            }
            Environment::Regtest => "http://node-use1-1.regtest.rskcomputing.net".to_string(),
            Environment::Testnet => "TBD".to_string(),
        }
    }

    /// returns the bitvmx endpoints for this environment
    pub fn user_api_endpoints(&self) -> Vec<String> {
        match self {
            Environment::Local | Environment::LocalDocker => USER_API_PORTS
                .iter()
                // same host, different port
                .map(|port| format!("{}:{}", LOCAL_HOST, port))
                .collect(),

            Environment::Regtest => USER_API_PORTS
                .iter()
                // same host, different port
                .map(|port| format!("{}:{}", REGTEST_HOST, port))
                .collect(),
            Environment::Alphanet => ALPHANET_HOSTS
                .iter()
                // different host, same port
                .map(|host| format!("{}:{}", host, USER_API_PORTS[0]))
                .collect(),

            Environment::Testnet => TESTNET_HOSTS
                .iter()
                // different host, same port
                .map(|host| format!("{}:{}", host, USER_API_PORTS[0]))
                .collect(),
        }
    }
}

const USER_API_PORTS: [u16; 4] = [40001, 40002, 40003, 40004];

const LOCAL_HOST: &str = "localhost";

const REGTEST_HOST: &str = "union-bridge-use2-1.regtest.rskcomputing.net";

const ALPHANET_HOSTS: [&str; 4] = [
    "union-bridge-use1-1.alphanet.rskcomputing.net",
    "union-bridge-use1-2.alphanet.rskcomputing.net",
    "union-bridge-use1-3.alphanet.rskcomputing.net",
    "union-bridge-use1-4.alphanet.rskcomputing.net",
];

const TESTNET_HOSTS: [&str; 4] = [
    "union-bridge-use1-1.TBD",
    "union-bridge-use1-2.TBD",
    "union-bridge-use1-3.TBD",
    "union-bridge-use1-4.TBD",
];
