use super::protocol::DnsChallengeRecord;
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Duration;

// A 50-zone page plus metadata fits comfortably; cap before parsing upstream JSON.
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_API_BASE: &str = "https://api.cloudflare.com/client/v4";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CloudflareError {
    pub message: String,
    pub status: Option<u16>,
    pub codes: Vec<u64>,
}
impl CloudflareError {
    fn local(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            codes: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudflareZone {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedTxtRecord {
    pub zone_id: String,
    pub record_id: String,
    pub name: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublishedRecords {
    pub created: Vec<CreatedTxtRecord>,
}
#[derive(Debug)]
pub struct CleanupFailure {
    pub record: CreatedTxtRecord,
    pub error: CloudflareError,
}
#[derive(Debug, thiserror::Error)]
#[error("{cause}; TXT 정리 실패 {count}건", count=.cleanup_failures.len())]
pub struct PublishError {
    pub cause: CloudflareError,
    pub cleanup_failures: Vec<CleanupFailure>,
}

/// Tokens are deliberately excluded from Debug and URLs. Dropping an in-flight
/// request cancels it; callers must retain PublishedRecords until cleanup succeeds.
pub struct CloudflareClient {
    http: Client,
    base: url::Url,
    token: String,
}
impl CloudflareClient {
    pub fn new(base: &str, token: &str, timeout: Duration) -> Result<Self, CloudflareError> {
        let base = url::Url::parse(base)
            .map_err(|_| CloudflareError::local("Cloudflare API 주소가 올바르지 않습니다."))?;
        if !matches!(base.scheme(), "https" | "http")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(CloudflareError::local(
                "Cloudflare API 주소가 올바르지 않습니다.",
            ));
        }
        let http = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| CloudflareError::local(e.to_string()))?;
        Ok(Self {
            http,
            base,
            token: token.into(),
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, CloudflareError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| CloudflareError::local("잘못된 API 주소"))?
            .pop_if_empty()
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v)));
        }
        let mut request = self.http.request(method, url).bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|e| {
            CloudflareError::local(format!("Cloudflare API에 연결하지 못했습니다: {e}"))
        })?;
        let status = response.status();
        let too_large = || CloudflareError {
            message: "Cloudflare API 응답이 너무 큽니다.".into(),
            status: Some(status.as_u16()),
            codes: vec![],
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| CloudflareError {
            message: format!("Cloudflare API 응답을 해석할 수 없습니다 (HTTP {status})."),
            status: Some(status.as_u16()),
            codes: vec![],
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope: Value = serde_json::from_slice(&bytes).map_err(|_| CloudflareError {
            message: format!("Cloudflare API 응답을 해석할 수 없습니다 (HTTP {status})."),
            status: Some(status.as_u16()),
            codes: vec![],
        })?;
        if !status.is_success() || envelope["success"] != true {
            let errors = envelope["errors"].as_array().cloned().unwrap_or_default();
            let codes = errors.iter().filter_map(|e| e["code"].as_u64()).collect();
            let mut message = errors
                .iter()
                .filter_map(|e| {
                    e["message"].as_str().map(|m| {
                        if let Some(code) = e["code"].as_u64() {
                            format!("{m} (code {code})")
                        } else {
                            m.into()
                        }
                    })
                })
                .collect::<Vec<_>>()
                .join("; ");
            if message.is_empty() {
                message = format!("HTTP {status}");
            }
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                message.push_str(
                    " (토큰이 유효한지, Zone:DNS:Edit + Zone:Zone:Read 권한이 있는지 확인하세요)",
                );
            }
            return Err(CloudflareError {
                message,
                status: Some(status.as_u16()),
                codes,
            });
        }
        Ok(envelope)
    }
    fn result<T: DeserializeOwned>(envelope: Value) -> Result<T, CloudflareError> {
        serde_json::from_value(envelope["result"].clone()).map_err(|_| {
            CloudflareError::local("Cloudflare API 응답의 result가 올바르지 않습니다.")
        })
    }

    pub async fn list_zones(&self) -> Result<Vec<CloudflareZone>, CloudflareError> {
        let mut zones = Vec::new();
        for page in 1..=1000u64 {
            let envelope = self
                .request(
                    Method::GET,
                    &["zones"],
                    &[("per_page", "50".into()), ("page", page.to_string())],
                    None,
                )
                .await?;
            let total = envelope["result_info"]["total_pages"].as_u64();
            let batch: Vec<CloudflareZone> = Self::result(envelope)?;
            let count = batch.len();
            zones.extend(batch);
            if total.is_some_and(|total| page >= total) || (total.is_none() && count < 50) {
                return Ok(zones);
            }
        }
        Err(CloudflareError::local(
            "Cloudflare zone 페이지가 너무 많습니다.",
        ))
    }

    /// Works for user-owned and account-owned API tokens. This confirms zone
    /// visibility; DNS Edit is checked by the actual create/delete operations.
    pub async fn verify_token(&self) -> Result<Vec<CloudflareZone>, CloudflareError> {
        let zones = self.list_zones().await?;
        if zones.is_empty() {
            return Err(CloudflareError::local(
                "접근할 수 있는 Cloudflare zone이 없습니다. Zone:DNS:Edit + Zone:Zone:Read 권한을 확인하세요.",
            ));
        }
        Ok(zones)
    }

    pub async fn find_zone(&self, hostname: &str) -> Result<CloudflareZone, CloudflareError> {
        let labels: Vec<_> = hostname.trim_end_matches('.').split('.').collect();
        let mut tried = Vec::new();
        for i in 0..labels.len().saturating_sub(1) {
            let candidate = labels[i..].join(".");
            tried.push(candidate.clone());
            let zones: Vec<CloudflareZone> = Self::result(
                self.request(
                    Method::GET,
                    &["zones"],
                    &[("name", candidate.clone())],
                    None,
                )
                .await?,
            )?;
            if let Some(zone) = zones
                .into_iter()
                .find(|z| z.name.eq_ignore_ascii_case(&candidate))
            {
                return Ok(zone);
            }
        }
        Err(CloudflareError::local(format!(
            "{hostname} 에 해당하는 Cloudflare zone 을 찾지 못했습니다. 이 도메인이 Cloudflare 네임서버를 쓰는지, 토큰이 그 zone 에 접근할 수 있는지 확인하세요. (조회한 이름: {})",
            tried.join(", ")
        )))
    }

    pub async fn create_txt_record(
        &self,
        zone_id: &str,
        name: &str,
        content: &str,
    ) -> Result<CreatedTxtRecord, CloudflareError> {
        self.create_owned_txt(
            zone_id,
            name,
            content,
            "nginx-web-ui ACME dns-01 (자동 생성/삭제)",
        )
        .await
    }

    /// Persist this unique owner marker BEFORE calling. A lost response can be
    /// reconciled by exact owner/name/content without touching another job's TXT.
    pub async fn create_owned_txt(
        &self,
        zone_id: &str,
        name: &str,
        content: &str,
        owner: &str,
    ) -> Result<CreatedTxtRecord, CloudflareError> {
        #[derive(Deserialize)]
        struct Record {
            id: String,
        }
        let record: Record = Self::result(
            self.request(
                Method::POST,
                &["zones", zone_id, "dns_records"],
                &[],
                Some(json!({"type":"TXT","name":name,"content":content,"ttl":60,"comment":owner})),
            )
            .await?,
        )?;
        if record.id.is_empty() {
            return Err(CloudflareError::local(
                "Cloudflare가 생성한 레코드 ID가 비어 있습니다.",
            ));
        }
        Ok(CreatedTxtRecord {
            zone_id: zone_id.into(),
            record_id: record.id,
            name: name.into(),
        })
    }

    pub async fn find_owned_txt(
        &self,
        zone_id: &str,
        name: &str,
        content: &str,
        owner: &str,
    ) -> Result<Vec<CreatedTxtRecord>, CloudflareError> {
        let mut found = Vec::new();
        for page in 1..=1000u64 {
            let envelope = self
                .request(
                    Method::GET,
                    &["zones", zone_id, "dns_records"],
                    &[
                        ("type", "TXT".into()),
                        ("name", name.into()),
                        ("per_page", "100".into()),
                        ("page", page.to_string()),
                    ],
                    None,
                )
                .await?;
            let records: Vec<Value> = Self::result(envelope.clone())?;
            for record in records {
                if record["type"] == "TXT"
                    && record["name"].as_str().is_some_and(|n| {
                        n.trim_end_matches('.')
                            .eq_ignore_ascii_case(name.trim_end_matches('.'))
                    })
                    && record["content"] == content
                    && record["comment"] == owner
                {
                    let id = record["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            CloudflareError::local("Cloudflare 레코드 ID가 없습니다.")
                        })?;
                    found.push(CreatedTxtRecord {
                        zone_id: zone_id.into(),
                        record_id: id.into(),
                        name: name.into(),
                    });
                }
            }
            if page >= envelope["result_info"]["total_pages"].as_u64().unwrap_or(1) {
                return Ok(found);
            }
        }
        Err(CloudflareError::local(
            "Cloudflare 레코드 페이지 수가 제한을 초과했습니다.",
        ))
    }

    pub async fn delete_record(
        &self,
        zone_id: &str,
        record_id: &str,
    ) -> Result<(), CloudflareError> {
        match self
            .request(
                Method::DELETE,
                &["zones", zone_id, "dns_records", record_id],
                &[],
                None,
            )
            .await
        {
            Ok(_) => Ok(()),
            // Only the documented missing-record code is idempotent. A missing
            // zone or permission failure must not masquerade as cleanup success.
            Err(error)
                if error.codes.contains(&81044) && !matches!(error.status, Some(401 | 403)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn publish_challenge_records(
        &self,
        records: &[DnsChallengeRecord],
    ) -> Result<PublishedRecords, PublishError> {
        let mut published = PublishedRecords::default();
        for record in records {
            let result = async {
                let zone = self.find_zone(&record.record_name).await?;
                self.create_txt_record(&zone.id, &record.record_name, &record.record_value)
                    .await
            }
            .await;
            match result {
                Ok(record) => published.created.push(record),
                Err(cause) => {
                    let cleanup_failures = self
                        .retract_challenge_records(&published)
                        .await
                        .err()
                        .unwrap_or_default();
                    return Err(PublishError {
                        cause,
                        cleanup_failures,
                    });
                }
            }
        }
        Ok(published)
    }

    pub async fn retract_challenge_records(
        &self,
        published: &PublishedRecords,
    ) -> Result<(), Vec<CleanupFailure>> {
        let mut failures = Vec::new();
        for record in &published.created {
            if let Err(error) = self.delete_record(&record.zone_id, &record.record_id).await {
                failures.push(CleanupFailure {
                    record: record.clone(),
                    error,
                });
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }
}
