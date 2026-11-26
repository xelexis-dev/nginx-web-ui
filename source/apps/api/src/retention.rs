//! Deletes eligible local records from the API serve process.
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_postgres::{GenericClient, Transaction};

use crate::{
    auth::audit::{self, AuditEntry},
    state::AppState,
};

pub(crate) const LOCK: &str = "hashtext('nginx-web-ui:retention')";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Collection {
    Audit,
    History,
}

impl Collection {
    pub(crate) const ALL: [Self; 2] = [Self::Audit, Self::History];

    pub(crate) fn key(self) -> &'static str {
        match self { Self::Audit => "audit", Self::History => "history" }
    }

    pub(crate) fn label(self) -> &'static str {
        match self { Self::Audit => "감사 로그", Self::History => "적용 이력" }
    }

    pub(crate) fn field(self) -> &'static str {
        match self { Self::Audit => "auditRetentionDays", Self::History => "historyRetentionDays" }
    }

    pub(crate) fn minimum(self) -> i32 {
        match self { Self::Audit => 90, Self::History => 30 }
    }

    pub(crate) fn table(self) -> &'static str {
        match self { Self::Audit => "audit_logs", Self::History => "config_snapshots" }
    }

    fn floor(self) -> i64 {
        match self { Self::Audit => 10_000, Self::History => 100 }
    }

    fn batch(self) -> i64 {
        match self { Self::Audit => 1_000, Self::History => 25 }
    }

    pub(crate) fn policy(self) -> &'static str {
        match self { Self::Audit => "audit_retention_days", Self::History => "history_retention_days" }
    }

    fn candidates(self, include_settings_event: bool) -> String {
        let table = self.table();
        let offset = self.floor() - 1;
        let ordinary = match self { Self::Audit => "WHERE action <> 'retention:prune'", Self::History => "" };
        let ordinary_records = format!("SELECT id, created_at FROM {table} {ordinary}");
        let boundary_records = if include_settings_event && matches!(self, Self::Audit) {
            // Changing either policy inserts an ordinary event before cleanup can start.
            format!("{ordinary_records} UNION ALL SELECT gen_random_uuid()::text, now()")
        } else {
            ordinary_records.clone()
        };
        let mut sql = format!(
            "SELECT id, created_at FROM ({ordinary_records}) AS records
             WHERE created_at < $1 AND (created_at, id) < (
                SELECT id_order.created_at, id_order.id FROM ({boundary_records}) AS id_order
                ORDER BY created_at DESC, id DESC OFFSET {offset} LIMIT 1
             )"
        );
        if matches!(self, Self::Audit) {
            sql.push_str(
                " UNION ALL SELECT id, created_at FROM audit_logs
                  WHERE action = 'retention:prune' AND created_at < $1 AND id NOT IN (
                    SELECT DISTINCT ON (\"after\"->>'collection') id FROM audit_logs
                    WHERE action = 'retention:prune' AND \"after\"->>'collection' IN ('audit', 'history')
                    ORDER BY \"after\"->>'collection', created_at DESC, id DESC
                  )"
            );
        }
        sql
    }

    pub(crate) async fn count(self, client: &impl GenericClient, cutoff: DateTime<Utc>) -> Result<i64, tokio_postgres::Error> {
        self.count_candidates(client, cutoff, false).await
    }

    async fn count_candidates(self, client: &impl GenericClient, cutoff: DateTime<Utc>, include_settings_event: bool) -> Result<i64, tokio_postgres::Error> {
        let sql = format!("SELECT count(*) FROM ({}) eligible", self.candidates(include_settings_event));
        Ok(client.query_one(&sql, &[&cutoff]).await?.get(0))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Cleanup {
    pub(crate) at: DateTime<Utc>,
    pub(crate) deleted: i64,
}

pub(crate) async fn last_cleanup(client: &impl GenericClient, collection: Collection) -> Result<Option<Cleanup>, tokio_postgres::Error> {
    let row = client.query_one(
        "SELECT last_cleanup_at, last_cleanup_count FROM retention_state WHERE collection=$1",
        &[&collection.key()],
    ).await?;
    Ok(row.get::<_, Option<DateTime<Utc>>>(0).map(|at| Cleanup { at, deleted: row.get(1) }))
}

pub fn start(state: AppState) -> tokio::task::JoinHandle<()> {
    let mut shutdown = state.runtime.shutdown.subscribe();
    tokio::spawn(async move {
        let start = tokio::time::Instant::now() + Duration::from_secs(600);
        let mut ticks = tokio::time::interval_at(start, Duration::from_secs(60));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !*shutdown.borrow() {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = ticks.tick() => {
                    for collection in Collection::ALL {
                        if *shutdown.borrow() { break; }
                        run(&state, collection, &shutdown).await;
                    }
                }
            }
        }
    })
}

async fn lock(tx: &Transaction<'_>) -> anyhow::Result<bool> {
    tx.batch_execute("SET LOCAL statement_timeout = '5s'").await?;
    Ok(tx.query_one(&format!("SELECT pg_try_advisory_xact_lock({LOCK})"), &[]).await?.get(0))
}

async fn run(state: &AppState, collection: Collection, shutdown: &tokio::sync::watch::Receiver<bool>) {
    let mut claimed = None;
    if let Err(error) = run_batches(state, collection, shutdown, &mut claimed).await {
        let message: String = error.downcast_ref::<tokio_postgres::Error>()
            .and_then(tokio_postgres::Error::as_db_error)
            .map_or_else(|| error.to_string(), |db| db.message().to_owned())
            .chars().filter(|ch| !ch.is_control()).take(500).collect();
        let saved = record_failure(state, collection, claimed, &message).await;
        eprintln!("[retention] {}: {message}{}", collection.key(),
            if saved.is_err() { " (could not save failure state)" } else { "" });
    }
}

async fn record_failure(state: &AppState, collection: Collection, claimed: Option<DateTime<Utc>>, message: &str) -> anyhow::Result<()> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    tx.batch_execute("SET LOCAL statement_timeout = '5s'").await?;
    tx.execute(
        "UPDATE retention_state SET run_started_at=NULL,
             next_run_at=CASE WHEN $3::timestamptz IS NULL THEN now()+interval '1 hour' ELSE next_run_at END,
             first_run_pending=CASE WHEN $3::timestamptz IS NULL THEN false ELSE first_run_pending END,
             failure_at=now(), failure_message=$2, failure_count=unreceipted_deleted
         WHERE collection=$1 AND run_started_at IS NOT DISTINCT FROM $3
           AND ($3 IS NOT NULL OR next_run_at <= now())",
        &[&collection.key(), &message, &claimed],
    ).await?;
    tx.commit().await?;
    Ok(())
}

async fn run_batches(
    state: &AppState,
    collection: Collection,
    shutdown: &tokio::sync::watch::Receiver<bool>,
    claimed: &mut Option<DateTime<Utc>>,
) -> anyhow::Result<()> {
    let mut client = state.db.get().await?;
    let key = collection.key();
    let tx = client.transaction().await?;
    let Some((started, due)) = claim_run(&tx, collection).await? else { return Ok(()); };
    tx.commit().await?;
    *claimed = Some(started);

    let tx = client.transaction().await?;
    if !lock(&tx).await? || !owns_run(&tx, key, started).await? { return Ok(()); }
    receipt(&tx, collection).await?;
    if !due {
        tx.execute("UPDATE retention_state SET run_started_at=NULL WHERE collection=$1", &[&key]).await?;
    }
    tx.commit().await?;
    if !due { return Ok(()); }

    let mut run_days = None;
    let mut cutoff = None;
    for _ in 0..20 {
        if *shutdown.borrow() { break; }
        let tx = client.transaction().await?;
        if !lock(&tx).await? { break; }
        let sql = format!(
            "SELECT s.{}, now()-make_interval(days=>s.{}), r.run_started_at=$2
             FROM retention_state r LEFT JOIN app_settings s ON s.id='current' WHERE r.collection=$1",
            collection.policy(), collection.policy(),
        );
        let row = tx.query_one(&sql, &[&key, &started]).await?;
        if row.get::<_, Option<bool>>(2) != Some(true) { return Ok(()); }
        let Some(days) = row.get::<_, Option<i32>>(0) else { break; };
        if run_days.is_some_and(|previous| previous != days) { break; }
        run_days = Some(days);
        let batch_cutoff = row.get(1);
        let deleted = delete_batch(&tx, collection, days, batch_cutoff).await?;
        tx.commit().await?;
        cutoff = Some(batch_cutoff);
        if deleted < collection.batch() { break; }
    }

    let tx = client.transaction().await?;
    finalize_run(&tx, collection, started, cutoff).await?;
    tx.commit().await?;
    Ok(())
}

async fn claim_run(tx: &Transaction<'_>, collection: Collection) -> anyhow::Result<Option<(DateTime<Utc>, bool)>> {
    if !lock(tx).await? { return Ok(None); }
    let sql = format!(
        "WITH work AS (
             SELECT r.collection, (s.{policy} IS NOT NULL AND COALESCE(r.next_run_at <= now(), false)) AS due,
                 r.unreceipted_deleted > 0 AS pending,
                 (r.failure_at IS NULL OR r.failure_at < r.last_cleanup_at
                    OR r.failure_at <= r.last_run_at OR r.failure_at <= now()-interval '1 hour') AS can_recover
             FROM retention_state r LEFT JOIN app_settings s ON s.id='current'
             WHERE r.collection=$1 AND (r.run_started_at IS NULL OR r.run_started_at <= now()-interval '5 minutes')
         )
         UPDATE retention_state r SET run_started_at=now(),
             next_run_at=CASE WHEN w.due THEN now()+interval '1 hour' ELSE r.next_run_at END,
             first_run_pending=CASE WHEN w.due THEN false ELSE r.first_run_pending END
         FROM work w WHERE r.collection=w.collection AND (w.due OR w.pending)
             AND (NOT w.pending OR w.can_recover OR (w.due AND r.first_run_pending))
         RETURNING r.run_started_at, w.due",
        policy = collection.policy(),
    );
    // Reserve the deletion budget before any batch can commit.
    Ok(tx.query_opt(&sql, &[&collection.key()]).await?.map(|row| (row.get(0), row.get(1))))
}

async fn finalize_run(
    tx: &Transaction<'_>, collection: Collection, started: DateTime<Utc>, cutoff: Option<DateTime<Utc>>,
) -> anyhow::Result<()> {
    let key = collection.key();
    if !lock(tx).await? || !owns_run(tx, key, started).await? { return Ok(()); }
    if receipt(tx, collection).await? == 0 {
        tx.execute(
            "UPDATE retention_state SET last_run_at=now(), last_run_count=0, last_run_cutoff=$2 WHERE collection=$1",
            &[&key, &cutoff],
        ).await?;
    }
    tx.execute("UPDATE retention_state SET run_started_at=NULL WHERE collection=$1", &[&key]).await?;
    Ok(())
}

async fn owns_run(tx: &Transaction<'_>, key: &str, started: DateTime<Utc>) -> anyhow::Result<bool> {
    let row = tx.query_one("SELECT run_started_at=$2 FROM retention_state WHERE collection=$1", &[&key, &started]).await?;
    Ok(row.get::<_, Option<bool>>(0) == Some(true))
}

async fn delete_batch(tx: &Transaction<'_>, collection: Collection, days: i32, cutoff: DateTime<Utc>) -> anyhow::Result<i64> {
    let table = collection.table();
    let sql = format!(
        "WITH candidates AS (SELECT id FROM ({}) eligible ORDER BY created_at, id LIMIT $2),
            gone AS (DELETE FROM {table} AS record USING candidates WHERE record.id=candidates.id RETURNING record.id, record.created_at)
         SELECT (SELECT count(*) FROM gone), (SELECT created_at FROM gone ORDER BY created_at DESC, id DESC LIMIT 1),
            (SELECT id FROM gone ORDER BY created_at DESC, id DESC LIMIT 1)",
        collection.candidates(false),
    );
    let row = tx.query_one(&sql, &[&cutoff, &collection.batch()]).await?;
    let deleted: i64 = row.get(0);
    if deleted != 0 {
        let history = matches!(collection, Collection::History);
        let newest_at = history.then(|| row.get::<_, DateTime<Utc>>(1));
        let newest_id = history.then(|| row.get::<_, String>(2));
        tx.execute(
            "UPDATE retention_state SET total_deleted=total_deleted+$2, unreceipted_deleted=unreceipted_deleted+$2,
                 pending_days=$3, pending_cutoff=$4, last_cleanup_at=now(), last_cleanup_count=unreceipted_deleted+$2,
                 newest_deleted_at=CASE WHEN ($5,$6) > (newest_deleted_at,newest_deleted_id) OR newest_deleted_at IS NULL THEN $5 ELSE newest_deleted_at END,
                 newest_deleted_id=CASE WHEN ($5,$6) > (newest_deleted_at,newest_deleted_id) OR newest_deleted_at IS NULL THEN $6 ELSE newest_deleted_id END
             WHERE collection=$1",
            &[&collection.key(), &deleted, &days, &cutoff, &newest_at, &newest_id],
        ).await?;
    }
    Ok(deleted)
}

async fn receipt(tx: &Transaction<'_>, collection: Collection) -> anyhow::Result<i64> {
    let row = tx.query_one(
        "SELECT unreceipted_deleted, pending_cutoff, pending_days FROM retention_state WHERE collection=$1",
        &[&collection.key()],
    ).await?;
    let deleted: i64 = row.get(0);
    if deleted == 0 { return Ok(0); }
    let cutoff = row.get::<_, DateTime<Utc>>(1);
    let days = row.get::<_, i32>(2);
    let id = uuid::Uuid::new_v4().to_string();
    let target = format!("{} · {}건 삭제", collection.label(), grouped_count(deleted));
    audit::write_with_id(tx, &id, &AuditEntry {
        actor_label: Some("system".into()),
        action: "retention:prune".into(),
        target: Some(target),
        after: Some(json!({"collection":collection.key(),"deleted":deleted,"cutoff":cutoff,
            "retentionDays":days,"keptNewest":collection.floor()})),
        ..AuditEntry::default()
    }).await?;
    // Inserting the new audit receipt may make the previously protected one eligible.
    let remaining = collection.count(tx, cutoff).await?;
    tx.execute(
        "UPDATE audit_logs SET \"after\"=jsonb_set(\"after\",'{remaining}',to_jsonb($2::bigint)) WHERE id=$1",
        &[&id, &remaining],
    ).await?;
    tx.execute(
        "UPDATE retention_state SET last_run_at=now(), last_run_count=unreceipted_deleted, last_run_cutoff=pending_cutoff,
             unreceipted_deleted=0, pending_cutoff=NULL, pending_days=NULL WHERE collection=$1",
        &[&collection.key()],
    ).await?;
    Ok(deleted)
}

fn grouped_count(count: i64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) { grouped.push(','); }
        grouped.push(digit);
    }
    grouped
}

pub(crate) async fn estimate(client: &impl GenericClient, collection: Collection, days: i32, include_settings_event: bool) -> Result<Value, tokio_postgres::Error> {
    let cutoff: DateTime<Utc> = client.query_one("SELECT now()-make_interval(days=>$1)", &[&days]).await?.get(0);
    Ok(json!({"cutoff":cutoff, "deleted":collection.count_candidates(client, cutoff, include_settings_event).await?}))
}

#[cfg(test)]
mod tests;

#[cfg(feature = "test-support")]
pub async fn prune_history_for_tests(state: &AppState) {
    run(state, Collection::History, &state.runtime.shutdown.subscribe()).await;
}
