//! A saved row that apply left out is not a new change waiting to be applied — it is a row that
//! was refused, and the pending list has to say so with the reason.
//!
//!     cargo test --test control_pending_skipped

use nwui_api::control::model::{ConfigSnapshotData, SkippedKind, SkippedRecord};
use nwui_api::control::pending::{ItemStatus, applied_state_value, pending_diff_with_skipped};
use serde_json::json;

fn server(id: &str, name: &str) -> serde_json::Value {
    json!({"id": id, "serverName": name, "listen": 80, "enabled": true, "config": {}})
}

fn snapshot(servers: Vec<serde_json::Value>) -> ConfigSnapshotData {
    ConfigSnapshotData { upstreams: vec![], servers, streams: vec![] }
}

#[test]
fn a_row_apply_refused_is_listed_as_excluded_with_its_reason() {
    let live = snapshot(vec![server("keep", "kept.test")]);
    let saved = snapshot(vec![server("keep", "kept.test"), server("bad", "broken.test")]);
    let skipped = vec![SkippedRecord {
        kind: SkippedKind::Server,
        id: "bad".into(),
        label: "broken.test:80".into(),
        reason: "listen 443 is already used by a stream".into(),
        kept_previous: false,
    }];

    let diff = pending_diff_with_skipped(Some(&live), &saved, &skipped);
    let item = diff.servers.iter().find(|i| i.key == "bad").expect("the refused row is listed");
    assert_eq!(item.status, ItemStatus::Skipped, "not 'added': it was refused, not queued");
    assert_eq!(item.reason.as_deref(), Some("listen 443 is already used by a stream"));
    assert!(diff.servers.iter().all(|i| i.key != "keep"), "an applied row is not pending");
}

#[test]
fn a_genuinely_new_row_is_still_added() {
    let live = snapshot(vec![]);
    let saved = snapshot(vec![server("new", "fresh.test")]);
    let diff = pending_diff_with_skipped(Some(&live), &saved, &[]);
    let item = diff.servers.first().expect("the new row is listed");
    assert_eq!((item.status, item.reason.as_deref()), (ItemStatus::Added, None));
}

#[test]
fn the_applied_marker_carries_the_refused_rows_so_the_next_read_can_explain_them() {
    let data = snapshot(vec![server("keep", "kept.test")]);
    let skipped = vec![SkippedRecord {
        kind: SkippedKind::Stream,
        id: "s1".into(),
        label: "15433/tcp".into(),
        reason: "port reserved".into(),
        kept_previous: false,
    }];
    let stored = applied_state_value(&data, &skipped);
    assert_eq!(stored["servers"][0]["id"], "keep", "the live rows stay where readers expect them");
    let read_back: Vec<SkippedRecord> = serde_json::from_value(stored["skipped"].clone()).expect("skipped rows round-trip");
    assert_eq!(read_back, skipped);
    // An older marker without the field must still read as "nothing was refused".
    let legacy = json!({"upstreams": [], "servers": [], "streams": []});
    assert!(nwui_api::control::pending::skipped_from_value(&legacy).is_empty());
}

#[test]
fn a_kept_previous_row_stays_a_changed_row_with_the_rejection_reason() {
    let before=snapshot(vec![server("b","previous.test")]);
    let current=snapshot(vec![server("b","changed.test")]);
    let kept:SkippedRecord=serde_json::from_value(json!({"kind":"server","id":"b","label":"changed.test","reason":"nginx rejected the saved change","kept_previous":true})).unwrap();
    let diff=pending_diff_with_skipped(Some(&before),&current,&[kept]);
    assert_eq!(diff.servers[0].status,ItemStatus::Changed);
    assert_eq!(diff.servers[0].reason.as_deref(),Some("nginx rejected the saved change"));
}
