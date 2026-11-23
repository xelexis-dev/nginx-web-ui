use crate::{auth::principal::db_unavailable, error::ApiError, state::AppState};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

pub const DAY_MS: i64 = 86_400_000;
pub fn range(raw: Option<&str>) -> (&'static str, i64) {
    match raw {
        Some("1h") => ("1h", 3_600_000),
        Some("6h") => ("6h", 21_600_000),
        Some("7d") => ("7d", 7 * DAY_MS),
        Some("30d") => ("30d", 30 * DAY_MS),
        Some("90d") => ("90d", 90 * DAY_MS),
        Some("1y") => ("1y", 365 * DAY_MS),
        _ => ("24h", DAY_MS),
    }
}
pub fn bucket_seconds(range_ms: i64, source_seconds: i64) -> i64 {
    let needed = (range_ms + 499_999) / 500_000;
    ((needed + source_seconds - 1) / source_seconds).max(1) * source_seconds
}
#[derive(Default)]
pub struct MetricsCache {
    entries: HashMap<&'static str, (std::time::Instant, Value)>,
}
pub async fn metrics(state: &AppState, raw: Option<&str>) -> Result<Value, ApiError> {
    let (key, ms) = range(raw);
    let cache = state.runtime.metrics_cache.lock().await;
    if let Some((at, value)) = cache.entries.get(key) {
        if at.elapsed().as_secs() < 60 {
            return Ok(value.clone());
        }
    }
    drop(cache);
    let value = metrics_at(state, key, ms, Utc::now()).await?;
    state.runtime.metrics_cache.lock().await
        .entries
        .insert(key, (std::time::Instant::now(), value.clone()));
    Ok(value)
}
pub async fn metrics_at(
    state: &AppState,
    key: &str,
    ms: i64,
    now: DateTime<Utc>,
) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let (global, domains, health) = tokio::try_join!(
        global_series(&**client, ms, now),
        domain_series(&**client, ms, now),
        collector_health(&**client, now)
    )
    .map_err(|_| db_unavailable())?;
    Ok(json!({"range":key,"global":global,"domains":domains,"health":health}))
}
pub async fn global_series(
    client: &impl tokio_postgres::GenericClient,
    ms: i64,
    now: DateTime<Utc>,
) -> Result<Value, tokio_postgres::Error> {
    let from = now - chrono::Duration::milliseconds(ms);
    let width = bucket_seconds(ms, 60) as f64;
    let rows=client.query("WITH d AS (SELECT collected_at,active, requests-lag(requests) OVER w AS dr, accepts-lag(accepts) OVER w AS da, handled-lag(handled) OVER w AS dh, extract(epoch FROM collected_at-lag(collected_at) OVER w)::float8 AS dt FROM nginx_metric_samples WHERE collected_at >= $1 AND collected_at < $2 WINDOW w AS (ORDER BY collected_at)) SELECT (floor(extract(epoch FROM collected_at)/$3::float8)*$3::float8*1000)::float8 AS t, avg(active)::float8 AS active, avg(CASE WHEN dr>=0 AND dt>0 AND dt<=150 THEN dr/dt END)::float8 AS r, avg(CASE WHEN da>=0 AND dt>0 AND dt<=150 THEN da/dt END)::float8 AS c, avg(CASE WHEN da>=0 AND dh>=0 AND dt>0 AND dt<=150 THEN (da-dh)/dt END)::float8 AS d FROM d GROUP BY 1 ORDER BY 1",&[&from,&now,&width]).await?;
    Ok(Value::Array(rows.iter().map(|r|json!({"t":r.get::<_,f64>("t"),"active":r.get::<_,Option<f64>>("active"),"requestsPerSec":r.get::<_,Option<f64>>("r"),"connectionsPerSec":r.get::<_,Option<f64>>("c"),"droppedPerSec":r.get::<_,Option<f64>>("d")})).collect()))
}
pub async fn domain_series(
    client: &impl tokio_postgres::GenericClient,
    ms: i64,
    now: DateTime<Utc>,
) -> Result<Value, tokio_postgres::Error> {
    let hours = ms > 30 * DAY_MS;
    let table = if hours {
        "domain_metric_hours"
    } else {
        "domain_metric_minutes"
    };
    let from = now - chrono::Duration::milliseconds(ms);
    let width = bucket_seconds(ms, if hours { 3600 } else { 60 });
    let ranking=client.query(&format!("SELECT h.host,sum(m.requests)::float8 requests,sum(m.bytes_sent)::float8 bytes,sum(m.s4xx)::float8 s4xx,sum(m.s5xx)::float8 s5xx FROM {table} m JOIN metric_hosts h ON h.id=m.host_id WHERE m.bucket >= $1 AND m.bucket < $2 GROUP BY h.host ORDER BY requests DESC,h.host"),&[&from,&now]).await?;
    let top: Vec<String> = ranking.iter().take(5).map(|r| r.get("host")).collect();
    let width_f = width as f64;
    let rows=client.query(&format!("SELECT h.host,(floor(extract(epoch FROM m.bucket)/$3::float8)*$3::float8)::int8 AS t,sum(m.requests)::float8 AS n FROM {table} m JOIN metric_hosts h ON h.id=m.host_id WHERE m.bucket >= $1 AND m.bucket < $2 AND h.host=ANY($4) GROUP BY h.host,t ORDER BY t"),&[&from,&now,&width_f,&top]).await?;
    let start = from.timestamp().div_euclid(width) * width;
    let end = (now.timestamp() - 1).div_euclid(width) * width;
    let times: Vec<i64> = (start..=end)
        .step_by(width as usize)
        .map(|t| t * 1000)
        .collect();
    let index: BTreeMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut points: HashMap<String, Vec<f64>> = top
        .iter()
        .map(|h| (h.clone(), vec![0.; times.len()]))
        .collect();
    for row in rows {
        if let (Some(p), Some(i)) = (
            points.get_mut(&row.get::<_, String>("host")),
            index.get(&(row.get::<_, i64>("t") * 1000)),
        ) {
            p[*i] = row.get("n");
        }
    }
    let series:Vec<Value>=ranking.iter().take(5).map(|r| { let host:String=r.get("host");json!({"points":points.remove(&host).unwrap_or_default(),"host":host,"total":r.get::<_,f64>("requests")}) }).collect();
    let other: f64 = ranking
        .iter()
        .skip(5)
        .map(|r| r.get::<_, f64>("requests"))
        .sum();
    let ranks:Vec<Value>=ranking.iter().map(|r|json!({"host":r.get::<_,String>("host"),"requests":r.get::<_,f64>("requests"),"bytesSent":r.get::<_,f64>("bytes"),"s4xx":r.get::<_,f64>("s4xx"),"s5xx":r.get::<_,f64>("s5xx")})).collect();
    Ok(
        json!({"times":times,"series":series,"otherTotal":other,"ranking":ranks,"source":if hours {"hours"}else{"minutes"}}),
    )
}
pub async fn collector_health(
    client: &impl tokio_postgres::GenericClient,
    now: DateTime<Utc>,
) -> Result<Value, tokio_postgres::Error> {
    let from = now - chrono::Duration::hours(6);
    let r=client.query_one("SELECT (SELECT max(collected_at) FROM nginx_metric_samples) AS sample,(SELECT last_bucket FROM metric_parse_state WHERE id='current') AS bucket,coalesce((SELECT unparsed_count FROM metric_parse_state WHERE id='current'),0)::int8 AS unparsed",&[]).await?;
    let span=client.query_one("WITH s AS (SELECT max(requests)-min(requests) AS delta,count(*) AS n,min(collected_at) AS lo,max(collected_at) AS hi FROM nginx_metric_samples WHERE collected_at >= $1 AND collected_at <= $2) SELECT CASE WHEN n>=30 AND delta>0 THEN (SELECT coalesce(sum(requests),0) FROM domain_metric_minutes WHERE bucket>=s.lo AND bucket<s.hi)::float8 / delta::float8 END AS ratio FROM s",&[&from,&now]).await?;
    Ok(
        json!({"lastSampleAt":r.get::<_,Option<DateTime<Utc>>>("sample").map(|d|d.timestamp_millis()),"lastBucketAt":r.get::<_,Option<DateTime<Utc>>>("bucket").map(|d|d.timestamp_millis()),"unparsedCount":r.get::<_,i64>("unparsed"),"parseRatio":span.get::<_,Option<f64>>("ratio")}),
    )
}

fn audit_row(r: &tokio_postgres::Row, detail: bool) -> Value {
    let mut v = json!({"id":r.get::<_,String>("id"),"createdAt":r.get::<_,DateTime<Utc>>("created_at").to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"action":r.get::<_,String>("action"),"target":r.get::<_,Option<String>>("target"),"actorEmail":r.get::<_,Option<String>>("email"),"actorLabel":r.get::<_,Option<String>>("actor_label"),"ip":r.get::<_,Option<String>>("ip"),"via":r.get::<_,Option<Value>>("via")});
    if detail {
        v["before"] = r.get::<_, Option<Value>>("before").unwrap_or(Value::Null);
        v["after"] = r.get::<_, Option<Value>>("after").unwrap_or(Value::Null);
    }
    v
}
pub async fn audit_list(
    state: &AppState,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let action = query
        .get("action")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let page = query
        .get("page")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n > 0 && *n <= i64::MAX / 50)
        .unwrap_or(1);
    let offset = (page - 1) * 50;
    let rows=client.query("SELECT a.id,a.created_at,a.action,a.target,CASE WHEN u.email IS NOT NULL THEN COALESCE(a.actor_label,u.email) END AS email,a.actor_label,a.ip,a.via FROM audit_logs a LEFT JOIN users u ON u.id=a.actor_id WHERE ($1::text IS NULL OR a.action=$1) ORDER BY a.created_at DESC,a.id DESC LIMIT 50 OFFSET $2",&[&action,&offset]).await.map_err(|_|db_unavailable())?;
    let groups = client
        .query(
            "SELECT action, count(*) FROM audit_logs GROUP BY action ORDER BY action",
            &[],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let mut total = 0_i64;
    let mut actions = Vec::with_capacity(groups.len());
    for group in groups {
        let name: String = group.get(0);
        if action.is_none() || action == Some(name.as_str()) {
            total += group.get::<_, i64>(1);
        }
        actions.push(name);
    }
    Ok(
        json!({"items":rows.iter().map(|r|audit_row(r,false)).collect::<Vec<_>>(),"total":total,"page":page,"totalPages":((total+49)/50).max(1),"actions":actions,"actionFilter":action.unwrap_or("")}),
    )
}
pub async fn audit_detail(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let row=client.query_opt("SELECT a.id,a.created_at,a.action,a.target,a.before,a.after,a.ip,CASE WHEN u.email IS NOT NULL THEN COALESCE(a.actor_label,u.email) END AS email,a.actor_label,a.via FROM audit_logs a LEFT JOIN users u ON u.id=a.actor_id WHERE a.id=$1",&[&id]).await.map_err(|_|db_unavailable())?.ok_or_else(ApiError::not_found)?;
    Ok(json!({"log":audit_row(&row,true)}))
}
