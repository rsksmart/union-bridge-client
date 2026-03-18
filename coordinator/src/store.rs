use std::collections::HashMap;
use std::hash::BuildHasher;

use anyhow::{Context, Result, ensure};
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
use mockall::mock;

pub trait CoordinatorStoreWriteApi {
    /// # Errors
    /// Returns an error if storage access fails or if serialization fails.
    fn save_context(&self, context: &GlobalContext) -> Result<()>;

    /// # Errors
    /// Returns an error if storage access fails or if serialization fails.
    fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()>;

    /// # Errors
    /// Returns an error if storage access fails.
    fn delete_flow(&self, key: &StoreKey) -> Result<()>;
}

// Minimal trait to allow mocking the coordinator store in tests
pub trait CoordinatorStoreApi: CoordinatorStoreWriteApi {
    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_context(&self) -> Result<Option<GlobalContext>>;

    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_flow<T: DeserializeOwned + 'static>(&self, key: &StoreKey) -> Result<Option<T>>;

    /// # Errors
    /// Returns an error if storage access fails or if deserialization fails.
    fn load_all_flows<T: DeserializeOwned + 'static>(
        &self,
        prefix: StorePrefix,
    ) -> Result<HashMap<Uuid, T>>;
}

#[cfg(test)]
mock! {
    pub CoordinatorStoreApi {}

    impl CoordinatorStoreWriteApi for CoordinatorStoreApi {
        fn save_context(&self, context: &GlobalContext) -> Result<()>;
        fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()>;
        fn delete_flow(&self, key: &StoreKey) -> Result<()>;
    }

    #[allow(clippy::struct_field_names)]
    impl CoordinatorStoreApi for CoordinatorStoreApi {
        fn load_context(&self) -> Result<Option<GlobalContext>>;
        fn load_flow<T: DeserializeOwned + 'static>(&self, key: &StoreKey) -> Result<Option<T>>;
        fn load_all_flows<T: DeserializeOwned + 'static>(
            &self,
            prefix: StorePrefix,
        ) -> Result<HashMap<Uuid, T>>;
    }
}

pub trait CoordinatorStoreTransactional {
    type Transaction<'a>: CoordinatorStoreWriteApi
    where
        Self: 'a;

    /// Executes several store operations atomically within a scoped transaction.
    ///
    /// # Errors
    /// Returns an error if any operation fails, if committing fails, or if rollback fails after an
    /// operation error.
    fn transaction<T, F>(&self, run_ops: F) -> Result<T>
    where
        F: for<'a> FnOnce(&Self::Transaction<'a>) -> Result<T>;
}

pub struct CoordinatorStore {
    db: Storage,
}

/// Scoped transaction handle for coordinator persistence operations.
///
/// This type is intentionally created through `CoordinatorStoreTransactional::transaction` so
/// callers keep the transactional work bounded to a single closure. If it is dropped before being
/// committed, it rolls back the underlying storage transaction.
pub struct CoordinatorStoreTransaction<'a> {
    store: &'a CoordinatorStore,
    transaction_id: Option<Uuid>,
}

impl CoordinatorStore {
    /// # Errors
    /// Returns an error if storage initialization fails.
    pub fn new(path: &str) -> Result<Self> {
        let config = StorageConfig::new(path.to_string(), None);
        let db = Storage::new(&config)?;
        Ok(Self { db })
    }

    fn persist_context(&self, context: &GlobalContext, transaction_id: Option<Uuid>) -> Result<()> {
        let dto = PersistentGlobalContext::from_memory(context);
        Ok(self.db.set(StoreKey::GlobalContext.value(), &dto, transaction_id)?)
    }

    fn persist_flow<T: Serialize + 'static>(
        &self,
        key: &StoreKey,
        flow_state: T,
        transaction_id: Option<Uuid>,
    ) -> Result<()> {
        Ok(self.db.set(key.value(), &flow_state, transaction_id)?)
    }

    fn remove_flow(&self, key: &StoreKey, transaction_id: Option<Uuid>) -> Result<()> {
        match transaction_id {
            Some(transaction_id) => {
                Ok(self.db.transactional_delete(&key.value(), transaction_id)?)
            }
            None => Ok(self.db.delete(&key.value())?),
        }
    }
}

impl CoordinatorStoreTransactional for CoordinatorStore {
    type Transaction<'a>
        = CoordinatorStoreTransaction<'a>
    where
        Self: 'a;

    fn transaction<T, F>(&self, run_ops: F) -> Result<T>
    where
        F: for<'a> FnOnce(&Self::Transaction<'a>) -> Result<T>,
    {
        let mut transaction = CoordinatorStoreTransaction::new(self);

        match run_ops(&transaction) {
            Ok(result) => {
                transaction.commit()?;
                Ok(result)
            }
            Err(ops_err) => {
                if let Err(rb_err) = transaction.rollback() {
                    return Err(ops_err.context(format!(
                        "failed to rollback coordinator store transaction: {rb_err}"
                    )));
                }
                Err(ops_err)
            }
        }
    }
}

impl<'a> CoordinatorStoreTransaction<'a> {
    fn new(store: &'a CoordinatorStore) -> CoordinatorStoreTransaction<'a> {
        Self { store, transaction_id: Some(store.db.begin_transaction()) }
    }

    fn transaction_id(&self) -> Result<Uuid> {
        self.transaction_id.context("coordinator store transaction is no longer active")
    }

    fn commit(&mut self) -> Result<()> {
        // `commit_transaction` consumes the backend transaction even if the commit fails, so the
        // handle must be cleared before delegating to storage.
        let transaction_id = self
            .transaction_id
            .take()
            .context("coordinator store transaction is no longer active")?;
        self.store.db.commit_transaction(transaction_id)?;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        // Keep the handle until rollback succeeds, so `Drop` can still attempt cleanup if storage
        // returns an error here.
        let transaction_id =
            self.transaction_id.context("coordinator store transaction is no longer active")?;
        self.store.db.rollback_transaction(transaction_id)?;
        self.transaction_id = None;
        Ok(())
    }
}

impl CoordinatorStoreWriteApi for CoordinatorStoreTransaction<'_> {
    fn save_context(&self, context: &GlobalContext) -> Result<()> {
        self.store.persist_context(context, Some(self.transaction_id()?))
    }

    fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()> {
        self.store.persist_flow(key, flow_state, Some(self.transaction_id()?))
    }

    fn delete_flow(&self, key: &StoreKey) -> Result<()> {
        self.store.remove_flow(key, Some(self.transaction_id()?))
    }
}

impl Drop for CoordinatorStoreTransaction<'_> {
    fn drop(&mut self) {
        if let Some(transaction_id) = self.transaction_id.take()
            && let Err(err) = self.store.db.rollback_transaction(transaction_id)
        {
            error!("Failed to rollback coordinator store transaction on drop: {err}");
        }
    }
}

impl CoordinatorStoreWriteApi for CoordinatorStore {
    fn save_context(&self, context: &GlobalContext) -> Result<()> {
        self.persist_context(context, None)
    }

    fn save_flow<T: Serialize + 'static>(&self, key: &StoreKey, flow_state: T) -> Result<()> {
        self.persist_flow(key, flow_state, None)
    }

    fn delete_flow(&self, key: &StoreKey) -> Result<()> {
        self.remove_flow(key, None)
    }
}

impl CoordinatorStoreApi for CoordinatorStore {
    fn load_context(&self) -> Result<Option<GlobalContext>> {
        let maybe: Option<PersistentGlobalContext> =
            self.db.get(StoreKey::GlobalContext.value())?;
        Ok(maybe.map(PersistentGlobalContext::load))
    }

    fn load_flow<T: DeserializeOwned + 'static>(&self, key: &StoreKey) -> Result<Option<T>> {
        Ok(self.db.get(key.value())?)
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

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestFlowState {
        name: String,
        step: u8,
    }

    fn test_store() -> Result<(TempDir, CoordinatorStore)> {
        let dir = TempDir::new()?;
        let path = dir.path().to_string_lossy().into_owned();
        let store = CoordinatorStore::new(&path)?;
        Ok((dir, store))
    }

    #[test]
    fn transaction_commits_several_flow_operations() -> Result<()> {
        let (_dir, store) = test_store()?;
        let deleted_flow_id = Uuid::new_v4();
        let saved_flow_id = Uuid::new_v4();
        let second_saved_flow_id = Uuid::new_v4();

        store.save_flow(
            &StoreKey::PegoutFlow(deleted_flow_id),
            TestFlowState { name: "obsolete".to_string(), step: 1 },
        )?;

        store.transaction(|tx_store| {
            tx_store.save_flow(
                &StoreKey::PeginFlow(saved_flow_id),
                TestFlowState { name: "pegin".to_string(), step: 2 },
            )?;
            tx_store.save_flow(
                &StoreKey::PegoutFlow(second_saved_flow_id),
                TestFlowState { name: "pegout".to_string(), step: 3 },
            )?;
            tx_store.delete_flow(&StoreKey::PegoutFlow(deleted_flow_id))?;
            Ok(())
        })?;

        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::PeginFlow(saved_flow_id))?,
            Some(TestFlowState { name: "pegin".to_string(), step: 2 })
        );
        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::PegoutFlow(second_saved_flow_id))?,
            Some(TestFlowState { name: "pegout".to_string(), step: 3 })
        );
        assert_eq!(store.load_flow::<TestFlowState>(&StoreKey::PegoutFlow(deleted_flow_id))?, None);

        Ok(())
    }

    #[test]
    fn transaction_rolls_back_all_operations_when_closure_fails() -> Result<()> {
        let (_dir, store) = test_store()?;
        let kept_flow_id = Uuid::new_v4();
        let transient_flow_id = Uuid::new_v4();

        store.save_flow(
            &StoreKey::SetupCommitteeFlow(kept_flow_id),
            TestFlowState { name: "kept".to_string(), step: 4 },
        )?;

        let err = store
            .transaction(|tx_store| -> Result<()> {
                tx_store.save_flow(
                    &StoreKey::PeginFlow(transient_flow_id),
                    TestFlowState { name: "transient".to_string(), step: 5 },
                )?;
                tx_store.delete_flow(&StoreKey::SetupCommitteeFlow(kept_flow_id))?;
                bail!("force rollback");
            })
            .expect_err("transaction should fail");

        assert_eq!(err.to_string(), "force rollback");
        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::SetupCommitteeFlow(kept_flow_id))?,
            Some(TestFlowState { name: "kept".to_string(), step: 4 })
        );
        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::PeginFlow(transient_flow_id))?,
            None
        );

        Ok(())
    }

    #[test]
    fn transaction_can_commit_context_and_flows_together() -> Result<()> {
        let (_dir, store) = test_store()?;
        let flow_id = Uuid::new_v4();
        let context = GlobalContext::new();

        store.transaction(|tx_store| {
            tx_store.save_context(&context)?;
            tx_store.save_flow(
                &StoreKey::SetupCommitteeFlow(flow_id),
                TestFlowState { name: "with-context".to_string(), step: 6 },
            )?;
            Ok(())
        })?;

        assert!(store.load_context()?.is_some());
        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::SetupCommitteeFlow(flow_id))?,
            Some(TestFlowState { name: "with-context".to_string(), step: 6 })
        );

        Ok(())
    }

    #[test]
    fn transaction_trait_supports_generic_atomic_writes() -> Result<()> {
        fn persist_flow<Store: CoordinatorStoreTransactional>(
            store: &Store,
            flow_id: Uuid,
        ) -> Result<()> {
            store.transaction(|tx_store| {
                tx_store.save_flow(
                    &StoreKey::PegoutFlow(flow_id),
                    TestFlowState { name: "generic".to_string(), step: 8 },
                )?;
                Ok(())
            })
        }

        let (_dir, store) = test_store()?;
        let flow_id = Uuid::new_v4();

        persist_flow(&store, flow_id)?;

        assert_eq!(
            store.load_flow::<TestFlowState>(&StoreKey::PegoutFlow(flow_id))?,
            Some(TestFlowState { name: "generic".to_string(), step: 8 })
        );

        Ok(())
    }

    #[test]
    fn transaction_supports_extracted_helper_logic() -> Result<()> {
        fn persist_flow_and_fail(
            tx: &CoordinatorStoreTransaction<'_>,
            flow_id: Uuid,
        ) -> Result<()> {
            tx.save_flow(
                &StoreKey::PegoutFlow(flow_id),
                TestFlowState { name: "hidden".to_string(), step: 7 },
            )?;
            Err(anyhow::anyhow!("logic failed"))
        }

        let (_dir, store) = test_store()?;
        let flow_id = Uuid::new_v4();

        let err = store
            .transaction(|tx| {
                persist_flow_and_fail(tx, flow_id)?;
                Ok(())
            })
            .expect_err("transaction should fail");

        assert_eq!(err.to_string(), "logic failed");
        assert_eq!(store.load_flow::<TestFlowState>(&StoreKey::PegoutFlow(flow_id))?, None);

        Ok(())
    }
}
