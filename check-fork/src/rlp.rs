use primitive_types::U256;

pub fn encode_coin_value(label: &str, value: &U256) -> Vec<u8> {
    let v = alloy_rlp::encode(u256_be_coin_bytes(value).as_slice());
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

pub fn encode_signed_coin_value(label: &str, value: &U256) -> Vec<u8> {
    // RLP integers are big-endian: 0 -> 0x80, 0x00–0x7f encode as-is,
    // and if the MSB≥0x80 we prefix 0x00 to keep the value positive
    // before adding the length prefix.
    let mut bytes = u256_be_coin_bytes(value);
    if bytes.first().is_some_and(|b| *b >= 0x80) {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0); // we add a "0x00" prefix to keep the value positive
        prefixed.extend_from_slice(&bytes);
        bytes = prefixed;
    }
    let v = alloy_rlp::encode(bytes.as_slice());
    println!("RLP encode {label}: {}", hex::encode(&v));
    v
}

fn u256_be_trimmed(value: &U256) -> Vec<u8> {
    // positive integers must be represented in big-endian binary form with
    // no leading zeroes (thus making the integer value zero equivalent to
    // the empty byte array). Deserialized positive integers with leading
    // zeroes must be treated as invalid by any higher-order protocol using RLP.
    // we inherit this from ethereum, if any doubts checkout the ethereum yellow paper.
    let buf = value.to_big_endian();
    let first_non_zero = buf.iter().position(|&b| b != 0).unwrap_or(buf.len());
    match first_non_zero {
        idx if idx == buf.len() => Vec::new(),
        idx => buf[idx..].to_vec(),
    }
}

fn u256_be_coin_bytes(value: &U256) -> Vec<u8> {
    // RSKJ encodes coin values using RLP's empty element for zero amounts,
    // not a single 0x00 byte. Returning an empty vec reproduces the same
    // `0x80` encoding for zero and trims leading zeroes otherwise.
    if value.is_zero() {
        return Vec::new();
    }
    u256_be_trimmed(value)
}
