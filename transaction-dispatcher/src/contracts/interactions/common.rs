use crate::contracts::peg_manager;
use crate::rsk_gateway::PegManagerErrors;
use alloy_contract::Error::TransportError;

pub fn handle_contract_result<I, O>(
    result: alloy_contract::Result<I>,
    on_success: impl FnOnce(&I) -> O,
) -> Result<O, PegManagerErrors> {
    match result {
        Ok(r) => Ok(on_success(&r)),
        Err(TransportError(err)) => match err.as_error_resp() {
            Some(e) => Err(peg_manager::decode_contract_error(e)),
            None => Err(PegManagerErrors::NoRevertError(format!("{:?}", err))),
        },
        Err(e) => Err(PegManagerErrors::NoRevertError(format!("{:?}", e))),
    }
}
