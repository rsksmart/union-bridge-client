use crate::types::Role;
use anyhow::{Result, bail};
use common::msg_broker::bitvmx_types::{P2PAddress, PeerId};
use common::types::CommitteeId;
use log::info;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default, Debug, Clone)]
pub(crate) struct GlobalContext {
    my_committees: MyCommittees,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct MyCommittees {
    committees: Rc<RefCell<HashMap<CommitteeId, Role>>>,
}

impl MyCommittees {
    pub fn new() -> Self {
        Self {
            committees: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn add(&mut self, committee_id: CommitteeId, role: Role) {
        self.committees.borrow_mut().insert(committee_id, role);
    }

    // TODO call when leaving a committee or when a committee is disbanded
    pub fn remove(&mut self, committee_id: CommitteeId) {
        self.committees.borrow_mut().remove(&committee_id);
    }

    pub fn im_member(&self, committee_id: &CommitteeId) -> bool {
        self.committees.borrow().contains_key(committee_id)
    }

    pub fn my_role(&self, committee_id: &CommitteeId) -> Option<Role> {
        self.committees.borrow().get(committee_id).cloned()
    }
}

impl GlobalContext {
    pub fn new() -> Self {
        Self {
            my_committees: MyCommittees::new(),
        }
    }

    // TODO rethink this mutable access
    pub fn my_committees(&mut self) -> &mut MyCommittees {
        &mut self.my_committees
    }
}

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
