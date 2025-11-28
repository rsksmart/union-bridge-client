use crate::types::Role;
use anyhow::{Result, bail};
use common::msg_broker::bitvmx_types::{P2PAddress, PeerId, SignedPublicKey};
use common::types::CommitteeId;
use log::info;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Key indices for committee member public keys
pub const TAKE_KEY_INDEX: usize = 0;
pub const DISPUTE_KEY_INDEX: usize = 1;
pub const COMM_KEY_INDEX: usize = 2;

#[derive(Default, Debug, Clone)]
pub struct GlobalContext {
    my_committees: MyCommittees,
    my_keys: MyKeys,
}

#[derive(Default, Debug, Clone)]
pub struct MyKeys {
    take_key: Rc<RefCell<Option<SignedPublicKey>>>,
    dispute_key: Rc<RefCell<Option<SignedPublicKey>>>,
    comm_key: Rc<RefCell<Option<SignedPublicKey>>>, // use a proper type when used
}

impl MyKeys {
    pub fn new() -> Self {
        Self {
            take_key: Rc::new(RefCell::new(None)),
            dispute_key: Rc::new(RefCell::new(None)),
            comm_key: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_take_key(&self, key: SignedPublicKey) {
        *self.take_key.borrow_mut() = Some(key);
    }

    pub fn set_dispute_key(&self, key: SignedPublicKey) {
        *self.dispute_key.borrow_mut() = Some(key);
    }

    pub fn set_comm_key(&self, key: SignedPublicKey) {
        *self.comm_key.borrow_mut() = Some(key);
    }

    pub fn is_set(&self) -> bool {
        self.take_key.borrow().is_some()
            && self.dispute_key.borrow().is_some()
            && self.comm_key.borrow().is_some()
    }

    pub fn take_key(&self) -> Option<SignedPublicKey> {
        self.take_key.borrow().clone()
    }

    pub fn dispute_key(&self) -> Option<SignedPublicKey> {
        self.dispute_key.borrow().clone()
    }

    pub fn comm_key(&self) -> Option<SignedPublicKey> {
        self.comm_key.borrow().clone()
    }
}

#[derive(Debug, Clone)]
pub struct MyCommittees {
    committees: Rc<RefCell<HashMap<CommitteeId, Role>>>,
}

impl Default for MyCommittees {
    fn default() -> Self {
        Self::new()
    }
}

impl MyCommittees {
    pub fn new() -> Self {
        Self {
            committees: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    // NOTE: &self is enough thanks to RefCell
    pub fn add(&self, committee_id: CommitteeId, role: Role) {
        self.committees.borrow_mut().insert(committee_id, role);
    }

    pub fn all_cloned(&self) -> HashMap<CommitteeId, Role> {
        self.committees.borrow().clone()
    }

    // TODO call when leaving a committee or when a committee is disbanded
    pub fn _remove(&self, committee_id: &CommitteeId) {
        self.committees.borrow_mut().remove(committee_id);
    }

    pub fn im_member(&self, committee_id: &CommitteeId) -> bool {
        self.committees.borrow().contains_key(committee_id)
    }

    #[allow(unused)]
    pub fn my_role(&self, committee_id: &CommitteeId) -> Option<Role> {
        self.committees.borrow().get(committee_id).cloned()
    }
}

impl GlobalContext {
    pub fn new() -> Self {
        Self {
            my_committees: MyCommittees::new(),
            my_keys: MyKeys::new(),
        }
    }

    pub fn my_committees(&self) -> &MyCommittees {
        &self.my_committees
    }

    pub fn my_keys(&self) -> &MyKeys {
        &self.my_keys
    }
}

/// We need this function because we are temporarily:
/// - storing PeerId as the communication key on applyToStream
/// - storing only the address as the communication data on depositCommunicationData
///   therefore get_communication_data does not bring everything we need, just the address
///   this was agreed with Fairgate
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
