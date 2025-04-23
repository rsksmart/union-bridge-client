use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct BitcoinTransaction {
    pub(crate) version: u32,
    pub(crate) inputs: Vec<BitcoinTransactionIn>,
    pub(crate) outputs: Vec<BitcoinTransactionOut>,
    pub(crate) lock_time: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct BitcoinTransactionIn {
    pub(crate) tx_id: String,
    pub(crate) v_out: u32,
    pub(crate) sequence: u32,
    pub(crate) script_sig: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct BitcoinTransactionOut {
    pub(crate) amount: u64,
    pub(crate) script_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PegInAddressInput {
    pub(crate) rootstock_deposit_address: String,
    pub(crate) value: u64,
    pub(crate) btc_reimbursement_pub_key: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PegInAddressOutput {
    pub(crate) address: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct RegisterPegInInput {
    pub(crate) block_hash: String,
    pub(crate) btc_tx: BitcoinTransaction,
    pub(crate) merkle_branch_path: String,
    pub(crate) merkle_branch_hashes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub(crate) struct RegisterPegInOutput {
    pub(crate) transaction_hash: String,
    pub(crate) success: bool,
}

pub type AcceptPegInInput = RegisterPegInInput;
pub type AcceptPegInOutput = RegisterPegInOutput;

/// Macro to format sol! errors printing error variant name and its parameters.
#[macro_export]
macro_rules! format_sol_err {
    ($err:expr $(, $arg:expr )* $(,)?) => {{
        use std::any::type_name_of_val;
        use std::fmt::Write;

        fn short_type_name<T>(val: &T) -> String {
            let full = type_name_of_val(val);
            let parts: Vec<_> = full.rsplit("::").take(2).collect();
            match parts.as_slice() {
                [variant, enum_name] => format!("{}::{}", enum_name, variant),
                [only] => only.to_string(),
                _ => full.to_string(),
            }
        }

        let mut msg = String::new();
        let _ = write!(msg, "{}:", short_type_name(&$err));

        $(
            let _ = write!(msg, " {:?}", $arg);
        )*

        msg
    }};
}
