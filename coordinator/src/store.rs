use std::collections::HashMap;
use std::hash::BuildHasher;

use anyhow::{Result, ensure};
use common::msg_broker::bitvmx_types::{ParticipantRole, SignedPublicKey};
use common::types::CommitteeId;
use log::{debug, error, info};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;
use uuid::Uuid;

use crate::flows::common::GlobalContext;
/// Key used to persist Coordinator data
pub enum StoreKey {
    GlobalContext,
    SetupCommitteeFlow(Uuid),
    PeginFlow(Uuid),
    PegoutFlow(Uuid),
}

#[derive(Debug, Clone, Copy)]
pub enum StorePrefix {
    SetupCommitteeFlow,
    PeginFlow,
    PegoutFlow,
}

impl StoreKey {
    fn value(&self) -> String {
        match self {
            StoreKey::GlobalContext => "global_context".to_string(),
            StoreKey::SetupCommitteeFlow(id) => {
                format!("{}/{}", StorePrefix::SetupCommitteeFlow.value(), id)
            }
            StoreKey::PeginFlow(id) => {
                format!("{}/{}", StorePrefix::PeginFlow.value(), id)
            }
            StoreKey::PegoutFlow(id) => {
                format!("{}/{}", StorePrefix::PegoutFlow.value(), id)
            }
        }
    }
}

impl StorePrefix {
    fn value(self) -> String {
        match self {
            StorePrefix::SetupCommitteeFlow => "setup_committee_flows".to_string(),
            StorePrefix::PeginFlow => "pegin_flows".to_string(),
            StorePrefix::PegoutFlow => "pegout_flows".to_string(),
        }
    }

    fn extract_id(self, key: &str) -> Result<Uuid> {
        let expected_prefix = format!("{}/", self.value());

        ensure!(
            key.starts_with(&expected_prefix),
            "Key '{key}' does not start with expected prefix '{expected_prefix}'"
        );

        let uuid_str = &key[expected_prefix.len()..];
        let flow_id = Uuid::parse_str(uuid_str)?;
        Ok(flow_id)
    }

    fn as_store_key(self, flow_id: Uuid) -> StoreKey {
        match self {
            StorePrefix::SetupCommitteeFlow => StoreKey::SetupCommitteeFlow(flow_id),
            StorePrefix::PeginFlow => StoreKey::PeginFlow(flow_id),
            StorePrefix::PegoutFlow => StoreKey::PegoutFlow(flow_id),
        }
    }
}

/// Internal serializable representation of `GlobalContext`
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistentGlobalContext {
    committees: HashMap<CommitteeId, ParticipantRole>,
    take_key: Option<SignedPublicKey>,
    dispute_key: Option<SignedPublicKey>,
    comm_key: Option<SignedPublicKey>,
}

impl PersistentGlobalContext {
    fn from_memory(ctx: &GlobalContext) -> Self {
        let keys = ctx.my_keys();

        Self {
            committees: ctx.my_committees().all_cloned(),
            take_key: keys.take_key(),
            dispute_key: keys.dispute_key(),
            comm_key: keys.comm_key(),
        }
    }

    fn load(self) -> GlobalContext {
        let ctx = GlobalContext::new();
        // restore committees
        for (id, role) in self.committees {
            ctx.my_committees().add(id, role);
        }
        // restore keys
        if let Some(k) = self.take_key {
            ctx.my_keys().set_take_key(k);
        }
        if let Some(k) = self.dispute_key {
            ctx.my_keys().set_dispute_key(k);
        }
        if let Some(k) = self.comm_key {
            ctx.my_keys().set_comm_key(k);
        }

        ctx
    }
}

#[cfg(test)]
use mockall::automock;

// Minimal trait to allow mocking the coordinator store in tests
#[cfg_attr(test, automock)]
pub trait CoordinatorStoreApi {
    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_context(&self) -> Result<Option<GlobalContext>>;

    /// # Errors
    /// Returns an error if storage access fails or if serialization fails.
    fn save_context(&self, context: &GlobalContext) -> Result<()>;

    /// # Errors
    /// Returns an error if storage access fails or if serialization fails.
    fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()>;

    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_flow<T: DeserializeOwned + 'static>(&self, key: &StoreKey) -> Result<Option<T>>;

    /// # Errors
    /// Returns an error if storage access fails.
    fn delete_flow(&self, key: &StoreKey) -> Result<()>;

    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_all_flows<T: DeserializeOwned + 'static>(
        &self,
        prefix: StorePrefix,
    ) -> Result<HashMap<Uuid, T>>;
}

pub struct CoordinatorStore {
    db: Storage,
}

impl CoordinatorStore {
    /// # Errors
    /// Returns an error if storage initialization fails.
    pub fn new(path: &str) -> Result<Self> {
        let config = StorageConfig::new(path.to_string(), None);
        let db = Storage::new(&config)?;
        Ok(Self { db })
    }

    fn set<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        Ok(self.db.set(key, value, None)?)
    }

    fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self.db.get(key)?)
    }

    fn delete(&self, key: &str) -> Result<()> {
        Ok(self.db.delete(key)?)
    }
}

impl CoordinatorStoreApi for CoordinatorStore {
    fn load_context(&self) -> Result<Option<GlobalContext>> {
        let maybe: Option<PersistentGlobalContext> = self.get(&StoreKey::GlobalContext.value())?;
        Ok(maybe.map(PersistentGlobalContext::load))
    }

    fn save_context(&self, context: &GlobalContext) -> Result<()> {
        let dto = PersistentGlobalContext::from_memory(context);
        self.set(&StoreKey::GlobalContext.value(), &dto)
    }

    fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()> {
        self.set(&key.value(), &flow_state)
    }

    fn load_flow<T: DeserializeOwned + 'static>(&self, key: &StoreKey) -> Result<Option<T>> {
        self.get(&key.value())
    }

    fn delete_flow(&self, key: &StoreKey) -> Result<()> {
        self.delete(&key.value())
    }

    fn load_all_flows<T: DeserializeOwned + 'static>(
        &self,
        prefix: StorePrefix,
    ) -> Result<HashMap<Uuid, T>> {
        self.db
            // get the keys starting with the prefix
            .partial_compare(&prefix.value())?
            .iter()
            .map(|(k, v)| {
                let flow_id = prefix.extract_id(k)?;
                let state: T = serde_json::from_str(v)?;
                Ok((flow_id, state))
            })
            .collect()
    }
}

/// # Errors
/// Returns an error if storage access fails or if deserialization fails.
pub fn restore_flows<Store, State, Flow, Factory>(
    store: &Store,
    prefix: StorePrefix,
    mut flow_factory: Factory,
) -> Result<HashMap<Uuid, Flow>>
where
    Store: CoordinatorStoreApi,
    State: DeserializeOwned + 'static,
    Factory: FnMut(State) -> Flow,
{
    debug!("Checking for {prefix:?} flows to restore from persistence");

    let saved_flows: HashMap<Uuid, State> = store.load_all_flows(prefix)?;
    let mut restored_flows = HashMap::with_capacity(saved_flows.len());

    for (flow_id, saved_state) in saved_flows {
        debug!("Restored {prefix:?} flow {flow_id} from persistence");
        restored_flows.insert(flow_id, flow_factory(saved_state));
    }

    if !restored_flows.is_empty() {
        info!("Restored {} {:?} flows from persistence", restored_flows.len(), prefix);
    }

    Ok(restored_flows)
}

pub fn cleanup_completed_flows<Store, Flow, IsDone, H>(
    store: &Store,
    prefix: StorePrefix,
    flows: &mut HashMap<Uuid, Flow, H>,
    mut is_done: IsDone,
) where
    Store: CoordinatorStoreApi,
    H: BuildHasher,
    IsDone: FnMut(&Flow) -> bool,
{
    let completed_flow_ids: Vec<Uuid> =
        flows.iter().filter(|(_, flow)| is_done(flow)).map(|(flow_id, _)| *flow_id).collect();

    for flow_id in &completed_flow_ids {
        flows.remove(flow_id);
    }

    for flow_id in completed_flow_ids {
        debug!("Removing completed {prefix:?} flow: {flow_id}");

        if let Err(err) = store.delete_flow(&prefix.as_store_key(flow_id)) {
            error!("Failed to remove completed {prefix:?} flow {flow_id} from persistence: {err}");
        }
    }
}
