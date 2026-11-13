//! Certificate mutations. Secret values never enter audit projections.
use super::validation::validate_domains;
use super::{
    cloudflare::CloudflareClient,
    pem::parse_certificate_pem,
    protocol::{DnsTxtStatus, DnsVerifier},
    store,
};
use crate::{auth::principal::db_unavailable, contracts::Actor, state::AppState};
use crate::{
    contracts::{FormInput, Patch},
    error::ApiError,
};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn conflict(message: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "conflict", message)
}
fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", message)
}
fn alias_value(alias: &Patch<String>, existing: Option<String>) -> Option<String> {
    match alias {
        Patch::Missing => existing,
        Patch::Null => None,
        Patch::Value(v) => Some(v.clone()),
    }
}
async fn automation(
    state: &AppState,
    input: &CertificateInput,
    existing: Option<&str>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    if input.challenge_type != "dns-01" || input.dns_provider.as_deref() != Some("cloudflare") {
        return Ok((None, None));
    }
    let raw = input.dns_token.as_deref().unwrap_or("");
    let token = if raw == "__keep__" {
        state.secrets.open(existing)
    } else {
        Some(raw.to_string()).filter(|s| !s.is_empty())
    }
    .ok_or_else(|| invalid("Cloudflare API 토큰을 입력하세요."))?;
    let client = CloudflareClient::new(
        &state.config.cloudflare_api_base,
        &token,
        state.config.acme_timeout,
    )
    .map_err(|e| invalid(e.to_string()))?;
    for domain in &input.domains {
        client
            .find_zone(&format!(
                "_acme-challenge.{}",
                domain.trim_start_matches("*.")
            ))
            .await
            .map_err(|e| invalid(format!("Cloudflare 토큰 확인 실패 — {e}")))?;
    }
    // Zone visibility proves only read access; DNS writes are checked when publishing.
    Ok((
        Some("cloudflare".into()),
        Some(
            state
                .secrets
                .seal(&token)
                .map_err(|_| ApiError::internal())?,
        ),
    ))
}
pub async fn create(
    state: &AppState,
    input: &CertificateInput,
    actor: &Actor,
) -> Result<String, ApiError> {
    let (provider, token) = automation(state, input, None).await?;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    store::account(&*tx, state, input.staging, &input.email).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let alias = alias_value(&input.alias, None);
    tx.execute("INSERT INTO certificates(id,domains,alias,staging,challenge_type,dns_provider,dns_provider_token_enc,status,actor_id) VALUES($1,$2,$3,$4,$5::text::challenge_type,$6,$7,'issuing',$8)",&[&id,&json!(input.domains),&alias,&input.staging,&input.challenge_type,&provider,&token,&actor.id]).await.map_err(|_|db_unavailable())?;
    let cert = store::get(&*tx, &id, false).await?;
    store::enqueue(&*tx, state, &cert, store::job_kind(input), "initial", actor).await?;
    store::audit(&*tx,actor,"cert:create",&id,json!({"domains":input.domains,"alias":alias,"staging":input.staging,"challengeType":input.challenge_type,"dnsProvider":provider})).await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    state.runtime.certificate_jobs.wake.notify_one();
    Ok(id)
}
pub async fn update(
    state: &AppState,
    id: &str,
    input: &CertificateInput,
    actor: &Actor,
) -> Result<(), ApiError> {
    let old = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        store::get(&**client, id, false).await?
    };
    if old.source != "acme" {
        return Err(conflict("수동 등록 인증서는 교체를 사용하세요."));
    }
    let (provider, token) = automation(state, input, old.dns_provider_token_enc.as_deref()).await?;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let current = store::get(&*tx, id, true).await?;
    if current.status == "issuing" || store::active(&*tx, id).await? {
        return Err(conflict(
            "발급이 진행 중입니다. 완료되거나 중단한 뒤 수정하세요.",
        ));
    }
    if current.dns_provider_token_enc != old.dns_provider_token_enc {
        return Err(conflict("인증서 설정이 변경되었습니다. 다시 불러오세요."));
    }
    store::account(&*tx, state, input.staging, &input.email).await?;
    let changed = current.names() != input.domains
        || current.staging != input.staging
        || current.challenge_type != input.challenge_type
        || current.dns_provider != provider;
    let alias = alias_value(&input.alias, current.alias);
    tx.execute("UPDATE certificates SET domains=$2,alias=$3,staging=$4,challenge_type=$5::text::challenge_type,dns_provider=$6,dns_provider_token_enc=$7,dns_order=CASE WHEN $8 THEN NULL ELSE dns_order END,status=CASE WHEN $8 AND status='awaiting-dns' THEN 'failed'::cert_status ELSE status END,last_error=CASE WHEN $8 AND status='awaiting-dns' THEN '설정이 바뀌어 다시 발급해야 합니다.' WHEN $8 THEN NULL ELSE last_error END,renewal_notice_sent_at=CASE WHEN challenge_type::text<>$5 THEN NULL ELSE renewal_notice_sent_at END WHERE id=$1",&[&id,&json!(input.domains),&alias,&input.staging,&input.challenge_type,&provider,&token,&changed]).await.map_err(|_|db_unavailable())?;
    store::audit(&*tx,actor,"cert:update",id,json!({"domains":input.domains,"alias":alias,"staging":input.staging,"challengeType":input.challenge_type,"dnsProvider":provider})).await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(())
}
pub async fn import(
    state: &AppState,
    input: &CertificatePemInput,
    actor: &Actor,
) -> Result<String, ApiError> {
    let parsed = parse_certificate_pem(&input.cert_pem, &input.key_pem)
        .map_err(|e| invalid(e.to_string()))?;
    let key = state
        .secrets
        .seal(&input.key_pem)
        .map_err(|_| ApiError::internal())?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    tx.execute("INSERT INTO certificates(id,domains,status,source,cert_pem,key_pem,issued_at,expires_at,actor_id) VALUES($1,$2,'issued','manual',$3,$4,$5,$6,$7)",&[&id,&json!(parsed.domains),&input.cert_pem,&key,&parsed.not_before,&parsed.not_after,&actor.id]).await.map_err(|_|db_unavailable())?;
    store::audit(
        &*tx,
        actor,
        "cert:import",
        &id,
        json!({"domains":parsed.domains,"expiresAt":parsed.not_after}),
    )
    .await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    Ok(id)
}
pub async fn replace(
    state: &AppState,
    id: &str,
    input: &CertificatePemInput,
    actor: &Actor,
) -> Result<Value, ApiError> {
    let parsed = parse_certificate_pem(&input.cert_pem, &input.key_pem)
        .map_err(|e| invalid(e.to_string()))?;
    let key = state
        .secrets
        .seal(&input.key_pem)
        .map_err(|_| ApiError::internal())?;
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let cert = store::get(&*tx, id, true).await?;
    if cert.source != "manual" {
        return Err(conflict("ACME 인증서는 갱신을 사용하세요."));
    }
    let linked = tx
        .query(
            "SELECT server_name FROM nginx_servers WHERE certificate_id=$1",
            &[&id],
        )
        .await
        .map_err(|_| db_unavailable())?;
    let mut uncovered = Vec::new();
    for row in &linked {
        let names: String = row.get(0);
        for name in names.split_whitespace() {
            if !parsed.domains.iter().any(|d| covers(d, name))
                && !uncovered.contains(&name.to_string())
            {
                uncovered.push(name.to_string());
            }
        }
    }
    tx.execute("UPDATE certificates SET domains=$2,cert_pem=$3,key_pem=$4,issued_at=$5,expires_at=$6,last_error=NULL,failure_count=0 WHERE id=$1",&[&id,&json!(parsed.domains),&input.cert_pem,&key,&parsed.not_before,&parsed.not_after]).await.map_err(|_|db_unavailable())?;
    store::audit(
        &*tx,
        actor,
        "cert:replace",
        id,
        json!({"domains":parsed.domains,"expiresAt":parsed.not_after}),
    )
    .await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    drop(client);
    let digest = crate::control::model::certificate_content_digest(&input.cert_pem, &key);
    let apply_error = apply(state, id, &digest, Some(actor)).await;
    Ok(json!({"id":id,"uncovered":uncovered,"applyError":apply_error}))
}
fn covers(pattern: &str, name: &str) -> bool {
    // ASCII DNS-name coverage only (RFC 9525 section 6.3), matching the UI warning.
    // No Unicode/IDNA conversion, IP-ID matching, trust/expiry or public-suffix policy.
    fn ascii_dns_name(value: &str) -> Option<String> {
        let name = value.strip_suffix('.').unwrap_or(value);
        if name.is_empty() || name.len() > 253 || !name.is_ascii()
            || name.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            || !name.split('.').all(|label| {
                !label.is_empty() && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }) {
            return None;
        }
        Some(name.to_ascii_lowercase())
    }
    let wildcard = pattern.starts_with("*.");
    let Some(pattern) = ascii_dns_name(if wildcard { &pattern[2..] } else { pattern }) else { return false };
    let Some(name) = ascii_dns_name(name) else { return false };
    if wildcard {
        pattern.len() + 2 <= 253 && name.split_once('.').is_some_and(|(_, suffix)| suffix == pattern)
    } else {
        pattern == name
    }
}

#[cfg(test)]
mod hostname_coverage_tests {
    use super::covers;

    #[test]
    fn matches_shared_ascii_dns_coverage_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/presentation/src/certificate-hostname.cases.json"
        ))).unwrap();
        for case in cases.as_array().unwrap() {
            let pattern = case["pattern"].as_str().unwrap();
            let hostname = case["hostname"].as_str().unwrap();
            assert_eq!(covers(pattern, hostname), case["expected"].as_bool().unwrap(),
                "{pattern:?} covering {hostname:?}");
        }
        let long_label = format!("{}.example.test", "a".repeat(64));
        assert!(!covers(&long_label, &long_label));
        let long_name = vec!["a".repeat(63); 5].join(".");
        assert!(!covers(&long_name, &long_name));
    }
}
pub(crate) async fn apply(state: &AppState, id: &str, digest: &str, actor: Option<&Actor>) -> Option<String> {
    let system = Actor {
        id: None,
        label: Some("system:certificate".into()),
        ip: None,
    };
    let actor = actor.unwrap_or(&system);
    match crate::publisher_client::deploy_certificate(state, id, digest, actor)
        .await
    {
        Ok(r) if r.ok => None,
        Ok(r) => Some(r.output),
        Err(e) => Some(match e.downcast_ref::<crate::publisher_client::ClientError>() {
            Some(crate::publisher_client::ClientError::Unavailable { .. }) => "설정 적용 서비스가 아직 준비되지 않아 인증서를 적용하지 못했습니다. 자동으로 다시 시도합니다.".to_owned(),
            Some(crate::publisher_client::ClientError::Unconfirmed { .. }) => "인증서 적용 결과를 아직 확인하지 못했습니다. 자동으로 다시 확인합니다.".to_owned(),
            _ => e.to_string(),
        }),
    }
}

pub async fn cancel(state: &AppState, id: &str, actor: &Actor) -> Result<(), ApiError> {
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    store::get(&*tx, id, true).await?;
    tx.execute("UPDATE certificate_jobs SET cancel_requested=true,updated_at=now() WHERE certificate_id=$1 AND state IN ('queued','active','cleanup')",&[&id]).await.map_err(|_|db_unavailable())?;
    tx.execute("UPDATE certificates SET status=CASE WHEN status IN ('issuing','awaiting-dns') THEN 'failed'::cert_status ELSE status END,dns_order=NULL,last_error=CASE WHEN status IN ('issuing','awaiting-dns') THEN '사용자가 발급을 중단했습니다.' ELSE last_error END,last_attempt_at=now() WHERE id=$1",&[&id]).await.map_err(|_|db_unavailable())?;
    store::audit(&*tx, actor, "cert:cancel", id, json!({"cancelled":true})).await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    if let Some((active_id, cancel)) = &*state.runtime.certificate_jobs.active.lock().await {
        if active_id == id {
            cancel.cancel();
        }
    }
    state.runtime.certificate_jobs.wake.notify_one();
    Ok(())
}
pub async fn remove(state: &AppState, id: &str, actor: &Actor) -> Result<(), ApiError> {
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let cert = store::get(&*tx, id, true).await?;
    if tx
        .query_opt(
            "SELECT id FROM nginx_servers WHERE certificate_id=$1 LIMIT 1",
            &[&id],
        )
        .await
        .map_err(|_| db_unavailable())?
        .is_some()
    {
        return Err(conflict("서버에 연결된 인증서는 삭제할 수 없습니다."));
    }
    tx.execute("UPDATE certificate_jobs SET cancel_requested=true,updated_at=now() WHERE certificate_id=$1 AND state IN ('queued','active','cleanup')",&[&id]).await.map_err(|_|db_unavailable())?;
    tx.execute("DELETE FROM certificates WHERE id=$1", &[&id])
        .await
        .map_err(|_| db_unavailable())?;
    store::audit(
        &*tx,
        actor,
        "cert:delete",
        id,
        json!({"domains":cert.domains}),
    )
    .await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    if let Some((active_id, cancel)) = &*state.runtime.certificate_jobs.active.lock().await {
        if active_id == id {
            cancel.cancel();
        }
    }
    state.runtime.certificate_jobs.wake.notify_one();
    Ok(())
}
pub(crate) async fn queue(
    state: &AppState,
    id: &str,
    actor: &Actor,
    mode: &str,
    dns_only: bool,
) -> Result<String, ApiError> {
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let cert = store::get(&*tx, id, true).await?;
    if cert.source != "acme" || cert.status != if mode == "renew" { "issued" } else { "failed" } {
        return Err(conflict("현재 상태에서는 발급을 시작할 수 없습니다."));
    }
    if mode == "renew"
        && (dns_only != (cert.challenge_type == "dns-01") || cert.dns_order.is_some())
    {
        return Err(conflict("갱신 방식이나 DNS 주문 상태를 확인하세요."));
    }
    let job = store::enqueue(&*tx, state, &cert, store::cert_kind(&cert), mode, actor).await?;
    tx.execute("UPDATE certificates SET status=CASE WHEN $2='initial' THEN 'issuing'::cert_status ELSE status END,last_error=NULL,last_attempt_at=now(),issue_log='[]',dns_order=NULL WHERE id=$1",&[&id,&mode]).await.map_err(|_|db_unavailable())?;
    store::audit(
        &*tx,
        actor,
        if mode == "renew" {
            "cert:renew"
        } else {
            "cert:retry"
        },
        id,
        json!({"queued":true}),
    )
    .await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    state.runtime.certificate_jobs.wake.notify_one();
    Ok(job)
}
pub async fn retry(state: &AppState, id: &str, actor: &Actor) -> Result<(), ApiError> {
    queue(state, id, actor, "initial", false).await.map(|_| ())
}
pub async fn renew_dns(state: &AppState, id: &str, actor: &Actor) -> Result<(), ApiError> {
    queue(state, id, actor, "renew", true).await.map(|_| ())
}
pub async fn renew(state: &AppState, id: &str, actor: &Actor) -> Result<Value, ApiError> {
    let job = queue(state, id, actor, "renew", false).await?;
    loop {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let row = client
            .query_one(
                "SELECT state,error FROM certificate_jobs WHERE id=$1",
                &[&job],
            )
            .await
            .map_err(|_| db_unavailable())?;
        let status: String = row.get(0);
        if status == "succeeded" {
            return Ok(json!({"renewed":true}));
        }
        if matches!(status.as_str(), "failed" | "cancelled") {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "apply_failed",
                row.get::<_, Option<String>>(1)
                    .unwrap_or_else(|| "갱신에 실패했습니다.".into()),
            ));
        }
        drop(client);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}
pub async fn submit_dns(state: &AppState, id: &str, actor: &Actor) -> Result<Value, ApiError> {
    let old = {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        store::get(&**client, id, false).await?
    };
    let mut order = old
        .dns_order
        .clone()
        .filter(|o| o.get("submittedAt").is_none())
        .ok_or_else(|| conflict("제출할 DNS 주문이 없습니다."))?;
    if old.challenge_type != "dns-01" || old.dns_provider.is_some() {
        return Err(conflict("수동 DNS 인증서가 아닙니다."));
    }
    let verifier = DnsVerifier::system(std::time::Duration::from_secs(5))
        .map_err(|e| invalid(e.to_string()))?;
    let mut pending = Vec::new();
    for record in order["records"].as_array().into_iter().flatten() {
        if verifier
            .verify(
                record["recordName"].as_str().unwrap_or(""),
                record["recordValue"].as_str().unwrap_or(""),
            )
            .await
            == DnsTxtStatus::Missing
        {
            pending.push(record["domain"].clone());
        }
    }
    if !pending.is_empty() {
        return Ok(json!({"submitted":false,"notPropagated":true,"pending":pending}));
    }
    let mut client = state.db.get().await.map_err(|_| db_unavailable())?;
    let tx = client.transaction().await.map_err(|_| db_unavailable())?;
    let mut cert = store::get(&*tx, id, true).await?;
    if cert.dns_order != old.dns_order {
        return Err(conflict("DNS 주문이 변경되었습니다."));
    }
    order["submittedAt"] = json!(chrono::Utc::now());
    cert.dns_order = Some(order.clone());
    store::enqueue(
        &*tx,
        state,
        &cert,
        "dns_complete",
        if cert.status == "issued" {
            "renew"
        } else {
            "initial"
        },
        actor,
    )
    .await?;
    tx.execute("UPDATE certificates SET dns_order=$2,status=CASE WHEN status='issued' THEN status ELSE 'issuing'::cert_status END WHERE id=$1",&[&id,&order]).await.map_err(|_|db_unavailable())?;
    store::audit(
        &*tx,
        actor,
        "cert:dns-submit",
        id,
        json!({"submitted":true}),
    )
    .await?;
    tx.commit().await.map_err(|_| db_unavailable())?;
    state.runtime.certificate_jobs.wake.notify_one();
    Ok(json!({"submitted":true}))
}

pub struct CertificateInput {
    pub domains: Vec<String>,
    pub email: String,
    pub staging: bool,
    pub challenge_type: String,
    pub dns_provider: Option<String>,
    pub dns_token: Option<String>,
    pub alias: Patch<String>,
}
impl std::fmt::Debug for CertificateInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateInput")
            .field("domains", &self.domains)
            .field("challenge_type", &self.challenge_type)
            .field("dns_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
pub struct CertificatePemInput {
    pub cert_pem: String,
    pub key_pem: String,
}
fn fields_error(fields: BTreeMap<String, String>) -> ApiError {
    let mut e = ApiError::new(
        StatusCode::BAD_REQUEST,
        "validation_failed",
        "입력값이 올바르지 않습니다.",
    );
    e.field_errors = Some(fields);
    e
}
pub fn parse_input(raw: &Value) -> Result<CertificateInput, ApiError> {
    let obj = raw.as_object().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "본문이 객체가 아닙니다.",
        )
    })?;
    let mut errors = BTreeMap::new();
    let challenge = obj
        .get("challengeType")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or("http-01")
        .to_string();
    let challenge_valid = matches!(obj.get("challengeType"), None | Some(Value::Null))
        || obj
            .get("challengeType")
            .and_then(Value::as_str)
            .is_some_and(|v| matches!(v, "" | "http-01" | "dns-01"));
    if !challenge_valid {
        errors.insert(
            "challengeType".into(),
            "검증 방식이 올바르지 않습니다.".into(),
        );
    }
    let domains: Vec<String> = obj
        .get("domains")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                        .trim()
                        .to_lowercase()
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if domains.is_empty() {
        errors.insert(
            "domains".into(),
            "도메인을 1개 이상 입력하세요. (줄당 1개)".into(),
        );
    } else if challenge_valid {
        if let Err(e) = validate_domains(&domains, challenge == "dns-01") {
            errors.insert("domains".into(), e.to_string());
        }
    }
    let email = obj
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let email_ok = email.split_once('@').is_some_and(|(local, host)| {
        !local.is_empty()
            && !host.contains('@')
            && host
                .split_once('.')
                .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
    }) && !email.chars().any(char::is_whitespace);
    if !email_ok {
        errors.insert("email".into(), "올바른 이메일을 입력하세요.".into());
    }
    let dns_provider = obj
        .get("dnsProvider")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    if !(matches!(obj.get("dnsProvider"), None | Some(Value::Null))
        || obj
            .get("dnsProvider")
            .and_then(Value::as_str)
            .is_some_and(|v| matches!(v, "" | "cloudflare")))
    {
        errors.insert(
            "dnsProvider".into(),
            "DNS 제공자가 올바르지 않습니다.".into(),
        );
    }
    let alias = match obj.get("alias") {
        None => Patch::Missing,
        Some(Value::Null) => Patch::Null,
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.encode_utf16().count() > 100 {
                errors.insert("alias".into(), "별칭은 100자 이하여야 합니다.".into());
            }
            if s.is_empty() {
                Patch::Null
            } else {
                Patch::Value(s.to_owned())
            }
        }
        _ => {
            errors.insert("alias".into(), "별칭은 문자열이어야 합니다.".into());
            Patch::Missing
        }
    };
    if !errors.is_empty() {
        return Err(fields_error(errors));
    }
    Ok(CertificateInput {
        domains,
        email,
        staging: obj.get("staging") == Some(&Value::Bool(true)),
        challenge_type: challenge,
        dns_provider,
        dns_token: obj
            .get("dnsToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        alias,
    })
}
pub fn parse_pem_input(raw: &Value) -> Result<CertificatePemInput, ApiError> {
    if !raw.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "본문이 객체가 아닙니다.",
        ));
    }
    let mut errors = BTreeMap::new();
    let mut values = Vec::new();
    for field in ["certPem", "keyPem"] {
        let s = raw
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if s.is_empty() {
            errors.insert(field.into(), "PEM 내용을 입력하세요.".into());
        } else if s.len() > 65536 {
            errors.insert(field.into(), "PEM은 64 KiB 이하여야 합니다.".into());
        }
        values.push(s);
    }
    if !errors.is_empty() {
        return Err(fields_error(errors));
    }
    Ok(CertificatePemInput {
        cert_pem: values.remove(0),
        key_pem: values.remove(0),
    })
}
pub fn form_input(form: &FormInput) -> Value {
    let mut raw = json!({"domains":form.get("domains").unwrap_or("").lines().collect::<Vec<_>>(),"email":form.get("email").unwrap_or(""),"staging":form.get("staging")==Some("true"),"challengeType":form.get("challengeType"),"dnsProvider":form.get("dnsProvider"),"dnsToken":form.get("dnsToken")});
    if let Some(alias) = form.get("alias") {
        raw["alias"] = json!(alias);
    }
    raw
}

#[cfg(test)]
mod publisher_caller_tests {
    use super::*;
    use crate::publisher_client::{Bounds,Client,fake::{Server,Script}};
    use crate::publisher::protocol::{Op,OpResult};
    use crate::control::operations::publisher_caller_tests::state_with_client;
    use std::time::Duration;

    #[tokio::test]
    async fn unavailable_publisher_keeps_certificate_deployment_pending_with_retry_guidance() {
        let socket_root=tempfile::tempdir().unwrap();
        let(_root,state)=state_with_client(Client::socket_for_tests(socket_root.path().join("missing.sock"),Bounds::default()),"postgres://fixture@127.0.0.1:1/unused");
        let result=tokio::time::timeout(Duration::from_secs(2),apply(&state,"fixture-certificate",&"0".repeat(64),None)).await.expect("publisher refusal must not fall back to the DB/writer");
        assert_eq!(result.as_deref(),Some("설정 적용 서비스가 아직 준비되지 않아 인증서를 적용하지 못했습니다. 자동으로 다시 시도합니다."));
        assert!(!state.config.nginx_conf_dir.exists());assert!(!state.config.nginx_stream_dir.exists());
    }
    fn reply(ok:bool)->OpResult {
        OpResult::Ok{apply:Some(crate::control::model::ApplyResult{ok,output:"deploy-fixture".into(),rolled_back_to:None}),skipped:vec![],uncertain:false,phase:"done".into()}
    }
    #[tokio::test] #[ignore="socket client implementation required"]
    async fn certificate_socket_outcomes_preserve_retry_text_and_actor_digest() {
        let cases=vec![
            (vec![Script::Reply(reply(true))],None,false),
            (vec![Script::Reply(reply(false))],Some("deploy-fixture"),true),
            (vec![Script::Reply(OpResult::Unavailable)],Some("설정 적용 서비스가 아직 준비되지 않아 인증서를 적용하지 못했습니다. 자동으로 다시 시도합니다."),false),
            (vec![Script::CloseAfterRead,Script::Reply(OpResult::Unknown)],Some("인증서 적용 결과를 아직 확인하지 못했습니다. 자동으로 다시 확인합니다."),false),
            (vec![Script::Reply(OpResult::Conflict)],Some("설정 적용 서비스 응답을 해석하지 못했습니다."),false),
            (vec![Script::Reply(OpResult::Failed{code:"writer_failed".into(),output:"writer-fixture".into()})],Some("writer-fixture"),false),
            (vec![Script::Reply(OpResult::StaleMaterial)],Some("stale_material"),false),
        ];
        for(script,expected,explicit_actor)in cases {
            let server=Server::start(script).await.unwrap();let(_root,state)=state_with_client(Client::socket_for_tests(server.path().to_owned(),Bounds::default()),"postgres://fixture@127.0.0.1:1/unused");
            let actor=Actor{id:Some("operator".into()),label:Some("operator-label".into()),ip:Some("198.51.100.8".into())};let id="3d521ab4-34ad-44e8-b7eb-882c0feff693";let digest="1".repeat(64);
            let result=tokio::time::timeout(Duration::from_secs(5),apply(&state,id,&digest,explicit_actor.then_some(&actor))).await.unwrap();assert_eq!(result.as_deref(),expected);
            let requests=server.requests.lock().await;assert!(!requests.is_empty());
            assert!(matches!(&requests[0].op,Op::DeployCertificate{certificate_id,content_digest} if certificate_id==id&&content_digest==&digest));
            if explicit_actor {assert_eq!(requests[0].actor.user_id,actor.id);assert_eq!(requests[0].actor.label,actor.label);assert_eq!(requests[0].actor.ip,actor.ip);}
            else {assert_eq!(requests[0].actor.user_id,None);assert_eq!(requests[0].actor.label.as_deref(),Some("system:certificate"));assert_eq!(requests[0].actor.ip,None);}
            if expected==Some("인증서 적용 결과를 아직 확인하지 못했습니다. 자동으로 다시 확인합니다.") {
                assert_eq!(requests.len(),2);assert!(matches!(&requests[1].op,Op::OperationStatus{operation_id} if operation_id==&requests[0].id));
            } else {assert_eq!(requests.len(),1);}
            assert!(server.errors.lock().await.is_empty());assert!(!state.config.nginx_conf_dir.exists());
        }
    }
}
