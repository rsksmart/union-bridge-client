use bitcoin::PublicKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::bitvmx_types::{
    FailConfiguration, ForceChallenge, ForceCondition, OutputType, PartialUtxo,
};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DisputeConfiguration {
    pub id: Uuid,
    pub operators_aggregated_pub: PublicKey,
    pub protocol_connection: (PartialUtxo, Vec<usize>),
    pub prover_actions: Vec<(PartialUtxo, Vec<usize>)>,
    pub prover_enablers: Vec<OutputType>,
    pub verifier_actions: Vec<(PartialUtxo, Vec<usize>)>,
    pub verifier_enablers: Vec<OutputType>,
    pub timelock_blocks: u16,
    pub program_definition: String,
    pub fail_force_config: Option<ConfigResults>,
    pub notify_protocol: Vec<(String, Uuid)>,
    pub auto_dispatch_input: Option<u8>,
}

impl DisputeConfiguration {
    pub const NAME: &'static str = "dispute_configuration";

    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        id: Uuid,
        operators_aggregated_pub: PublicKey,
        protocol_connection: (PartialUtxo, Vec<usize>),
        prover_actions: Vec<(PartialUtxo, Vec<usize>)>,
        prover_enablers: Vec<OutputType>,
        verifier_actions: Vec<(PartialUtxo, Vec<usize>)>,
        verifier_enablers: Vec<OutputType>,
        timelock_blocks: u16,
        program_definition: String,
        fail_force_config: Option<ConfigResults>,
        notify_protocol: Vec<(String, Uuid)>,
        auto_dispatch_input: Option<u8>,
    ) -> Self {
        Self {
            id,
            operators_aggregated_pub,
            protocol_connection,
            prover_actions,
            prover_enablers,
            verifier_actions,
            verifier_enablers,
            timelock_blocks,
            program_definition,
            fail_force_config,
            notify_protocol,
            auto_dispatch_input,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ConfigResult {
    pub fail_config_prover: Option<FailConfiguration>,
    pub fail_config_verifier: Option<FailConfiguration>,
    pub force_challenge: ForceChallenge,
    pub force_condition: ForceCondition,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ConfigResults {
    pub main: ConfigResult,
    pub read: ConfigResult, // for read challenge (2nd n-ary search)
}

impl Default for ConfigResult {
    fn default() -> Self {
        Self {
            fail_config_prover: None,
            fail_config_verifier: None,
            force_challenge: ForceChallenge::No,
            force_condition: ForceCondition::No,
        }
    }
}
