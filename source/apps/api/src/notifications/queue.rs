//! Durable per-target delivery. Certificate transactions only write notifications; this worker
//! observes committed events. External delivery is at least once across an ambiguous crash.
use super::{
    mail::mail_content,
    smtp::{self, MailInput},
};
use crate::{
    notify::{NotificationEvent, RenderContext, send, target},
    state::AppState,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const BATCH: i64 = 4;
const LEASE_SECONDS: i64 = 60;

pub async fn plan(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    // Unconfigured renewal reminders remain discoverable without starving other events.
    let events=tx.query("SELECT id,type FROM notifications n WHERE delivery_planned_at IS NULL AND (type<>'cert.renewal_due' OR EXISTS(SELECT 1 FROM webhook_targets t WHERE t.enabled AND t.events ? n.type) OR EXISTS(SELECT 1 FROM mail_recipients r WHERE r.enabled AND r.events ? n.type)) ORDER BY created_at,id LIMIT 100 FOR UPDATE SKIP LOCKED",&[]).await?;
    let mut count = 0;
    for event in events {
        let id: String = event.get("id");
        let event_type: String = event.get("type");
        for (channel, table) in [("mail", "mail_recipients"), ("webhook", "webhook_targets")] {
            let targets = tx
                .query(
                    &format!("SELECT id FROM {table} WHERE enabled AND events ? $1 ORDER BY id"),
                    &[&event_type],
                )
                .await?;
            for target in targets {
                let target: String = target.get(0);
                let delivery = uuid::Uuid::new_v4().to_string();
                count+=tx.execute("INSERT INTO notification_deliveries(id,notification_id,channel,target_id,next_attempt_at) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(notification_id,channel,target_id) DO NOTHING",&[&delivery,&id,&channel,&target,&now]).await? as usize;
            }
        }
        tx.execute(
            "UPDATE notifications SET delivery_planned_at=$1 WHERE id=$2",
            &[&now, &id],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(count)
}

struct Delivery {
    id: String,
    notification_id: String,
    channel: String,
    target_id: String,
    owner: String,
    attempts: i32,
    event: NotificationEvent,
}
async fn claim(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<Vec<Delivery>> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    // Retention and lease acquisition must not race: pruning sees a committed lease.
    tx.execute("SELECT pg_advisory_xact_lock(71203841)", &[])
        .await?;
    let rows=tx.query("SELECT id,notification_id,channel,target_id,attempts FROM notification_deliveries WHERE (status='pending' AND next_attempt_at<=$1) OR (status='delivering' AND lease_until<=$1) ORDER BY next_attempt_at,id LIMIT $2 FOR UPDATE SKIP LOCKED",&[&now,&BATCH]).await?;
    let mut work = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        let notification_id: String = row.get("notification_id");
        let owner = uuid::Uuid::new_v4().to_string();
        let attempts = row.get::<_, i32>("attempts").saturating_add(1);
        let lease = now + chrono::Duration::seconds(LEASE_SECONDS);
        tx.execute("UPDATE notification_deliveries SET status='delivering',attempts=$1,lease_until=$2,lease_owner=$3,last_attempt_at=$4 WHERE id=$5",&[&attempts,&lease,&owner,&now,&id]).await?;
        let n=tx.query_one("SELECT type,severity::text,title,body,link,created_at,data FROM notifications WHERE id=$1",&[&notification_id]).await?;
        let event = NotificationEvent {
            event: n.get("type"),
            severity: n.get("severity"),
            title: n.get("title"),
            message: n.get("body"),
            path: n.get("link"),
            occurred_at_ms: n.get::<_, DateTime<Utc>>("created_at").timestamp_millis(),
            data: n
                .get::<_, Option<Value>>("data")
                .and_then(|v| v.as_object().cloned()),
        };
        work.push(Delivery {
            id,
            notification_id,
            channel: row.get("channel"),
            target_id: row.get("target_id"),
            owner,
            attempts,
            event,
        });
    }
    tx.commit().await?;
    Ok(work)
}
struct Outcome {
    ok: bool,
    cancelled: bool,
    retryable: bool,
    record_target: bool,
    error: Option<String>,
    status: Option<i32>,
}
impl Outcome {
    fn cancelled() -> Self {
        Self {
            ok: false,
            cancelled: true,
            retryable: false,
            record_target: false,
            error: None,
            status: None,
        }
    }
    fn failed(error: impl Into<String>, retryable: bool) -> Self {
        Self {
            ok: false,
            cancelled: false,
            retryable,
            record_target: true,
            error: Some(error.into()),
            status: None,
        }
    }
}
fn renewal_identity(event: &NotificationEvent) -> Option<(&str, DateTime<Utc>)> {
    let data = event.data.as_ref()?;
    let id = data.get("certificateId")?.as_str()?;
    let expires = DateTime::parse_from_rfc3339(data.get("expiresAt")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    Some((id, expires))
}
async fn deliver(state: &AppState, work: &Delivery) -> anyhow::Result<Outcome> {
    let client = state.db.get().await?;
    if work.event.event == "cert.renewal_due" {
        let Some((id, expires)) = renewal_identity(&work.event) else {
            return Ok(Outcome::cancelled());
        };
        if !client.query_one("SELECT EXISTS(SELECT 1 FROM certificates WHERE id=$1 AND expires_at=$2 AND status='issued' AND source='acme' AND challenge_type='dns-01' AND dns_provider IS NULL)",&[&id,&expires]).await?.get::<_,bool>(0){return Ok(Outcome::cancelled());}
    }
    let base: Option<String> = client
        .query_opt(
            "SELECT public_base_url FROM app_settings WHERE id='current'",
            &[],
        )
        .await?
        .and_then(|r| r.get(0));
    let ctx = RenderContext {
        base_url: base.as_deref(),
    };
    if work.channel == "webhook" {
        let row=client.query_opt("SELECT url_enc,headers_enc,format::text,body_template FROM webhook_targets WHERE id=$1 AND enabled AND events ? $2",&[&work.target_id,&work.event.event]).await?;
        let Some(row) = row else {
            return Ok(Outcome::cancelled());
        };
        let config = target::open_target(
            &state.secrets,
            row.get("url_enc"),
            row.get("headers_enc"),
            row.get("format"),
            row.get("body_template"),
        );
        drop(client);
        let mut config = match config {
            Ok(c) => c,
            Err(e) => return Ok(Outcome::failed(e, true)),
        };
        config
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("idempotency-key"));
        config
            .headers
            .push(("Idempotency-Key".into(), format!("nwui-{}", work.id)));
        let result = target::deliver_with_config(
            &state.webhook_http,
            &config,
            &work.event,
            ctx,
            &send::SendOptions {
                timeout: std::time::Duration::from_secs(10),
                attempts: 1,
                backoff: vec![],
            },
        )
        .await;
        // A receiver may echo credentials in its body or error; only status/category is persisted.
        let error = if result.ok {
            None
        } else {
            Some(match result.status {
                Some(s) => format!("웹훅 전송 실패 (HTTP {s})."),
                None => "웹훅 연결 또는 요청을 확인하세요.".into(),
            })
        };
        Ok(Outcome {
            ok: result.ok,
            cancelled: false,
            retryable: result.status.is_none_or(|s| s == 429 || s >= 500),
            record_target: true,
            error,
            status: result.status.map(i32::from),
        })
    } else {
        let recipient = client
            .query_opt(
                "SELECT address FROM mail_recipients WHERE id=$1 AND enabled AND events ? $2",
                &[&work.target_id, &work.event.event],
            )
            .await?;
        let Some(recipient) = recipient else {
            return Ok(Outcome::cancelled());
        };
        let address: String = recipient.get(0);
        drop(client);
        let config = match smtp::mailer_config(state).await {
            Ok(c) => c,
            Err(e) => {
                let mut outcome = Outcome::failed(e, true);
                outcome.record_target = false;
                return Ok(outcome);
            }
        };
        let content = mail_content(&work.event, ctx);
        let result = smtp::send_with_config(
            &config,
            &MailInput {
                to: address,
                subject: content.subject,
                text: content.text,
                html: Some(content.html),
                message_id: Some(format!("<nwui-{}@nginx-web-ui.local>", work.id)),
            },
            &state.config,
        )
        .await;
        Ok(Outcome {
            ok: result.ok,
            cancelled: false,
            retryable: result.retryable,
            record_target: true,
            error: result.error,
            status: None,
        })
    }
}
async fn finish(
    state: &AppState,
    work: &Delivery,
    outcome: Outcome,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    let status = if outcome.cancelled {
        "cancelled"
    } else if outcome.ok {
        "delivered"
    } else if outcome.retryable {
        "pending"
    } else {
        "failed"
    };
    let next = now
        + chrono::Duration::seconds(
            (60_i64.saturating_mul(1_i64 << work.attempts.saturating_sub(1).min(9))).min(6 * 3600),
        );
    let delivered = outcome.ok.then_some(now);
    let changed=tx.execute("UPDATE notification_deliveries SET status=$1,next_attempt_at=$2,lease_until=NULL,lease_owner=NULL,delivered_at=$3,last_error=$4,http_status=$5 WHERE id=$6 AND status='delivering' AND lease_owner=$7",&[&status,&next,&delivered,&outcome.error,&outcome.status,&work.id,&work.owner]).await?;
    if changed == 0 {
        return Ok(());
    }
    if outcome.record_target {
        if work.channel == "webhook" {
            tx.execute("UPDATE webhook_targets SET last_attempt_at=$1,last_success_at=CASE WHEN $2 THEN $1 ELSE last_success_at END,last_error=$3,last_status=$4 WHERE id=$5",&[&now,&outcome.ok,&outcome.error,&outcome.status,&work.target_id]).await?;
        } else {
            tx.execute("UPDATE mail_recipients SET last_attempt_at=$1,last_success_at=CASE WHEN $2 THEN $1 ELSE last_success_at END,last_error=$3 WHERE id=$4",&[&now,&outcome.ok,&outcome.error,&work.target_id]).await?;
        }
    }
    if outcome.ok && work.event.event == "cert.renewal_due" {
        if let Some((id, expires)) = renewal_identity(&work.event) {
            // Only the first success in this generation advances the seven-day reminder marker.
            tx.execute("UPDATE certificates SET renewal_notice_sent_at=$1 WHERE id=$2 AND expires_at=$3 AND (renewal_notice_sent_at IS NULL OR renewal_notice_sent_at < (SELECT created_at FROM notifications WHERE id=$4))",&[&now,&id,&expires,&work.notification_id]).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
pub async fn dispatch_once(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let work = claim(state, now).await?;
    let count = work.len();
    let mut tasks = tokio::task::JoinSet::new();
    for delivery in work {
        let state = state.clone();
        tasks.spawn(async move {
            let outcome = match deliver(&state, &delivery).await {
                Ok(o) => o,
                Err(_) => Outcome::failed("알림 전송 설정을 읽을 수 없습니다.", true),
            };
            finish(&state, &delivery, outcome, now).await
        });
    }
    let mut error = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error = Some(e),
            Err(e) => error = Some(e.into()),
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    Ok(count)
}

pub struct WorkerHandle {
    stop: tokio::sync::watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl WorkerHandle {
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
pub fn start(state: AppState) -> WorkerHandle {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    if state
        .runtime
        .notifications_started
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return WorkerHandle { stop, task: None };
    }
    let mut shutdown = state.runtime.shutdown.subscribe();
    let task = tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(5));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_sweep = None::<DateTime<Utc>>;
        loop {
            if *stopped.borrow() || *shutdown.borrow() {
                break;
            }
            tokio::select! {_=stopped.changed()=>break,_=shutdown.changed()=>break,_=timer.tick()=>{}}
            let now = Utc::now();
            if last_sweep.is_none_or(|last| now - last >= chrono::Duration::hours(12)) {
                if sweep_due(&state, now).await.is_ok() {
                    last_sweep = Some(now);
                } else {
                    eprintln!("notification reminder sweep failed");
                }
            }
            if plan(&state, now).await.is_err() {
                eprintln!("notification delivery planning failed");
            }
            if dispatch_once(&state, now).await.is_err() {
                eprintln!("notification delivery checkpoint failed");
            }
        }
    });
    WorkerHandle {
        stop,
        task: Some(task),
    }
}

pub async fn sweep_due(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    let before = now + chrono::Duration::days(30);
    let last = now - chrono::Duration::days(7);
    let rows=tx.query("SELECT id,domains,expires_at,staging,renewal_notice_sent_at FROM certificates WHERE status='issued' AND source='acme' AND challenge_type='dns-01' AND dns_provider IS NULL AND expires_at<$1 AND (renewal_notice_sent_at IS NULL OR renewal_notice_sent_at<$2) ORDER BY id FOR UPDATE SKIP LOCKED",&[&before,&last]).await?;
    let mut count = 0;
    for row in rows {
        let id: String = row.get("id");
        let expires: DateTime<Utc> = row.get("expires_at");
        let previous: Option<DateTime<Utc>> = row.get("renewal_notice_sent_at");
        let domains: Value = row.get("domains");
        let names = domains
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let days = ((expires - now).num_milliseconds().max(0) + 86_399_999) / 86_400_000;
        let key = format!(
            "cert.renewal_due:{id}:{}:{}",
            expires.timestamp_micros(),
            previous
                .map(|d| d.timestamp_micros().to_string())
                .unwrap_or_else(|| "initial".into())
        );
        let notification = uuid::Uuid::new_v4().to_string();
        let link = format!("/certificates/{id}");
        let body = format!("{names} — 약 {days}일 남음");
        let data = json!({"certificateId":id,"domains":domains,"expiresAt":expires.to_rfc3339_opts(chrono::SecondsFormat::Micros,true),"daysLeft":days,"staging":row.get::<_,bool>("staging")});
        count+=tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,created_at,dedupe_key) VALUES ($1,'cert.renewal_due','warning','인증서 갱신 필요 (DNS-01)',$2,$3,$4,$5,$6) ON CONFLICT(dedupe_key) DO NOTHING",&[&notification,&body,&link,&data,&now,&key]).await? as usize;
        // Rediscover newly configured destinations on the next sweep. Existing per-target
        // receipts remain unique, so a successful destination is never resent by planning.
        tx.execute(
            "UPDATE notifications SET delivery_planned_at=NULL WHERE dedupe_key=$1",
            &[&key],
        )
        .await?;
        // A corrected target can retry the same reminder; successful channels keep their receipt.
        tx.execute("UPDATE notification_deliveries SET status='pending',next_attempt_at=$1 WHERE status='failed' AND notification_id=(SELECT id FROM notifications WHERE dedupe_key=$2)",&[&now,&key]).await?;
    }
    tx.commit().await?;
    drop(client);
    prune(state, now).await?;
    Ok(count)
}

/// Keep the existing 90-day feed retention, bounded to 20,000 rows per sweep. Active or
/// expired delivery leases survive until a worker completes/reclaims them. Claim and prune
/// serialize only their short database transactions; no network request holds this lock.
pub async fn prune(state: &AppState, now: DateTime<Utc>) -> anyhow::Result<u64> {
    let mut client = state.db.get().await?;
    let tx = client.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(71203841)", &[])
        .await?;
    let cutoff = now - chrono::Duration::days(90);
    let mut total = 0;
    for _ in 0..20 {
        let count=tx.execute("DELETE FROM notifications WHERE id IN (SELECT n.id FROM notifications n WHERE created_at<$1 AND NOT EXISTS(SELECT 1 FROM notification_deliveries d WHERE d.notification_id=n.id AND d.status='delivering') ORDER BY created_at,id LIMIT 1000 FOR UPDATE SKIP LOCKED)",&[&cutoff]).await?;
        total += count;
        if count < 1000 {
            break;
        }
    }
    tx.commit().await?;
    Ok(total)
}
