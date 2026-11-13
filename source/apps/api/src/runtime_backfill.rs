//! Explicit, opt-in conversion of legacy plaintext secrets.
use crate::{
    crypto::{SecretBox, is_sealed},
    state::AppState,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackfillResult {
    pub cert_keys: u64,
    pub account_keys: u64,
    pub totp_secrets: u64,
    pub dns_orders: u64,
    pub failed: u64,
}

/// Failure is deliberately a count, never a database error containing secrets.
/// Each group commits atomically, and row locks prevent concurrent writes from
/// replacing a newer key or sealing the same value twice.
pub async fn run_if_requested(state: &AppState, requested: Option<&str>) -> Option<BackfillResult> {
    run_with(&state.db, &state.secrets, requested).await
}
pub(crate) async fn run_with(pool: &deadpool_postgres::Pool, secrets: &SecretBox, requested: Option<&str>) -> Option<BackfillResult> {
    if requested != Some("1") {
        return None;
    }
    let mut result = BackfillResult::default();
    match certificates(pool, secrets).await {
        Ok((keys, orders)) => {
            result.cert_keys = keys;
            result.dns_orders = orders;
        }
        Err(_) => {
            result.failed += 1;
            eprintln!(
                "[backfill] Certificate secret conversion could not be confirmed; retry is safe."
            );
        }
    }
    match scalar(pool, secrets, true).await {
        Ok(count) => result.account_keys = count,
        Err(_) => {
            result.failed += 1;
            eprintln!(
                "[backfill] ACME account secret conversion could not be confirmed; retry is safe."
            );
        }
    }
    match scalar(pool, secrets, false).await {
        Ok(count) => result.totp_secrets = count,
        Err(_) => {
            result.failed += 1;
            eprintln!("[backfill] TOTP secret conversion could not be confirmed; retry is safe.");
        }
    }
    eprintln!(
        "[backfill] Completed: certificate keys {}, account keys {}, TOTP secrets {}, DNS orders {}, failed groups {}. Remove ENCRYPT_BACKFILL after verification.",
        result.cert_keys,
        result.account_keys,
        result.totp_secrets,
        result.dns_orders,
        result.failed
    );
    Some(result)
}

fn seal_plain(secrets: &SecretBox, value: Option<&str>) -> Result<Option<String>> {
    value
        .filter(|v| !v.is_empty() && !is_sealed(v))
        .map(|v| secrets.seal(v).map_err(Into::into))
        .transpose()
}

async fn certificates(pool: &deadpool_postgres::Pool, secrets: &SecretBox) -> Result<(u64, u64)> {
    let mut db = pool.get().await?;
    let tx = db.transaction().await?;
    tx.batch_execute("SET LOCAL statement_timeout='5s'").await?;
    let mut keys = 0;
    let mut orders = 0;
    for row in tx
        .query(
            "SELECT id,key_pem,dns_order FROM certificates ORDER BY id FOR UPDATE",
            &[],
        )
        .await?
    {
        let id: String = row.get(0);
        let old_key: Option<String> = row.get(1);
        let mut order: Option<Value> = row.get(2);
        let key = seal_plain(secrets, old_key.as_deref())?;
        let order_key = seal_plain(
            secrets,
            order
                .as_ref()
                .and_then(|v| v.get("keyPem"))
                .and_then(Value::as_str),
        )?;
        if key.is_none() && order_key.is_none() {
            continue;
        }
        if key.is_some() {
            keys += 1;
        }
        if let Some(value) = order_key {
            order.as_mut().expect("order key implies an object")["keyPem"] = Value::String(value);
            orders += 1;
        }
        tx.execute(
            "UPDATE certificates SET key_pem=$2,dns_order=$3 WHERE id=$1",
            &[&id, &key.or(old_key), &order],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((keys, orders))
}

async fn scalar(pool: &deadpool_postgres::Pool, secrets: &SecretBox, account: bool) -> Result<u64> {
    let mut db = pool.get().await?;
    let tx = db.transaction().await?;
    tx.batch_execute("SET LOCAL statement_timeout='5s'").await?;
    let (select, update) = if account {
        (
            "SELECT id,account_key_pem FROM acme_accounts ORDER BY id FOR UPDATE",
            "UPDATE acme_accounts SET account_key_pem=$2 WHERE id=$1",
        )
    } else {
        (
            "SELECT id,totp_secret FROM users ORDER BY id FOR UPDATE",
            "UPDATE users SET totp_secret=$2 WHERE id=$1",
        )
    };
    let mut count = 0;
    for row in tx.query(select, &[]).await? {
        let id: String = row.get(0);
        let old: Option<String> = row.get(1);
        if let Some(value) = seal_plain(secrets, old.as_deref())? {
            tx.execute(update, &[&id, &value]).await?;
            count += 1;
        }
    }
    tx.commit().await?;
    Ok(count)
}
