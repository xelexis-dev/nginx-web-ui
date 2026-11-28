use super::{
    CertificateError, Result,
    pem::{certificate_expiry, create_csr, csr_der},
    validation::validate_domains,
};
use chrono::{DateTime, Utc};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, HttpClient, Identifier, Key, NewOrder, Order,
    OrderStatus, Problem, RetryPolicy,
};
use rustls_pki_types::{PrivatePkcs8KeyDer, pem::PemObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsTxtStatus {
    Ok,
    Missing,
    Inconclusive,
}

pub struct DnsVerifier {
    resolver: hickory_resolver::TokioResolver,
    timeout: Duration,
}
impl DnsVerifier {
    pub fn system(timeout: Duration) -> Result<Self> {
        let mut builder = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|e| CertificateError::Callback(e.to_string()))?;
        builder.options_mut().cache_size = 0;
        builder.options_mut().timeout = timeout;
        let resolver = builder
            .build()
            .map_err(|e| CertificateError::Callback(e.to_string()))?;
        Ok(Self { resolver, timeout })
    }
    pub fn with_config(
        config: hickory_resolver::config::ResolverConfig,
        timeout: Duration,
    ) -> Result<Self> {
        let mut builder = hickory_resolver::TokioResolver::builder_with_config(
            config,
            hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
        );
        builder.options_mut().cache_size = 0;
        builder.options_mut().timeout = timeout;
        let resolver = builder
            .build()
            .map_err(|e| CertificateError::Callback(e.to_string()))?;
        Ok(Self { resolver, timeout })
    }
    pub async fn verify(&self, name: &str, expected: &str) -> DnsTxtStatus {
        // Absolute names avoid resolver search suffixes and an application cache
        // must not hide new TXT values during propagation checks.
        let name = format!("{}.", name.trim_end_matches('.'));
        match tokio::time::timeout(self.timeout,self.resolver.txt_lookup(name)).await {
            Ok(Ok(answer))=>{
                if answer.answers().iter().any(|record|matches!(&record.data,hickory_resolver::proto::rr::RData::TXT(txt) if txt.txt_data.concat()==expected.as_bytes())) {DnsTxtStatus::Ok}else{DnsTxtStatus::Missing}
            },
            Ok(Err(error)) if error.is_no_records_found()=>DnsTxtStatus::Missing,
            _=>DnsTxtStatus::Inconclusive,
        }
    }
}

/// Cooperative cancellation lets HTTP issuance finish its challenge cleanup.
/// Jobs should cancel and await, rather than abort the task containing issuance.
#[derive(Clone)]
pub struct Cancellation(tokio::sync::watch::Sender<bool>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

pub fn resolve_directory_url(staging: bool, override_url: Option<&str>) -> String {
    override_url
        .filter(|s| !s.is_empty())
        .unwrap_or(if staging {
            "https://acme-staging-v02.api.letsencrypt.org/directory"
        } else {
            "https://acme-v02.api.letsencrypt.org/directory"
        })
        .into()
}

pub fn create_account_key() -> Result<String> {
    Ok(rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?.serialize_pem())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsChallengeRecord {
    pub domain: String,
    pub record_name: String,
    pub record_value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsBeginResult {
    pub order: Value,
    pub csr_pem: String,
    pub key_pem: String,
    pub records: Vec<DnsChallengeRecord>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsCompleteResult {
    pub cert_pem: String,
    pub expires_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueResult {
    pub cert_pem: String,
    pub key_pem: String,
    pub expires_at: DateTime<Utc>,
    pub cleanup_failures: Vec<HttpCleanupFailure>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct HttpCleanupFailure {
    pub token: String,
    pub error: String,
}

pub trait HttpChallengeStore: Sync {
    /// Complete only once the backend write has definitively finished, even on
    /// error. A store timeout must settle/cancel the backend operation before
    /// returning; dropping its client future alone does not prove that.
    fn set(&self, token: &str, key_authorization: &str) -> impl Future<Output = Result<()>> + Send;
    fn remove(&self, token: &str) -> impl Future<Output = Result<()>> + Send;
}

// instant-acme's OrderState omits RFC 8555 `expires`. Retain just this field
// during one DNS begin operation, indexed by the authenticated order URL.
type OrderExpiries = Arc<Mutex<BTreeMap<String, Value>>>;
// Allow ample room for PEM chains and ACME JSON while bounding upstream memory use.
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
struct BoundedHttp(reqwest::Client, Option<OrderExpiries>);
impl HttpClient for BoundedHttp {
    fn request(
        &self,
        request: axum::http::Request<instant_acme::BodyWrapper<axum::body::Bytes>>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<instant_acme::BytesResponse, instant_acme::Error>,
                > + Send,
        >,
    > {
        let client = self.0.clone();
        let expiries = self.1.clone();
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(30), async move {
                let (parts, body) = request.into_parts();
                let request_url = parts.uri.to_string();
                let body = axum::body::to_bytes(axum::body::Body::new(body), 2 * 1024 * 1024)
                    .await
                    .map_err(|e| instant_acme::Error::Other(Box::new(e)))?;
                let mut response = client
                    .request(parts.method, parts.uri.to_string())
                    .headers(parts.headers)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| instant_acme::Error::Other(Box::new(e)))?;
                let mut reply = axum::http::Response::builder().status(response.status());
                *reply.headers_mut().ok_or("invalid response headers")? =
                    response.headers().clone();
                let order_url = response
                    .headers()
                    .get(axum::http::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or(&request_url)
                    .to_owned();
                let success = response.status().is_success();
                if response
                    .content_length()
                    .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
                {
                    return Err("ACME 응답이 너무 큽니다.".into());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|e| instant_acme::Error::Other(Box::new(e)))?
                {
                    if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                        return Err("ACME 응답이 너무 큽니다.".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if success
                    && let Some(expiries) = expiries
                    && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
                    && value.get("finalize").and_then(Value::as_str).is_some()
                    && let Some(expires) = value.get("expires").filter(|v| !v.is_null())
                {
                    expiries
                        .lock()
                        .map_err(|_| "order expiry capture failed")?
                        .insert(order_url, expires.clone());
                }
                Ok(instant_acme::BytesResponse::from(
                    reply.body(axum::body::Body::from(bytes))?,
                ))
            })
            .await
            .map_err(|_| instant_acme::Error::Timeout(None))?
        })
    }
}

fn invalid_order_message(
    status: OrderStatus,
    order_problem: Option<&Problem>,
    challenge_problems: &[(String, Problem)],
) -> String {
    let start = format!("ACME 주문 상태가 유효하지 않습니다: {status:?}.");
    let mut by_name = BTreeMap::<Option<String>, Vec<String>>::new();
    for (name, problem) in order_problem
        .into_iter()
        .map(|problem| (None, problem))
        .chain(
            challenge_problems
                .iter()
                .map(|(name, problem)| (Some(name), problem)),
        )
    {
        let reasons = std::iter::once((
            name.cloned(),
            problem.r#type.as_deref(),
            problem.detail.as_deref(),
        ))
        .chain(problem.subproblems.iter().map(|sub| {
            let name = sub
                .identifier
                .as_ref()
                .map(|id| id.authorized(false).to_string())
                .or_else(|| name.cloned());
            (name, sub.r#type.as_deref(), sub.detail.as_deref())
        }));
        for (name, kind, detail) in reasons {
            let Some(reason) = format_acme_problem(kind, detail) else {
                continue;
            };
            let name = name.map(|name| strip_controls(&name));
            let entries = by_name.entry(name).or_default();
            if !entries.contains(&reason) {
                entries.push(reason);
            }
        }
    }
    if by_name.is_empty() {
        return format!("{start} 인증기관이 실패 이유를 알려 주지 않았습니다.");
    }
    let mut reasons = by_name.remove(&None).unwrap_or_default();
    let hidden = by_name.len().saturating_sub(5);
    reasons.extend(
        by_name.into_iter().take(5).map(|(name, entries)| {
            format!("{}: {}", name.unwrap_or_default(), entries.join("; "))
        }),
    );
    if hidden > 0 {
        reasons.push(format!("외 {hidden}개"));
    }
    format!("{start} 인증기관이 알려 준 이유 — {}", reasons.join("; "))
}

fn strip_controls(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn format_acme_problem(kind: Option<&str>, detail: Option<&str>) -> Option<String> {
    let kind = strip_controls(kind.unwrap_or_default());
    let kind = kind.trim();
    let kind = kind
        .strip_prefix("urn:ietf:params:acme:error:")
        .unwrap_or(kind);
    let detail = strip_controls(detail.unwrap_or_default());
    let mut chars = detail.trim().chars();
    let mut detail: String = chars.by_ref().take(300).collect();
    if chars.next().is_some() {
        detail.push('…');
    }
    let mut reason = match (kind.is_empty(), detail.is_empty()) {
        (false, false) => format!("{kind} — {detail}"),
        (false, true) => kind.to_owned(),
        (true, false) => detail,
        (true, true) => return None,
    };
    let hint = match kind {
        "connection" => "인증기관이 이 서버의 80 포트에 연결하지 못했습니다. 해외에서도 80 포트로 접속되는지 확인하세요.",
        "dns" => "인증기관이 이 도메인의 DNS 레코드를 찾지 못했습니다.",
        "unauthorized" => "검증 주소의 응답이 맞지 않습니다. 도메인이 이 서버를 가리키는지, 앞단 프록시가 요청을 그대로 넘기는지 확인하세요.",
        "caa" => "도메인의 CAA 레코드가 이 인증기관의 발급을 막고 있습니다.",
        _ => "",
    };
    if !hint.is_empty() {
        reason.push(' ');
        reason.push_str(hint);
    }
    Some(reason)
}

#[derive(Default)]
pub(crate) struct IssuanceTrace {
    pub(crate) order_url: Option<String>,
    pub(crate) secrets: Vec<String>,
}

pub struct AcmeClient {
    directory_url: String,
    timeout: Duration,
}
impl AcmeClient {
    pub fn new(directory_url: &str, timeout: Duration) -> Result<Self> {
        let url = url::Url::parse(directory_url).map_err(|_| {
            CertificateError::Invalid("ACME directory 주소가 올바르지 않습니다.".into())
        })?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(CertificateError::Invalid(
                "ACME directory 주소가 올바르지 않습니다.".into(),
            ));
        }
        Ok(Self {
            directory_url: directory_url.into(),
            timeout,
        })
    }
    async fn bounded<T>(
        &self,
        cancel: &Cancellation,
        deadline: Instant,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::select! {biased;
            _=cancel.cancelled()=>Err(CertificateError::Aborted),
            result=tokio::time::timeout_at(deadline,future)=>result.map_err(|_|CertificateError::Timeout)?,
        }
    }
    async fn account(
        &self,
        pem: &str,
        email: Option<&str>,
        expiries: Option<OrderExpiries>,
    ) -> Result<Account> {
        let der = PrivatePkcs8KeyDer::from_pem_slice(pem.as_bytes()).map_err(|_| {
            CertificateError::Invalid("ACME 계정키는 P-256 PKCS#8 PEM이어야 합니다.".into())
        })?;
        let key = Key::from_pkcs8_der(der.clone_key())?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| CertificateError::Callback(e.to_string()))?;
        let builder = Account::builder_with_http(Box::new(BoundedHttp(http, expiries)));
        let (account, _) = if email.is_some() {
            builder
                .create_from_key((key, der.into()), self.directory_url.clone())
                .await?
        } else {
            builder
                .from_key((key, der.into()), self.directory_url.clone())
                .await?
        };
        if let Some(email) = email {
            account
                .update_contacts(&[&format!("mailto:{email}")])
                .await?;
        }
        Ok(account)
    }
    async fn new_order(
        &self,
        key: &str,
        email: &str,
        domains: &[String],
        expiries: Option<OrderExpiries>,
    ) -> Result<Order> {
        let account = self.account(key, Some(email), expiries).await?;
        let ids: Vec<_> = domains.iter().cloned().map(Identifier::Dns).collect();
        Ok(account.new_order(&NewOrder::new(&ids)).await?)
    }
    pub async fn begin_dns(
        &self,
        account_key_pem: &str,
        email: &str,
        domains: &[String],
        cancel: &Cancellation,
    ) -> Result<DnsBeginResult> {
        self.begin_dns_traced(account_key_pem, email, domains, cancel, &mut IssuanceTrace::default()).await
    }

    pub(crate) async fn begin_dns_traced(
        &self,
        account_key_pem: &str,
        email: &str,
        domains: &[String],
        cancel: &Cancellation,
        trace: &mut IssuanceTrace,
    ) -> Result<DnsBeginResult> {
        validate_domains(domains, true)?;
        self.bounded(cancel,Instant::now() + self.timeout, async {
            let expiries=OrderExpiries::default();
            let mut order=self.new_order(account_key_pem,email,domains,Some(expiries.clone())).await?;
            trace.order_url = Some(order.url().to_owned());
            let mut records=Vec::new();
            let mut authorizations=order.authorizations();
            while let Some(auth)=authorizations.next().await {
                let mut auth=auth?;
                // A valid authorization requires no new challenge (RFC 8555 §7.5).
                if auth.status==AuthorizationStatus::Valid {continue;}
                if auth.status!=AuthorizationStatus::Pending {return Err(CertificateError::Invalid(format!("ACME authorization 상태가 유효하지 않습니다: {:?}",auth.status)));}
                let challenge=auth.challenge(ChallengeType::Dns01).ok_or_else(||CertificateError::Invalid("이 CA가 dns-01 challenge를 제공하지 않습니다.".into()))?;
                let Identifier::Dns(domain)=challenge.identifier().identifier else {return Err(CertificateError::Invalid("DNS 도메인만 지원합니다.".into()));};
                let authorization = challenge.key_authorization();
                let value = authorization.dns_value();
                trace.secrets.extend([challenge.token.clone(), authorization.as_str().to_owned(), value.clone()]);
                records.push(DnsChallengeRecord {domain:domain.clone(),record_name:format!("_acme-challenge.{domain}"),record_value:value});
            }
            let url=order.url().to_owned();let state=order.state();
            let mut saved=json!({"url":url,"status":format!("{:?}",state.status).to_lowercase(),"identifiers":domains.iter().map(|d|json!({"type":"dns","value":d})).collect::<Vec<_>>(),"authorizations":state.authorizations.iter().map(|a|a.url.clone()).collect::<Vec<_>>(),"finalize":state.finalize,"certificate":state.certificate});
            if let Some(expires)=expiries.lock().map_err(|_|CertificateError::Callback("order expiry capture failed".into()))?.remove(&url) {saved["expires"]=expires;}
            let names=domains.to_vec();
            let csr=tokio::task::spawn_blocking(move||create_csr(&names)).await.map_err(|e|CertificateError::Callback(e.to_string()))??;
            Ok(DnsBeginResult {order:saved,csr_pem:csr.csr_pem,key_pem:csr.key_pem,records})
        }).await
    }
    pub async fn complete_dns(
        &self,
        account_key_pem: &str,
        saved_order: &Value,
        csr_pem: &str,
        cancel: &Cancellation,
    ) -> Result<DnsCompleteResult> {
        self.complete_dns_traced(account_key_pem, saved_order, csr_pem, cancel, &mut IssuanceTrace::default()).await
    }

    pub(crate) async fn complete_dns_traced(
        &self,
        account_key_pem: &str,
        saved_order: &Value,
        csr_pem: &str,
        cancel: &Cancellation,
        trace: &mut IssuanceTrace,
    ) -> Result<DnsCompleteResult> {
        let url = saved_order["url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CertificateError::Invalid("저장된 ACME 주문에 order.url이 없습니다.".into())
            })?;
        let deadline = Instant::now() + self.timeout;
        self.bounded(cancel, deadline, async {
            let account = self.account(account_key_pem, None, None).await?;
            let mut order = account.order(url.into()).await?;
            if order.state().status == OrderStatus::Pending {
                let mut auths = order.authorizations();
                while let Some(auth) = auths.next().await {
                    let mut auth = auth?;
                    if auth.status == AuthorizationStatus::Valid {
                        continue;
                    }
                    if auth.status != AuthorizationStatus::Pending {
                        return Err(CertificateError::Invalid(format!(
                            "ACME authorization 상태가 유효하지 않습니다: {:?}",
                            auth.status
                        )));
                    }
                    let mut challenge = auth.challenge(ChallengeType::Dns01)
                        .ok_or_else(|| {
                            CertificateError::Invalid(
                                "이 CA가 dns-01 challenge를 제공하지 않습니다.".into(),
                            )
                        })?;
                    let authorization = challenge.key_authorization();
                    trace.secrets.extend([challenge.token.clone(), authorization.as_str().to_owned(), authorization.dns_value()]);
                    challenge.set_ready().await?;
                }
            }
            self.finish(&mut order, csr_pem, deadline).await
        })
        .await
    }
    async fn finish(
        &self,
        order: &mut Order,
        csr_pem: &str,
        deadline: Instant,
    ) -> Result<DnsCompleteResult> {
        let retry = RetryPolicy::new().timeout(self.timeout);
        let mut status = order.state().status;
        if status == OrderStatus::Pending {
            match order.poll_ready(&retry).await {
                Ok(ready) => status = ready,
                // poll_ready returns an API error before returning an invalid status with a problem.
                Err(_) if order.state().status == OrderStatus::Invalid => {
                    status = OrderStatus::Invalid;
                }
                Err(error) => return Err(error.into()),
            }
        }
        match status {
            OrderStatus::Ready => order.finalize_csr(&csr_der(csr_pem)?).await?,
            OrderStatus::Processing | OrderStatus::Valid => {}
            _ => {
                let order_problem = order.state().error.clone();
                let mut problems = Vec::new();
                let mut authorizations = order.authorizations();
                // Poll this lookup against the issuance deadline so a slow CA cannot mask the invalid order.
                let _ = tokio::time::timeout_at(deadline, async {
                    while let Some(result) = authorizations.next().await {
                        if let Ok(auth) = result {
                            let name = auth.identifier().to_string();
                            problems.extend(auth.challenges.iter().filter_map(|challenge| {
                                Some((name.clone(), challenge.error.as_ref()?.clone()))
                            }));
                        }
                    }
                })
                .await;
                return Err(CertificateError::Invalid(invalid_order_message(
                    status,
                    order_problem.as_ref(),
                    &problems,
                )));
            }
        }
        let cert_pem = if order.state().status == OrderStatus::Valid {
            order.certificate().await?.ok_or_else(|| {
                CertificateError::Invalid("CA가 인증서를 반환하지 않았습니다.".into())
            })?
        } else {
            order.poll_certificate(&retry).await?
        };
        Ok(DnsCompleteResult {
            expires_at: certificate_expiry(&cert_pem)?,
            cert_pem,
        })
    }
    pub async fn issue_http<S: HttpChallengeStore>(
        &self,
        account_key_pem: &str,
        email: &str,
        domains: &[String],
        store: &S,
        cancel: &Cancellation,
    ) -> Result<IssueResult> {
        self.issue_http_traced(account_key_pem, email, domains, store, cancel, &mut IssuanceTrace::default()).await
    }

    pub(crate) async fn issue_http_traced<S: HttpChallengeStore>(
        &self,
        account_key_pem: &str,
        email: &str,
        domains: &[String],
        store: &S,
        cancel: &Cancellation,
        trace: &mut IssuanceTrace,
    ) -> Result<IssueResult> {
        validate_domains(domains, false)?;
        let mut tokens = Vec::new();
        // The issuance watchdog may drop its borrow of a write, but must not
        // drop the write itself: an already-sent DB INSERT can still commit.
        let mut pending_write: Option<Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>> = None;
        let deadline = Instant::now() + self.timeout;
        let result = self
            .bounded(cancel, deadline, async {
                let mut order = self
                    .new_order(account_key_pem, email, domains, None)
                    .await?;
                trace.order_url = Some(order.url().to_owned());
                let names = domains.to_vec();
                let csr = tokio::task::spawn_blocking(move || create_csr(&names))
                    .await
                    .map_err(|e| CertificateError::Callback(e.to_string()))??;
                let mut auths = order.authorizations();
                while let Some(auth) = auths.next().await {
                    let mut auth = auth?;
                    if auth.status == AuthorizationStatus::Valid {
                        continue;
                    }
                    if auth.status != AuthorizationStatus::Pending {
                        return Err(CertificateError::Invalid(format!(
                            "ACME authorization 상태가 유효하지 않습니다: {:?}",
                            auth.status
                        )));
                    }
                    let mut challenge = auth.challenge(ChallengeType::Http01).ok_or_else(|| {
                        CertificateError::Invalid(
                            "이 CA가 http-01 challenge를 제공하지 않습니다.".into(),
                        )
                    })?;
                    tokens.push(challenge.token.clone());
                    let token = challenge.token.clone();
                    let authorization = challenge.key_authorization().as_str().to_owned();
                    trace.secrets.extend([token.clone(), authorization.clone()]);
                    pending_write =
                        Some(Box::pin(
                            async move { store.set(&token, &authorization).await },
                        ));
                    let written = pending_write.as_mut().expect("write just installed").await;
                    pending_write = None;
                    written?;
                    challenge.set_ready().await?;
                }
                let cert = self.finish(&mut order, &csr.csr_pem, deadline).await?;
                Ok(IssueResult {
                    cert_pem: cert.cert_pem,
                    key_pem: csr.key_pem,
                    expires_at: cert.expires_at,
                    cleanup_failures: Vec::new(),
                })
            })
            .await;
        if let Some(write) = pending_write {
            // Cancellation/timeout stops CA work immediately, but cleanup must
            // wait for the pending write's definitive outcome. This wait has no
            // competing timeout: that would recreate the late-commit race.
            // Preserve the original cancellation/timeout result if writing fails.
            let _ = write.await;
        }
        let mut failures = Vec::new();
        for token in tokens {
            match tokio::time::timeout(Duration::from_secs(30), store.remove(&token)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(HttpCleanupFailure {
                    token,
                    error: error.to_string(),
                }),
                Err(_) => failures.push(HttpCleanupFailure {
                    token,
                    error: "정리 시간 초과".into(),
                }),
            }
        }
        match result {
            Ok(mut result) => {
                result.cleanup_failures = failures;
                Ok(result)
            }
            Err(cause) if failures.is_empty() => Err(cause),
            Err(cause) => Err(CertificateError::HttpCleanup {
                cause: Box::new(cause),
                failures,
            }),
        }
    }
}

#[cfg(test)]
mod response_body_tests {
    use super::*;
    use crate::certificates::cloudflare::CloudflareClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const LIMIT: usize = 2 * 1024 * 1024;
    enum Mode {
        Declared,
        Chunked,
        Boundary,
    }

    // Raw HTTP keeps framing explicit: Content-Length alone must fail before any body arrives.
    async fn server(mode: Mode, cloudflare: bool) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                    return;
                }
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            if matches!(mode, Mode::Declared) {
                let _ = stream
                    .write_all(
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", LIMIT + 1)
                            .as_bytes(),
                    )
                    .await;
                std::future::pending::<()>().await;
            }
            let size = LIMIT + usize::from(matches!(mode, Mode::Chunked));
            let mut body = if cloudflare {
                br#"{"success":true,"result":[{"id":"z1","name":"example.com"}],"result_info":{"total_pages":1}}"#.to_vec()
            } else {
                b"certificate-chain-boundary".to_vec()
            };
            body.resize(size, b' ');
            let header = if matches!(mode, Mode::Chunked) {
                "Transfer-Encoding: chunked".to_owned()
            } else {
                format!("Content-Length: {}", body.len())
            };
            if stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{header}\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .is_err()
            {
                return;
            }
            if matches!(mode, Mode::Chunked) {
                for chunk in body.chunks(16 * 1024) {
                    if stream
                        .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                        .await
                        .is_err()
                        || stream.write_all(chunk).await.is_err()
                        || stream.write_all(b"\r\n").await.is_err()
                    {
                        return;
                    }
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
            } else {
                let _ = stream.write_all(&body).await;
            }
        });
        (url, task)
    }
    async fn acme(mode: Mode, success: bool) {
        let (url, task) = server(mode, false).await;
        let request = axum::http::Request::builder()
            .uri(url)
            .body(instant_acme::BodyWrapper::from(Vec::new()))
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            BoundedHttp(reqwest::Client::new(), None).request(request),
        )
        .await
        .expect("must reject oversized headers without waiting for the body");
        task.abort();
        if success {
            assert_eq!(
                result.unwrap().body.into_bytes().await.unwrap().len(),
                LIMIT
            );
        } else {
            let error = match result {
                Err(e) => e.to_string(),
                Ok(_) => panic!("oversized ACME body accepted"),
            };
            assert!(error.contains("ACME 응답이 너무 큽니다"), "{error}");
        }
    }
    async fn cloudflare(mode: Mode, success: bool) {
        let (url, task) = server(mode, true).await;
        let client =
            CloudflareClient::new(&url, "test-token-must-not-leak", Duration::from_secs(3))
                .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), client.list_zones())
            .await
            .expect("must reject oversized headers without waiting for the body");
        task.abort();
        if success {
            assert_eq!(result.unwrap()[0].name, "example.com");
        } else {
            let error = result.unwrap_err();
            assert!(
                error.message.contains("Cloudflare API 응답이 너무 큽니다"),
                "{error}"
            );
            assert_eq!(error.status, Some(200));
            assert!(!error.to_string().contains("test-token-must-not-leak"));
        }
    }
    #[tokio::test]
    async fn acme_oversized_content_length() {
        acme(Mode::Declared, false).await;
    }
    #[tokio::test]
    async fn acme_oversized_chunked_body() {
        acme(Mode::Chunked, false).await;
    }
    #[tokio::test]
    async fn acme_boundary_body_succeeds() {
        acme(Mode::Boundary, true).await;
    }
    #[tokio::test]
    async fn cloudflare_oversized_content_length() {
        cloudflare(Mode::Declared, false).await;
    }
    #[tokio::test]
    async fn cloudflare_oversized_chunked_body() {
        cloudflare(Mode::Chunked, false).await;
    }
    #[tokio::test]
    async fn cloudflare_boundary_body_succeeds() {
        cloudflare(Mode::Boundary, true).await;
    }
}

#[cfg(test)]
mod invalid_order_tests {
    use super::*;

    #[test]
    fn known_types_include_hints_and_unknown_types_are_preserved() {
        for (kind, hint) in [
            ("connection", "인증기관이 이 서버의 80 포트에 연결하지 못했습니다. 해외에서도 80 포트로 접속되는지 확인하세요."),
            ("dns", "인증기관이 이 도메인의 DNS 레코드를 찾지 못했습니다."),
            ("unauthorized", "검증 주소의 응답이 맞지 않습니다. 도메인이 이 서버를 가리키는지, 앞단 프록시가 요청을 그대로 넘기는지 확인하세요."),
            ("caa", "도메인의 CAA 레코드가 이 인증기관의 발급을 막고 있습니다."),
        ] {
            let problem_type = format!("urn:ietf:params:acme:error:{kind}");
            assert_eq!(
                format_acme_problem(Some(&problem_type), Some("detail")),
                Some(format!("{kind} — detail {hint}")),
                "{kind}"
            );
        }
        assert_eq!(
            format_acme_problem(Some("urn:other:problem"), Some("detail")),
            Some("urn:other:problem — detail".into())
        );
    }

    #[test]
    fn missing_detail_keeps_the_type() {
        assert_eq!(
            format_acme_problem(Some("custom"), None),
            Some("custom".into())
        );
        assert_eq!(
            format_acme_problem(None, Some("detail")),
            Some("detail".into())
        );
    }

    #[test]
    fn ca_text_drops_control_characters_and_caps_multibyte_detail() {
        let problem = serde_json::from_value(json!({
            "type": "cus\rtom",
            "detail": format!("{}\n가\u{1b}", "가".repeat(300))
        })).unwrap();
        let message = invalid_order_message(
            OrderStatus::Invalid, None, &[("name\u{7}".into(), problem)],
        );
        assert_eq!(
            message,
            format!("ACME 주문 상태가 유효하지 않습니다: Invalid. 인증기관이 알려 준 이유 — name: custom — {}…", "가".repeat(300))
        );
        let boundary = "가".repeat(300);
        assert_eq!(format_acme_problem(None, Some(&boundary)), Some(boundary));
    }

    #[test]
    fn five_distinct_names_include_all_their_reasons() {
        let problem: Problem = serde_json::from_value(json!({"type": "custom"})).unwrap();
        let mut challenges: Vec<_> = (1..=7)
            .map(|i| (format!("name-{i}"), problem.clone()))
            .collect();
        challenges.push(("name-1".into(), serde_json::from_value(json!({"type": "other"})).unwrap()));
        challenges.push(challenges[0].clone());
        let message = invalid_order_message(OrderStatus::Invalid, None, &challenges);
        assert!(message.ends_with("name-1: custom; other; name-2: custom; name-3: custom; name-4: custom; name-5: custom; 외 2개"), "{message}");
    }

    #[test]
    fn order_problem_includes_named_subproblems() {
        let problem = serde_json::from_value(json!({
            "type": "order",
            "detail": "rejected",
            "subproblems": [{
                "identifier": {"type": "dns", "value": "name"},
                "type": "sub",
                "detail": "failed"
            }]
        })).unwrap();
        let message = invalid_order_message(OrderStatus::Invalid, Some(&problem), &[]);
        assert!(message.ends_with("order — rejected; name: sub — failed"), "{message}");
    }

    #[test]
    fn no_problem_uses_the_no_reason_text() {
        let expected = "ACME 주문 상태가 유효하지 않습니다: Invalid. 인증기관이 실패 이유를 알려 주지 않았습니다.";
        assert_eq!(invalid_order_message(OrderStatus::Invalid, None, &[]), expected);
        let empty = serde_json::from_value(json!({"type": "", "detail": "\n"})).unwrap();
        assert_eq!(invalid_order_message(OrderStatus::Invalid, Some(&empty), &[]), expected);
    }
}
