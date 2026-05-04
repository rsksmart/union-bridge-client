//! Integration tests for the migrate-v031 helpers.
//!
//! Each test creates a temporary `Storage`, writes hand-crafted v0.3.1-shaped
//! JSON rows, calls into the migration helpers, and asserts the result on
//! disk. Fixtures are minimal — they exercise the migration paths but are
//! not full v0.4.x `State` values; full deserialization round-trips are
//! covered by the local E2E procedure documented in
//! `docs/v031-to-v04x-migration.md`.

use std::io::Write;

use migrate_v031::{
    RunOptions, check_config_no_legacy_bridge, migrate_committee, migrate_pegin, migrate_pegout,
    run, run_with_options, verify_v04x_schema,
};
use serde_json::{Value, json};
use storage_backend::storage::{KeyValueStore, Storage};
use storage_backend::storage_config::StorageConfig;
use tempfile::{NamedTempFile, TempDir};

fn make_storage() -> (TempDir, Storage) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().to_str().expect("utf-8 path").to_string();
    let storage = Storage::new(&StorageConfig::new(path, None)).expect("storage init");
    (dir, storage)
}

fn write_raw(storage: &Storage, key: &str, value: &Value) {
    storage.set(key, value, None).expect("write");
}

fn read_raw(storage: &Storage, key: &str) -> Value {
    storage.get::<&str, Value>(key).expect("get").expect("present")
}

#[test]
fn migrates_committee_missing_full_penalization() {
    let (_dir, storage) = make_storage();
    let key = "setup_committee_flows/00000000-0000-0000-0000-000000000001";
    write_raw(
        &storage,
        key,
        &json!({
            "internal_id": "00000000-0000-0000-0000-000000000001",
            "step": "Init",
            "ctx": { "pairwise_keys": {} },
        }),
    );

    let n = migrate_committee(&storage).expect("migrate");
    assert_eq!(n, 1);
    let row = read_raw(&storage, key);
    assert_eq!(row["ctx"]["setup_full_penalization_req"], json!([]));
}

#[test]
fn migrates_pegout_missing_request_tx_hash() {
    let (_dir, storage) = make_storage();
    let key = "pegout_flows/00000000-0000-0000-0000-000000000002";
    write_raw(
        &storage,
        key,
        &json!({
            "flow_id": "00000000-0000-0000-0000-000000000002",
            "step": "Done",
            "ctx": { "pegout_registered_tx": "0xabcd" },
        }),
    );

    let n = migrate_pegout(&storage).expect("migrate");
    assert_eq!(n, 1);
    let row = read_raw(&storage, key);
    assert_eq!(row["ctx"]["request_pegout_tx_hash"], json!(""));
    assert_eq!(row["ctx"]["pegout_registered_tx"], json!("0xabcd"));
}

#[test]
fn migrates_pegin_lifts_legacy_txids_when_present() {
    let (_dir, storage) = make_storage();
    let key = "pegin_flows/00000000-0000-0000-0000-000000000003";
    write_raw(
        &storage,
        key,
        &json!({
            "flow_id": "00000000-0000-0000-0000-000000000003",
            "ctx": {
                "step": "AddOperatorTakeHash",
                "bitvmx_pegin_accepted": {
                    "operator_take_txid": "1111111111111111111111111111111111111111111111111111111111111111",
                    "operator_won_txid": "2222222222222222222222222222222222222222222222222222222222222222"
                }
            },
        }),
    );

    let n = migrate_pegin(&storage).expect("migrate");
    assert_eq!(n, 1);
    let row = read_raw(&storage, key);
    assert_eq!(
        row["ctx"]["operator_take_txid"],
        json!("1111111111111111111111111111111111111111111111111111111111111111")
    );
    assert_eq!(
        row["ctx"]["operator_won_txid"],
        json!("2222222222222222222222222222222222222222222222222222222222222222")
    );
    // Legacy nested values are not removed.
    assert_eq!(
        row["ctx"]["bitvmx_pegin_accepted"]["operator_take_txid"],
        json!("1111111111111111111111111111111111111111111111111111111111111111")
    );
}

#[test]
fn pegin_without_legacy_txids_leaves_them_null() {
    let (_dir, storage) = make_storage();
    let key = "pegin_flows/00000000-0000-0000-0000-000000000004";
    write_raw(
        &storage,
        key,
        &json!({
            "flow_id": "00000000-0000-0000-0000-000000000004",
            "ctx": { "step": "PreparePeginSetup", "bitvmx_pegin_accepted": null },
        }),
    );

    migrate_pegin(&storage).expect("migrate");
    let row = read_raw(&storage, key);
    assert_eq!(row["ctx"]["operator_take_txid"], Value::Null);
    assert_eq!(row["ctx"]["operator_won_txid"], Value::Null);
    // The lift must not have promoted the null `bitvmx_pegin_accepted` into an object.
    assert_eq!(row["ctx"]["bitvmx_pegin_accepted"], Value::Null);
}

#[test]
fn run_is_idempotent() {
    let (_dir, storage) = make_storage();
    write_raw(
        &storage,
        "setup_committee_flows/00000000-0000-0000-0000-000000000005",
        &json!({"internal_id": "00000000-0000-0000-0000-000000000005", "step": "Init", "ctx": {"pairwise_keys": {}}}),
    );
    write_raw(
        &storage,
        "pegout_flows/00000000-0000-0000-0000-000000000006",
        &json!({"flow_id": "00000000-0000-0000-0000-000000000006", "step": "Done", "ctx": {}}),
    );
    write_raw(
        &storage,
        "pegin_flows/00000000-0000-0000-0000-000000000007",
        &json!({"flow_id": "00000000-0000-0000-0000-000000000007", "ctx": {"step": "GetCommInfo", "bitvmx_pegin_accepted": null}}),
    );

    let first = run(&storage).expect("first");
    assert!(first > 0, "first run must mutate something");

    let second = run(&storage).expect("second");
    assert_eq!(second, 0, "second run must be a no-op");
}

#[test]
fn already_migrated_rows_are_unchanged() {
    let (_dir, storage) = make_storage();
    let committee_key = "setup_committee_flows/00000000-0000-0000-0000-000000000008";
    let pegout_key = "pegout_flows/00000000-0000-0000-0000-000000000009";
    let pegin_key = "pegin_flows/00000000-0000-0000-0000-00000000000a";

    let committee_row = json!({
        "internal_id": "00000000-0000-0000-0000-000000000008",
        "step": "Done",
        "ctx": {
            "pairwise_keys": {},
            "setup_full_penalization_req": [["aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", true]]
        },
    });
    let pegout_row = json!({
        "flow_id": "00000000-0000-0000-0000-000000000009",
        "step": "Done",
        "ctx": { "request_pegout_tx_hash": "0xdeadbeef" },
    });
    let pegin_row = json!({
        "flow_id": "00000000-0000-0000-0000-00000000000a",
        "ctx": {
            "step": "Done",
            "operator_take_txid": "3333333333333333333333333333333333333333333333333333333333333333",
            "operator_won_txid": "4444444444444444444444444444444444444444444444444444444444444444",
            "bitvmx_pegin_accepted": null
        },
    });

    write_raw(&storage, committee_key, &committee_row);
    write_raw(&storage, pegout_key, &pegout_row);
    write_raw(&storage, pegin_key, &pegin_row);

    let n = run(&storage).expect("run");
    assert_eq!(n, 0);
    assert_eq!(read_raw(&storage, committee_key), committee_row);
    assert_eq!(read_raw(&storage, pegout_key), pegout_row);
    assert_eq!(read_raw(&storage, pegin_key), pegin_row);
}

#[test]
fn verify_v04x_schema_passes_after_run() {
    let (_dir, storage) = make_storage();
    write_raw(
        &storage,
        "setup_committee_flows/00000000-0000-0000-0000-00000000000b",
        &json!({
            "internal_id": "00000000-0000-0000-0000-00000000000b",
            "step": "Init",
            "ctx": { "pairwise_keys": {} },
        }),
    );
    write_raw(
        &storage,
        "pegout_flows/00000000-0000-0000-0000-00000000000c",
        &json!({"flow_id": "00000000-0000-0000-0000-00000000000c", "step": "Done", "ctx": {}}),
    );

    run(&storage).expect("run");
    verify_v04x_schema(&storage).expect("schema must be v0.4.x after run");
}

#[test]
fn verify_v04x_schema_passes_for_empty_db() {
    let (_dir, storage) = make_storage();
    verify_v04x_schema(&storage).expect("empty DB must pass");
}

#[test]
fn verify_v04x_schema_fails_for_unmigrated_committee_row() {
    let (_dir, storage) = make_storage();
    write_raw(
        &storage,
        "setup_committee_flows/00000000-0000-0000-0000-00000000000d",
        &json!({"internal_id": "00000000-0000-0000-0000-00000000000d", "step": "Init", "ctx": {}}),
    );
    let err = verify_v04x_schema(&storage).expect_err("legacy row must fail");
    assert!(format!("{err}").contains("setup_full_penalization_req"));
}

#[test]
fn check_config_passes_for_clean_toml() {
    let mut file = NamedTempFile::new().expect("tempfile");
    writeln!(
        file,
        r#"
            environment = "local"
            [coordinator]
            check_period_secs = 1
            [flows.common]
            rsk_confirmations = 5
        "#
    )
    .expect("write");
    check_config_no_legacy_bridge(file.path()).expect("clean config must pass");
}

#[test]
fn check_config_fails_for_legacy_bridge() {
    let mut file = NamedTempFile::new().expect("tempfile");
    writeln!(
        file,
        r#"
            environment = "local"
            [coordinator]
            check_period_secs = 1
            [bridge.coordinator]
            required_confirmations = 5
        "#
    )
    .expect("write");
    let err = check_config_no_legacy_bridge(file.path()).expect_err("legacy section must fail");
    let msg = format!("{err}");
    assert!(msg.contains("Legacy [bridge.*]"), "error must mention legacy section: {msg}");
    assert!(msg.contains("docs/v031-to-v04x-migration"), "error must point at the doc: {msg}");
}

#[test]
fn dry_run_does_not_write_back() {
    let (_dir, storage) = make_storage();
    let key = "setup_committee_flows/00000000-0000-0000-0000-00000000000e";
    let row = json!({
        "internal_id": "00000000-0000-0000-0000-00000000000e",
        "step": "Init",
        "ctx": { "pairwise_keys": {} },
    });
    write_raw(&storage, key, &row);

    let report = run_with_options(&storage, RunOptions { dry_run: true }).expect("dry run");
    assert!(report.dry_run);
    assert_eq!(report.committee_mutated.len(), 1);
    assert_eq!(report.committee_mutated[0], key);

    // The on-disk row must be unchanged.
    let after = read_raw(&storage, key);
    assert_eq!(after, row);
}

#[test]
fn report_lists_mutated_keys_per_prefix() {
    let (_dir, storage) = make_storage();
    let committee_key = "setup_committee_flows/00000000-0000-0000-0000-00000000000f";
    let pegout_key = "pegout_flows/00000000-0000-0000-0000-000000000010";
    let pegin_key = "pegin_flows/00000000-0000-0000-0000-000000000011";

    write_raw(
        &storage,
        committee_key,
        &json!({"internal_id": committee_key, "step": "Init", "ctx": {"pairwise_keys": {}}}),
    );
    write_raw(
        &storage,
        pegout_key,
        &json!({"flow_id": pegout_key, "step": "Done", "ctx": {}}),
    );
    write_raw(
        &storage,
        pegin_key,
        &json!({
            "flow_id": pegin_key,
            "ctx": {"step": "AddOperatorTakeHash", "bitvmx_pegin_accepted": {
                "operator_take_txid": "1111111111111111111111111111111111111111111111111111111111111111",
                "operator_won_txid": "2222222222222222222222222222222222222222222222222222222222222222"
            }},
        }),
    );

    let report = run_with_options(&storage, RunOptions::default()).expect("run");
    assert_eq!(report.total(), 3);
    assert_eq!(report.committee_mutated, vec![committee_key.to_string()]);
    assert_eq!(report.pegout_mutated, vec![pegout_key.to_string()]);
    assert_eq!(report.pegin_mutated, vec![pegin_key.to_string()]);
}
