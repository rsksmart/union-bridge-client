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
const REGTEST_FEE_RATE: u64 = 10; // TODO(iago) this should come from config or contracts settings
const NON_REGTEST_FEE_RATE: u64 = 1; // TODO(iago) this should come from config or contracts settings

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

fn stream_denomination(stream_id: u64) -> Option<u64> {
    usize::try_from(stream_id).ok().and_then(|index| STREAM_DENOMINATIONS.get(index)).copied()
}

fn calculate_advance_funds_value(stream_denomination: u64) -> u64 {
    stream_denomination * 12 / 10
}

fn estimate_fee(input_quantity: u64, output_quantity: u64, fee_rate: u64) -> u64 {
    (46 + input_quantity * 68 + output_quantity * 34) * fee_rate
}

fn fee_rate(is_regtest: bool) -> u64 {
    if is_regtest { REGTEST_FEE_RATE } else { NON_REGTEST_FEE_RATE }
}

fn operator_funding_value(packet_size: u64, is_regtest: bool) -> u64 {
    FUNDING_AMOUNT_PER_SLOT * packet_size
        + SPEEDUP_VALUE
        + estimate_fee(1, packet_size + 2, fee_rate(is_regtest))
}

fn watchtower_funding_value(operator_count: u64, is_regtest: bool) -> u64 {
    DISPUTE_CHANNEL_FUNDING_PER_MEMBER * operator_count
        + SPEEDUP_VALUE
        + estimate_fee(1, operator_count + 2, fee_rate(is_regtest))
}

fn funding_wt_disabler_directory_value(prover_count: u64, is_regtest: bool) -> u64 {
    DUST_VALUE * prover_count * 2
        + SPEEDUP_VALUE
        + estimate_fee(2, prover_count * 2, fee_rate(is_regtest))
}

fn funding_op_disabler_directory_value(packet_size: u64, is_regtest: bool) -> u64 {
    DUST_VALUE * packet_size
        + SPEEDUP_VALUE
        + estimate_fee(2, packet_size + 1, fee_rate(is_regtest))
}

fn speedup_funds_value(is_regtest: bool) -> u64 {
    if is_regtest { REGTEST_SPEEDUP_FUNDS_VALUE } else { NON_REGTEST_SPEEDUP_FUNDS_VALUE }
}

fn operator_funding_margin(denomination: u64) -> u64 {
    denomination * OPERATOR_FUNDING_MARGIN_NUMERATOR / OPERATOR_FUNDING_MARGIN_DENOMINATOR
}

#[must_use]
pub fn derive_stream_funding_profile(
    stream_id: u64,
    is_regtest: bool,
    packet_size: u64,
    operator_count: u64,
    prover_count: u64,
) -> Option<StreamFundingProfile> {
    let denomination = stream_denomination(stream_id)?;
    let speed_up_utxo = speedup_funds_value(is_regtest);
    let advance_funds = calculate_advance_funds_value(denomination);
    let protocol_funding = operator_funding_value(packet_size, is_regtest)
        + funding_op_disabler_directory_value(packet_size, is_regtest)
        + watchtower_funding_value(operator_count, is_regtest)
        + funding_wt_disabler_directory_value(prover_count, is_regtest)
        + EXAMPLE_PROTOCOL_FUNDING_SAFETY_BUFFER;
    let operator_fund_amount = speed_up_utxo
        + advance_funds
        + operator_funding_value(packet_size, is_regtest)
        + funding_op_disabler_directory_value(packet_size, is_regtest)
        + watchtower_funding_value(operator_count, is_regtest)
        + funding_wt_disabler_directory_value(prover_count, is_regtest)
        + EXAMPLE_TOTAL_FUNDING_SAFETY_BUFFER
        + operator_funding_margin(denomination);

    Some(StreamFundingProfile {
        denomination,
        protocol_funding,
        speed_up_utxo,
        advance_funds,
        operator_fund_amount,
    })
}

#[must_use]
pub fn required_rsk_balance(min_deposit: U256) -> U256 {
    let percentage_buffer =
        (min_deposit * U256::from(RSK_GAS_BUFFER_PERCENT)) / U256::from(100_u64);
    min_deposit + percentage_buffer
}

#[must_use]
/// # Panics
/// Panics if the computed slot budget does not fit in `u64`.
pub fn budgeted_slot_count(packet_size: u64, operator_count: u64) -> u64 {
    let packet_size = u128::from(packet_size);
    let operator_count = u128::from(operator_count.max(1));

    if packet_size == 0 {
        return 0;
    }

    let sqrt_term = ceil_sqrt(packet_size * (operator_count - 1));
    let numerator =
        packet_size * SLOT_BUDGET_Z_SCORE_DENOMINATOR + SLOT_BUDGET_Z_SCORE_NUMERATOR * sqrt_term;
    let denominator = SLOT_BUDGET_Z_SCORE_DENOMINATOR * operator_count;

    u64::try_from(ceil_div(numerator, denominator)).expect("slot budget fits in u64")
}

#[must_use]
pub fn required_member_rsk_balance(
    min_deposit: U256,
    packet_size: u64,
    operator_count: u64,
) -> U256 {
    let percentage_buffer =
        (min_deposit * U256::from(RSK_GAS_BUFFER_PERCENT)) / U256::from(100_u64);
    let slot_budget = budgeted_slot_count(packet_size, operator_count);
    let probabilistic_reserve = U256::from(MEMBER_RSK_SETUP_RESERVE_WEI)
        + U256::from(MEMBER_RSK_RESERVE_PER_SLOT_WEI) * U256::from(slot_budget);

    min_deposit + percentage_buffer.max(probabilistic_reserve)
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
        let mid = low + (high - low) / 2;
        let square = mid.saturating_mul(mid);

        if square < value {
            low = mid + 1;
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
        budgeted_slot_count, derive_stream_funding_profile, required_member_rsk_balance,
        required_rsk_balance,
    };

    const COMMITTEE_PACKET_SIZE: u64 = 100;
    const OPERATOR_COUNT: u64 = 4; // TODO(iago) this should come from config or contracts settings
    const PROVER_COUNT: u64 = 2; // TODO(iago) this should come from config or contracts settings

    #[test]
    fn derives_regtest_stream_zero_profile() {
        let profile = derive_stream_funding_profile(
            0,
            true,
            COMMITTEE_PACKET_SIZE,
            OPERATOR_COUNT,
            PROVER_COUNT,
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
            COMMITTEE_PACKET_SIZE,
            OPERATOR_COUNT,
            PROVER_COUNT,
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
            COMMITTEE_PACKET_SIZE,
            OPERATOR_COUNT,
            PROVER_COUNT,
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
        assert!(
            derive_stream_funding_profile(
                99,
                true,
                COMMITTEE_PACKET_SIZE,
                OPERATOR_COUNT,
                PROVER_COUNT
            )
            .is_none()
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
    fn budgets_probabilistic_slot_count_for_hundred_slots_and_four_operators() {
        assert_eq!(budgeted_slot_count(100, OPERATOR_COUNT), 36);
    }

    #[test]
    fn budgets_probabilistic_slot_count_for_ten_slots_and_four_operators() {
        assert_eq!(budgeted_slot_count(10, OPERATOR_COUNT), 6);
    }

    #[test]
    fn uses_probabilistic_reserve_for_small_min_deposit() {
        let min_deposit = U256::from(25_000_000_000_000_000_u64);
        assert_eq!(
            required_member_rsk_balance(min_deposit, COMMITTEE_PACKET_SIZE, OPERATOR_COUNT),
            U256::from(143_000_000_000_000_000_u64)
        );
    }

    #[test]
    fn keeps_percentage_buffer_for_large_min_deposit() {
        let min_deposit = U256::from(1_000_000_000_000_000_000_u64);
        assert_eq!(
            required_member_rsk_balance(min_deposit, COMMITTEE_PACKET_SIZE, OPERATOR_COUNT),
            U256::from(1_200_000_000_000_000_000_u64)
        );
    }
}
