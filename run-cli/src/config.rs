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
}

impl Default for Environment {
    fn default() -> Self {
        Environment::Local
    }
}

impl Environment {
    /// returns the bitvmx endpoints for this environment
    pub fn user_api_endpoints(&self) -> Vec<String> {
        match self {
            Environment::Local | Environment::LocalDocker => LOCAL_USER_API_ENDPOINTS
                .iter()
                .map(|s| s.to_string())
                .collect(),

            Environment::Alphanet => ALPHANET_HOSTS
                .iter()
                .map(|host| format!("{}:40001", host))
                .collect(),
        }
    }
}

const LOCAL_USER_API_ENDPOINTS: [&'static str; 4] = [
    "localhost:40001",
    "localhost:40002",
    "localhost:40003",
    "localhost:40004",
];

pub const ALPHANET_HOSTS: [&str; 4] = [
    "union-bridge-use1-1.alphanet.rskcomputing.net",
    "union-bridge-use1-2.alphanet.rskcomputing.net",
    "union-bridge-use1-3.alphanet.rskcomputing.net",
    "union-bridge-use1-4.alphanet.rskcomputing.net",
];

// local docker compose project names
pub const ALL_OPS_COMPOSE_PROJECTS: [&str; 4] = ["op_1", "op_2", "op_3", "op_4"];

pub const ONE_OP_COMPOSE_PROJECT: &str = "union-operator";

// alphanet host addresses

// alphanet configuration
pub const SSH_USER: &str = "ubuntu";

// local anvil default address
pub const LOCAL_ANVIL_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

// default operator IDs for local deployments
pub const OPERATOR_IDS: [u8; 4] = [1, 2, 3, 4];
