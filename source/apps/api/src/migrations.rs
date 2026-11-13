//! Reads the existing Drizzle journal without rewriting history. The transaction-scoped
//! PostgreSQL lock also covers metadata DDL, making simultaneous API starts safe.
use anyhow::{Context, ensure};
use deadpool_postgres::Pool;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

const MIGRATION_LOCK: i64 = 0x4e575549_4d494752;

#[derive(Deserialize)]
struct Journal { version: String, dialect: String, entries: Vec<Entry> }
#[derive(Deserialize)]
struct Entry { idx: usize, when: i64, tag: String }

pub struct Migration {
    pub tag: String,
    pub timestamp: i64,
    pub hash: String,
    pub statements: Vec<String>,
}

#[derive(Debug)]
pub struct NewerDatabase {
    pub applied: i64,
    pub packaged: Option<i64>,
}

impl std::fmt::Display for NewerDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let packaged = self.packaged.map(|time| time.to_string()).unwrap_or_else(|| "none".into());
        write!(f, "database was used by a newer image (newest applied migration {} > this image's newest {}); start the newer image, or restore the DB and app backup pair taken before the update", self.applied, packaged)
    }
}
impl std::error::Error for NewerDatabase {}

pub async fn read_migrations(directory: &Path) -> anyhow::Result<Vec<Migration>> {
    let bytes = tokio::fs::read(directory.join("meta/_journal.json")).await.context("read migration journal")?;
    let journal: Journal = serde_json::from_slice(&bytes).context("parse migration journal")?;
    ensure!(journal.version == "7" && journal.dialect == "postgresql", "unsupported migration journal format");
    let mut last = None;
    let mut tags = HashSet::new();
    for (index, entry) in journal.entries.iter().enumerate() {
        ensure!(entry.idx == index, "migration indices must be contiguous");
        ensure!(entry.when > 0 && last.is_none_or(|time| entry.when > time), "migration timestamps must increase");
        ensure!(!entry.tag.is_empty() && entry.tag.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'), "invalid migration tag");
        ensure!(tags.insert(&entry.tag), "duplicate migration tag");
        last = Some(entry.when);
    }
    let mut migrations = Vec::with_capacity(journal.entries.len());
    for entry in journal.entries {
        let bytes = tokio::fs::read(directory.join(format!("{}.sql", entry.tag))).await
            .with_context(|| format!("read migration {}", entry.tag))?;
        let hash = hex::encode(Sha256::digest(&bytes));
        let sql = String::from_utf8(bytes).context("migration SQL is not UTF-8")?;
        migrations.push(Migration {
            tag: entry.tag, timestamp: entry.when, hash,
            statements: sql.split("--> statement-breakpoint").map(str::to_owned).collect(),
        });
    }
    Ok(migrations)
}

pub async fn run(pool: &Pool, directory: &Path) -> anyhow::Result<usize> {
    let migrations = read_migrations(directory).await?;
    let mut client = pool.get().await.context("acquire migration database connection")?;
    let transaction = client.transaction().await.context("begin migration transaction")?;
    transaction.batch_execute("SET LOCAL lock_timeout = '30s'").await?;
    // A session lock could leak into the pool if its future were cancelled before unlock.
    // A transaction lock is released on commit, rollback, cancellation or connection loss.
    transaction.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK]).await?;
    transaction.batch_execute(
        "CREATE SCHEMA IF NOT EXISTS drizzle; \
         CREATE TABLE IF NOT EXISTS drizzle.__drizzle_migrations \
         (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at bigint)"
    ).await.context("initialize migration history")?;
    let rows = transaction.query("SELECT created_at FROM drizzle.__drizzle_migrations ORDER BY created_at", &[]).await?;
    let mut applied_times = HashSet::new();
    for row in rows {
        let timestamp: Option<i64> = row.try_get(0)?;
        let timestamp = timestamp.context("migration history contains a null timestamp")?;
        // Legacy Drizzle boots had no lock and could record an idempotent migration twice.
        // Preserve those rows; a duplicate does not change which timestamps are applied.
        if !applied_times.insert(timestamp) {
            eprintln!("[api] duplicate historical migration timestamp; preserving existing history");
        }
    }
    let latest = applied_times.iter().copied().max();
    let packaged = migrations.last().map(|migration| migration.timestamp);
    if let Some(applied) = latest {
        if packaged.is_none_or(|packaged| applied > packaged) {
            return Err(NewerDatabase { applied, packaged }.into());
        }
    }
    let mut applied = 0;
    for migration in migrations {
        if latest.is_some_and(|time| migration.timestamp <= time) {
            ensure!(applied_times.contains(&migration.timestamp), "migration {} is missing below the latest applied timestamp", migration.tag);
            // Drizzle records but never compares historical hashes. Preserve that contract.
            continue;
        }
        for statement in &migration.statements {
            transaction.batch_execute(statement).await.with_context(|| format!("apply migration {}", migration.tag))?;
        }
        transaction.execute(
            "INSERT INTO drizzle.__drizzle_migrations (hash, created_at) VALUES ($1, $2)",
            &[&migration.hash, &migration.timestamp],
        ).await?;
        applied += 1;
    }
    transaction.commit().await.context("commit migrations")?;
    Ok(applied)
}
