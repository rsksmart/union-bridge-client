use primitive_types::U256;

#[must_use]
pub fn encode_coin_value(value: &U256) -> Vec<u8> {
    alloy_rlp::encode(u256_be_coin_bytes(value).as_slice())
}

#[must_use]
pub fn encode_signed_coin_value(value: &U256) -> Vec<u8> {
    // RLP integers are big-endian: 0 -> 0x80, 0x00–0x7f encode as-is,
    // and if the MSB≥0x80 we prefix 0x00 to keep the value positive
    // before adding the length prefix.
    let mut bytes = u256_be_coin_bytes(value);
    if bytes.first().is_some_and(|b| *b >= 0x80) {
        let mut prefixed =
            Vec::with_capacity(bytes.len().checked_add(1).expect("prefixed coin value overflow"));
        prefixed.push(0); // we add a "0x00" prefix to keep the value positive
        prefixed.extend_from_slice(&bytes);
        bytes = prefixed;
    }
    alloy_rlp::encode(bytes.as_slice())
}

#[must_use]
fn u256_be_trimmed(value: &U256) -> Vec<u8> {
    // positive integers must be represented in big-endian binary form with
    // no leading zeroes (thus making the integer value zero equivalent to
    // the empty byte array). Deserialized positive integers with leading
    // zeroes must be treated as invalid by any higher-order protocol using RLP.
    // we inherit this from ethereum, if any doubts checkout the ethereum yellow paper.
    let buf = value.to_big_endian();
    let first_non_zero = buf.iter().position(|&b| b != 0).unwrap_or(buf.len());
    buf.get(first_non_zero..).map_or_else(Vec::new, <[u8]>::to_vec)
}

#[must_use]
fn u256_be_coin_bytes(value: &U256) -> Vec<u8> {
    // RSKJ encodes coin values using RLP's empty element for zero amounts,
    // not a single 0x00 byte. Returning an empty vec reproduces the same
    // `0x80` encoding for zero and trims leading zeroes otherwise.
    if value.is_zero() {
        return Vec::new();
    }
    u256_be_trimmed(value)
}
