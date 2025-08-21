use anyhow::{Result, bail};
use common::msg_broker::bitvmx_types::{P2PAddress, PeerId};
use log::info;

/// We need this function because we are temporarily:
/// - storing PeerId as the communication key on applyToStream
/// - storing only the address as the communication data on depositCommunicationData
/// therefore get_communication_data does not bring everything we need, just the address
/// this was agreed with Fairgate
pub fn build_communication_data(
    my_p2p_address: String,
    committee_addresses: Vec<String>,
    committee_peer_ids: Vec<PeerId>,
) -> Result<Vec<P2PAddress>> {
    if committee_addresses.len() != committee_peer_ids.len() {
        bail!(
            "Inconsistent committee size: {} vs {}",
            committee_addresses.len(),
            committee_peer_ids.len()
        );
    }

    let mut p2p_addresses = vec![];
    for i in 0..committee_addresses.len() {
        let mut addr = committee_addresses[i].to_string();
        // contracts require zeroed communication data for my own address on deposit, so we have to tweak it here
        if addr.is_empty() {
            addr = my_p2p_address.clone();
        }

        let peer_id = committee_peer_ids[i].clone();

        p2p_addresses.push(P2PAddress {
            address: addr,
            peer_id,
        });
    }

    info!("Built communication data: {:?}", p2p_addresses);

    Ok(p2p_addresses)
}
