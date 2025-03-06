use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct PeginAddressInput {
    pub rootstock_deposit_address: String, // TODO(iago) create/reuse custom type
    pub value: u64,
    pub btc_reimbursement_pub_key: String, // TODO(iago) create/reuse custom type
}

#[derive(Serialize, Debug)]
pub struct PeginAddressOutput {
    pub address: String, // TODO(iago) create/reuse custom type
}
