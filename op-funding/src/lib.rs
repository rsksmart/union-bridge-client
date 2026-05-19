#![warn(unreachable_pub)]

use std::fmt;

use alloy_primitives::U256;

const STREAM_DENOMINATIONS: [u64; 5] = [100_000, 1_000_000, 10_000_000, 100_000_000, 1_000_000_000];
// Matches BitVMX's current example-side approximation in
// `rust-bitvmx-client/examples/union/participants/committee.rs:27`.
const FUNDING_AMOUNT_PER_SLOT: u64 = 12_000;
// Matches BitVMX's current example-side value in
// `rust-bitvmx-client/examples/union/participants/committee.rs:28`.
const DISPUTE_CHANNEL_FUNDING_PER_MEMBER: u64 = 50_000;
// Matches BitVMX/contract dust value in
// `rust-bitvmx-client/src/program/protocols/union/types.rs:74` and
// `bitvmx-union-bridge-contracts/src/libraries/Constants.sol:167`.
const DUST_VALUE: u64 = 540;
// Matches BitVMX/contract speed-up value in
// `rust-bitvmx-client/src/program/protocols/union/types.rs:75` and
// `bitvmx-union-bridge-contracts/src/libraries/Constants.sol:163`.
const SPEEDUP_VALUE: u64 = 540;
// Matches BitVMX's current example-side value in
// `rust-bitvmx-client/examples/union/participants/committee.rs:340`.
const REGTEST_SPEEDUP_FUNDS_VALUE: u64 = 1_000_000;
// Matches BitVMX's current example-side value in
// `rust-bitvmx-client/examples/union/participants/committee.rs:342`.
const NON_REGTEST_SPEEDUP_FUNDS_VALUE: u64 = 30_000;
// Matches BitVMX's current example-side safety buffer in
// `rust-bitvmx-client/examples/union/participants/committee.rs:404`.
const EXAMPLE_PROTOCOL_FUNDING_SAFETY_BUFFER: u64 = 5_000;
// Matches BitVMX's current example-side extra fee buffer in
// `rust-bitvmx-client/examples/union/participants/committee.rs:384`.
const EXAMPLE_TOTAL_FUNDING_SAFETY_BUFFER: u64 = 10_000;
// Extra operator BTC headroom on top of the example-derived estimator. The real BitVMX builder
// currently needs slightly more than the coarse example formula predicts, and the observed gap
// grows with stream size, so we intentionally overfund by a denomination-scaled margin here to
// avoid setup failures when creating the funding transaction.
const OPERATOR_FUNDING_MARGIN_NUMERATOR: u64 = 25;
const OPERATOR_FUNDING_MARGIN_DENOMINATOR: u64 = 100;
// Mirrors BitVMX's get_fee_rate (sats/vbyte): regtest uses 10, mainnet/testnet use 1.
const REGTEST_FEE_RATE: u64 = 10; // TODO this should come from config or contracts settings
const NON_REGTEST_FEE_RATE: u64 = 1; // TODO this should come from config or contracts settings

// Pure shared baseline for operator/member RSK funding. Callers remain responsible for obtaining
// `min_deposit`; this crate adds both a percentage uplift and a probabilistic gas reserve so the
// wallet can still submit later RSK transactions after posting the required deposit.
const RSK_GAS_BUFFER_PERCENT: u64 = 20;
const MEMBER_RSK_SETUP_RESERVE_WEI: u64 = 10_000_000_000_000_000;
const MEMBER_RSK_RESERVE_PER_SLOT_WEI: u64 = 3_000_000_000_000_000;
const SLOT_BUDGET_Z_SCORE_NUMERATOR: u128 = 233;
const SLOT_BUDGET_Z_SCORE_DENOMINATOR: u128 = 100;
#[derive(Clone, Copy, Debug)]
pub struct StreamFundingProfile {
    pub denomination: u64,
    pub protocol_funding: u64,
    pub speed_up_utxo: u64,
    pub advance_funds: u64,
    pub operator_fund_amount: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FundingProfileError {
    UnknownStreamId(u64),
    ArithmeticOverflow,
}

impl fmt::Display for FundingProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStreamId(stream_id) => {
                write!(f, "invalid stream id {stream_id} (expected 0-4)")
            }
            Self::ArithmeticOverflow => write!(f, "funding profile arithmetic overflow"),
        }
    }
}

impl std::error::Error for FundingProfileError {}

fn stream_denomination(stream_id: u64) -> Option<u64> {
    usize::try_from(stream_id).ok().and_then(|index| STREAM_DENOMINATIONS.get(index)).copied()
}

fn calculate_advance_funds_value(stream_denomination: u64) -> Option<u64> {
    stream_denomination.checked_mul(12)?.checked_div(10)
}

fn estimate_fee(input_quantity: u64, output_quantity: u64, fee_rate: u64) -> Option<u64> {
    46_u64
        .checked_add(input_quantity.checked_mul(68)?)?
        .checked_add(output_quantity.checked_mul(34)?)?
        .checked_mul(fee_rate)
}

fn fee_rate(is_regtest: bool) -> u64 {
    if is_regtest { REGTEST_FEE_RATE } else { NON_REGTEST_FEE_RATE }
}

fn operator_funding_value(slots_per_package: u64, is_regtest: bool) -> Option<u64> {
    FUNDING_AMOUNT_PER_SLOT
        .checked_mul(slots_per_package)?
        .checked_add(SPEEDUP_VALUE)?
        .checked_add(estimate_fee(1, slots_per_package.checked_add(2)?, fee_rate(is_regtest))?)
}

fn watchtower_funding_value(committee_member_count: u64, is_regtest: bool) -> Option<u64> {
    DISPUTE_CHANNEL_FUNDING_PER_MEMBER
        .checked_mul(committee_member_count)?
        .checked_add(SPEEDUP_VALUE)?
        .checked_add(estimate_fee(1, committee_member_count.checked_add(2)?, fee_rate(is_regtest))?)
}

fn funding_wt_disabler_directory_value(prover_count: u64, is_regtest: bool) -> Option<u64> {
    let output_quantity = prover_count.checked_mul(2)?;

    DUST_VALUE
        .checked_mul(prover_count)?
        .checked_mul(2)?
        .checked_add(SPEEDUP_VALUE)?
        .checked_add(estimate_fee(2, output_quantity, fee_rate(is_regtest))?)
}

fn funding_op_disabler_directory_value(slots_per_package: u64, is_regtest: bool) -> Option<u64> {
    DUST_VALUE
        .checked_mul(slots_per_package)?
        .checked_add(SPEEDUP_VALUE)?
        .checked_add(estimate_fee(2, slots_per_package.checked_add(1)?, fee_rate(is_regtest))?)
}

fn speedup_funds_value(is_regtest: bool) -> u64 {
    if is_regtest { REGTEST_SPEEDUP_FUNDS_VALUE } else { NON_REGTEST_SPEEDUP_FUNDS_VALUE }
}

fn operator_funding_margin(denomination: u64) -> Option<u64> {
    denomination
        .checked_mul(OPERATOR_FUNDING_MARGIN_NUMERATOR)?
        .checked_div(OPERATOR_FUNDING_MARGIN_DENOMINATOR)
}

/// Derives the Bitcoin funding profile for a supported stream.
///
/// # Errors
///
/// Returns [`FundingProfileError::UnknownStreamId`] when `stream_id` is outside the supported
/// denomination table. Returns [`FundingProfileError::ArithmeticOverflow`] when the requested
/// sizing parameters cannot be represented in the funding arithmetic.
pub fn derive_stream_funding_profile(
    stream_id: u64,
    is_regtest: bool,
    slots_per_package: u64,
    committee_member_count: u64,
    prover_count: u64,
) -> Result<StreamFundingProfile, FundingProfileError> {
    let denomination =
        stream_denomination(stream_id).ok_or(FundingProfileError::UnknownStreamId(stream_id))?;
    let speed_up_utxo = speedup_funds_value(is_regtest);
    let advance_funds = calculate_advance_funds_value(denomination)
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let operator_funding = operator_funding_value(slots_per_package, is_regtest)
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let op_disabler_funding = funding_op_disabler_directory_value(slots_per_package, is_regtest)
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let watchtower_funding = watchtower_funding_value(committee_member_count, is_regtest)
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let wt_disabler_funding = funding_wt_disabler_directory_value(prover_count, is_regtest)
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let protocol_funding = operator_funding
        .checked_add(op_disabler_funding)
        .and_then(|value| value.checked_add(watchtower_funding))
        .and_then(|value| value.checked_add(wt_disabler_funding))
        .and_then(|value| value.checked_add(EXAMPLE_PROTOCOL_FUNDING_SAFETY_BUFFER))
        .ok_or(FundingProfileError::ArithmeticOverflow)?;
    let operator_fund_amount = speed_up_utxo
        .checked_add(advance_funds)
        .and_then(|value| value.checked_add(operator_funding))
        .and_then(|value| value.checked_add(op_disabler_funding))
        .and_then(|value| value.checked_add(watchtower_funding))
        .and_then(|value| value.checked_add(wt_disabler_funding))
        .and_then(|value| value.checked_add(EXAMPLE_TOTAL_FUNDING_SAFETY_BUFFER))
        .and_then(|value| {
            operator_funding_margin(denomination).and_then(|margin| value.checked_add(margin))
        })
        .ok_or(FundingProfileError::ArithmeticOverflow)?;

    Ok(StreamFundingProfile {
        denomination,
        protocol_funding,
        speed_up_utxo,
        advance_funds,
        operator_fund_amount,
    })
}

#[must_use]
pub fn required_rsk_balance(min_deposit: U256) -> U256 {
    // Realistic RSK deposit values cannot saturate U256, but use saturating ops
    // so accidental overflow returns U256::MAX instead of panicking.
    let percentage_buffer =
        min_deposit.saturating_mul(U256::from(RSK_GAS_BUFFER_PERCENT)) / U256::from(100_u64);
    min_deposit.saturating_add(percentage_buffer)
}

#[must_use]
/// # Panics
/// Panics if the computed slot budget does not fit in `u64`.
pub fn budgeted_slot_count(slots_per_package: u64, committee_member_count: u64) -> u64 {
    checked_budgeted_slot_count(slots_per_package, committee_member_count)
        .expect("slot budget arithmetic does not overflow")
}

fn checked_budgeted_slot_count(slots_per_package: u64, committee_member_count: u64) -> Option<u64> {
    let slots_per_package = u128::from(slots_per_package);
    let committee_member_count = u128::from(committee_member_count.max(1));

    if slots_per_package == 0 {
        return Some(0);
    }

    let sqrt_term =
        ceil_sqrt(slots_per_package.checked_mul(committee_member_count.checked_sub(1)?)?);
    let numerator = slots_per_package
        .checked_mul(SLOT_BUDGET_Z_SCORE_DENOMINATOR)?
        .checked_add(SLOT_BUDGET_Z_SCORE_NUMERATOR.checked_mul(sqrt_term)?)?;
    let denominator = SLOT_BUDGET_Z_SCORE_DENOMINATOR.checked_mul(committee_member_count)?;

    u64::try_from(ceil_div(numerator, denominator)).ok()
}

#[must_use]
pub fn required_member_rsk_balance(
    min_deposit: U256,
    slots_per_package: u64,
    committee_member_count: u64,
) -> U256 {
    let percentage_buffer =
        min_deposit.saturating_mul(U256::from(RSK_GAS_BUFFER_PERCENT)) / U256::from(100_u64);
    let slot_budget = budgeted_slot_count(slots_per_package, committee_member_count);
    let probabilistic_reserve = U256::from(MEMBER_RSK_SETUP_RESERVE_WEI).saturating_add(
        U256::from(MEMBER_RSK_RESERVE_PER_SLOT_WEI).saturating_mul(U256::from(slot_budget)),
    );

    min_deposit.saturating_add(percentage_buffer.max(probabilistic_reserve))
}

fn ceil_div(numerator: u128, denominator: u128) -> u128 {
    numerator.div_ceil(denominator)
}

fn ceil_sqrt(value: u128) -> u128 {
    if value <= 1 {
        return value;
    }

    let mut low = 1_u128;
    let mut high = value;

    while low < high {
        // `low + (high - low) / 2` cannot overflow because `low <= high <= value`.
        // Using saturating ops keeps semgrep happy without changing semantics.
        let mid = low.saturating_add((high - low) / 2);
        let square = mid.saturating_mul(mid);

        if square < value {
            low = mid.saturating_add(1);
        } else {
            high = mid;
        }
    }

    low
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::{
        FundingProfileError, budgeted_slot_count, derive_stream_funding_profile,
        required_member_rsk_balance, required_rsk_balance,
    };

    const DEFAULT_COMMITTEE_MEMBER_COUNT: u64 = 4;
    const DEFAULT_PROVER_COUNT: u64 = 2;
    const DEFAULT_SLOTS_PER_PACKAGE: u64 = 100;

    #[test]
    fn derives_regtest_stream_zero_profile() {
        let profile = derive_stream_funding_profile(
            0,
            true,
            DEFAULT_SLOTS_PER_PACKAGE,
            DEFAULT_COMMITTEE_MEMBER_COUNT,
            DEFAULT_PROVER_COUNT,
        )
        .expect("stream 0 should exist");

        assert_eq!(profile.denomination, 100_000);
        assert_eq!(profile.protocol_funding, 1_541_660);
        assert_eq!(profile.speed_up_utxo, 1_000_000);
        assert_eq!(profile.advance_funds, 120_000);
        assert_eq!(profile.operator_fund_amount, 2_691_660);
    }

    #[test]
    fn derives_non_regtest_stream_zero_profile() {
        let profile = derive_stream_funding_profile(
            0,
            false,
            DEFAULT_SLOTS_PER_PACKAGE,
            DEFAULT_COMMITTEE_MEMBER_COUNT,
            DEFAULT_PROVER_COUNT,
        )
        .expect("stream 0 should exist");

        assert_eq!(profile.denomination, 100_000);
        assert_eq!(profile.protocol_funding, 1_471_154);
        assert_eq!(profile.speed_up_utxo, 30_000);
        assert_eq!(profile.advance_funds, 120_000);
        assert_eq!(profile.operator_fund_amount, 1_651_154);
    }

    #[test]
    fn derives_regtest_stream_one_profile() {
        let profile = derive_stream_funding_profile(
            1,
            true,
            DEFAULT_SLOTS_PER_PACKAGE,
            DEFAULT_COMMITTEE_MEMBER_COUNT,
            DEFAULT_PROVER_COUNT,
        )
        .expect("stream 1 should exist");

        assert_eq!(profile.denomination, 1_000_000);
        assert_eq!(profile.protocol_funding, 1_541_660);
        assert_eq!(profile.speed_up_utxo, 1_000_000);
        assert_eq!(profile.advance_funds, 1_200_000);
        assert_eq!(profile.operator_fund_amount, 3_996_660);
    }

    #[test]
    fn rejects_unknown_stream_id() {
        assert_eq!(
            derive_stream_funding_profile(
                99,
                true,
                DEFAULT_SLOTS_PER_PACKAGE,
                DEFAULT_COMMITTEE_MEMBER_COUNT,
                DEFAULT_PROVER_COUNT,
            )
            .unwrap_err(),
            FundingProfileError::UnknownStreamId(99)
        );
    }

    #[test]
    fn reports_arithmetic_overflow() {
        assert_eq!(
            derive_stream_funding_profile(
                0,
                true,
                u64::MAX,
                DEFAULT_COMMITTEE_MEMBER_COUNT,
                DEFAULT_PROVER_COUNT,
            )
            .unwrap_err(),
            FundingProfileError::ArithmeticOverflow
        );
    }

    #[test]
    fn uses_percentage_buffer_when_it_exceeds_fixed_headroom() {
        let min_deposit = U256::from(200_000_000_000_000_000_u64);
        assert_eq!(required_rsk_balance(min_deposit), U256::from(240_000_000_000_000_000_u64));
    }

    #[test]
    fn adds_twenty_percent_to_small_min_deposit() {
        let min_deposit = U256::from(25_000_000_000_000_000_u64);
        assert_eq!(required_rsk_balance(min_deposit), U256::from(30_000_000_000_000_000_u64));
    }

    #[test]
    fn uses_percentage_buffer_for_large_min_deposits() {
        let min_deposit = U256::from(500_000_000_000_000_000_u64);
        assert_eq!(required_rsk_balance(min_deposit), U256::from(600_000_000_000_000_000_u64));
    }

    #[test]
    fn budgets_probabilistic_slot_count_for_hundred_slots_and_four_members() {
        assert_eq!(budgeted_slot_count(100, DEFAULT_COMMITTEE_MEMBER_COUNT), 36);
    }

    #[test]
    fn budgets_probabilistic_slot_count_for_ten_slots_and_four_members() {
        assert_eq!(budgeted_slot_count(10, DEFAULT_COMMITTEE_MEMBER_COUNT), 6);
    }

    #[test]
    fn uses_probabilistic_reserve_for_small_min_deposit() {
        let min_deposit = U256::from(25_000_000_000_000_000_u64);
        assert_eq!(
            required_member_rsk_balance(
                min_deposit,
                DEFAULT_SLOTS_PER_PACKAGE,
                DEFAULT_COMMITTEE_MEMBER_COUNT
            ),
            U256::from(143_000_000_000_000_000_u64)
        );
    }

    #[test]
    fn keeps_percentage_buffer_for_large_min_deposit() {
        let min_deposit = U256::from(1_000_000_000_000_000_000_u64);
        assert_eq!(
            required_member_rsk_balance(
                min_deposit,
                DEFAULT_SLOTS_PER_PACKAGE,
                DEFAULT_COMMITTEE_MEMBER_COUNT
            ),
            U256::from(1_200_000_000_000_000_000_u64)
        );
    }
}
