use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use storage_backend::storage::{KeyValueStore, Storage};

use crate::flows::common::GlobalContext;
use crate::types::Role;
use common::msg_broker::bitvmx_types::SignedPublicKey;
use common::types::CommitteeId;

/// Key used to persist Coordinator global context
enum StoreKey {
    GlobalContext,
}

impl StoreKey {
    fn value(&self) -> String {
        match self {
            StoreKey::GlobalContext => "global_context".to_string(),
        }
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

    pub fn save_context(&self, context: GlobalContext) -> Result<()> {
        let dto = PersistentGlobalContext::from_memory(&context);
        self.set(&StoreKey::GlobalContext.value(), &dto)
    }

    pub fn load_context(&self) -> Result<Option<GlobalContext>> {
        let maybe: Option<PersistentGlobalContext> = self.get(&StoreKey::GlobalContext.value())?;
        Ok(maybe.map(|p| p.to_memory()))
    }
}

impl CoordinatorStoreApi for CoordinatorStore {
    fn load_context(&self) -> Result<Option<GlobalContext>> {
        self.load_context()
    }

    fn save_context(&self, context: GlobalContext) -> Result<()> {
        self.save_context(context)
    }
}
