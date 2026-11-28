//! The API process owns one durable certificate worker.
use super::protocol::Cancellation;
use super::{
    CertificateError,
    cloudflare::CloudflareClient,
    protocol::{AcmeClient, DnsTxtStatus, DnsVerifier, HttpChallengeStore, IssuanceTrace},
    store::{self, JobPayload},
};
use crate::{contracts::Actor, state::AppState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

// Skip a busy monitor tick promptly so the work can release the only connection.
const MONITOR_POOL_WAIT: Duration = Duration::from_millis(20);

#[derive(Default)]
pub struct JobRuntime {
    pub(crate) wake: tokio::sync::Notify,
    pub(crate) active: tokio::sync::Mutex<Option<(String, Cancellation)>>,
    pub(crate) started: AtomicBool,
}

pub struct JobWorker {
    state: AppState,
    stop: tokio::sync::watch::Sender<bool>,
    failed: tokio::sync::watch::Receiver<bool>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}
struct FailureSignal {
    sender: tokio::sync::watch::Sender<bool>,
    armed: bool,
}
impl Drop for FailureSignal {
    fn drop(&mut self) {
        if self.armed {
            self.sender.send_replace(true);
        }
    }
}
impl JobWorker {
    /// Main watches unexpected worker exit and gracefully stops the API. The
    /// underlying diagnostic stays on the join result, not a public health DTO.
    pub fn failure_receiver(&self) -> tokio::sync::watch::Receiver<bool> {
        self.failed.clone()
    }
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.stop.send_replace(true);
        if let Some((_, cancel)) = &*self.state.runtime.certificate_jobs.active.lock().await {
            cancel.cancel();
        }
        self.task.await??;
        Ok(())
    }
}
pub async fn start(state: AppState) -> anyhow::Result<JobWorker> {
    if state
        .runtime
        .certificate_jobs
        .started
        .swap(true, Ordering::SeqCst)
    {
        anyhow::bail!("certificate worker already started");
    }
    let result = async {
        let mut pg = state
            .config
            .database_url
            .parse::<tokio_postgres::Config>()?;
        pg.connect_timeout(Duration::from_secs(5));
        pg.application_name("nwui-certificate-owner");
        pg.options("-c statement_timeout=5000");
        let (lock, connection) = pg.connect(tokio_postgres::NoTls).await?;
        // Wakes an idle worker as soon as the ownership connection ends, rather than
        // at its next poll.
        let (closed, lost) = tokio::sync::watch::channel(false);
        let connection = tokio::spawn(async move {
            let _ = connection.await;
            closed.send_replace(true);
        });
        if !lock
            .query_one("SELECT pg_try_advisory_lock(718234921)", &[])
            .await?
            .get::<_, bool>(0)
        {
            anyhow::bail!("another API owns certificate jobs");
        }
        if let Err(error) = recover(&state).await {
            let _ = lock
                .query_one("SELECT pg_advisory_unlock(718234921)", &[])
                .await;
            return Err(error);
        }
        let (stop, rx) = tokio::sync::watch::channel(false);
        let (failed, failed_rx) = tokio::sync::watch::channel(false);
        let owned = state.clone();
        let task = tokio::spawn(async move {
            let mut failure = FailureSignal {
                sender: failed,
                armed: true,
            };
            let result = run_loop(&owned, rx, &lock, lost).await;
            if result.is_err() {
                failure.sender.send_replace(true);
            } else {
                failure.armed = false;
            }
            // This dedicated connection owns no writes; close it after releasing
            // the session lock, including a lost-connection shutdown.
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                lock.query_one("SELECT pg_advisory_unlock(718234921)", &[]),
            )
            .await;
            drop(lock);
            connection.abort();
            let _ = connection.await;
            owned
                .runtime
                .certificate_jobs
                .started
                .store(false, Ordering::SeqCst);
            result
        });
        Ok(JobWorker {
            state: state.clone(),
            stop,
            failed: failed_rx,
            task,
        })
    }
    .await;
    if result.is_err() {
        state
            .runtime
            .certificate_jobs
            .started
            .store(false, Ordering::SeqCst);
    }
    result
}

/// Recovery is deliberately separate from queue dispatch: shutdown immediately
/// after start still leaves interrupted manual orders ready for resubmission.
async fn recover(state: &AppState) -> anyhow::Result<()> {
    let mut db = state.db.get().await?;
    let tx = db.transaction().await?;
    // Only interrupted manual validations are resumable. A previous failure or
    // an unsubmitted TXT wait is not a new issuance request.
    let now = chrono::Utc::now();
    let interrupted_jobs = tx.query("SELECT j.certificate_id,c.domains,j.payload_enc,c.dns_order FROM certificate_jobs j LEFT JOIN certificates c ON c.id=j.certificate_id WHERE j.state='active'",&[]).await?;
    let mut expired_orders = Vec::new();
    let rows=tx.query("SELECT c.id,c.status::text,c.dns_order,c.last_attempt_at,c.created_at,EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.certificate_id=c.id AND j.kind='dns_complete' AND j.state='active' AND NOT j.cancel_requested) AS interrupted,c.domains,c.last_error FROM certificates c WHERE c.dns_order IS NOT NULL AND c.dns_provider IS NULL AND NOT EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.certificate_id=c.id AND (j.state IN ('queued','cleanup') OR (j.state='active' AND (j.kind<>'dns_complete' OR j.cancel_requested)))) ORDER BY c.id FOR UPDATE OF c",&[]).await?;
    for row in rows {
        let id: String = row.get(0);
        let status: String = row.get(1);
        let order: Value = row.get(2);
        let last_attempt: Option<chrono::DateTime<chrono::Utc>> = row.get(3);
        let created: chrono::DateTime<chrono::Utc> = row.get(4);
        let interrupted: bool = row.get(5);
        // RFC 8555 order.expires is authoritative. Missing expiry in older
        // adapters has a bounded fallback; malformed supplied expiry is not
        // silently replaced with a later local deadline.
        let expired = match order
            .get("order")
            .and_then(|o| o.get("expires"))
            .filter(|v| !v.is_null())
        {
            Some(value) => value
                .as_str()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .is_none_or(|expires| expires <= now),
            None => last_attempt.unwrap_or(created) + chrono::Duration::days(7) <= now,
        };
        if expired {
            let message = "DNS 주문이 만료되었거나 유효기간을 확인할 수 없습니다. 다시 발급하세요.";
            if !interrupted && status != "failed" && row.get::<_, Option<String>>(7).as_deref() != Some(DNS_RESTARTED) {
                let names: Vec<String> = serde_json::from_value(row.get(6)).unwrap_or_default();
                expired_orders.push(issuance_line(&id, &names, IssuanceEvent::Failure {
                    reason: message,
                    order_url: order["order"]["url"].as_str(),
                }));
            }
            tx.execute("UPDATE certificates SET dns_order=NULL,status=CASE WHEN status IN ('issuing','awaiting-dns') THEN 'failed'::cert_status ELSE status END,last_error=CASE WHEN status='failed' THEN last_error ELSE $2 END,issue_log=issue_log||$3::jsonb WHERE id=$1",&[&id,&message,&json!([{"at":now,"level":if status=="issued"{"info"}else{"error"},"msg":message}])]).await?;
        } else if status != "failed"
            && (interrupted || order.get("submittedAt").is_some_and(|v| !v.is_null()))
        {
            let message = DNS_RESTARTED;
            tx.execute("UPDATE certificates SET dns_order=dns_order-'submittedAt',status=CASE WHEN status='issuing' THEN 'awaiting-dns'::cert_status ELSE status END,last_error=$2,issue_log=issue_log||$3::jsonb WHERE id=$1",&[&id,&message,&json!([{"at":now,"level":"info","msg":if status=="issued"{"갱신 검증이 재시작으로 중단됨 — 기존 인증서는 그대로 유지됩니다"}else{message}}])]).await?;
        }
    }
    let stopped_log = json!([{"at":now,"level":"error","msg":"API 재시작으로 발급이 중단되었습니다. 다시 시도하세요."}]);
    tx.execute("UPDATE certificates c SET status=CASE WHEN c.status='issued' THEN c.status ELSE 'failed'::cert_status END,last_error='API 재시작으로 발급이 중단되었습니다.',last_attempt_at=now(),issue_log=issue_log||$1::jsonb WHERE EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.certificate_id=c.id AND j.state='active' AND j.kind<>'dns_complete')",&[&stopped_log]).await?;
    tx.execute("UPDATE certificate_jobs SET state='cleanup',error=COALESCE(error,'API 재시작으로 발급이 중단되었습니다.'),updated_at=now()-interval '1 minute' WHERE state='active'",&[]).await?;
    // Legacy TS jobs had no durable ownership row; unrecoverable issuing rows
    // become actionable failures with the same visible restart explanation.
    tx.execute("UPDATE certificates c SET status='failed',last_error='API 재시작으로 발급이 중단되었습니다.',dns_order=NULL,issue_log=issue_log||$1::jsonb WHERE status='issuing' AND NOT EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.certificate_id=c.id AND j.state IN ('queued','active','cleanup'))",&[&stopped_log]).await?;
    tx.execute("DELETE FROM acme_challenges a WHERE created_at<now()-interval '1 hour' AND NOT EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.http_tokens ? a.token)",&[]).await?;
    tx.commit().await?;
    for row in interrupted_jobs {
        let id: String = row.get(0);
        let sealed: String = row.get(2);
        let payload: Option<JobPayload> = state.secrets.open(Some(&sealed))
            .and_then(|value| serde_json::from_str(&value).ok());
        let names = payload.as_ref().map(|payload| payload.domains.clone())
            .or_else(|| serde_json::from_value(row.get::<_, Option<Value>>(1)?).ok())
            .unwrap_or_default();
        let stored_order: Option<Value> = row.get(3);
        let order_url = payload.as_ref().and_then(|payload| saved_order_url(payload.dns_order.as_ref()))
            .or_else(|| saved_order_url(stored_order.as_ref()));
        eprintln!("{}", issuance_line(&id, &names, IssuanceEvent::Failure {
            reason: RESTARTED, order_url,
        }));
    }
    for line in expired_orders {
        eprintln!("{line}");
    }
    Ok(())
}
/// Safety net only. Every path that queues a job (create, retry, renew, DNS
/// submit, automatic renewal) and cancel/delete wake the worker at once, and
/// cleanup retries start at 30 seconds; repeated deployment failures wait 15 minutes.
const IDLE_POLL: Duration = Duration::from_secs(10);
/// Maximum transient database retry delay; also bounds end-to-end worker test waits.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.min(6)).min(MAX_RETRY_DELAY.as_secs()))
}
fn transient_database_error(error: &anyhow::Error) -> bool {
    if let Some(error) = error
        .chain()
        .find_map(|e| e.downcast_ref::<tokio_postgres::Error>())
    {
        return error.code().is_none();
    }
    error.chain().any(|e| {
        matches!(
            e.downcast_ref::<deadpool_postgres::PoolError>(),
            Some(deadpool_postgres::PoolError::Timeout(_))
        )
    })
}
struct Job {
    id: String,
    certificate_id: String,
    kind: String,
    mode: String,
    payload: JobPayload,
    actor: Actor,
}
async fn run_loop(
    state: &AppState,
    mut stop: tokio::sync::watch::Receiver<bool>,
    lock: &tokio_postgres::Client,
    mut lost: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut shutdown = state.runtime.shutdown.subscribe();
    let mut renew_at = Instant::now();
    let mut db_attempt = 0u32;
    loop {
        if *stop.borrow() || *shutdown.borrow() {
            break;
        }
        // Loss of this connection means loss of sole ownership; do not start more work.
        if lock.is_closed() || *lost.borrow() {
            anyhow::bail!("certificate job ownership connection lost");
        }
        let owner_check =
            tokio::time::timeout(Duration::from_secs(5), lock.simple_query("SELECT 1")).await;
        if !matches!(&owner_check, Ok(Ok(_))) {
            if let Ok(Err(error)) = owner_check {
                if error.code().is_some() {
                    return Err(error.into());
                }
            }
            let delay = retry_delay(db_attempt);
            db_attempt = db_attempt.saturating_add(1);
            tokio::select! {_=tokio::time::sleep(delay)=>{},_=lost.wait_for(|lost|*lost)=>{},_=stop.changed()=>{},_=shutdown.changed()=>{}}
            continue;
        }
        if Instant::now() >= renew_at {
            if let Err(error) = renew_due(state).await {
                eprintln!("[certificates] automatic renewal sweep failed: {error:#}");
            }
            if let Err(error) = prune_finished(state).await {
                eprintln!("[certificates] pruning finished jobs failed: {error:#}");
            }
            renew_at = Instant::now() + Duration::from_secs(12 * 3600);
        }
        let attempt:anyhow::Result<()> = async {
        let db = state.db.get().await?;
        let row=db.query_opt("SELECT id,certificate_id,kind,mode,state,payload_enc,actor FROM certificate_jobs WHERE state='queued' OR (state='cleanup' AND updated_at<now()-interval '30 seconds') ORDER BY created_at LIMIT 1",&[]).await?;
        drop(db);
        if let Some(row) = row {
            let id: String = row.get(0);
            let status: String = row.get(4);
            let stored: String = row.get(5);
            let payload = state
                .secrets
                .open(Some(&stored))
                .and_then(|v| serde_json::from_str(&v).ok());
            let Some(payload) = payload else {
                // Without the sealed snapshot there is no Cloudflare token: HTTP-01 tokens are
                // still removed, and each TXT record is kept for the record and given up as a
                // permanent failure. A job that never ran fails its certificate like any error.
                let certificate_id: String = row.get(1);
                let db = state.db.get().await?;
                db.execute("UPDATE certificate_jobs SET state='cleanup',updated_at=now(),error=COALESCE(error,$2) WHERE id=$1",&[&id,&SEALED_UNAVAILABLE]).await?;
                if status != "cleanup" {
                    let issue_log = issue_entry("error", SEALED_UNAVAILABLE);
                    let cert = db.query_opt("UPDATE certificates SET status=CASE WHEN status='issued' THEN status ELSE 'failed'::cert_status END,dns_order=NULL,last_error=$2,last_attempt_at=now(),issue_log=issue_log||$3::jsonb WHERE id=$1 RETURNING domains",&[&certificate_id,&SEALED_UNAVAILABLE,&issue_log]).await?;
                    let names = cert
                        .and_then(|row| serde_json::from_value::<Vec<String>>(row.get(0)).ok())
                        .unwrap_or_default();
                    eprintln!("{}", issuance_line(&certificate_id, &names, IssuanceEvent::Failure {
                        reason: SEALED_UNAVAILABLE, order_url: None,
                    }));
                }
                drop(db);
                cleanup_attempt(state, &id, &certificate_id, None).await?;
                return Ok(());
            };
            let job = Job {
                id,
                certificate_id: row.get(1),
                kind: row.get(2),
                mode: row.get(3),
                payload,
                actor: serde_json::from_value(row.get(6))?,
            };
            if status == "cleanup" {
                cleanup_attempt(state, &job.id, &job.certificate_id, job.payload.dns_token.as_deref()).await?;
                return Ok(());
            }
            let db = state.db.get().await?;
            let cert = db.query_opt("SELECT staging,last_error FROM certificates WHERE id=$1", &[&job.certificate_id]).await?;
            db.execute("UPDATE certificate_jobs SET state='active',updated_at=now() WHERE id=$1 AND state='queued'",&[&job.id]).await?;
            drop(db);
            let staging = cert.as_ref().is_some_and(|row| row.get(0));
            let resumed = cert.and_then(|row| row.get::<_, Option<String>>(1)).as_deref() == Some(DNS_RESTARTED);
            if job.kind != "dns_complete" || resumed {
                eprintln!("{}", issuance_line(&job.certificate_id, &job.payload.domains, IssuanceEvent::Start {
                    mode: &job.mode, staging, directory: &job.payload.directory,
                }));
            }
            let mut trace = IssuanceTrace {
                order_url: saved_order_url(job.payload.dns_order.as_ref()).map(str::to_owned),
                ..IssuanceTrace::default()
            };
            let cancel = Cancellation::default();
            *state.runtime.certificate_jobs.active.lock().await =
                Some((job.certificate_id.clone(), cancel.clone()));
            let mut work = Box::pin(execute(state, &job, &cancel, &mut trace));
            let mut poll = tokio::time::interval(Duration::from_millis(200));
            let mut owner_lost = false;
            let result = loop {
                tokio::select! {
                 result=&mut work=>break result,
                 _=poll.tick()=>{
                  if *stop.borrow()||*shutdown.borrow(){cancel.cancel();}
                  // A closed owner connection has released the lock: stop the work. A slow
                  // reply, a busy pool or a failed read is a slow database, not a cancel
                  // request; the next tick asks again.
                  if matches!(tokio::time::timeout(Duration::from_secs(5),lock.simple_query("SELECT 1")).await,Ok(Err(_))){owner_lost=true;cancel.cancel();}
                  if let Ok(Ok(db))=tokio::time::timeout(MONITOR_POOL_WAIT,state.db.get()).await{if let Ok(r)=db.query_one("SELECT cancel_requested FROM certificate_jobs WHERE id=$1",&[&job.id]).await{if r.get::<_,bool>(0){cancel.cancel();}}}
                 },
                 _=stop.changed()=>cancel.cancel(),
                 _=shutdown.changed()=>cancel.cancel(),
                }
            };
            drop(work);
            let result = result.map_err(|error| issuance_reason(&job.payload, &trace, &error.to_string()));
            let reason = if *stop.borrow() || *shutdown.borrow() {
                Some(RESTARTED)
            } else if owner_lost {
                Some(OWNERSHIP_LOST)
            } else {
                None
            };
            // The operation future is ALWAYS awaited, including shutdown. Its database
            // writes and Cloudflare create response settle before cleanup starts.
            // Keep the completed outcome in memory until the guarded finish commits.
            let mut finish_attempt=0u32;
            let mut final_attempt=false;
            loop {
                match finish(state, &job, &result, cancel.is_cancelled(), reason, trace.order_url.as_deref()).await {
                    Ok(())=>break,
                    Err(error)=>{
                        if lock.is_closed() || *lost.borrow() || !transient_database_error(&error) || final_attempt {return Err(error);}
                        if *stop.borrow() || *shutdown.borrow() {final_attempt=true;continue;}
                        let delay=retry_delay(finish_attempt);finish_attempt=finish_attempt.saturating_add(1);
                        eprintln!("[certificates] finish will retry in {delay:?}: {error:#}");
                        tokio::select! {
                            _=tokio::time::sleep(delay)=>{},
                            _=lost.wait_for(|lost|*lost)=>return Err(error),
                            _=stop.changed()=>final_attempt=true,
                            _=shutdown.changed()=>final_attempt=true,
                        }
                    }
                }
            }
            *state.runtime.certificate_jobs.active.lock().await = None;
            cleanup_attempt(state, &job.id, &job.certificate_id, job.payload.dns_token.as_deref()).await?;
        } else {
            tokio::select! {_=state.runtime.certificate_jobs.wake.notified()=>{},_=tokio::time::sleep(IDLE_POLL)=>{},_=stop.changed()=>{},_=shutdown.changed()=>{},_=lost.wait_for(|lost|*lost)=>{}}
        }
            Ok(())
        }.await;
        if let Err(error) = attempt {
            if lock.is_closed() || *lost.borrow() {
                return Err(error);
            }
            if *stop.borrow()
                || *shutdown.borrow()
                || stop.has_changed().is_err()
                || shutdown.has_changed().is_err()
            {
                eprintln!("[certificates] stopped before database work committed: {error:#}");
                break;
            }
            if !transient_database_error(&error) {
                return Err(error);
            }
            let delay = retry_delay(db_attempt);
            db_attempt = db_attempt.saturating_add(1);
            eprintln!("[certificates] database work will retry in {delay:?}: {error:#}");
            tokio::select! {_=tokio::time::sleep(delay)=>{},_=lost.wait_for(|lost|*lost)=>{},_=stop.changed()=>{},_=shutdown.changed()=>{}}
        } else {
            db_attempt = 0;
        }
    }
    Ok(())
}

pub struct DatabaseChallengeStore {
    state: AppState,
    job_id: String,
}
impl DatabaseChallengeStore {
    pub fn new(state: AppState, job_id: String) -> Self {
        Self { state, job_id }
    }
}
impl HttpChallengeStore for DatabaseChallengeStore {
    async fn set(&self, token: &str, authorization: &str) -> super::Result<()> {
        let result: anyhow::Result<()>=async{
   let mut db=self.state.db.get().await?;let tx=db.transaction().await?;
   tx.batch_execute("SET LOCAL statement_timeout='15s'").await?;
   let row=tx.query_one("SELECT cancel_requested FROM certificate_jobs WHERE id=$1 FOR UPDATE",&[&self.job_id]).await?;
   if row.get::<_,bool>(0){anyhow::bail!("요청이 중단되었습니다.");}
   tx.execute("INSERT INTO acme_challenges(token,key_authorization) VALUES($1,$2)",&[&token,&authorization]).await?;
   tx.execute("UPDATE certificate_jobs SET http_tokens=http_tokens||jsonb_build_array($2::text),updated_at=now() WHERE id=$1",&[&self.job_id,&token]).await?;
   tx.commit().await?;Ok(())
  }.await;
        result.map_err(|e| CertificateError::Callback(e.to_string()))
    }
    async fn remove(&self, token: &str) -> super::Result<()> {
        let result: anyhow::Result<()> = async {
            let mut db = self.state.db.get().await?;
            let tx = db.transaction().await?;
            tx.batch_execute("SET LOCAL statement_timeout='15s'")
                .await?;
            // This lock also fences a lost COMMIT response from set: deletion cannot
            // overtake an in-flight INSERT transaction on another connection.
            let row = tx
                .query_one(
                    "SELECT http_tokens FROM certificate_jobs WHERE id=$1 FOR UPDATE",
                    &[&self.job_id],
                )
                .await?;
            let tokens: Value = row.get(0);
            if tokens
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == token))
            {
                tx.execute("DELETE FROM acme_challenges WHERE token=$1", &[&token])
                    .await?;
                tx.execute(
                    "UPDATE certificate_jobs SET http_tokens=http_tokens-$2::text WHERE id=$1",
                    &[&self.job_id, &token],
                )
                .await?;
            }
            tx.commit().await?;
            Ok(())
        }
        .await;
        result.map_err(|e| CertificateError::Callback(e.to_string()))
    }
}
#[derive(Serialize, Deserialize, Clone)]
struct CfIntent {
    zone_id: String,
    name: String,
    value: String,
    owner: String,
    #[serde(default)]
    record_id: Option<String>,
    #[serde(default)]
    record_ids: Vec<String>,
    #[serde(default)]
    rejected: bool,
    /// First cleanup lookup that could not see a create whose response was lost.
    #[serde(default)]
    unconfirmed_since: Option<chrono::DateTime<chrono::Utc>>,
    /// First of the consecutive cleanup attempts that failed for this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failing_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Cleanup stopped trying: the record may still exist and is kept on the finished job as
    /// the manual cleanup reference, together with the last failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    abandoned_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}
/// How long a create with a lost response may stay invisible before cleanup
/// treats it as never created. Without a bound the job never leaves cleanup and
/// blocks retry, renewal and edits of its certificate.
const UNCONFIRMED_CREATE_LIMIT: chrono::Duration = chrono::Duration::minutes(10);
/// How long a TXT record may keep failing to be removed before cleanup gives up on it. A cleanup
/// job holds its certificate's only job slot, so an unbounded retry blocks retry, renewal and
/// edits, and the certificate expires without a renewal failure. A refusal (revoked token,
/// deleted zone) never heals: the token is a snapshot the user cannot replace while the job
/// runs. An outage or a network failure usually does.
const PERMANENT_CLEANUP_LIMIT: chrono::Duration = chrono::Duration::minutes(10);
const TRANSIENT_CLEANUP_LIMIT: chrono::Duration = chrono::Duration::hours(24);
const SEALED_UNAVAILABLE: &str = "발급 작업의 봉인 데이터를 열 수 없습니다.";
#[derive(Debug, thiserror::Error)]
enum CleanupError {
    /// Will not succeed by retrying.
    #[error("{0}")]
    Permanent(String),
    /// Not a failure yet: the unconfirmed create has its own limit.
    #[error("{0}")]
    Pending(String),
}
fn permanent(error: &anyhow::Error) -> bool {
    if let Some(CleanupError::Permanent(_)) = error.downcast_ref() {
        return true;
    }
    error
        .downcast_ref::<super::cloudflare::CloudflareError>()
        .and_then(|e| e.status)
        .is_some_and(|status| (400..500).contains(&status) && !matches!(status, 408 | 429))
}
const CLEANUP_INCOMPLETE: &str = "발급 후 정리가 완료되지 않았습니다: ";
const CLEANUP_RETRY: &str = "발급 후 정리를 재시도합니다: ";
enum Outcome {
    Pending(Value),
    Issued {
        cert: String,
        key: String,
        expires: chrono::DateTime<chrono::Utc>,
    },
}
const CANCELLED: &str = "요청이 중단되었습니다.";
const RESTARTED: &str = "API 재시작으로 발급이 중단되었습니다.";
const DNS_RESTARTED: &str = "서버 재시작으로 검증이 중단되었습니다. TXT 레코드를 그대로 두고 다시 확인하세요.";
const OWNERSHIP_LOST: &str = "데이터베이스 연결이 끊겨 발급이 중단되었습니다. 다시 시도하세요.";
fn issue_entry(level: &str, message: &str) -> Value {
    json!([{"at":chrono::Utc::now(),"level":level,"msg":message}])
}

fn finish_issue_entry(message: &str, error: Option<&str>, cancelled: bool) -> Value {
    match error {
        Some(reason) if !cancelled => issue_entry("error", reason),
        Some(_) => issue_entry("error", message),
        None => issue_entry("info", message),
    }
}

fn saved_order_url(order: Option<&Value>) -> Option<&str> {
    order?["order"]["url"].as_str()
}

fn single_line(text: &str) -> String {
    text.replace(char::is_control, " ")
        .split_whitespace().collect::<Vec<_>>().join(" ")
}

fn issuance_reason(payload: &JobPayload, trace: &IssuanceTrace, reason: &str) -> String {
    let mut secrets = vec![payload.email.as_str(), payload.account_key.as_str()];
    secrets.extend(payload.account_key.lines());
    secrets.extend(payload.dns_token.as_deref());
    secrets.extend(trace.secrets.iter().map(String::as_str));
    if let Some(order) = &payload.dns_order {
        secrets.extend(order["records"].as_array().into_iter().flatten()
            .filter_map(|record| record["recordValue"].as_str()));
    }
    // CA and DNS provider error text can echo submitted values.
    secrets.sort_unstable_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut reason = reason.to_owned();
    for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
        reason = reason.replace(secret, "[redacted]");
    }
    single_line(&reason)
}

enum IssuanceEvent<'a> {
    Start { mode: &'a str, staging: bool, directory: &'a str },
    Success(chrono::DateTime<chrono::Utc>),
    Failure { reason: &'a str, order_url: Option<&'a str> },
    DnsWait,
}

fn issuance_line(id: &str, names: &[String], event: IssuanceEvent<'_>) -> String {
    let names = single_line(&names.join(", "));
    let id = single_line(id);
    let detail = match event {
        IssuanceEvent::Start { mode, staging, directory } => {
            let mode = if mode == "renew" { "renewal" } else { "initial" };
            let environment = if staging { "staging" } else { "production" };
            let directory = url::Url::parse(directory).ok();
            let host = directory.as_ref().and_then(url::Url::host_str).unwrap_or("unknown");
            format!("issuance started: id={id} names={names} mode={mode} ca={environment} directory_host={host}")
        }
        IssuanceEvent::Success(expires) => {
            format!("issuance succeeded: id={id} names={names} expires={}", expires.to_rfc3339())
        }
        IssuanceEvent::Failure { reason, order_url } => {
            let reason = single_line(reason);
            let mut detail = format!("issuance failed: id={id} names={names} reason={reason}");
            if let Some(mut url) = order_url.and_then(|value| url::Url::parse(value).ok())
                .filter(|url| matches!(url.scheme(), "https" | "http") && url.username().is_empty() && url.password().is_none())
            {
                url.set_query(None);
                url.set_fragment(None);
                detail.push_str(&format!(" order_url={url}"));
            }
            detail
        }
        IssuanceEvent::DnsWait => {
            format!("issuance waiting for DNS TXT records: id={id} names={names}")
        }
    };
    format!("[certificates] {detail}")
}

pub(super) fn log_stopped_dns_wait(cert: &store::Certificate, reason: &str) {
    let Some(order) = cert.dns_order.as_ref() else {
        return;
    };
    // Submitted orders end in the worker. Recovery already ended an interrupted verification.
    if order.get("submittedAt").is_none() && cert.last_error.as_deref() != Some(DNS_RESTARTED) {
        eprintln!("{}", issuance_line(&cert.id, &cert.names(), IssuanceEvent::Failure {
            reason,
            order_url: saved_order_url(Some(order)),
        }));
    }
}

fn check(cancel: &Cancellation) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        anyhow::bail!(CANCELLED);
    }
    Ok(())
}
async fn execute(
    state: &AppState,
    job: &Job,
    cancel: &Cancellation,
    trace: &mut IssuanceTrace,
) -> anyhow::Result<Outcome> {
    let db = state.db.get().await?;
    if db
        .query_one(
            "SELECT cancel_requested FROM certificate_jobs WHERE id=$1",
            &[&job.id],
        )
        .await?
        .get::<_, bool>(0)
    {
        cancel.cancel();
    }
    drop(db);
    check(cancel)?;
    let client = AcmeClient::new(&job.payload.directory, state.config.acme_timeout)?;
    if job.kind == "http" {
        let store = DatabaseChallengeStore::new(state.clone(), job.id.clone());
        let result = client
            .issue_http_traced(
                &job.payload.account_key,
                &job.payload.email,
                &job.payload.domains,
                &store,
                cancel,
                trace,
            )
            .await?;
        return Ok(Outcome::Issued {
            cert: result.cert_pem,
            key: result.key_pem,
            expires: result.expires_at,
        });
    }
    let order = if job.kind == "dns_complete" {
        job.payload
            .dns_order
            .clone()
            .ok_or_else(|| anyhow::anyhow!("DNS 주문이 없습니다."))?
    } else {
        let begun = client
            .begin_dns_traced(
                &job.payload.account_key,
                &job.payload.email,
                &job.payload.domains,
                cancel,
                trace,
            )
            .await?;
        json!({"order":begun.order,"csrPem":begun.csr_pem,"keyPem":state.secrets.seal(&begun.key_pem)?,"records":begun.records,"createdAt":chrono::Utc::now(),"jobId":job.id})
    };
    if job.kind == "dns_begin" {
        return Ok(Outcome::Pending(order));
    }
    if job.kind == "dns_auto" {
        let cf = CloudflareClient::new(
            &state.config.cloudflare_api_base,
            job.payload
                .dns_token
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Cloudflare 토큰을 열 수 없습니다."))?,
            state.config.acme_timeout,
        )?;
        for record in order["records"].as_array().into_iter().flatten() {
            check(cancel)?;
            let name = record["recordName"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("DNS 이름이 없습니다."))?;
            let value = record["recordValue"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("DNS 값이 없습니다."))?;
            let zone = cf.find_zone(name).await?;
            let mut intent = CfIntent {
                zone_id: zone.id,
                name: name.into(),
                value: value.into(),
                owner: format!("nwui:{}:{}", job.id, uuid::Uuid::new_v4()),
                record_id: None,
                record_ids: vec![],
                rejected: false,
                unconfirmed_since: None,
                failing_since: None,
                abandoned_at: None,
                last_error: None,
            };
            // Write-ahead ownership survives process death and response loss.
            let db = state.db.get().await?;
            db.execute("UPDATE certificate_jobs SET cf_records=cf_records||$2::jsonb,updated_at=now() WHERE id=$1",&[&job.id,&json!([intent])]).await?;
            drop(db);
            // Do not drop this side-effecting request on cancellation.
            let created = cf
                .create_owned_txt(&intent.zone_id, &intent.name, &intent.value, &intent.owner)
                .await;
            match &created {
                Ok(record) => intent.record_id = Some(record.record_id.clone()),
                Err(e) => {
                    intent.rejected = e.status.is_some_and(|status| (400..500).contains(&status))
                }
            }
            let db = state.db.get().await?;
            db.execute("UPDATE certificate_jobs SET cf_records=(SELECT jsonb_agg(CASE WHEN item->>'owner'=$2 THEN $3::jsonb ELSE item END) FROM jsonb_array_elements(cf_records) item),updated_at=now() WHERE id=$1",&[&job.id,&intent.owner,&json!(intent)]).await?;
            drop(db);
            created?;
            check(cancel)?;
        }
        let verifier = DnsVerifier::system(Duration::from_secs(5))?;
        let deadline = Instant::now() + state.config.dns_propagation_timeout;
        loop {
            check(cancel)?;
            let mut missing = false;
            for record in order["records"].as_array().into_iter().flatten() {
                if verifier
                    .verify(
                        record["recordName"].as_str().unwrap_or(""),
                        record["recordValue"].as_str().unwrap_or(""),
                    )
                    .await
                    == DnsTxtStatus::Missing
                {
                    missing = true;
                }
            }
            if !missing || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    check(cancel)?;
    let csr = order["csrPem"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("CSR이 없습니다."))?;
    let result = client
        .complete_dns_traced(&job.payload.account_key, &order["order"], csr, cancel, trace)
        .await?;
    let key = state
        .secrets
        .open_maybe_plain(order["keyPem"].as_str())
        .ok_or_else(|| anyhow::anyhow!("주문 개인키를 열 수 없습니다."))?;
    Ok(Outcome::Issued {
        cert: result.cert_pem,
        key,
        expires: result.expires_at,
    })
}
/// `reason` says why the worker itself stopped the work (restart, lost database
/// ownership). A user cancel or deletion always reads as a cancel.
async fn finish(
    state: &AppState,
    job: &Job,
    result: &Result<Outcome, String>,
    cancelled: bool,
    reason: Option<&str>,
    order_url: Option<&str>,
) -> anyhow::Result<()> {
    let mut db = state.db.get().await?;
    let tx = db.transaction().await?;
    let cert = tx
        .query_opt(
            "SELECT id FROM certificates WHERE id=$1 FOR UPDATE",
            &[&job.certificate_id],
        )
        .await?;
    let row = tx
        .query_one(
            "SELECT cancel_requested,state FROM certificate_jobs WHERE id=$1 FOR UPDATE",
            &[&job.id],
        )
        .await?;
    let requested = row.get::<_, bool>(0) || cert.is_none();
    // A completed CA issuance cannot be undone by a late cancel; keep its validated leaf.
    let completed = cert.is_some()
        && matches!(result, Ok(Outcome::Issued { .. }))
        && reason != Some(OWNERSHIP_LOST);
    let cancelled = (cancelled || requested) && !completed;
    let status: String = row.get(1);
    if status != "active" {
        return Ok(());
    }
    let mut error = None;
    let mut issued = false;
    let mut pending = false;
    if cancelled {
        error = Some(
            match reason {
                Some(reason) if !requested => reason,
                _ => CANCELLED,
            }
            .to_string(),
        );
        tx.execute("UPDATE certificates SET dns_order=NULL,status=CASE WHEN status='issued' THEN status ELSE 'failed'::cert_status END,last_error=CASE WHEN status='issued' THEN last_error ELSE $2 END,last_attempt_at=now() WHERE id=$1",&[&job.certificate_id,&error]).await?;
    } else {
        match result {
            Ok(Outcome::Pending(order)) => {
                pending = true;
                tx.execute("UPDATE certificates SET dns_order=$2,status=CASE WHEN status='issued' THEN status ELSE 'awaiting-dns'::cert_status END,last_error=NULL,last_attempt_at=now() WHERE id=$1",&[&job.certificate_id,&order]).await?;
                store::audit(
                    &*tx,
                    &job.actor,
                    "cert:dns-begin",
                    &job.certificate_id,
                    json!({"domains":job.payload.domains}),
                )
                .await?;
            }
            Ok(Outcome::Issued { cert, key, expires }) => {
                issued = true;
                let sealed = state.secrets.seal(&key)?;
                tx.execute("UPDATE certificates SET cert_pem=$2,key_pem=$3,expires_at=$4,issued_at=now(),status='issued',dns_order=NULL,last_error=NULL,last_attempt_at=now(),failure_count=0,renewal_notice_sent_at=NULL WHERE id=$1",&[&job.certificate_id,&cert,&sealed,&expires]).await?;
                store::audit(
                    &*tx,
                    &job.actor,
                    if job.mode == "renew" {
                        "cert:renew-result"
                    } else {
                        "cert:issue-result"
                    },
                    &job.certificate_id,
                    json!({"ok":true,"domains":job.payload.domains,"expiresAt":expires}),
                )
                .await?;
            }
            Err(e) => {
                error = Some(e.clone());
                tx.execute("UPDATE certificates SET status=CASE WHEN status='issued' THEN status ELSE 'failed'::cert_status END,dns_order=NULL,last_error=$2,last_attempt_at=COALESCE(last_attempt_at,now()),failure_count=failure_count+1 WHERE id=$1",&[&job.certificate_id,&error]).await?;
                store::audit(
                    &*tx,
                    &job.actor,
                    if job.mode == "renew" {
                        "cert:renew-result"
                    } else {
                        "cert:issue-result"
                    },
                    &job.certificate_id,
                    json!({"ok":false,"error":error}),
                )
                .await?;
            }
        }
    }
    tx.execute("UPDATE certificate_jobs SET state='cleanup',error=$2,cancel_requested=CASE WHEN $4 THEN false ELSE cancel_requested OR $3 END,deploy_pending=$4,updated_at=now()-interval '1 minute' WHERE id=$1",&[&job.id,&error,&cancelled,&issued]).await?;
    let message = if pending {
        "DNS TXT 레코드를 등록한 뒤 검증을 제출하세요."
    } else if issued {
        "인증서 발급이 완료되었습니다."
    } else {
        "인증서 발급이 중단되었거나 실패했습니다."
    };
    let issue_log = finish_issue_entry(message, error.as_deref(), cancelled);
    tx.execute("UPDATE certificates SET issue_log=issue_log||$2::jsonb WHERE id=$1",&[&job.certificate_id,&issue_log]).await?;
    if !cancelled && !issued {
        let kind = if pending {
            "cert.dns_awaiting"
        } else if issued {
            if job.mode == "renew" {
                "cert.renewed"
            } else {
                "cert.issued"
            }
        } else if job.mode == "renew" {
            "cert.renew_failed"
        } else {
            "cert.failed"
        };
        let mut data:Value=tx.query_one("SELECT jsonb_build_object('certificateId',id,'domains',domains,'expiresAt',expires_at,'staging',staging) FROM certificates WHERE id=$1",&[&job.certificate_id]).await?.get(0);
        if let Some(error) = &error {
            data["error"] = json!(error);
        }
        let body = if let Some(error) = &error {
            format!(
                "{} — {}",
                job.payload.domains.join(", "),
                error
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>()
            )
        } else {
            job.payload.domains.join(", ")
        };
        let severity = if pending {
            "info"
        } else if issued {
            "success"
        } else if job.mode == "renew" {
            "warning"
        } else {
            "error"
        };
        tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data) VALUES($1,$2,$3::text::notification_severity,$4,$5,$6,$7)",&[&uuid::Uuid::new_v4().to_string(),&kind,&severity,&message,&body,&format!("/certificates/{}",job.certificate_id),&data]).await?;
    }
    tx.commit().await?;
    let event = if let Some(error) = error.as_deref() {
        IssuanceEvent::Failure { reason: error, order_url }
    } else if let Ok(Outcome::Issued { expires, .. }) = result {
        IssuanceEvent::Success(*expires)
    } else {
        IssuanceEvent::DnsWait
    };
    eprintln!("{}", issuance_line(&job.certificate_id, &job.payload.domains, event));
    Ok(())
}
// Reuse cleanup for durable deployment retries; repeated failures are deferred to avoid audit floods.
async fn retry_deployment(
    state: &AppState,
    job_id: &str,
    certificate_id: &str,
) -> anyhow::Result<()> {
    let db = state.db.get().await?;
    let row = db
        .query_one(
            "SELECT deploy_pending,cancel_requested FROM certificate_jobs WHERE id=$1",
            &[&job_id],
        )
        .await?;
    if !row.get::<_, bool>(0) {
        return Ok(());
    }
    let exists = db
        .query_opt(
            "SELECT id FROM certificates WHERE id=$1",
            &[&certificate_id],
        )
        .await?
        .is_some();
    if row.get::<_, bool>(1) || !exists {
        db.execute(
            "UPDATE certificate_jobs SET deploy_pending=false,cancel_requested=true WHERE id=$1",
            &[&job_id],
        )
        .await?;
        return Ok(());
    }
    let material = db
        .query_opt(
            "SELECT cert_pem,key_pem FROM certificates WHERE id=$1 AND status='issued'",
            &[&certificate_id],
        )
        .await?;
    let digest = material.and_then(|row| {
        let cert: Option<String> = row.get(0);
        let key: Option<String> = row.get(1);
        Some(crate::control::model::certificate_content_digest(&cert?, &key?))
    });
    drop(db);
    let error = match digest {
        Some(digest) => super::service::apply(state, certificate_id, &digest, None).await,
        None => Some("인증서 자료를 확인할 수 없습니다.".into()),
    };
    let mut db = state.db.get().await?;
    let tx = db.transaction().await?;
    let data=tx.query_opt("SELECT jsonb_build_object('certificateId',id,'domains',domains,'expiresAt',expires_at,'staging',staging) FROM certificates WHERE id=$1 FOR UPDATE",&[&certificate_id]).await?;
    let row=tx.query_one("SELECT deploy_pending,mode,cancel_requested,error FROM certificate_jobs WHERE id=$1 FOR UPDATE",&[&job_id]).await?;
    if !row.get::<_, bool>(0) {
        return Ok(());
    }
    let Some(data) = data else {
        tx.execute(
            "UPDATE certificate_jobs SET deploy_pending=false,cancel_requested=true WHERE id=$1",
            &[&job_id],
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    };
    let cancelled = row.get::<_, bool>(2);
    let retry = error.is_some() && !cancelled;
    let repeated = retry
        && row
            .get::<_, Option<String>>(3)
            .is_some_and(|e| e.starts_with("인증서 적용 대기: "));
    let shown = error.as_ref().map(|e| format!("인증서 적용 대기: {e}"));
    tx.execute("UPDATE certificate_jobs SET deploy_pending=$2,error=CASE WHEN $3::text IS NOT NULL THEN $3 WHEN starts_with(error,'인증서 적용 대기: ') THEN NULL ELSE error END,cancel_requested=CASE WHEN $3::text IS NULL THEN false ELSE cancel_requested END,updated_at=CASE WHEN $4 THEN now()+interval '15 minutes' ELSE now() END WHERE id=$1",&[&job_id,&retry,&shown,&repeated]).await?;
    tx.execute(
        "UPDATE certificates SET last_error=$2 WHERE id=$1",
        &[&certificate_id, &shown],
    )
    .await?;
    let renew = row.get::<_, String>(1) == "renew";
    let kind = match (renew, error.is_some()) {
        (true, true) => "cert.renew_failed",
        (false, true) => "cert.failed",
        (true, false) => "cert.renewed",
        (false, false) => "cert.issued",
    };
    let title = if retry {
        "인증서를 발급했지만 nginx 적용에 실패했습니다. 다시 적용합니다."
    } else if error.is_some() {
        "인증서를 발급했지만 nginx 적용에 실패했습니다."
    } else {
        "인증서 발급이 완료되었습니다."
    };
    let severity = if error.is_some() {
        "warning"
    } else {
        "success"
    };
    let mut data: Value = data.get(0);
    if let Some(e) = &error {
        data["error"] = json!(e);
    }
    let names = data["domains"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let body = if let Some(error) = &error {
        format!(
            "{names} — {}",
            error
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>()
        )
    } else {
        names
    };
    let key = format!("{kind}:deploy:{job_id}");
    tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,dedupe_key) VALUES($1,$2,$3::text::notification_severity,$4,$5,$6,$7,$8) ON CONFLICT(dedupe_key) DO NOTHING",&[&uuid::Uuid::new_v4().to_string(),&kind,&severity,&title,&body,&format!("/certificates/{certificate_id}"),&data,&key]).await?;
    tx.commit().await?;
    Ok(())
}
const REPLACE_INTENT: &str = "UPDATE certificate_jobs SET cf_records=(SELECT jsonb_agg(CASE WHEN item->>'owner'=$2 THEN $3::jsonb ELSE item END) FROM jsonb_array_elements(cf_records) item) WHERE id=$1";
/// `token` is the job's Cloudflare token, or `None` when the sealed snapshot cannot be opened.
async fn cleanup(
    state: &AppState,
    job_id: &str,
    certificate_id: &str,
    token: Option<&str>,
) -> anyhow::Result<()> {
    // A DNS cleanup outage must not delay deployment of an already issued leaf.
    retry_deployment(state, job_id, certificate_id).await?;
    let db = state.db.get().await?;
    let row = db
        .query_one(
            "SELECT http_tokens,cf_records FROM certificate_jobs WHERE id=$1",
            &[&job_id],
        )
        .await?;
    let tokens: Value = row.get(0);
    let intents: Vec<CfIntent> = serde_json::from_value(row.get(1))?;
    drop(db);
    let store = DatabaseChallengeStore::new(state.clone(), job_id.to_owned());
    let mut failures = Vec::new();
    for token in tokens
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Err(e) = store.remove(token).await {
            failures.push(e.to_string());
        }
    }
    let cf = match token {
        Some(token) if intents.iter().any(|i| i.abandoned_at.is_none() && !i.rejected) => Some(CloudflareClient::new(
            &state.config.cloudflare_api_base,
            token,
            state.config.acme_timeout,
        )?),
        _ => None,
    };
    for mut intent in intents.into_iter().filter(|i| i.abandoned_at.is_none()) {
        let attempt:anyhow::Result<()>=async{
    // A refused create made no record: nothing to delete, with or without a token.
    if !intent.rejected {
     let Some(cf)=&cf else {
      return Err(CleanupError::Permanent("Cloudflare 토큰을 열 수 없어 TXT 레코드를 지울 수 없습니다.".into()).into());
     };
     let ids=if !intent.record_ids.is_empty(){intent.record_ids.clone()}else if let Some(id)=&intent.record_id{vec![id.clone()]}else{
      let records=cf.find_owned_txt(&intent.zone_id,&intent.name,&intent.value,&intent.owner).await?;
      if records.is_empty(){
       let now=chrono::Utc::now();
       match intent.unconfirmed_since{
        Some(since) if now-since>=UNCONFIRMED_CREATE_LIMIT=>{
         // Invisible for the whole limit: treat the create as never made and let the job end.
         let db=state.db.get().await?;
         db.execute("UPDATE certificates SET issue_log=issue_log||$2::jsonb WHERE id=$1",&[&certificate_id,&json!([{"at":now,"level":"error","msg":format!("Cloudflare TXT 레코드({})가 만들어졌는지 {}분 동안 확인하지 못해 만들어지지 않은 것으로 보고 정리를 마쳤습니다. Cloudflare 에 남아 있으면 직접 지우세요.",intent.name,UNCONFIRMED_CREATE_LIMIT.num_minutes())}])]).await?;
        },
        Some(_)=>return Err(CleanupError::Pending("Cloudflare 생성 결과를 아직 확인할 수 없습니다. 정리를 재시도합니다.".into()).into()),
        None=>{
         intent.unconfirmed_since=Some(now);
         let db=state.db.get().await?;db.execute(REPLACE_INTENT,&[&job_id,&intent.owner,&json!(intent)]).await?;
         return Err(CleanupError::Pending("Cloudflare 생성 결과를 아직 확인할 수 없습니다. 정리를 재시도합니다.".into()).into());
        },
       }
       Vec::new()
      }else{
       intent.record_ids=records.into_iter().map(|r|r.record_id).collect();
       // Persist discovered IDs BEFORE deleting: a crash after remote deletion
       // must not turn a known record back into an ambiguous missing create.
       let db=state.db.get().await?;db.execute(REPLACE_INTENT,&[&job_id,&intent.owner,&json!(intent)]).await?;
       intent.record_ids.clone()
      }
     };
     for id in ids {cf.delete_record(&intent.zone_id,&id).await?;}
    }
    let db=state.db.get().await?;db.execute("UPDATE certificate_jobs SET cf_records=COALESCE((SELECT jsonb_agg(item) FROM jsonb_array_elements(cf_records) item WHERE item->>'owner'<>$2),'[]'::jsonb) WHERE id=$1",&[&job_id,&intent.owner]).await?;Ok(())
   }.await;
        let Err(error) = attempt else { continue };
        if let Some(CleanupError::Pending(_)) = error.downcast_ref() {
            failures.push(error.to_string());
            continue;
        }
        // The failure streak survives restarts in the intent itself.
        let now = chrono::Utc::now();
        let since = *intent.failing_since.get_or_insert(now);
        let limit = if permanent(&error) {
            PERMANENT_CLEANUP_LIMIT
        } else {
            TRANSIENT_CLEANUP_LIMIT
        };
        intent.last_error = Some(error.to_string());
        if now - since >= limit {
            intent.abandoned_at = Some(now);
        } else {
            failures.push(error.to_string());
        }
        let db = state.db.get().await?;
        db.execute(REPLACE_INTENT, &[&job_id, &intent.owner, &json!(intent)])
            .await?;
    }
    if !failures.is_empty() {
        let db = state.db.get().await?;
        db.execute(
            "UPDATE certificate_jobs SET updated_at=GREATEST(updated_at,now()) WHERE id=$1",
            &[&job_id],
        )
        .await?;
        db.execute(
            "UPDATE certificates SET last_error=$2 WHERE id=$1",
            &[
                &certificate_id,
                &format!("{CLEANUP_INCOMPLETE}{}", failures.join("; ")),
            ],
        )
        .await?;
        return Ok(());
    }
    // Every record is removed or given up. Given-up records stay in cf_records: they are the
    // only reference for removing them by hand. The sealed snapshot (token, account key) goes.
    let mut db = state.db.get().await?;
    let tx = db.transaction().await?;
    let done = tx.query_opt("UPDATE certificate_jobs SET state=CASE WHEN cancel_requested THEN 'cancelled' WHEN error IS NOT NULL THEN 'failed' ELSE 'succeeded' END,payload_enc='',http_tokens='[]',updated_at=now() WHERE id=$1 AND state='cleanup' AND NOT deploy_pending AND http_tokens='[]'::jsonb AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(cf_records) item WHERE item->>'abandoned_at' IS NULL) RETURNING error,cf_records",&[&job_id]).await?;
    if let Some(done) = done {
        let error: Option<String> = done.get(0);
        let left: Vec<CfIntent> = serde_json::from_value(done.get(1))?;
        // A finished cleanup gives the certificate back the job's own outcome in place of the
        // cleanup progress message it showed meanwhile. A job that succeeded but left records
        // shows what to remove instead.
        let shown = if left.is_empty() {
            error.clone()
        } else {
            let message = announce_abandoned(&tx, job_id, certificate_id, &left).await?;
            error.clone().or(Some(message))
        };
        tx.execute("UPDATE certificates SET last_error=$2 WHERE id=$1 AND (starts_with(last_error,$3) OR starts_with(last_error,$4))",&[&certificate_id,&shown,&CLEANUP_INCOMPLETE,&CLEANUP_RETRY]).await?;
    }
    tx.commit().await?;
    Ok(())
}
/// Records that cleanup gave up on, as an operator can find them in the Cloudflare dashboard.
/// Never a token or sealed data: zone, name, TXT value (public in DNS) and record IDs only.
fn abandoned_details(left: &[CfIntent]) -> (String, Value) {
    let text = left
        .iter()
        .map(|i| {
            let ids = if !i.record_ids.is_empty() {
                i.record_ids.join(", ")
            } else {
                i.record_id.clone().unwrap_or_else(|| "알 수 없음".into())
            };
            format!("{} (zone {}, 레코드 ID {ids}, 값 {})", i.name, i.zone_id, i.value)
        })
        .collect::<Vec<_>>()
        .join("; ");
    let records = left
        .iter()
        .map(|i| {
            json!({"zoneId":i.zone_id,"name":i.name,"value":i.value,"recordId":i.record_id,
                "recordIds":i.record_ids,"error":i.last_error,"failingSince":i.failing_since,"abandonedAt":i.abandoned_at})
        })
        .collect();
    (text, Value::Array(records))
}
/// Once per job, in the transaction that ends it: the certificate log, the audit trail and one
/// in-app notification. Returns the message shown on the certificate.
async fn announce_abandoned(
    tx: &deadpool_postgres::Transaction<'_>,
    job_id: &str,
    certificate_id: &str,
    left: &[CfIntent],
) -> anyhow::Result<String> {
    let (records, details) = abandoned_details(left);
    let message = format!(
        "Cloudflare TXT 레코드를 지우지 못해 정리를 멈췄습니다. 레코드가 Cloudflare 에 남아 있을 수 있으니 직접 지우세요: {records}"
    );
    let reasons: Vec<_> = left.iter().filter_map(|record| record.last_error.as_deref()).map(single_line).collect();
    let log_message = if reasons.is_empty() {
        message.clone()
    } else {
        format!("{message} — {}", reasons.join("; "))
    };
    tx.execute(
        "UPDATE certificates SET issue_log=issue_log||$2::jsonb WHERE id=$1",
        &[&certificate_id, &issue_entry("error", &log_message)],
    )
    .await?;
    let system = Actor {
        via: None,
        id: None,
        label: Some("system:certificate-cleanup".into()),
        ip: None,
    };
    store::audit(
        &**tx,
        &system,
        "cert:cleanup-abandoned",
        certificate_id,
        json!({"jobId":job_id,"records":details}),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.message))?;
    let certificate = tx
        .query_opt(
            "SELECT jsonb_build_object('certificateId',id,'domains',domains) FROM certificates WHERE id=$1",
            &[&certificate_id],
        )
        .await?;
    // A deleted certificate still leaves its records behind; the notice then has no link.
    let link = certificate
        .is_some()
        .then(|| format!("/certificates/{certificate_id}"));
    let mut data: Value = certificate
        .map(|row| row.get(0))
        .unwrap_or_else(|| json!({"certificateId":certificate_id}));
    data["records"] = details;
    // In-app only, like other operational notices: it is not a subscribable event type.
    tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,dedupe_key) VALUES($1,'cert.cleanup_failed','warning',$2,$3,$4,$5,$6) ON CONFLICT(dedupe_key) DO NOTHING",&[&uuid::Uuid::new_v4().to_string(),&"인증서 발급 후 Cloudflare TXT 레코드를 지우지 못했습니다",&format!("직접 지우세요: {records}"),&link,&data,&format!("cert.cleanup_failed:{job_id}")]).await?;
    Ok(message)
}
async fn cleanup_attempt(
    state: &AppState,
    job_id: &str,
    certificate_id: &str,
    token: Option<&str>,
) -> anyhow::Result<()> {
    if let Err(error) = cleanup(state, job_id, certificate_id, token).await {
        let db = state.db.get().await?;
        db.execute(
            "UPDATE certificate_jobs SET updated_at=GREATEST(updated_at,now()) WHERE id=$1",
            &[&job_id],
        )
        .await?;
        db.execute("UPDATE certificates SET last_error=$2 WHERE id=$1 AND EXISTS(SELECT 1 FROM certificate_jobs WHERE id=$3 AND state='cleanup')",&[&certificate_id,&format!("{CLEANUP_RETRY}{error}"),&job_id]).await?;
    }
    Ok(())
}
/// Finished jobs hold no secrets (payload is erased at the end of cleanup); keep
/// them as long as the notification feed, then drop them. Jobs in progress stay. The
/// given-up TXT records a finished job keeps go with it; the audit trail keeps them.
async fn prune_finished(state: &AppState) -> anyhow::Result<u64> {
    let db = state.db.get().await?;
    Ok(db.execute("DELETE FROM certificate_jobs WHERE state IN ('succeeded','failed','cancelled') AND updated_at<now()-interval '90 days'",&[]).await?)
}
const RENEWAL_DUE: &str = "source='acme' AND status='issued' AND expires_at<now()+interval '30 days' AND (last_attempt_at IS NULL OR last_attempt_at<now()-interval '12 hours') AND (challenge_type='http-01' OR dns_provider='cloudflare') AND dns_order IS NULL AND NOT EXISTS(SELECT 1 FROM certificate_jobs j WHERE j.certificate_id=c.id AND j.state IN ('queued','active','cleanup'))";
/// Every due certificate is attempted: one that cannot be queued is reported and the sweep
/// goes on. Refusals that only mean the certificate stopped being due since the sweep read it
/// (a job started, it was changed or deleted) are not failures.
async fn renew_due(state: &AppState) -> anyhow::Result<()> {
    let db = state.db.get().await?;
    // Soonest expiry first.
    let rows = db
        .query(
            &format!("SELECT id,challenge_type::text,domains,expires_at FROM certificates c WHERE {RENEWAL_DUE} ORDER BY expires_at,id"),
            &[],
        )
        .await?;
    drop(db);
    let actor = Actor {
        via: None,
        id: None,
        label: Some("system:certificate-renewal".into()),
        ip: None,
    };
    for row in rows {
        let id: String = row.get(0);
        let challenge: String = row.get(1);
        let domains: Value = row.get(2);
        let expires: Option<chrono::DateTime<chrono::Utc>> = row.get(3);
        let names = domains
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let Err(error) =
            super::service::queue(state, &id, &actor, "renew", challenge == "dns-01").await
        else {
            continue;
        };
        match record_renewal_error(state, &id, &names, expires, &error.message).await {
            Ok(true) => eprintln!("[certificates] automatic renewal of {id} ({names}) could not be queued: {}", error.message),
            Ok(false) => eprintln!("[certificates] automatic renewal of {id} ({names}) skipped: no longer due ({})", error.message),
            // Usually the database itself; the next sweep tries again.
            Err(record) => eprintln!("[certificates] automatic renewal of {id} ({names}) could not be queued: {} (not recorded: {record:#})", error.message),
        }
    }
    Ok(())
}
/// Records the failure only if the certificate is still due, checked under its row lock — the
/// lock `service::queue` takes. A job queued in the meantime (a manual renewal, a retry) has
/// either committed, and the certificate is no longer due, or waits until this is recorded and
/// then replaces it; a stale failure never lands on top of a newer attempt. Returns whether it
/// was recorded.
///
/// `last_attempt_at` spaces the next try by the sweep's own 12 hours, restarts included. The
/// same failure is announced once per certificate expiry; a different one is announced again.
async fn record_renewal_error(
    state: &AppState,
    id: &str,
    names: &str,
    expires: Option<chrono::DateTime<chrono::Utc>>,
    error: &str,
) -> anyhow::Result<bool> {
    use sha2::Digest as _;
    let error = single_line(error);
    let message = "인증서 자동 갱신을 시작하지 못했습니다.";
    let mut db = state.db.get().await?;
    let tx = db.transaction().await?;
    let due = tx
        .query_opt(&format!("SELECT 1 FROM certificates c WHERE {RENEWAL_DUE} AND id=$1 FOR UPDATE"), &[&id])
        .await?
        .is_some();
    if due {
        tx.execute("UPDATE certificates SET last_error=$2,last_attempt_at=now(),issue_log=issue_log||$3::jsonb WHERE id=$1",&[&id,&error,&issue_entry("error", &format!("{message} {error}"))]).await?;
        let mut data: Value = tx.query_one("SELECT jsonb_build_object('certificateId',id,'domains',domains,'expiresAt',expires_at,'staging',staging) FROM certificates WHERE id=$1",&[&id]).await?.get(0);
        data["error"] = json!(error);
        let key = format!(
            "cert.renew_failed:queue:{id}:{}:{}",
            expires.map(|e| e.to_rfc3339()).unwrap_or_default(),
            hex::encode(&sha2::Sha256::digest(error.as_bytes())[..8])
        );
        let body = format!("{names} — {}", error.chars().take(120).collect::<String>());
        tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,dedupe_key) VALUES($1,'cert.renew_failed','warning',$2,$3,$4,$5,$6) ON CONFLICT(dedupe_key) DO NOTHING",&[&uuid::Uuid::new_v4().to_string(),&message,&body,&format!("/certificates/{id}"),&data,&key]).await?;
    }
    tx.commit().await?;
    Ok(due)
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn retry_classification_keeps_deterministic_errors_fatal() {
        let actor = serde_json::from_value::<Actor>(json!("not-an-actor")).unwrap_err();
        assert!(!transient_database_error(&actor.into()));
        assert!(!transient_database_error(
            &deadpool_postgres::PoolError::NoRuntimeSpecified.into()
        ));
        assert!(transient_database_error(
            &deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Wait).into()
        ));
    }
    #[test]
    fn retry_gaps_grow_and_cap_at_sixty_seconds() {
        let actual: Vec<_> = (0..9).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(actual, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }
}

#[cfg(test)]
mod issuance_feedback_tests {
    use super::*;

    #[test]
    fn failed_issue_entry_has_the_same_reason_and_keys() {
        let reason = "ACME 주문 상태가 유효하지 않습니다: Invalid. 인증기관이 알려 준 이유 — example.test: dns — NXDOMAIN 인증기관이 이 도메인의 DNS 레코드를 찾지 못했습니다.";
        let generic = "인증서 발급이 중단되었거나 실패했습니다.";
        let entries = finish_issue_entry(generic, Some(reason), false);
        let entry = &entries[0];
        assert_eq!(entries.as_array().unwrap().len(), 1);
        assert_eq!(entry.as_object().unwrap().len(), 3);
        assert!(chrono::DateTime::parse_from_rfc3339(entry["at"].as_str().unwrap()).is_ok());
        assert_eq!(entry["level"], "error");
        assert_eq!(entry["msg"], reason);
        assert_eq!(finish_issue_entry(generic, Some(CANCELLED), true)[0]["msg"], generic);
    }

    #[test]
    fn issuance_lines_cover_start_success_failure_and_dns_wait() {
        let payload = JobPayload {
            directory: "https://reader:directory-secret@ca.example.test/directory?q=directory-secret".into(),
            account_key: "private-test-key".into(),
            email: "account@example.test".into(),
            domains: vec!["example.test".into(), "www.example.test".into()],
            dns_token: Some("provider-test-token".into()),
            dns_order: Some(json!({"records":[{"recordValue":"dns-test-value"}]})),
        };
        let id = "cert-123";
        let names = &payload.domains;
        let start = issuance_line(id, names, IssuanceEvent::Start {
            mode: "renew", staging: true, directory: &payload.directory,
        });
        assert_eq!(start, "[certificates] issuance started: id=cert-123 names=example.test, www.example.test mode=renewal ca=staging directory_host=ca.example.test");
        assert!(issuance_line(id, names, IssuanceEvent::Start {
            mode: "initial", staging: false, directory: &payload.directory,
        }).ends_with("mode=initial ca=production directory_host=ca.example.test"));
        let expiry = chrono::DateTime::parse_from_rfc3339("2027-02-26T00:00:00Z").unwrap().to_utc();
        assert_eq!(issuance_line(id, names, IssuanceEvent::Success(expiry)),
            "[certificates] issuance succeeded: id=cert-123 names=example.test, www.example.test expires=2027-02-26T00:00:00+00:00");
        assert_eq!(issuance_line(id, names, IssuanceEvent::DnsWait),
            "[certificates] issuance waiting for DNS TXT records: id=cert-123 names=example.test, www.example.test");

        let trace = IssuanceTrace {
            order_url: Some("https://ca.example.test/order/123?token=order-test-token".into()),
            secrets: vec!["challenge-test-token".into(), "challenge-test-token.thumbprint".into(), "eab-test-value".into()],
        };
        let reason = issuance_reason(&payload, &trace, "dns — NXDOMAIN\naccount@example.test\tchallenge-test-token.thumbprint challenge-test-token eab-test-value private-test-key provider-test-token dns-test-value");
        let failure = issuance_line(id, names, IssuanceEvent::Failure {
            reason: &reason, order_url: trace.order_url.as_deref(),
        });
        assert_eq!(failure, format!("[certificates] issuance failed: id=cert-123 names=example.test, www.example.test reason={reason} order_url=https://ca.example.test/order/123"));
        assert!(reason.starts_with("dns — NXDOMAIN "));
        assert_eq!(failure.lines().count(), 1);
        for secret in ["account@example.test", "challenge-test-token", "thumbprint", "eab-test-value",
            "private-test-key", "provider-test-token", "dns-test-value", "order-test-token"] {
            assert!(!failure.contains(secret), "leaked {secret}");
        }
        let failure = issuance_line(id, names, IssuanceEvent::Failure { reason: RESTARTED, order_url: None });
        assert!(failure.ends_with(RESTARTED));
    }
}
