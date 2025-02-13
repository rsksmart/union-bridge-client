use common::types::RskBlock;
use primitive_types::U256;

/// This function returns a default RSK test block.
///
/// # Example
///
/// ```
/// let block = get_default_rsk_block();
/// assert_eq!(block.number, 7_238_268);
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,238,268](https://explorer.rootstock.io/block/7238268)
/// ```
pub fn get_default_rsk_block() -> RskBlock {
    RskBlock::new(
        7_238_268,
        "0x56e5ec05a0502135f8da5fc55e3771b30c1177617e2037da8fb29a8c2fc8e841".to_string(),
        "0x66ed57e409aa2afcd0b373fb9438a665840b1bb8fec71505cea0c1de96b724bf".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739445352,
        "0xcc018a4152524f57484541442d".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a default RSK child test block.
///
/// # Example
/// 
/// ```
/// let block = get_default_rsk_block();
/// assert_eq!(block.number, 7_234_708);
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,708](https://explorer.rootstock.io/block/7234708)
/// ```
pub fn get_default_rsk_child_block() -> RskBlock {
    RskBlock::new(
        7_234_708,
        "0x9971862c7475888178eae1e2cd03dde72e3791ddd72853a8f781022a49a95228".to_string(),
        "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128), // difficulty (10 ZH)
        1739358667,
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128), // total difficulty (26,000 YH)
    )
}

/// This function returns a default RSK parent test block.
///
/// # Example
/// 
/// ```
/// let block = get_default_rsk_parent_block();
/// assert_eq!(block.number, 7_234_707);
///
/// # Links
/// For more information about this block, see the Rootstock Explorer:
/// [Rootstock Block 7,234,707](https://explorer.rootstock.io/block/7234707)
/// ```
pub fn get_default_rsk_parent_block() -> RskBlock {
    RskBlock::new(
        7_234_707,
        "0xb1b77a1d9e6d18f6668a0db6bead24bea4c507fc6779ab211899c008484384ca".to_string(),
        "0x5d164d93bf09ee215cc67420f24d31b8d86c46ced6e770e8abf69c16bea3a67c".to_string(),
        U256::from(10_000_000_000_000_000_000_000_u128),  // difficulty (10 ZH)
        1739358657,
        "pow_string".to_string(),
        U256::from(26_000_000_000_000_000_000_000_000_u128),  // total difficulty (26,000 YH)
    )
}
