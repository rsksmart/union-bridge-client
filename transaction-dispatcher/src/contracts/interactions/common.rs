use crate::contracts::peg_manager;
use crate::rsk_gateway::PegManagerErrors;
use alloy_contract::Error::TransportError;

impl From<alloy_contract::Error> for PegManagerErrors {
    fn from(err: alloy_contract::Error) -> Self {
        match err {
            TransportError(err) => match err.as_error_resp() {
                Some(e) => peg_manager::decode_contract_error(e),
                None => PegManagerErrors::NoRevertError(format!("{:?}", err)),
            },
            e => PegManagerErrors::NoRevertError(format!("{:?}", e)),
        }
    }
}
