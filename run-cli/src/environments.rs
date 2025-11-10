use clap::ValueEnum;

/// unified environment enum for all cli commands
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum Environment {
    /// local cargo-run services (no docker)
    Local,
    /// local docker compose services
    LocalDocker,
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
            Environment::Alphanet => "alphanet".to_string(),
            Environment::Testnet => "testnet".to_string(),
        }
    }

    /// returns the remote hosts for AWS-based environments (Alphanet, Testnet)
    pub fn hosts(&self) -> Vec<String> {
        match self {
            Environment::Alphanet => ALPHANET_HOSTS.iter().map(|&s| s.to_string()).collect(),
            Environment::Testnet => TESTNET_HOSTS.iter().map(|&s| s.to_string()).collect(),
            Environment::Local | Environment::LocalDocker => {
                unreachable!("hosts() only called for AWS environments")
            }
        }
    }

    /// returns the RPC URL for this environment
    pub fn rpc_url(&self) -> String {
        match self {
            Environment::Local | Environment::LocalDocker => "http://localhost:4444".to_string(),
            Environment::Alphanet => "http://node-use1-1.alphanet.rskcomputing.net".to_string(),
            Environment::Testnet => "TBD".to_string(),
        }
    }

    /// returns the bitvmx endpoints for this environment
    pub fn user_api_endpoints(&self) -> Vec<String> {
        match self {
            Environment::Local | Environment::LocalDocker => LOCAL_USER_API_ENDPOINTS
                .iter()
                .map(|s| s.to_string())
                .collect(),

            Environment::Alphanet => ALPHANET_HOSTS
                .iter()
                .map(|host| format!("{}:{}", host, DEFAULT_USER_API_PORT))
                .collect(),

            Environment::Testnet => TESTNET_HOSTS
                .iter()
                .map(|host| format!("{}:{}", host, DEFAULT_USER_API_PORT))
                .collect(),
        }
    }
}

const DEFAULT_USER_API_PORT: u16 = 40001;

const LOCAL_USER_API_ENDPOINTS: [&'static str; 4] = [
    "localhost:40001",
    "localhost:40002",
    "localhost:40003",
    "localhost:40004",
];

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
