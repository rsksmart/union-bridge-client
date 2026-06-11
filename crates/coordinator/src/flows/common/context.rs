use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use common_bitvmx::bitvmx_types::{ParticipantRole, SignedPublicKey};
use common_core::types::CommitteeId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Identifier for a flow.
///
/// Uniform across all flow types (pegin, pegout, operator-take, committee
/// setup). Each flow decides how to generate its uuid — randomly
/// (committee setup) via `FlowId::from_random`, or deterministically from
/// a canonical on-chain input (pegin/pegout/operator-take) via
/// `FlowId::from_tx`.
///
/// `Display` renders just the uuid, which is grep-friendly and uniform.
/// For richer log lines that include the per-flow canonical identifier
/// (`txid`, `tx_hash`, `stream_id`), use the `log_id` field on each flow's
/// `State` — pre-formatted at construction so log sites can write
/// `{state.log_id}` without indirection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FlowId(Uuid);

impl FlowId {
    /// Random uuid. Use when no canonical input exists at flow creation
    /// (committee setup at `ApplyToStream`).
    #[must_use]
    pub fn from_random() -> Self {
        FlowId(Uuid::new_v4())
    }

    /// Deterministically derive a flow id from canonical bytes and a salt.
    /// The salt namespaces the derivation per flow type so collisions
    /// across flow types are impossible by construction.
    ///
    /// # Panics
    /// Panics only if SHA-256 produces fewer than 32 bytes, which is impossible.
    #[must_use]
    pub fn from_tx(salt: &str, tx_id_or_hash: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(tx_id_or_hash);
        hasher.update(salt);
        let hash = hasher.finalize();
        let bytes: [u8; 16] = hash[..16].try_into().expect("SHA256 is always 32 bytes");
        FlowId(Uuid::from_bytes(bytes))
    }

    #[must_use]
    pub fn value(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for FlowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

// Key indices for committee member public keys
pub(crate) const TAKE_KEY_INDEX: usize = 0;
pub(crate) const DISPUTE_KEY_INDEX: usize = 1;
pub(crate) const COMM_KEY_INDEX: usize = 2;

#[derive(Default, Debug, Clone)]
pub struct GlobalContext {
    my_committees: MyCommittees,
    my_keys: MyKeys,
}

#[derive(Default, Debug, Clone)]
#[allow(clippy::struct_field_names)]
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
    committees: Rc<RefCell<HashMap<CommitteeId, ParticipantRole>>>,
}

impl Default for MyCommittees {
    fn default() -> Self {
        Self::new()
    }
}

impl MyCommittees {
    pub fn new() -> Self {
        Self { committees: Rc::new(RefCell::new(HashMap::new())) }
    }

    // NOTE: &self is enough thanks to RefCell
    pub fn add(&self, committee_id: CommitteeId, role: ParticipantRole) {
        self.committees.borrow_mut().insert(committee_id, role);
    }

    pub fn all_cloned(&self) -> HashMap<CommitteeId, ParticipantRole> {
        self.committees.borrow().clone()
    }

    pub fn _remove(&self, committee_id: &CommitteeId) {
        self.committees.borrow_mut().remove(committee_id);
    }

    pub fn im_member(&self, committee_id: &CommitteeId) -> bool {
        self.committees.borrow().contains_key(committee_id)
    }

    #[allow(unused)]
    pub fn my_role(&self, committee_id: &CommitteeId) -> Option<ParticipantRole> {
        self.committees.borrow().get(committee_id).cloned()
    }
}

impl GlobalContext {
    pub fn new() -> Self {
        Self { my_committees: MyCommittees::new(), my_keys: MyKeys::new() }
    }

    pub fn my_committees(&self) -> &MyCommittees {
        &self.my_committees
    }

    pub fn my_keys(&self) -> &MyKeys {
        &self.my_keys
    }
}
