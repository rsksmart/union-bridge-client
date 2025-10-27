use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};
use uuid::Uuid;

use crate::flows::common::GlobalContext;
use crate::types::Role;
use common::msg_broker::bitvmx_types::SignedPublicKey;
use common::types::CommitteeId;

/// Key used to persist Coordinator data
pub enum StoreKey {
    GlobalContext,
    SetupCommitteeFlow(Uuid),
}

pub enum StorePrefix {
    SetupCommitteeFlow,
}

impl StoreKey {
    fn value(&self) -> String {
        match self {
            StoreKey::GlobalContext => "global_context".to_string(),
            StoreKey::SetupCommitteeFlow(id) => {
                format!("{}/{}", StorePrefix::SetupCommitteeFlow.value(), id)
            }
        }
    }
}

impl StorePrefix {
    fn value(&self) -> String {
        match self {
            StorePrefix::SetupCommitteeFlow => "setup_committee_flows".to_string(),
        }
    }

    fn extract_id(&self, key: &str) -> Result<Uuid> {
        let expected_prefix = format!("{}/", self.value());

        ensure!(
            key.starts_with(&expected_prefix),
            "Key '{}' does not start with expected prefix '{}'",
            key,
            expected_prefix
        );

        let uuid_str = &key[expected_prefix.len()..];
        let flow_id = Uuid::parse_str(uuid_str)?;
        Ok(flow_id)
    }
}

/// Internal serializable representation of GlobalContext
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistentGlobalContext {
    committees: HashMap<CommitteeId, Role>,
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

    fn to_memory(self) -> GlobalContext {
        let ctx = GlobalContext::new();
        // restore committees
        for (id, role) in self.committees.into_iter() {
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
    fn load_context(&self) -> Result<Option<GlobalContext>>;

    fn save_context(&self, context: GlobalContext) -> Result<()>;

    fn save_flow<T: Serialize + 'static>(&self, key: StoreKey, flow_state: T) -> Result<()>;

    fn load_flow<T: for<'de> Deserialize<'de> + 'static>(&self, key: StoreKey)
    -> Result<Option<T>>;

    fn delete_flow(&self, key: StoreKey) -> Result<()>;

    fn load_all_flows<T: for<'de> Deserialize<'de> + 'static>(
        &self,
        key: StorePrefix,
    ) -> Result<HashMap<Uuid, T>>;
}

pub struct CoordinatorStore {
    db: Storage,
}

impl CoordinatorStore {
    pub fn new(path: &str) -> Result<Self> {
        let db = Storage::new_with_path(&PathBuf::from(path))?;
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

    pub fn save_context(&self, context: GlobalContext) -> Result<()> {
        let dto = PersistentGlobalContext::from_memory(&context);
        self.set(&StoreKey::GlobalContext.value(), &dto)
    }

    pub fn load_context(&self) -> Result<Option<GlobalContext>> {
        let maybe: Option<PersistentGlobalContext> = self.get(&StoreKey::GlobalContext.value())?;
        Ok(maybe.map(|p| p.to_memory()))
    }

    pub fn save_flow<T: Serialize>(&self, key: StoreKey, flow_state: T) -> Result<()> {
        self.set(&key.value(), &flow_state)
    }

    pub fn load_flow<T: for<'de> Deserialize<'de>>(&self, key: StoreKey) -> Result<Option<T>> {
        self.get(&key.value())
    }

    pub fn delete_flow(&self, key: StoreKey) -> Result<()> {
        self.delete(&key.value())
    }

    pub fn load_all_flows<T: for<'de> Deserialize<'de>>(
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

impl CoordinatorStoreApi for CoordinatorStore {
    fn load_context(&self) -> Result<Option<GlobalContext>> {
        self.load_context()
    }

    fn save_context(&self, context: GlobalContext) -> Result<()> {
        self.save_context(context)
    }

    fn save_flow<T: Serialize + 'static>(&self, key: StoreKey, flow_state: T) -> Result<()> {
        self.save_flow(key, flow_state)
    }

    fn load_flow<T: for<'de> Deserialize<'de> + 'static>(
        &self,
        key: StoreKey,
    ) -> Result<Option<T>> {
        self.load_flow(key)
    }

    fn delete_flow(&self, key: StoreKey) -> Result<()> {
        self.delete_flow(key)
    }

    fn load_all_flows<T: for<'de> Deserialize<'de> + 'static>(
        &self,
        prefix: StorePrefix,
    ) -> Result<HashMap<Uuid, T>> {
        self.load_all_flows(prefix)
    }
}
