//! Semantic configuration comparison and the current unapplied-change marker.
use super::model::{self, ConfigSnapshotData, SkippedKind, SkippedRecord};
use crate::{auth::principal::db_unavailable, error::ApiError, state::AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cmp::Ordering, collections::BTreeMap};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    /// Saved, but apply refused it: it is not live, and `reason` says why.
    Skipped,
    Added,
    Changed,
    Removed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemDiff {
    pub key: String,
    pub label: String,
    pub status: ItemStatus,
    pub changes: Vec<FieldChange>,
    /// Why the saved change was refused; a kept previous version remains Changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDiff {
    pub servers: Vec<ItemDiff>,
    pub streams: Vec<ItemDiff>,
    pub upstreams: Vec<ItemDiff>,
    pub has_changes: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingStatus {
    pub count: usize,
    pub has_baseline: bool,
    pub diff: ConfigDiff,
}

fn value_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Number(a)), Some(Value::Number(b))) => a.as_f64() == b.as_f64(),
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| value_eq(Some(a), Some(b)))
        }
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            a.len() == b.len() && a.iter().all(|(k, v)| value_eq(Some(v), b.get(k)))
        }
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
/// Runtime DNS is opt-in. Comparing a historical omission with the new default
/// must not manufacture unapplied changes during a 1.20 upgrade.
fn field_value_eq(field: &str, a: Option<&Value>, b: Option<&Value>) -> bool {
    if field == "resolveAtRuntime" {
        let default = Value::Bool(false);
        return value_eq(Some(a.unwrap_or(&default)), Some(b.unwrap_or(&default)));
    }
    if field == "config" {
        let normalized = |raw: Option<&Value>| {
            raw.map(|raw| {
                let mut value = raw.clone();
                if let Some(config) = value.as_object_mut() {
                    config
                        .entry("resolveAtRuntime")
                        .or_insert(Value::Bool(false));
                    if let Some(locations) =
                        config.get_mut("locations").and_then(Value::as_array_mut)
                    {
                        for location in locations {
                            if let Some(location) = location.as_object_mut() {
                                location
                                    .entry("resolveAtRuntime")
                                    .or_insert(Value::Bool(false));
                            }
                        }
                    }
                }
                value
            })
        };
        let left = normalized(a);
        let right = normalized(b);
        return value_eq(left.as_ref(), right.as_ref());
    }
    value_eq(a, b)
}

fn display(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(v)) => v.clone(),
        Some(Value::Number(n)) => n
            .as_f64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| n.to_string()),
        Some(v) => v.to_string(),
    }
}
// Valid nginx server/upstream labels are ASCII. Reproduce the old en-US
// localeCompare ordering for that domain, with case as a secondary distinction.
// PostgreSQL jsonb canonicalizes object keys, while arrays remain order-sensitive.
fn label_cmp(a: &str, b: &str) -> Ordering {
    fn weight(c: char) -> u32 {
        let c = c.to_ascii_lowercase();
        " _-:.*/0123456789abcdefghijklmnopqrstuvwxyz"
            .find(c)
            .map(|n| n as u32)
            .unwrap_or(512 + c as u32)
    }
    a.chars()
        .map(weight)
        .cmp(b.chars().map(weight))
        .then_with(|| {
            a.chars()
                .map(|c| c.is_ascii_uppercase())
                .cmp(b.chars().map(|c| c.is_ascii_uppercase()))
        })
}
fn diff_resource(
    previous: &[Value],
    current: &[Value],
    fields: &[&str],
    label: impl Fn(&Value) -> String,
) -> Vec<ItemDiff> {
    let key = |row: &Value| display(row.get("id"));
    let previous_by_id: BTreeMap<_, _> = previous.iter().map(|r| (key(r), r)).collect();
    let current_by_id: BTreeMap<_, _> = current.iter().map(|r| (key(r), r)).collect();
    let mut out = Vec::new();
    for row in previous {
        if !current_by_id.contains_key(&key(row)) {
            out.push(ItemDiff {
                key: key(row),
                label: label(row),
                status: ItemStatus::Removed,
                changes: Vec::new(),
                reason: None,
            });
        }
    }
    for row in current {
        let Some(before) = previous_by_id.get(&key(row)) else {
            out.push(ItemDiff {
                key: key(row),
                label: label(row),
                status: ItemStatus::Added,
                changes: Vec::new(),
                reason: None,
            });
            continue;
        };
        let changes: Vec<_> = fields
            .iter()
            .filter(|field| !field_value_eq(field, before.get(**field), row.get(**field)))
            .map(|field| FieldChange {
                field: (*field).into(),
                before: before.get(*field).cloned(),
                after: row.get(*field).cloned(),
            })
            .collect();
        if !changes.is_empty() {
            out.push(ItemDiff {
                key: key(row),
                label: label(row),
                status: ItemStatus::Changed,
                changes,
                reason: None,
            });
        }
    }
    out.sort_by(|a, b| {
        a.status
            .cmp(&b.status)
            .then_with(|| label_cmp(&a.label, &b.label))
    });
    out
}
pub fn diff_config(
    previous: Option<&ConfigSnapshotData>,
    current: &ConfigSnapshotData,
) -> ConfigDiff {
    let Some(previous) = previous else {
        return ConfigDiff::default();
    };
    let servers = diff_resource(
        &previous.servers,
        &current.servers,
        &[
            "serverName",
            "listen",
            "upstreamId",
            "config",
            "enabled",
            "certificateId",
        ],
        |s| {
            format!(
                "{}:{}",
                display(s.get("serverName")),
                display(s.get("listen"))
            )
        },
    );
    let streams = diff_resource(
        &previous.streams,
        &current.streams,
        &[
            "listen",
            "protocol",
            "proxyPass",
            "enabled",
            "resolveAtRuntime",
        ],
        |s| {
            format!(
                "{}/{}",
                display(s.get("listen")),
                display(s.get("protocol"))
            )
        },
    );
    let upstreams = diff_resource(
        &previous.upstreams,
        &current.upstreams,
        &["name", "servers", "resolveAtRuntime"],
        |u| display(u.get("name")),
    );
    let has_changes = !servers.is_empty() || !streams.is_empty() || !upstreams.is_empty();
    ConfigDiff {
        servers,
        streams,
        upstreams,
        has_changes,
    }
}
pub fn pending_diff(
    baseline: Option<&ConfigSnapshotData>,
    current: &ConfigSnapshotData,
) -> ConfigDiff {
    diff_config(
        Some(baseline.unwrap_or(&ConfigSnapshotData::default())),
        current,
    )
}
/// The pending list, with the rows the last apply refused marked as such.
///
/// The baseline holds what is live, so a refused row looks exactly like a row nobody has applied
/// yet — `added`, waiting its turn. It is not waiting: apply already looked at it and said no.
/// The records kept beside the baseline tell the two apart and carry the reason.
pub fn pending_diff_with_skipped(
    baseline: Option<&ConfigSnapshotData>,
    current: &ConfigSnapshotData,
    skipped: &[SkippedRecord],
) -> ConfigDiff {
    let mut diff = pending_diff(baseline, current);
    if skipped.is_empty() {
        return diff;
    }
    let mark = |kind: SkippedKind, items: &mut Vec<ItemDiff>| {
        for item in items.iter_mut() {
            let Some(record) = skipped.iter().find(|s| s.kind == kind && s.id == item.key) else {
                continue;
            };
            if !record.kept_previous { item.status = ItemStatus::Skipped; }
            item.reason = Some(record.reason.clone());
        }
        items.sort_by(|a, b| {
            a.status
                .cmp(&b.status)
                .then_with(|| label_cmp(&a.label, &b.label))
        });
    };
    mark(SkippedKind::Server, &mut diff.servers);
    mark(SkippedKind::Stream, &mut diff.streams);
    mark(SkippedKind::Upstream, &mut diff.upstreams);
    diff
}
/// What goes into the `applied_config` marker: the live snapshot plus the rows apply refused.
///
/// The refused rows live under their own key, so a reader that predates them — an older node, a
/// marker written before this version — still finds the snapshot exactly where it looks for it.
pub fn applied_state_value(data: &ConfigSnapshotData, skipped: &[SkippedRecord]) -> Value {
    let mut value = serde_json::json!({
        "upstreams": data.upstreams,
        "servers": data.servers,
        "streams": data.streams,
    });
    value["skipped"] = serde_json::to_value(skipped).unwrap_or_else(|_| Value::Array(Vec::new()));
    value
}
/// The refused rows in an `applied_config` marker; empty for a marker written before they existed.
pub fn skipped_from_value(raw: &Value) -> Vec<SkippedRecord> {
    raw.get("skipped")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}
pub(crate) fn snapshot_from_value(raw: &Value) -> ConfigSnapshotData {
    ConfigSnapshotData {
        upstreams: raw["upstreams"].as_array().cloned().unwrap_or_default(),
        servers: raw["servers"].as_array().cloned().unwrap_or_default(),
        streams: raw["streams"].as_array().cloned().unwrap_or_default(),
    }
}
pub async fn status(state: &AppState) -> Result<PendingStatus, ApiError> {
    let uncertain=super::model::apply_uncertain(&state.config).await;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|_| db_unavailable())?;
    let marker = tx
        .query_opt("SELECT data FROM applied_config WHERE id='current'", &[])
        .await
        .map_err(|_| db_unavailable())?;
    let marker = marker.map(|r| r.get::<_, Value>(0));
    let mut baseline = marker.as_ref().map(snapshot_from_value);
    let skipped = marker.as_ref().map(skipped_from_value).unwrap_or_default();
    let current = model::load_snapshot(&tx)
        .await
        .map_err(|_| db_unavailable())?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    if uncertain || super::model::apply_uncertain(&state.config).await {baseline=None;}
    let diff = pending_diff_with_skipped(baseline.as_ref(), &current, &skipped);
    Ok(PendingStatus {
        count: diff.servers.len() + diff.streams.len() + diff.upstreams.len(),
        has_baseline: baseline.is_some(),
        diff,
    })
}
