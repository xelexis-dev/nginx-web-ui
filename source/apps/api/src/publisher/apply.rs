#[cfg(test)]
use super::{resources::Resource, validation};
use super::{
    files::{self, CertificateMaterial, FileBackup, RenderedConfig},
    render,
};
#[cfg(test)]
use super::process::NginxProcess;
use crate::{config::RuntimeConfig, contracts::Actor, state::AppState};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap,BTreeSet};

#[cfg(test)]
pub use crate::control::model::RestoredCounts;
pub use crate::control::model::{ ApplyResult, SkippedKind, SkippedRecord, RenderOutcome, ConfigSnapshotData};

pub(crate) use crate::control::model::{load_snapshot, load_applied_snapshot, skipped_summary};

#[cfg(test)]
pub fn render_config(config: &RuntimeConfig, data: &ConfigSnapshotData, certificates: &[CertificateMaterial]) -> RenderOutcome {
    let certificates: Vec<_> = certificates.iter().map(|cert| crate::control::model::CertificateRef {
        id: cert.id.clone(), status: "issued".into(),
        usable: !cert.cert_pem.trim().is_empty() && !cert.key_pem.trim().is_empty(),
    }).collect();
    render_with_refs(config, data, &certificates)
}

pub fn render_with_refs(config: &RuntimeConfig, data: &ConfigSnapshotData, certificates: &[crate::control::model::CertificateRef]) -> RenderOutcome {
    let runtime_dns = crate::dns_guard::status(config);
    crate::control::model_render::render_with_context(config, data, certificates, &crate::control::model_render::RenderContext {
        runtime_dns: &runtime_dns, resolve: &super::internal_targets::system_resolve,
        is_local_addr: &super::internal_targets::is_local_addr,
    })
}

pub(crate) fn extend_containment(outcome: &mut RenderOutcome, data: &ConfigSnapshotData, actions: &[super::containment::Action]) {
    use super::containment::Unit;
    for action in actions {
        let (kind, row) = match &action.unit {
            Unit::ConfFile(name) => (SkippedKind::Server, name.strip_prefix("server-").and_then(|n| n.strip_suffix(".conf")).and_then(|id| data.servers.iter().find(|r| r["id"].as_str() == Some(id)))),
            Unit::StreamFile(name) => (SkippedKind::Stream, name.strip_prefix("stream-").and_then(|n| n.strip_suffix(".conf")).and_then(|id| data.streams.iter().find(|r| r["id"].as_str() == Some(id)))),
            Unit::UpstreamBlock(name) => (SkippedKind::Upstream, data.upstreams.iter().find(|r| r["name"].as_str() == Some(name))),
        };
        let Some(row) = row else { continue };
        let label = match kind {
            SkippedKind::Server => row_label(row, "serverName"),
            SkippedKind::Upstream => row_label(row, "name"),
            SkippedKind::Stream => format!("{} :{} → {}", row["protocol"].as_str().unwrap_or("?"), row["listen"], row["proxyPass"].as_str().unwrap_or("")),
        };
        let record=SkippedRecord { kind, id: row["id"].as_str().unwrap_or("").into(), label, reason: format!("nginx -t: {}", action.reason), kept_previous: action.kept_previous };
        if let Some(previous)=outcome.skipped.iter_mut().find(|s|s.kind==record.kind&&s.id==record.id){*previous=record;}else{outcome.skipped.push(record);}
    }
}
use crate::control::model_render::row_label;

#[cfg(test)]
pub(crate) struct AppliedFiles {
    pub result: ApplyResult,
    pub backup: Option<FileBackup>,
    pub actions: Vec<super::containment::Action>,
    pub notices: Vec<String>,
    restored: Option<RestoredCounts>,
    pub previous_confirmed: bool,
}

pub(super) async fn inspect_targets(config:std::sync::Arc<RuntimeConfig>)->Result<(RenderedConfig,super::guard_targets::TargetScan,Vec<String>)> {
    let live=RenderedConfig {
        conf:files::managed_files(&config.nginx_conf_dir,files::ManagedKind::Conf).await?,
        stream:files::managed_files(&config.nginx_stream_dir,files::ManagedKind::Stream).await?,
    };
    Ok(super::internal_targets::blocking(move || {
        // One answer per endpoint within a pass; each pass resolves afresh.
        let answers=std::cell::RefCell::new(BTreeMap::new());
        let resolve=|host:&str,port:u16|answers.borrow_mut().entry((host.to_owned(),port)).or_insert_with(||super::internal_targets::system_resolve(host,port)).clone();
        let mut scan=super::guard_targets::scan(&config,&live.conf,&live.stream,&resolve);
        let socket_actions=super::unix_targets::extend_scan(&live.conf,&live.stream,&mut scan);
        let notices=if scan.is_empty(){super::guard_targets::cycle_notices(&config,&live.conf,&resolve)}else{super::unix_targets::notices(&socket_actions)};
        (live,scan,notices)
    }).await?)
}

async fn isolate_targets(config:&RuntimeConfig,mut live:RenderedConfig,scan:super::guard_targets::TargetScan,actions:&mut Vec<super::containment::Action>,mutation:Option<&mut super::mutation::Mutation>)->Result<String> {
    use super::containment::{Action,Unit};
    let start=actions.len();
    let mut changed=false;
    let reason="대상 재검사에서 내부 서비스·자기 포트·사용할 수 없는 인증서를 발견해 제외했습니다.";
    for name in scan.conf {
        if name==super::guard_targets::UPSTREAMS_FILE {continue;}
        changed|=live.conf.remove(&name).is_some();
        actions.push(Action{unit:Unit::ConfFile(name),kept_previous:false,reason:reason.into()});
    }
    for name in scan.stream {
        changed|=live.stream.remove(&name).is_some();
        actions.push(Action{unit:Unit::StreamFile(name),kept_previous:false,reason:reason.into()});
    }
    if let Some(bytes)=scan.upstreams {
        changed|=live.conf.get(super::guard_targets::UPSTREAMS_FILE)!=Some(&bytes);
        live.conf.insert(super::guard_targets::UPSTREAMS_FILE.into(),bytes);
    }
    for name in scan.removed_upstreams {actions.push(Action{unit:Unit::UpstreamBlock(name),kept_previous:false,reason:reason.into()});}
    ensure!(changed&&actions.len()>start,"대상 재검사에서 위험한 설정을 제외하지 못했습니다. 격리할 파일·블록이 없어 적용을 중단합니다.");
    if let Some(writer)=mutation {writer.write_candidate(config,&live).await?;}else{files::write_rendered(config,&live).await?;}
    Ok(actions[start..].iter().map(|action| {
        let name=match &action.unit {Unit::ConfFile(name)|Unit::StreamFile(name)|Unit::UpstreamBlock(name)=>name};
        format!("\n{name}: {}",action.reason)
    }).collect())
}

pub(super) fn target_hit_summary(live:&RenderedConfig,scan:&super::guard_targets::TargetScan)->String {
    let mut hits=Vec::new();
    for name in scan.conf.iter().chain(&scan.stream) {
        let bytes=live.conf.get(name).or_else(||live.stream.get(name));
        let mut endpoints=BTreeSet::new();
        for statement in bytes.into_iter().flat_map(|bytes|super::guard_targets::statements(&String::from_utf8_lossy(bytes))) {
            if !statement.words.first().is_some_and(|name|matches!(name.as_str(),"proxy_pass"|"server")){continue;}
            let Some(raw)=statement.words.get(1) else {continue;};
            let target=if raw.contains("://"){raw.clone()}else{format!("http://{raw}")};
            if let Ok(url)=url::Url::parse(&target) {
                if let (Some(host),Some(port))=(url.host_str(),url.port_or_known_default()){endpoints.insert(format!("{host}:{port}"));}
            }
        }
        hits.push(format!("{name} [{}]",endpoints.into_iter().collect::<Vec<_>>().join(", ")));
    }
    hits.extend(scan.removed_upstreams.iter().map(|name|format!("upstream {name}")));
    hits.join(", ")
}
#[cfg(test)]
pub(crate) async fn restore_after_reload(state: &AppState, backup: &FileBackup) -> Result<()> {
    backup.restore(&state.config).await?;
    let witness = super::confirmation::Witness::from_config(&state.config)?;
    let first = uuid::Uuid::new_v4().to_string();
    super::confirmation::publish(&state.config, &first).await?;
    let valid = NginxProcess::new(&state.config.nginx_bin).validate().await;
    ensure!(valid.ok, "Restored files failed nginx validation: {}", valid.output);
    witness.confirm(&state.config, &first, std::time::Instant::now()+std::time::Duration::from_secs(15)).await
}

// A restored worker alone cannot attest that the metadata transaction rolled back.
#[cfg(test)]
pub(crate) async fn finish_rollback(state: &AppState, staged: &mut AppliedFiles) -> Result<()> {
    if let Some(counts) = staged.restored.take().filter(|_|staged.previous_confirmed) {
        super::confirmation::clear_uncertain(&state.config).await?;
        staged.result.rolled_back_to = Some(counts);
    }
    Ok(())
}
#[cfg(test)]
pub(crate) async fn stage_files(
    state: &AppState,
    rendered: &RenderedConfig,
    certificates: &[CertificateMaterial],
    previous: Option<&ConfigSnapshotData>,
    budget: &super::confirmation::Budget,
) -> Result<AppliedFiles> {
    budget.forward()?;
    let previous_confirmed=!super::confirmation::uncertain(&state.config).await;
    let backup = FileBackup::capture(&state.config).await?;
    let witness = super::confirmation::Witness::from_config(&state.config)?;
    super::confirmation::begin_uncertain(&state.config).await?;
    let process = NginxProcess::new(&state.config.nginx_bin);
    let mut actions = Vec::new();
    let mut notices = Vec::new();
    let attempt = async {
        files::materialize_certificates(&state.config, &state.secrets, certificates).await?;
        let previous = if previous_confirmed {match previous {
            Some(snapshot) => super::containment::eligible_previous(&backup, snapshot),
            None => RenderedConfig { conf: backup.conf.clone(), stream: backup.stream.clone() },
        }}else{RenderedConfig::default()};
        let mut report = super::containment::contain_rendered_until(&state.config, rendered, &previous, budget.containment_deadline()).await?;
        if !previous_confirmed {super::containment::explain_unconfirmed_previous(&mut report);}
        actions = report.actions;
        ensure!(report.validation.ok, "{}", report.validation.output);
        let mut output=report.validation.output;
        loop {
            // P is created only after every non-marker write. C changes only its UUID literal.
            budget.forward()?;
            let first = uuid::Uuid::new_v4().to_string();
            super::confirmation::publish(&state.config, &first).await?;
            let frozen = process.validate().await;
            ensure!(frozen.ok, "{}", frozen.output);
            let (live,scan,isolation_notices)=inspect_targets(state.config.clone()).await?;
            budget.forward()?;
            if !scan.is_empty() {
                output.push_str(&isolate_targets(&state.config,live,scan,&mut actions,None).await?);
                for notice in isolation_notices {output.push_str(&format!("\n{notice}"));}
                continue;
            }
            witness.confirm(&state.config, &first, budget.ack_deadline()?).await?;
            let (live,scan,cycle_notices)=inspect_targets(state.config.clone()).await?;
            budget.forward()?;
            if scan.is_empty() {
                notices=cycle_notices;
                for notice in &notices {output.push_str(&format!("\n\n{notice}"));}
                break;
            }
            output.push_str(&isolate_targets(&state.config,live,scan,&mut actions,None).await?);
            for notice in cycle_notices {output.push_str(&format!("\n{notice}"));}
        }
        Ok::<_, anyhow::Error>(output)
    }.await;
    match attempt {
        Ok(output) => Ok(AppliedFiles {
            result: ApplyResult {ok:true, output, rolled_back_to:None},
            backup:Some(backup), actions, notices, restored:None, previous_confirmed,
        }),
        Err(error) => {
            let mut output=format!("{error:#}");
            let restored=match restore_after_reload(state,&backup).await {
                Ok(())=>Some(RestoredCounts{conf:backup.conf.len(),stream:backup.stream.len()}),
                Err(error)=>{output.push_str(&format!("\n이전 설정의 동작을 확인하지 못했습니다. 설정을 확인하세요. ({error:#})"));None},
            };
            if restored.is_some() && !previous_confirmed {
                output.push_str("\n원래 파일의 동작은 확인했지만, 직전 적용 상태가 불확실해 기준 설정을 확정하지 않았습니다. 다시 적용하세요.");
            }
            Ok(AppliedFiles{result:ApplyResult{ok:false,output,rolled_back_to:None},backup:None,actions:Vec::new(),notices:Vec::new(),restored,previous_confirmed})
        },
    }
}

pub(super) struct DurableApplied {
    pub result:ApplyResult,
    pub actions:Vec<super::containment::Action>,
    pub notices:Vec<String>,
}
/// Caller retains the transaction and kernel so any failure first rolls back SQL, then restores.
pub(super) async fn stage_durable(
    state:&AppState,rendered:&RenderedConfig,previous:&ConfigSnapshotData,budget:&super::confirmation::Budget,mutation:&mut super::mutation::Mutation,
)->Result<DurableApplied> {
    budget.forward()?;
    let backup=FileBackup::capture(&state.config).await?;
    let previous=if mutation.previous_confirmed {super::containment::eligible_previous(&backup,previous)}else{RenderedConfig::default()};
    let mut report=super::containment::contain_durable_until(&state.config,rendered,&previous,budget.containment_deadline(),mutation).await?;
    if !mutation.previous_confirmed {super::containment::explain_unconfirmed_previous(&mut report);}
    ensure!(report.validation.ok,"{}",report.validation.output);
    let mut output=report.validation.output;let mut actions=report.actions;
    let notices=loop {
        // Preserve both W-11 passes: after frozen validation, and after the fresh C observation.
        let (first,_)=mutation.freeze(&state.config,budget).await?;
        let (live,scan,isolation_notices)=inspect_targets(state.config.clone()).await?;
        budget.forward()?;
        if !scan.is_empty() {
            output.push_str(&isolate_targets(&state.config,live,scan,&mut actions,Some(mutation)).await?);
            for notice in isolation_notices {output.push_str(&format!("\n{notice}"));}
            continue;
        }
        mutation.observe(&state.config,&first,budget).await?;
        let (live,scan,notices)=inspect_targets(state.config.clone()).await?;
        budget.forward()?;
        if scan.is_empty() {break notices;}
        output.push_str(&isolate_targets(&state.config,live,scan,&mut actions,Some(mutation)).await?);
        for notice in notices {output.push_str(&format!("\n{notice}"));}
    };
    for notice in &notices {output.push_str(&format!("\n\n{notice}"));}
    Ok(DurableApplied{result:ApplyResult{ok:true,output,rolled_back_to:None},actions,notices})
}

/// Low-level serialized file application, without history/DB mutation. The
/// spawned owner finishes even when the requesting HTTP future is cancelled.
/// `certificates` must contain the complete issued-certificate set, since absent
/// certificate material is removed. Resource/API callers use the DB-backed apply
/// entrypoints below instead.
#[cfg(test)]
pub async fn apply_rendered(
    state: &AppState,
    rendered: &RenderedConfig,
    certificates: &[CertificateMaterial],
) -> Result<ApplyResult> {
    let state = state.clone();
    let rendered = rendered.clone();
    let certificates = certificates.to_vec();
    tokio::spawn(async move {
        let budget=super::confirmation::Budget::new();
        let _guard = budget.admit(state.runtime.apply_lock.lock()).await?;
        let _files = budget.admit(super::nginx_lock::acquire(&state.config)).await??;
        let mut staged=stage_files(&state, &rendered, &certificates, None, &budget).await?;
        if staged.result.ok {super::confirmation::clear_uncertain(&state.config).await?;}
        else {finish_rollback(&state,&mut staged).await?;}
        Ok(staged.result)
    })
    .await?
}

pub(crate) async fn lock_tables(tx: &deadpool_postgres::Transaction<'_>, budget:&super::confirmation::Budget) -> Result<()> {
    let milliseconds=budget.admission_remaining()?.as_millis().max(1);
    tx.batch_execute(&format!("SET LOCAL lock_timeout='{milliseconds}ms'; SET LOCAL statement_timeout='{milliseconds}ms'; LOCK TABLE upstreams,nginx_servers,nginx_streams IN SHARE ROW EXCLUSIVE MODE; LOCK TABLE certificates IN SHARE MODE")).await?;
    Ok(())
}
pub(crate) async fn set_completion_timeout(tx:&deadpool_postgres::Transaction<'_>,budget:&super::confirmation::Budget)->Result<()> {
    let remaining=budget.remaining().saturating_sub(super::confirmation::RESTORE_RESERVE);ensure!(!remaining.is_zero(),"설정 적용의 완료 대기 시간이 지났습니다.");
    tx.batch_execute(&format!("SET LOCAL lock_timeout='{}ms'; SET LOCAL statement_timeout='{}ms'",remaining.as_millis().max(1),remaining.as_millis().max(1))).await?;Ok(())
}
pub(crate) async fn record_applied_state(
    client: &impl deadpool_postgres::GenericClient,
    data: &ConfigSnapshotData,
    skipped: &[SkippedRecord],
    actor_id: Option<&str>,
) -> Result<()> {
    let state = super::pending::applied_state_value(data, skipped);
    client.execute("INSERT INTO applied_config(id,data,actor_id,updated_at) VALUES('current',$1,$2,now()) ON CONFLICT(id) DO UPDATE SET data=excluded.data,actor_id=excluded.actor_id,updated_at=excluded.updated_at",&[&state,&actor_id]).await?;
    Ok(())
}
pub(crate) async fn insert_snapshot(
    client: &impl deadpool_postgres::GenericClient,
    data: &ConfigSnapshotData,
    actor_id: Option<&str>,
    label: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO config_snapshots(id,actor_id,label,data) VALUES($1,$2,$3,$4)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &actor_id,
                &label,
                &json!(data),
            ],
        )
        .await?;
    Ok(())
}
async fn post_execute(
    pool: &deadpool_postgres::Pool,
    budget: &super::confirmation::Budget,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<()> {
    let allowance = budget.remaining().min(std::time::Duration::from_secs(2));
    let deadline = tokio::time::Instant::now() + allowance;
    let mut client = tokio::time::timeout_at(deadline, pool.get()).await??;
    let attempt = tokio::time::timeout_at(deadline, async {
        let tx = client.transaction().await?;
        let ms = deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis().max(1);
        tx.batch_execute(&format!("SET LOCAL lock_timeout='{ms}ms'; SET LOCAL statement_timeout='{ms}ms'")).await?;
        tx.execute(sql, params).await?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(())
    }).await.context("설정 적용 후 기록 대기 시간이 지났습니다.").and_then(|r| r);
    if attempt.is_err() { drop(deadpool_postgres::Client::take(client)); }
    attempt
}

pub(crate) async fn notify_skipped(state: &AppState, skipped: &[SkippedRecord], budget: &super::confirmation::Budget) {
    notify_skipped_pool(&state.db,skipped,budget).await
}
pub(crate) async fn notify_skipped_pool(pool:&deadpool_postgres::Pool, skipped:&[SkippedRecord], budget:&super::confirmation::Budget) {
    if skipped.is_empty() {
        return;
    }
    use sha2::Digest as _;
    let mut identity: Vec<_> = skipped
        .iter()
        .map(|s| (s.kind as u8, s.id.as_str(), s.reason.as_str(), s.kept_previous))
        .collect();
    identity.sort();
    let dedupe_key = format!(
        "config.apply-skipped:{}",
        hex::encode(sha2::Sha256::digest(json!(identity).to_string().as_bytes()))
    );
    let title = if skipped.iter().any(|s| s.kept_previous) {
        format!("설정 {}개의 새 변경을 적용하지 못했습니다", skipped.len())
    } else { format!("설정 {}개를 적용하지 못해 제외했습니다", skipped.len()) };
    let body = format!(
        "나머지 설정은 적용됐습니다. 제외한 항목을 고친 뒤 다시 적용하세요.{}",
        skipped_summary(skipped)
    );
    let attempt = post_execute(pool,budget,"INSERT INTO notifications(id,type,severity,title,body,data,dedupe_key) VALUES($1,'config.apply-skipped','warning',$2,$3,$4,$5) ON CONFLICT(dedupe_key) DO NOTHING",&[&uuid::Uuid::new_v4().to_string(),&title,&body,&json!({"skipped":skipped}),&dedupe_key]).await;
    if let Err(e) = attempt {
        eprintln!("[apply] could not record skipped-configuration notification: {e:#}");
    }
}
pub(crate) async fn notify_notices(state:&AppState,notices:&[String],budget:&super::confirmation::Budget) {
    notify_notices_pool(&state.db,notices,budget).await
}
pub(crate) async fn notify_notices_pool(pool:&deadpool_postgres::Pool,notices:&[String],budget:&super::confirmation::Budget) {
    if notices.is_empty(){return;}
    use sha2::Digest as _;
    let body=notices.join("\n\n");
    let key=format!("config.apply-notice:{}",hex::encode(sha2::Sha256::digest(body.as_bytes())));
    let title=if notices.iter().any(|notice|notice.starts_with("인증서 배포 후")){"인증서 배포 후 프록시 대상을 확인하세요"}else if notices.iter().any(|notice|notice.contains("서버 간 프록시 순환")){"서버 간 프록시 순환 가능성을 확인하세요"}else{"관리 화면 upstream의 접속 정보 전달을 확인하세요"};
    if let Err(error)=post_execute(pool,budget,"INSERT INTO notifications(id,type,severity,title,body,data,dedupe_key) VALUES($1,'config.apply-notice','warning',$2,$3,$4,$5) ON CONFLICT(dedupe_key) DO NOTHING",&[&uuid::Uuid::new_v4().to_string(),&title,&body,&json!({"notices":notices}),&key]).await {
        eprintln!("[apply] could not record configuration notice: {error:#}");
    }
}

pub use crate::control::model::certificate_content_digest;

pub(super) async fn certificate_material(
    tx: &deadpool_postgres::Transaction<'_>,
    state: &AppState,
    id: &str,
    expected: Option<&str>,
) -> Result<CertificateMaterial> {
    let uuid = uuid::Uuid::parse_str(id)?;
    ensure!(uuid.to_string() == id, "Invalid certificate id");
    let row = tx.query_opt("SELECT id,cert_pem,key_pem FROM certificates WHERE id=$1 AND status='issued'", &[&id]).await?
        .context("인증서 자료를 확인할 수 없습니다.")?;
    let cert: String = row.get::<_, Option<String>>(1).context("Missing certificate")?;
    let sealed: String = row.get::<_, Option<String>>(2).context("Missing certificate key")?;
    if let Some(expected) = expected {
        ensure!(certificate_content_digest(&cert, &sealed) == expected, "stale_material");
    }
    ensure!(cert.len() <= 1024 * 1024 && sealed.len() <= 1024 * 1024, "Certificate material exceeds 1 MiB");
    let key = state.secrets.open_maybe_plain(Some(&sealed)).context("인증서 개인키를 열 수 없습니다.")?;
    ensure!(key.len() <= 1024 * 1024, "Certificate key exceeds 1 MiB");
    let material = CertificateMaterial { id: id.into(), cert_pem: cert, key_pem: key };
    tokio::task::spawn_blocking(move || {
        crate::certificates::pem::parse_certificate_pem(&material.cert_pem, &material.key_pem)?;
        Ok::<_, anyhow::Error>(material)
    }).await?
}

pub(super) async fn certificate_activation_notice(
    tx: &deadpool_postgres::Transaction<'_>,
    id: &str,
    digest: &str,
    consumers: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let saved = tx.query("SELECT id FROM nginx_servers WHERE certificate_id=$1 AND enabled ORDER BY id", &[&id]).await?;
    let inactive: Vec<String> = saved.into_iter().map(|row| row.get::<_, String>(0))
        .filter(|server| !consumers.contains(&format!("server-{server}.conf"))).collect();
    if inactive.is_empty() { return Ok(()); }
    let body = format!("인증서가 발급됐습니다 — 연결된 서버 {}개는 「설정 적용」 후 HTTPS로 동작합니다", inactive.len());
    let key = format!("certificate:apply-needed:{id}:{digest}:{}", inactive.join(","));
    tx.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,dedupe_key) VALUES($1,'config.apply-needed','info','인증서 연결 설정을 적용하세요',$2,'/reverse-proxy',$3,$4) ON CONFLICT(dedupe_key) DO NOTHING", &[&uuid::Uuid::new_v4().to_string(),&body,&json!({"certificateId":id,"serverIds":inactive}),&key]).await?;
    Ok(())
}

pub(super) async fn certificate_audit(
    client: &impl deadpool_postgres::GenericClient,
    actor: &Actor,
    id: &str,
    ok: bool,
    output: &str,
) -> Result<()> {
    client.execute("INSERT INTO audit_logs(id,actor_id,actor_label,action,target,\"after\",ip) VALUES($1,$2,$3,'nginx:apply',$4,$5,$6)", &[&uuid::Uuid::new_v4().to_string(),&actor.id,&actor.label,&id,&json!({"source":"certificate","certificateId":id,"ok":ok,"output":output}),&actor.ip]).await?;
    Ok(())
}

#[cfg(test)]
mod fxb_security_tests {
    use super::*;
    #[tokio::test]
    async fn w11_isolation_without_progress_fails_before_writing() {
        let root=tempfile::tempdir().unwrap();
        let config=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://test@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("test-only".into()),
            "NGINX_CONF_D"=>Some(root.path().join("conf").display().to_string()),"NGINX_STREAM_D"=>Some(root.path().join("stream").display().to_string()),_=>None,
        }).unwrap();
        let live=RenderedConfig{conf:BTreeMap::from([("upstreams.conf".into(),b"upstream safe { server 192.0.2.1:9000; }".to_vec())]),stream:BTreeMap::new()};
        for replacement in [None,live.conf.get("upstreams.conf").cloned(),Some(Vec::new())] {
            let mut scan=super::super::guard_targets::TargetScan::default();
            scan.conf.insert("upstreams.conf".into());scan.upstreams=replacement;
            let error=isolate_targets(&config,live.clone(),scan,&mut Vec::new(),None).await.expect_err("a no-op isolation must fail immediately");
            assert!(error.to_string().contains("제외하지 못했습니다"));
            assert!(!config.nginx_conf_dir.exists(),"no-op isolation must not rewrite files");
        }
    }
    #[test]
    fn w11_hit_summary_names_endpoints_without_credentials_or_queries() {
        let live=RenderedConfig{conf:BTreeMap::from([("server-a.conf".into(),b"server { location / { proxy_pass http://test-user:test-password@127.0.0.1:3001/private?token=test-token; } }".to_vec())]),stream:BTreeMap::new()};
        let mut scan=super::super::guard_targets::TargetScan::default();scan.conf.insert("server-a.conf".into());
        let text=target_hit_summary(&live,&scan);
        assert!(text.contains("server-a.conf")&&text.contains("127.0.0.1:3001"));
        for secret in ["test-user","test-password","test-token","/private"]{assert!(!text.contains(secret));}
    }
    #[test]
    fn w11_isolation_overrides_an_earlier_keep_previous_action() {
        use super::super::containment::{Action,Unit};
        let data=ConfigSnapshotData{servers:vec![json!({"id":"a","serverName":"a.test"})],..Default::default()};
        let mut outcome=RenderOutcome::default();
        let actions=[true,false].map(|kept_previous|Action{unit:Unit::ConfFile("server-a.conf".into()),kept_previous,reason:"test".into()});
        extend_containment(&mut outcome,&data,&actions);
        assert_eq!(outcome.skipped.len(),1);
        assert!(!outcome.skipped[0].kept_previous);
        assert!(outcome.effective(&data,Some(&data)).servers.is_empty());
    }
    #[test]
    fn fxb_numeric_named_group_is_not_mistaken_for_a_metadata_ip() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let upstream=json!({"id":"numeric","name":"2852039166","servers":["10.0.0.5:9000"]});
        let row=json!({"id":"good","enabled":true,"serverName":"good.test","listen":18087,"config":{"locations":[{"path":"/","proxyPass":"http://2852039166"}]}});
        super::super::internal_targets::check_built_with_upstreams(&config,Resource::Server,&row,&[upstream.clone()]).unwrap();
        let outcome=render_config(&config,&ConfigSnapshotData{upstreams:vec![upstream],servers:vec![row],streams:vec![]},&[]);
        assert!(outcome.skipped.is_empty(),"{:?}",outcome.skipped);
        let scan=super::super::guard_targets::scan(&config,&outcome.rendered.conf,&outcome.rendered.stream,&|_,_|Vec::new());
        assert!(scan.is_empty(),"{scan:?}");
    }
    #[test]
    fn fxb3_legacy_self_edges_are_skipped_while_admin_paths_remain_live() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let data=ConfigSnapshotData{
            upstreams:vec![json!({"id":"up","name":"backend","servers":["127.0.0.1:18086"]}),json!({"id":"admin","name":"admin_backend","servers":["127.0.0.1:8080"]})],
            servers:vec![json!({"id":"shared-good","enabled":true,"serverName":"shared.test","listen":18088,"config":{"locations":[{"path":"/","upstreamId":"up"}]}}),json!({"id":"loop","enabled":true,"serverName":"loop.test","listen":18086,"config":{"locations":[{"path":"/","upstreamId":"up"}]}}),json!({"id":"good","enabled":true,"serverName":"good.test","listen":18087,"config":{"proxyPass":"http://10.0.0.5:9000"}})],
            streams:vec![json!({"id":"loopstream","enabled":true,"protocol":"tcp","listen":19000,"proxyPass":"127.0.0.1:19000"}),json!({"id":"adminstream","enabled":true,"protocol":"tcp","listen":19001,"proxyPass":"127.0.0.1:8080"})],
        };
        let mut before=data.clone();
        before.upstreams[0]["servers"]=json!(["10.0.0.5:18086"]);
        assert!(render_config(&config,&before,&[]).rendered.conf.contains_key("server-loop.conf"));
        // An upstream edit can create a loop in a previously valid consuming server.
        let rendered=render_config(&config,&data,&[]);
        assert!(rendered.rendered.conf.contains_key("server-good.conf"));
        assert!(rendered.rendered.conf.contains_key("server-shared-good.conf"));
        for id in ["loop","loopstream"] {assert!(rendered.skipped.iter().any(|s|s.id==id),"{id} must be explicitly skipped");}
        assert!(!rendered.rendered.conf.contains_key("server-loop.conf"));assert!(!rendered.rendered.stream.contains_key("stream-loopstream.conf"));
        assert!(rendered.rendered.stream.contains_key("stream-adminstream.conf"));assert!(!rendered.skipped.iter().any(|s|s.id=="admin"||s.id=="adminstream"));
    }
}

#[cfg(test)]
mod fxb2_tests {
    use super::*;
    #[test]
    fn fxb2_unavailable_tls_never_becomes_plaintext_or_blocks_other_servers() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),_=>None}).unwrap();
        let cert_id="cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        let data=ConfigSnapshotData{servers:vec![json!({"id":"bad","serverName":"tls.test","listen":18086,"enabled":true,"certificateId":cert_id,"config":{"proxyPass":"http://127.0.0.1:9000","tlsPort":18443,"redirectHttp":true}}),json!({"id":"good","serverName":"good.test","listen":18087,"enabled":true,"config":{"proxyPass":"http://127.0.0.1:9000"}})],..Default::default()};
        for materials in [vec![],vec![CertificateMaterial{id:cert_id.into(),cert_pem:"".into(),key_pem:"key".into()}],vec![CertificateMaterial{id:cert_id.into(),cert_pem:"pem".into(),key_pem:"".into()}],vec![CertificateMaterial{id:cert_id.into(),cert_pem:" \n".into(),key_pem:"key".into()}]] {
            let outcome=render_config(&config,&data,&materials);
            assert!(!outcome.rendered.conf.contains_key("server-bad.conf"),"unavailable certificate rendered a live server");
            assert!(outcome.rendered.conf.contains_key("server-good.conf"));
            let skipped=outcome.skipped.iter().find(|s|s.id=="bad").expect("report the excluded server");
            assert_eq!(skipped.label,"tls.test");assert!(skipped.reason.contains(cert_id),"{}",skipped.reason);
        }
        let valid=render_config(&config,&data,&[CertificateMaterial{id:cert_id.into(),cert_pem:"pem".into(),key_pem:"key".into()}]);
        assert!(valid.skipped.is_empty());assert!(String::from_utf8_lossy(&valid.rendered.conf["server-bad.conf"]).contains("ssl_certificate "));
    }
}

#[cfg(test)]
mod fxb3_tests {
    use super::*;
    const ADMIN: &str="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    #[test]
    fn fxb3_all_admin_upstream_is_protected_but_mixed_group_is_untrusted() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let header=|name:&str,value:&str|json!({"name":"proxy_set_header","args":[name,value]});
        let data=ConfigSnapshotData{upstreams:vec![json!({"id":ADMIN,"name":"admin_ui","servers":["127.0.0.1:8080","localhost:8080"]}),json!({"id":"mixed","name":"mixed_ui","servers":["127.0.0.1:8080 weight=100","127.0.0.1:19111 weight=1"]})],servers:vec![json!({"id":"all-admin","serverName":"admin.test","listen":18086,"enabled":true,"upstreamId":ADMIN,"config":{"locationDirectives":[header("Host","$host"),header("X-Real-IP","203.0.113.77"),header("X-Forwarded-Proto","$http_x_forwarded_proto")]}}),json!({"id":"mixed","serverName":"mixed.test","listen":18087,"enabled":true,"upstreamId":"mixed","config":{}}),json!({"id":"mapped-direct","serverName":"mapped.test","listen":18085,"enabled":true,"config":{"proxyPass":"http://[::ffff:127.0.0.1]:8080"}})],..Default::default()};
        let outcome=render_config(&config,&data,&[]);assert!(outcome.skipped.is_empty(),"{:?}",outcome.skipped);
        let protected=String::from_utf8_lossy(&outcome.rendered.conf["server-all-admin.conf"]);
        assert!(protected.contains("proxy_bind 127.0.0.2;"));assert_eq!(protected.matches("proxy_set_header X-Real-IP ").count(),1);assert!(protected.contains("proxy_set_header X-Real-IP $remote_addr;"));assert!(protected.contains("proxy_set_header X-Forwarded-Proto $scheme;"));assert!(!protected.contains("203.0.113.77"));
        assert!(!String::from_utf8_lossy(&outcome.rendered.conf["server-mixed.conf"]).contains("proxy_bind"));
        for bytes in outcome.rendered.conf.values(){let mut depth=0;for statement in super::super::guard_targets::statements(std::str::from_utf8(bytes).unwrap()){use super::super::guard_targets::End;match statement.end {End::Open=>depth+=1,End::Close=>depth-=1,End::Semicolon=>if statement.words.first().is_some_and(|name|name=="proxy_set_header"){assert!(depth>=2,"rendered http/server proxy header would need preservation");}}}}
        if let Ok(path)=std::env::var("NWUI_FXB3_NATIVE_EXPORT") {std::fs::write(path,serde_json::to_vec(&json!({"conf":outcome.rendered.conf,"stream":outcome.rendered.stream})).unwrap()).unwrap();}
    }
    #[test]
    fn fxb3_admin_normalization_keeps_parameter_variants_and_mixed_group_bytes() {
        assert!(validation::assert_valid_upstream_server("[::1]:8080").is_err(),"member grammar is intentionally unchanged");
        assert!(validation::assert_valid_upstream_server("localhost:8080 backup").is_err());
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://test@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let data=ConfigSnapshotData{upstreams:vec![json!({"id":"canonical","name":"canonical_admin","servers":["127.0.0.1:8080 weight=2","localhost:8080 weight=2","127.0.0.2:8080 weight=3","127.0.0.3:8080 weight=3","127.0.0.4:8080"]}),json!({"id":"mixed","name":"mixed_admin","servers":["127.0.0.2:8080","127.0.0.1:19111"]})],..Default::default()};
        let outcome=render_config(&config,&data,&[]);assert!(outcome.skipped.is_empty(),"{:?}",outcome.skipped);let text=String::from_utf8_lossy(&outcome.rendered.conf["upstreams.conf"]);
        let canonical=text.split("upstream canonical_admin {").nth(1).unwrap().split('}').next().unwrap();assert_eq!(canonical.matches("server 127.0.0.1:8080").count(),3,"{canonical}");assert!(canonical.contains("weight=2;"));assert!(canonical.contains("weight=3;"));assert!(canonical.contains("server 127.0.0.1:8080;"));assert!(!canonical.contains("localhost")&&!canonical.contains("::"));
        let mixed=text.split("upstream mixed_admin {").nth(1).unwrap().split('}').next().unwrap();assert!(mixed.contains("127.0.0.2:8080"));assert!(mixed.contains("127.0.0.1:19111"));
    }
    #[test]
    fn fxb3_direct_admin_normalization_preserves_uri_and_proxy_phase_authority() {
        for target in ["http://localhost:8080/path?x=1","http://[::ffff:127.0.0.1]:8080/path?x=1"] {
            let authority=target.split("://").nth(1).unwrap().split('/').next().unwrap();
            let cfg=json!({"serverId":ADMIN,"serverName":"admin.test","listen":18085,"locations":[{"path":"/","proxyPass":target,"directives":[{"name":"proxy_set_header","args":["X-Original","$proxy_host:$proxy_port"]},{"name":"proxy_redirect","args":["default"]}]}]});
            let text=render::render_reverse_proxy_with_dns_and_resolve(&cfg,None,&|_,_|Vec::new()).unwrap();
            assert!(text.contains("proxy_pass http://127.0.0.1:8080/path?x=1;"),"{text}");assert!(text.contains(&format!("proxy_set_header Host {authority};")),"{text}");assert!(text.contains(&format!("{authority}:8080")),"{text}");assert!(text.contains(&format!("proxy_redirect {target} /;")),"{text}");
        }
        if let Ok(path)=std::env::var("NWUI_FXB3_NATIVE_EXPORT"){
            let mut cfg=json!({"serverId":ADMIN,"serverName":"host.test","listen":18091,"locations":[{"path":"/","proxyPass":"http://[::ffff:127.0.0.1]:8080/api/probe?x=1","directives":[{"name":"proxy_set_header","args":["X-Original","$proxy_host:$proxy_port"]},{"name":"add_header","args":["X-Observed","$proxy_host","always"]},{"name":"add_header","args":["X-Observed-Port","$proxy_port","always"]},{"name":"expires","args":["$proxy_port"]}]}]});
            for (name,port) in [("host-preserved",18091),("host-return",18092),("host-deny",18093),("host-proxy-deny",18094)]{
                cfg["listen"]=json!(port);let mut item=cfg.clone();
                if name=="host-return"{item["locations"][0]["directives"].as_array_mut().unwrap().push(json!({"name":"return","args":["403"]}));}
                if name=="host-deny"{item["locations"][0]["ipAccess"]=json!({"mode":"whitelist","entries":["192.0.2.1"]});}
                if name=="host-proxy-deny"{item["locations"][0]["proxyPass"]=json!("http://[::ffff:127.0.0.1]:8080/api/probe?fail=1");}
                let text=render::render_reverse_proxy_with_dns_and_resolve(&item,None,&|_,_|Vec::new()).unwrap();std::fs::write(std::path::Path::new(&path).with_file_name(format!("{name}.conf")),text).unwrap();
            }
        }
    }
    #[test]
    fn fxb3_admin_uri_variables_preserve_implicit_and_explicit_redirect_defaults() {
        let mut cfg=json!({"serverId":ADMIN,"serverName":"variable.test","listen":18095,"locations":[{"path":"/","proxyPass":"http://[::ffff:127.0.0.1]:8080/redirect-probe?q=$arg_q","directives":[{"name":"proxy_set_header","args":["X-Original","$proxy_host:$proxy_port"]}]}]});
        let text=render::render_reverse_proxy_with_dns_and_resolve(&cfg,None,&|_,_|Vec::new()).unwrap();
        if let Ok(path)=std::env::var("NWUI_FXB3_NATIVE_EXPORT"){std::fs::write(std::path::Path::new(&path).with_file_name("uri-variable.conf"),&text).unwrap();}
        cfg["locations"][0]["directives"].as_array_mut().unwrap().push(json!({"name":"proxy_redirect","args":["default"]}));
        let explicit=render::render_reverse_proxy_with_dns_and_resolve(&cfg,None,&|_,_|Vec::new()).unwrap();
        if let Ok(path)=std::env::var("NWUI_FXB3_NATIVE_EXPORT"){std::fs::write(std::path::Path::new(&path).with_file_name("uri-explicit.conf"),&explicit).unwrap();}
        assert!(!text.contains("proxy_redirect "),"complex proxy_pass has no implicit redirect mapping: {text}");
        assert!(explicit.contains("proxy_redirect default;"),"retain nginx's rejection of explicit default with variable proxy_pass");
    }
    #[test]
    fn fxb3_static_direct_admin_binds_but_runtime_external_keeps_no_bind_and_no_nss() {
        let static_cfg=json!({"serverId":ADMIN,"serverName":"admin.test","listen":18086,"proxyPass":"http://127.0.0.1:8080"});
        let static_text=render::render_reverse_proxy_with_dns_and_resolve(&static_cfg,None,&|_,_|Vec::new()).unwrap();assert!(static_text.contains("proxy_bind 127.0.0.2;"));
        let external=json!({"serverId":ADMIN,"serverName":"external.test","listen":18188,"proxyPass":"http://app.test:8080","locationDirectives":[{"name":"proxy_set_header","args":["X-Real-IP","custom"]}]});
        let external_text=render::render_reverse_proxy_with_dns_and_resolve(&external,None,&|_,_|vec!["192.0.2.5".parse().unwrap()]).unwrap();assert!(!external_text.contains("proxy_bind"));assert!(external_text.contains("proxy_set_header X-Real-IP custom;"));
        let dynamic=json!({"serverId":ADMIN,"serverName":"app.test","listen":18187,"proxyPass":"http://app.test:8080","resolveAtRuntime":true});
        let text=render::render_reverse_proxy_with_dns_and_resolve(&dynamic,Some("127.0.0.1:5354".parse().unwrap()),&|_,_|panic!("runtime names must not enter NSS")).unwrap();assert!(!text.contains("proxy_bind"));assert!(text.contains("proxy_set_header X-Real-IP $remote_addr;"));
        if let Ok(path)=std::env::var("NWUI_FXB3_RUNTIME_EXPORT"){std::fs::write(path,&text).unwrap();}
    }
}

const OPERATION_AUDIT_SQL:&str="INSERT INTO audit_logs(id,actor_id,actor_label,action,target,\"after\",ip) VALUES($1,$2,COALESCE($3,(SELECT email FROM users WHERE id=$2)),$4,$5,$6,$7) ON CONFLICT(id) DO NOTHING";

/// Root caller holds the store and application locks; this function owns SQL and files admission.
pub(super) async fn mutate(
    state:&AppState,store:&super::store::PublisherStore,operation:super::journal::Operation,
    command:super::protocol::Op,actor:&Actor,budget:&super::confirmation::Budget,
)->super::protocol::OpResult {
    use super::protocol::{Op,OpResult};
    let (action,target)=match &command {
        Op::ApplySaved{expected_saved_digest:Some(_),..}=>return OpResult::BadRequest{code:"unsupported_precondition".into()},
        Op::ApplySaved{..}=>("nginx:apply",None),
        Op::RollbackSnapshot{snapshot_id}=>("config:rollback",Some(snapshot_id.clone())),
        _=>return OpResult::BadRequest{code:"unsupported_operation".into()},
    };
    let operation_id=operation.id.clone();
    let attempt=mutate_locked(state,store,operation,command,actor,budget).await;
    let result=match attempt {Ok(result)=>result,Err(error)if super::capacity::is_full(&error)=>super::capacity::result(),Err(error)=>OpResult::Failed{code:"publish_failed".into(),output:error.to_string()}};
    let failure=match &result {
        OpResult::Ok{apply:Some(result),..} if !result.ok=>Some(result.output.clone()),
        OpResult::Failed{code,output} if code!="snapshot_not_found"=>Some(output.clone()),
        _=>None,
    };
    if let Some(output)=failure {
        // The operation UUID makes the success/unknown/failure audit one row, even after a lost COMMIT reply.
        let attempt=post_execute(&state.db,budget,OPERATION_AUDIT_SQL,&[&operation_id,&actor.id,&actor.label,&action,&target,&json!({"ok":false,"output":output}),&actor.ip]).await;
        if let Err(error)=attempt {eprintln!("[publisher] could not record operation failure audit: {error:#}");}
    }
    result
}

async fn mutate_locked(
    state:&AppState,store:&super::store::PublisherStore,operation:super::journal::Operation,
    command:super::protocol::Op,actor:&Actor,budget:&super::confirmation::Budget,
)->Result<super::protocol::OpResult> {
    use super::{journal::Phase,protocol::{Op,OpResult},mutation::Mutation};
    let (action,target,requested_label)=match &command {
        Op::ApplySaved{label,..}=>("nginx:apply",None,label.clone()),
        Op::RollbackSnapshot{snapshot_id}=>("config:rollback",Some(snapshot_id.clone()),None),
        _=>bail!("Unsupported root mutation"),
    };
    let mut client=budget.admit(state.db.get()).await??;
    let mut refusal=None;
    let prepared=budget.admit(async {
        let tx=client.build_transaction().isolation_level(tokio_postgres::IsolationLevel::RepeatableRead).start().await?;
        lock_tables(&tx,budget).await?;
        let snapshot_label=if let Some(id)=&target {
            let Some(row)=tx.query_opt("SELECT label,data FROM config_snapshots WHERE id=$1",&[&id]).await? else {return Ok::<_,anyhow::Error>((tx,None));};
            let label:Option<String>=row.get(0);let raw:Value=row.get(1);
            let data=super::pending::snapshot_from_value(&raw);
            let issued:BTreeSet<String>=tx.query("SELECT id FROM certificates WHERE status='issued'",&[]).await?.into_iter().map(|row|row.get(0)).collect();
            let unavailable:Vec<_>=data.servers.iter().filter_map(|server| {
                let id=server["certificateId"].as_str()?;
                (!issued.contains(id)).then(||format!("{} ({id})",server["serverName"].as_str().unwrap_or("?")))
            }).collect();
            if !unavailable.is_empty() {
                refusal=Some(format!("사용할 수 없는 인증서를 참조하므로 롤백할 수 없습니다: {}. 인증서를 복구하거나 다른 스냅샷을 선택하세요.",unavailable.join(", ")));
                return Ok((tx,None));
            }
            super::rollback::replace_rows(&tx,&data).await?;
            label
        } else {None};
        let data=load_snapshot(&tx).await?;let previous=load_applied_snapshot(&tx).await?;
        if let Err(error)=render::validate_http_group_names(&data.upstreams,&data.servers) {
            refusal=Some(error.to_string());return Ok((tx,None));
        }
        let mut refs=crate::control::model::load_certificate_refs(&tx).await?;
        let mut validated=Vec::new();
        for certificate in &mut refs {
            if certificate.status!="issued" {continue;}
            match super::certificates::read(&tx,&certificate.id,&state.secrets,None).await {
                Ok(material)=>{certificate.usable=true;validated.push(material);},
                Err(super::certificates::Error::Invalid)=>certificate.usable=false,
                Err(error)=>return Err(error.into()),
            }
        }
        let config=state.config.clone();let render_data=data.clone();
        let outcome=super::internal_targets::blocking(move||{
            let mut outcome=render_with_refs(&config,&render_data,&refs);
            super::unix_targets::filter_outcome(&render_data,&mut outcome);
            outcome
        }).await?;
        let files=super::nginx_lock::acquire(&state.config).await?;
        Ok((tx,Some((snapshot_label,data,previous,validated,outcome,files))))
    }).await.and_then(|result|result);
    if prepared.is_err() {
        let error=prepared.err().unwrap();
        drop(deadpool_postgres::Client::take(client));return Err(error);
    }
    let (tx,prepared)=prepared.unwrap();
    let Some((snapshot_label,data,previous,validated,mut outcome,files))=prepared else {
        tx.rollback().await?;
        return Ok(if let Some(output)=refusal {OpResult::Ok{apply:Some(ApplyResult{ok:false,output,rolled_back_to:None}),skipped:Vec::new(),uncertain:super::confirmation::uncertain(&state.config).await,phase:"done".into()}}
        else {OpResult::Failed{code:"snapshot_not_found".into(),output:"스냅샷을 찾을 수 없습니다.".into()}});
    };
    if let Err(error)=super::vocab::validate(&state.config,&outcome.rendered.conf,&outcome.rendered.stream) {
        tx.rollback().await?;
        return Ok(OpResult::Ok{apply:Some(ApplyResult{ok:false,output:error.to_string(),rolled_back_to:None}),skipped:outcome.skipped,uncertain:super::confirmation::uncertain(&state.config).await,phase:"done".into()});
    }
    let mut mutation=Mutation::begin(&state.config,store.clone(),operation,validated).await?;
    let staged=stage_durable(state,&outcome.rendered,&previous,budget,&mut mutation).await;
    let mut staged=match staged {
        Ok(staged)=>staged,
        Err(error)=>{
            let rollback=tokio::time::timeout(budget.remaining().min(std::time::Duration::from_secs(10)),tx.rollback()).await;
            let rollback_failed=!matches!(rollback,Ok(Ok(())));
            let capacity_refusal=super::capacity::is_full(&error);
            let mut output=if capacity_refusal{super::capacity::OUTPUT.into()}else{format!("{error:#}")};
            if !mutation.previous_confirmed && !capacity_refusal {
                output.push_str("\n원래 파일의 동작은 확인했지만, 직전 적용 상태가 불확실해 기준 설정을 확정하지 않았습니다. 다시 적용하세요.");
            }
            let intended=if rollback_failed {
                if target.is_some() {OpResult::Failed{code:"db_restore_failed".into(),output:format!("{output}\n\n롤백에 실패했고 이전 설정으로 되돌리지도 못했습니다 — 설정을 확인하세요.")}}
                else {OpResult::Failed{code:"publish_failed".into(),output:"Could not roll back apply metadata".into()}}
            }else if capacity_refusal {super::capacity::result()}else{OpResult::Ok{apply:Some(ApplyResult{ok:false,output:output.clone(),rolled_back_to:None}),skipped:outcome.skipped.clone(),uncertain:true,phase:"done".into()}};
            let result=if matches!(mutation.operation.phase,Phase::Intent|Phase::PayloadStaged) {
                mutation.operation.result=Some(serde_json::to_value(&intended)?);mutation.operation.advance(store,Phase::Done)?;
                ApplyResult{ok:false,output,rolled_back_to:None}
            }else{mutation.restore(&state.config,budget,"apply_validation_failed",intended).await?};
            if rollback_failed {
                drop(deadpool_postgres::Client::take(client));
                return Ok(serde_json::from_value(mutation.operation.result.clone().context("Failure result missing")?)?);
            }
            if capacity_refusal {return Ok(super::capacity::result());}
            return Ok(OpResult::Ok{apply:Some(result),skipped:outcome.skipped,uncertain:super::confirmation::uncertain(&state.config).await,phase:"done".into()});
        },
    };
    extend_containment(&mut outcome,&data,&staged.actions);staged.result.output.push_str(&outcome.summary());
    let label=if let Some(id)=&target {format!("rollback ← {}",snapshot_label.as_deref().unwrap_or_else(||id.get(..8).unwrap_or(id)))}
        else{requested_label.unwrap_or_else(||format!("apply @ {}",chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true)))};
    let result=OpResult::Ok{apply:Some(staged.result.clone()),skipped:outcome.skipped.clone(),uncertain:false,phase:"done".into()};
    let persist=async {
        set_completion_timeout(&tx,budget).await?;
        insert_snapshot(&tx,&data,actor.id.as_deref(),&label).await?;
        set_completion_timeout(&tx,budget).await?;
        record_applied_state(&tx,&outcome.effective(&data,Some(&previous)),&outcome.skipped,actor.id.as_deref()).await?;
        set_completion_timeout(&tx,budget).await?;
        tx.execute(OPERATION_AUDIT_SQL,&[&mutation.operation.id,&actor.id,&actor.label,&action,&target,&json!({"ok":true,"output":staged.result.output}),&actor.ip]).await?;
        // Active status stays running; persist the complete intended result before COMMIT can be lost.
        mutation.operation.result=Some(serde_json::to_value(&result)?);mutation.operation.save(store)?;
        Ok::<_,anyhow::Error>(())
    }.await;
    if let Err(error)=persist {
        let rollback=tokio::time::timeout(budget.remaining().min(std::time::Duration::from_secs(10)),tx.rollback()).await;
        let restore=mutation.restore(&state.config,budget,"apply_metadata_failed",OpResult::Failed{code:"publish_failed".into(),output:error.to_string()}).await;
        if !matches!(rollback,Ok(Ok(()))) {drop(deadpool_postgres::Client::take(client));}
        restore?;return Err(error);
    }
    let committed=tokio::time::timeout(budget.remaining().saturating_sub(super::confirmation::RESTORE_RESERVE),tx.commit()).await;
    if !matches!(committed,Ok(Ok(()))) {
        // Persist the unknown branch before deciding to restore, including a kill between them.
        mutation.unknown_commit()?;
        drop(deadpool_postgres::Client::take(client));
        mutation.restore(&state.config,budget,"apply_commit_unknown",OpResult::Failed{code:"publish_failed".into(),output:"설정 DB 커밋 결과를 확인하지 못했습니다.".into()}).await?;
        bail!("설정 DB 커밋 결과를 확인하지 못했습니다.");
    }
    mutation.complete(&state.config,&result).await?;
    drop(files);drop(client);
    notify_skipped(state,&outcome.skipped,budget).await;
    notify_notices(state,&outcome.notices,budget).await;
    notify_notices(state,&staged.notices,budget).await;
    Ok(result)
}

#[cfg(test)]
#[path="g13_apply_kernel_tests.rs"]
mod g13_kernel;
