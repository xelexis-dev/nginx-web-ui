use super::{
    logs::identity,
    parser::{self, Counts, MAX_LINE_BYTES, NginxMetrics},
    query::DAY_MS,
};
use crate::state::AppState;
use chrono::{DateTime, Utc};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    task::JoinHandle,
};

const READ_CHUNK: usize = 1024 * 1024;
const MAX_PASS: u64 = 64 * 1024 * 1024;
const BACKFILL: u64 = 256 * 1024 * 1024;
const GRACE_MS: i64 = 90_000;
const MAX_HOSTS: usize = 1000;

#[derive(Default)]
struct ConfiguredHosts {
    exact: HashSet<String>,
    suffix: HashSet<String>,
    prefix: HashSet<String>,
}
impl ConfiguredHosts {
    fn add(&mut self, names: &str) {
        for name in names.split_whitespace() {
            let name = name.to_ascii_lowercase();
            if let Some(suffix) = name.strip_prefix("*.") {
                if !suffix.contains('*') {
                    self.suffix.insert(suffix.into());
                }
            } else if let Some(suffix) = name.strip_prefix('.') {
                if !suffix.contains('*') {
                    self.exact.insert(suffix.into());
                    self.suffix.insert(suffix.into());
                }
            } else if let Some(prefix) = name.strip_suffix(".*") {
                if !prefix.contains('*') {
                    self.prefix.insert(prefix.into());
                }
            } else if !name.contains('*') {
                self.exact.insert(parser::normalize_host(&name));
            }
        }
    }
    fn contains(&self, host: &str) -> bool {
        self.exact.contains(host) || host.match_indices('.').any(|(i, _)| {
            self.suffix.contains(&host[i + 1..]) || self.prefix.contains(&host[..i])
        })
    }
}

pub struct CollectorHandle {
    stop: tokio::sync::watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}
impl CollectorHandle {
    /// Finish an in-flight transaction before returning. Never abort a commit on shutdown.
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}
impl Drop for CollectorHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
pub fn start(state: AppState) -> CollectorHandle {
    let (stop, _) = tokio::sync::watch::channel(false);
    let mut tasks = Vec::new();
    if !state
        .runtime
        .observability_started
        .swap(true, Ordering::AcqRel)
    {
        for gauge in [true, false] {
            let state = state.clone();
            let mut stopped = stop.subscribe();
            let mut shutdown = state.runtime.shutdown.subscribe();
            tasks.push(tokio::spawn(async move {
                let mut timer=tokio::time::interval(Duration::from_secs(60));
                timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    if *stopped.borrow() || *shutdown.borrow(){break;}
                    tokio::select! { _=stopped.changed()=>break, _=shutdown.changed()=>break, _=timer.tick()=>{} }
                    if gauge {
                        if collect_gauge(&state,Utc::now()).await.is_err(){eprintln!("metrics gauge collection failed");}
                    } else {
                        if parse_pass(&state,Utc::now()).await.is_err(){eprintln!("metrics log collection failed");}
                        if rollup_and_prune(&state,Utc::now()).await.is_err(){eprintln!("metrics rollup failed");}
                    }
                }
            }));
        }
    }
    CollectorHandle { stop, tasks }
}
pub async fn read_nginx_metrics(state: &AppState) -> Option<NginxMetrics> {
    // No redirects or proxy environment: this endpoint is a local, trusted runtime setting.
    // A dedicated source keeps stub status unavailable through other proxy/stream paths.
    let client = reqwest::Client::builder()
        .local_address(std::net::IpAddr::from([127,0,0,3]))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let mut response = client
        .get(&state.config.nginx_status_url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 16 * 1024 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    parser::parse_stub_status(std::str::from_utf8(&body).ok()?)
}
pub async fn collect_gauge(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<bool> {
    let Some(m) = read_nginx_metrics(state).await else {
        return Ok(false);
    };
    let bucket =
        DateTime::from_timestamp_millis(now.timestamp_millis().div_euclid(60_000) * 60_000)
            .unwrap();
    state.db.get().await?.execute("INSERT INTO nginx_metric_samples (collected_at,active,reading,writing,waiting,accepts,handled,requests) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING",&[&bucket,&m.active,&m.reading,&m.writing,&m.waiting,&m.accepts,&m.handled,&m.requests]).await?;
    Ok(true)
}
async fn fingerprint(file: &mut File, length: u64) -> anyhow::Result<Option<String>> {
    if length == 0 {
        return Ok(None);
    }
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let mut bytes = vec![0; length.min(256) as usize];
    file.read_exact(&mut bytes).await?;
    Ok(Some(hex::encode(Sha1::digest(&bytes))))
}
#[derive(Debug)]
pub struct PassResult {
    pub offset: u64,
    pub requests: i64,
    pub unparsed: i64,
}

pub async fn parse_pass(
    state: &AppState,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<PassResult>> {
    let _parse = state.runtime.metrics_parse_lock.lock().await;
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    // Snapshot briefly under the same advisory lock as commits and retention.
    tx.execute("SELECT pg_advisory_xact_lock(683291012)", &[])
        .await?;
    let prior=tx.query_opt("SELECT \"offset\",size,ino,head_fingerprint,unparsed_count,jsonb_build_array(\"offset\",size,ino,head_fingerprint,unparsed_count,extract(epoch from last_bucket),extract(epoch from updated_at)) AS checkpoint FROM metric_parse_state WHERE id='current'",&[]).await?;
    // Ignore rollup-only changes; epoch timestamps compare equally across DB session time zones.
    let checkpoint: Option<serde_json::Value> = prior.as_ref().map(|row| row.get("checkpoint"));
    let settings = tx
        .query_opt(
            "SELECT metrics_excluded_hosts FROM app_settings LIMIT 1",
            &[],
        )
        .await?;
    let excluded: HashSet<String> = settings
        .and_then(|r| r.get::<_, Option<String>>(0))
        .unwrap_or_default()
        .split([',', '\n'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let hosts = tx
        .query("SELECT id,host FROM metric_hosts ORDER BY id", &[])
        .await?;
    let mut ids: HashMap<String, i32> =
        hosts.iter().map(|r| (r.get("host"), r.get("id"))).collect();
    let mut configured = ConfiguredHosts::default();
    for row in tx
        .query("SELECT server_name FROM nginx_servers WHERE enabled", &[])
        .await?
    {
        configured.add(row.get::<_, &str>(0));
    }
    let mut admitted: HashSet<String> = ids.keys()
        .filter(|host| configured.contains(host))
        .cloned()
        .collect();
    tx.commit().await?;
    drop(client);
    // No pooled connection is held while opening, fingerprinting or scanning the log.
    let mut file = match File::open(&state.config.nginx_access_log).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata().await?;
    let size = meta.len();
    let ino = identity(&meta);
    let mut offset = 0_u64;
    let mut unparsed = 0_i64;
    let mut discard = false;
    let mut first = true;
    let before = prior.as_ref().map(|row| (
        row.get::<_, i64>("offset"), row.get::<_, i64>("size"),
        row.get::<_, Option<String>>("ino"), row.get::<_, Option<String>>("head_fingerprint"),
        row.get::<_, i64>("unparsed_count"),
    ));
    if let Some(row) = prior {
        unparsed = row.get("unparsed_count");
        let old_ino: Option<String> = row.get("ino");
        first = old_ino.is_none();
        let old_offset = row.get::<_, i64>("offset").max(0) as u64;
        let old_size = row.get::<_, i64>("size").max(0) as u64;
        let old_hash: Option<String> = row.get("head_fingerprint");
        // 'skip:' persists oversized-line discard across a bounded pass/restart; plain SHA1 stays compatible.
        let expected = old_hash
            .as_deref()
            .map(|s| s.strip_prefix("skip:").unwrap_or(s));
        let actual = if size >= old_size.min(256) {
            fingerprint(&mut file, old_size).await?
        } else {
            None
        };
        let same_head = expected.is_none() || actual.as_deref() == expected;
        if old_ino.as_deref() == Some(&ino) && size >= old_offset && same_head {
            offset = old_offset;
            discard = old_hash.as_deref().is_some_and(|s| s.starts_with("skip:"));
        }
    }
    if first && size > BACKFILL {
        offset = size - BACKFILL;
        discard = true;
    }
    let hash = fingerprint(&mut file, size).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let end = size.min(offset.saturating_add(MAX_PASS));
    let mut position = offset;
    let mut line_start = offset;
    let mut line = Vec::new();
    let mut counts = BTreeMap::<(i64, String), Counts>::new();
    let mut held = false;
    let mut buffer = vec![0; READ_CHUNK];
    'read: while position < end {
        let limit = (end - position).min(READ_CHUNK as u64) as usize;
        let n = file.read(&mut buffer[..limit]).await?;
        if n == 0 {
            break;
        }
        for part in buffer[..n].split_inclusive(|c| *c == b'\n') {
            position += part.len() as u64;
            if !discard {
                if line.len() + part.len() > MAX_LINE_BYTES + 1 {
                    line.clear();
                    discard = true;
                    unparsed += 1;
                } else {
                    line.extend_from_slice(part);
                }
            }
            if part.ends_with(b"\n") {
                if !discard {
                    let scan =
                        parser::scan_access_log(&line, now.timestamp_millis(), GRACE_MS, &excluded);
                    if scan.consumed == 0 {
                        offset = line_start;
                        held = true;
                        break 'read;
                    }
                    unparsed += scan.unparsed;
                    for ((time, host), value) in scan.buckets {
                        let host = if !configured.contains(&host) {
                            parser::HOST_OTHER.into()
                        } else if admitted.contains(&host) {
                            host
                        } else if admitted.len() < MAX_HOSTS {
                            admitted.insert(host.clone());
                            host
                        } else {
                            parser::HOST_OTHER.into()
                        };
                        counts.entry((time, host)).or_default().merge(&value);
                    }
                }
                line.clear();
                discard = false;
                offset = position;
                line_start = position;
            }
        }
    }
    if !held && discard {
        offset = position;
    } // rest of this oversized line is discarded next pass
    let total: i64 = counts.values().map(|c| c.requests).sum();
    let last_bucket = counts
        .keys()
        .map(|(t, _)| *t)
        .max()
        .and_then(DateTime::<Utc>::from_timestamp_millis);
    let hash = hash.map(|h| if discard { format!("skip:{h}") } else { h });
    // An unchanged checkpoint with no counts would only rewrite unused updated_at and generate WAL.
    if counts.is_empty()
        && before == Some((offset as i64, size as i64, Some(ino.clone()), hash.clone(), unparsed))
    {
        return Ok(Some(PassResult { offset, requests: 0, unparsed }));
    }
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(683291012)", &[]).await?;
    let current: Option<serde_json::Value> = tx.query_opt(
        "SELECT jsonb_build_array(\"offset\",size,ino,head_fingerprint,unparsed_count,extract(epoch from last_bucket),extract(epoch from updated_at)) FROM metric_parse_state WHERE id='current' FOR UPDATE", &[]
    ).await?.map(|row| row.get(0));
    if current != checkpoint {
        // Another process committed while we scanned; its checkpoint wins. Retry next pass.
        tx.rollback().await?;
        return Ok(None);
    }
    // Retention may have removed unused host IDs during the scan.
    ids = tx.query("SELECT id,host FROM metric_hosts ORDER BY id", &[]).await?
        .iter().map(|row| (row.get("host"), row.get("id"))).collect();
    let mut folded = BTreeMap::<(i64, i32), Counts>::new();
    for ((time, host), count) in counts {
        let key = host;
        let id = if let Some(id) = ids.get(&key) {
            *id
        } else {
            let id:i32=tx.query_one("INSERT INTO metric_hosts(host) VALUES ($1) ON CONFLICT(host) DO UPDATE SET host=excluded.host RETURNING id",&[&key]).await?.get(0);
            ids.insert(key, id);
            id
        };
        folded.entry((time, id)).or_default().merge(&count);
    }
    let mut hourly = BTreeMap::<(i64, i32), Counts>::new();
    for ((time, id), c) in folded {
        let bucket = DateTime::<Utc>::from_timestamp_millis(time).unwrap();
        let requests = i32::try_from(c.requests)?;
        let s2xx = i32::try_from(c.s2xx)?;
        let s3xx = i32::try_from(c.s3xx)?;
        let s4xx = i32::try_from(c.s4xx)?;
        let s5xx = i32::try_from(c.s5xx)?;
        tx.execute("INSERT INTO domain_metric_minutes(bucket,host_id,requests,bytes_sent,s2xx,s3xx,s4xx,s5xx) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(bucket,host_id) DO UPDATE SET requests=domain_metric_minutes.requests+excluded.requests,bytes_sent=domain_metric_minutes.bytes_sent+excluded.bytes_sent,s2xx=domain_metric_minutes.s2xx+excluded.s2xx,s3xx=domain_metric_minutes.s3xx+excluded.s3xx,s4xx=domain_metric_minutes.s4xx+excluded.s4xx,s5xx=domain_metric_minutes.s5xx+excluded.s5xx",&[&bucket,&id,&requests,&c.bytes_sent,&s2xx,&s3xx,&s4xx,&s5xx]).await?;
        let hour = time.div_euclid(3_600_000) * 3_600_000;
        if hour + 3_600_000 <= now.timestamp_millis() {
            hourly.entry((hour, id)).or_default().merge(&c);
        }
    }
    // Backfill may arrive after this hour was rolled up (and its old minute rows pruned).
    // Existing hourly history receives only this transaction's delta. A missing summary uses
    // all available minutes, including checkpoints produced by the legacy collector.
    for ((hour, id), c) in hourly {
        let bucket = DateTime::<Utc>::from_timestamp_millis(hour).unwrap();
        let end = bucket + chrono::Duration::hours(1);
        tx.execute("INSERT INTO domain_metric_hours(bucket,host_id,requests,bytes_sent,s2xx,s3xx,s4xx,s5xx) SELECT $1,$2,sum(requests),sum(bytes_sent),sum(s2xx),sum(s3xx),sum(s4xx),sum(s5xx) FROM domain_metric_minutes WHERE bucket >= $1 AND bucket < $3 AND host_id=$2 ON CONFLICT(bucket,host_id) DO UPDATE SET requests=domain_metric_hours.requests+$4,bytes_sent=domain_metric_hours.bytes_sent+$5,s2xx=domain_metric_hours.s2xx+$6,s3xx=domain_metric_hours.s3xx+$7,s4xx=domain_metric_hours.s4xx+$8,s5xx=domain_metric_hours.s5xx+$9",&[&bucket,&id,&end,&c.requests,&c.bytes_sent,&c.s2xx,&c.s3xx,&c.s4xx,&c.s5xx]).await?;
    }
    tx.execute("INSERT INTO metric_parse_state(id,\"offset\",size,ino,head_fingerprint,last_bucket,unparsed_count,updated_at) VALUES ('current',$1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO UPDATE SET \"offset\"=excluded.\"offset\",size=excluded.size,ino=excluded.ino,head_fingerprint=excluded.head_fingerprint,last_bucket=coalesce(excluded.last_bucket,metric_parse_state.last_bucket),unparsed_count=excluded.unparsed_count,updated_at=excluded.updated_at",&[&(offset as i64),&(size as i64),&ino,&hash,&last_bucket,&unparsed,&now]).await?;
    tx.commit().await?;
    Ok(Some(PassResult {
        offset,
        requests: total,
        unparsed,
    }))
}

pub async fn rollup_and_prune(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<()> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(683291012)", &[])
        .await?;
    let last: Option<DateTime<Utc>> = tx
        .query_opt(
            "SELECT last_rollup_at FROM metric_parse_state WHERE id='current'",
            &[],
        )
        .await?
        .and_then(|r| r.get(0));
    if last.is_some_and(|d| now - d < chrono::Duration::hours(1)) {
        return Ok(());
    }
    let end = DateTime::<Utc>::from_timestamp_millis(
        now.timestamp_millis().div_euclid(3_600_000) * 3_600_000,
    )
    .unwrap();
    let min = end - chrono::Duration::days(40);
    let from = last
        .map(|d| d - chrono::Duration::hours(3))
        .unwrap_or(min)
        .max(min);
    let from = DateTime::<Utc>::from_timestamp_millis(
        from.timestamp_millis().div_euclid(3_600_000) * 3_600_000,
    )
    .unwrap();
    let full_minutes_from = now - chrono::Duration::days(30);
    tx.execute("INSERT INTO domain_metric_hours(bucket,host_id,requests,bytes_sent,s2xx,s3xx,s4xx,s5xx) SELECT date_trunc('hour',bucket,'UTC'),host_id,sum(requests),sum(bytes_sent),sum(s2xx),sum(s3xx),sum(s4xx),sum(s5xx) FROM domain_metric_minutes WHERE bucket >= $1 AND bucket < $2 GROUP BY 1,host_id ON CONFLICT(bucket,host_id) DO UPDATE SET requests=excluded.requests,bytes_sent=excluded.bytes_sent,s2xx=excluded.s2xx,s3xx=excluded.s3xx,s4xx=excluded.s4xx,s5xx=excluded.s5xx WHERE excluded.bucket >= $3 OR excluded.requests >= domain_metric_hours.requests",&[&from,&end,&full_minutes_from]).await?;
    prune(
        &*tx,
        "domain_metric_minutes",
        "bucket",
        now - chrono::Duration::milliseconds(30 * DAY_MS),
    )
    .await?;
    let retention: Option<i32> = tx
        .query_opt(
            "SELECT metrics_retention_days FROM app_settings LIMIT 1",
            &[],
        )
        .await?
        .and_then(|r| r.get(0));
    if let Some(days) = retention.filter(|d| *d > 0) {
        let cutoff = now - chrono::Duration::days(days.into());
        prune(&*tx, "domain_metric_hours", "bucket", cutoff).await?;
        prune(&*tx, "nginx_metric_samples", "collected_at", cutoff).await?;
        tx.execute("DELETE FROM metric_hosts h WHERE NOT EXISTS(SELECT 1 FROM domain_metric_minutes m WHERE m.host_id=h.id) AND NOT EXISTS(SELECT 1 FROM domain_metric_hours m WHERE m.host_id=h.id)",&[]).await?;
    }
    tx.execute("INSERT INTO metric_parse_state(id,last_rollup_at) VALUES ('current',$1) ON CONFLICT(id) DO UPDATE SET last_rollup_at=excluded.last_rollup_at",&[&now]).await?;
    tx.commit().await?;
    Ok(())
}
async fn prune(
    client: &impl tokio_postgres::GenericClient,
    table: &str,
    column: &str,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<()> {
    for _ in 0..200 {
        let n=client.execute(&format!("DELETE FROM {table} WHERE ctid IN (SELECT ctid FROM {table} WHERE {column}<$1 LIMIT 20000)"),&[&cutoff]).await?;
        if n < 20000 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod admission_tests {
    use super::ConfiguredHosts;
    #[test]
    fn audit_e6_s17_configured_name_boundaries() {
        let mut names=ConfiguredHosts::default();
        names.add("Exact.TEST *.wild.test .both.test mail.* _");
        for name in ["exact.test","a.wild.test","a.b.wild.test","both.test","a.both.test","mail.example","mail.a.example","__default_vhost__"] {
            assert!(names.contains(name),"{name}");
        }
        for name in ["wild.test","notwild.test","notboth.test","mail","notmail.example","junk.invalid"] {
            assert!(!names.contains(name),"{name}");
        }
        names.add("* in*valid.test");
        assert!(!names.contains("junk.invalid"));
    }
}
