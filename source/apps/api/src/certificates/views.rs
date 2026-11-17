//! Certificate read projections. No raw certificate or account row crosses this boundary.
use crate::{
    auth::{
        principal::{UserPrincipal, db_unavailable},
        rbac::Action,
    },
    error::ApiError,
    state::AppState,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn text(value: &Value) -> Value {
    value.as_str().map(|s| json!(s)).unwrap_or(Value::Null)
}
fn instant(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| {
            json!(
                d.with_timezone(&chrono::Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            )
        })
        .unwrap_or(Value::Null)
}
pub fn certificate_view(raw: &Value, email: Option<&str>) -> Value {
    let mut safe = json!({});
    for (target, source) in [
        ("id", "id"),
        ("alias", "alias"),
        ("status", "status"),
        ("source", "source"),
        ("challengeType", "challenge_type"),
        ("dnsProvider", "dns_provider"),
        ("lastError", "last_error"),
    ] {
        safe[target] = text(&raw[source]);
    }
    for (target, source) in [
        ("issuedAt", "issued_at"),
        ("expiresAt", "expires_at"),
        ("lastAttemptAt", "last_attempt_at"),
        ("renewalNoticeSentAt", "renewal_notice_sent_at"),
        ("createdAt", "created_at"),
    ] {
        safe[target] = instant(&raw[source]);
    }
    safe["domains"] = json!(
        raw["domains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
    );
    safe["staging"] = json!(raw["staging"].as_bool().unwrap_or(false));
    safe["failureCount"] = json!(raw["failure_count"].as_i64().unwrap_or(0));
    safe["hasDnsToken"] = json!(raw["dns_provider_token_enc"].is_string());
    safe["acmeEmail"] = json!(email);
    safe["dnsRecords"]=raw["dns_order"]["records"].as_array().map(|rows|json!(rows.iter().map(|r|json!({"domain":text(&r["domain"]),"recordName":text(&r["recordName"]),"recordValue":text(&r["recordValue"])})).collect::<Vec<_>>())).unwrap_or(Value::Null);
    safe["dnsSubmittedAt"] = text(&raw["dns_order"]["submittedAt"]);
    safe["issueLog"]=json!(raw["issue_log"].as_array().into_iter().flatten().filter(|v|v["at"].is_string()&&v["msg"].is_string()).map(|v|json!({"at":v["at"],"level":if v["level"]=="error"{"error"}else{"info"},"msg":v["msg"]})).collect::<Vec<_>>());
    safe
}
async fn emails(
    client: &impl tokio_postgres::GenericClient,
) -> Result<BTreeMap<String, String>, ApiError> {
    Ok(client
        .query("SELECT directory_url,email FROM acme_accounts", &[])
        .await
        .map_err(|_| db_unavailable())?
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect())
}
fn project(state: &AppState, raw: &Value, emails: &BTreeMap<String, String>) -> Value {
    let directory = super::protocol::resolve_directory_url(
        raw["staging"].as_bool().unwrap_or(false),
        state.config.acme_directory_url.as_deref(),
    );
    certificate_view(raw, emails.get(&directory).map(String::as_str))
}
/// Columns used by the view; the sealed DNS token is reduced to a presence marker.
pub const VIEW_COLUMNS: &str = "id,domains,alias,status,source,staging,challenge_type,dns_provider,\
CASE WHEN dns_provider_token_enc IS NOT NULL THEN '' END AS dns_provider_token_enc,dns_order,\
issued_at,expires_at,last_attempt_at,renewal_notice_sent_at,created_at,last_error,failure_count,issue_log";

pub async fn list(state: &AppState) -> Result<Vec<Value>, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let emails = emails(&**client).await?;
    let rows = client
        .query(
            &format!("SELECT to_jsonb(c) FROM (SELECT {VIEW_COLUMNS} FROM certificates) c ORDER BY created_at DESC,id DESC"),
            &[],
        )
        .await
        .map_err(|_| db_unavailable())?;
    Ok(rows
        .into_iter()
        .map(|r| project(state, &r.get::<_, Value>(0), &emails))
        .collect())
}
pub async fn detail(state: &AppState, id: &str) -> Result<Value, ApiError> {
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let raw = client
        .query_opt(&format!("SELECT to_jsonb(c) FROM (SELECT {VIEW_COLUMNS} FROM certificates WHERE id=$1) c"), &[&id])
        .await
        .map_err(|_| db_unavailable())?
        .ok_or_else(ApiError::not_found)?
        .get::<_, Value>(0);
    let emails = emails(&**client).await?;
    Ok(project(state, &raw, &emails))
}
fn domains(values: &Value) -> Value {
    let names = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    json!({"primary":names.first(),"all":names.join(", "),"extra":names.len().saturating_sub(1)})
}
pub async fn view(
    state: &AppState,
    session: &UserPrincipal,
    path: &str,
    query: &BTreeMap<String, String>,
) -> Result<Option<Value>, ApiError> {
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if parts.first() != Some(&"certificates") {
        return Ok(None);
    }
    let action = match parts.as_slice() {
        ["certificates", "new"] | ["certificates", "import"] | ["certificates", _, "edit"] => {
            Action::ConfigWrite
        }
        ["certificates", _, "replace"] => Action::NginxReload,
        _ => Action::ConfigRead,
    };
    if !session.can(action) {
        return Err(ApiError::forbidden());
    }
    if parts.len() > 3 {
        return Ok(None);
    }
    let can_write = session.can(Action::ConfigWrite);
    if parts == ["certificates", "import"] {
        return Ok(Some(json!({})));
    }
    if parts == ["certificates", "new"] {
        let fallback = if session.email.is_none() {
            let client = state.db.get().await.map_err(|_| db_unavailable())?;
            client
                .query_opt(
                    "SELECT email FROM acme_accounts ORDER BY created_at,id LIMIT 1",
                    &[],
                )
                .await
                .map_err(|_| db_unavailable())?
                .map(|r| r.get::<_, String>(0))
        } else {
            None
        };
        return Ok(Some(
            json!({"defaultEmail":session.email.as_ref().or(fallback.as_ref()).map(String::as_str).unwrap_or("")}),
        ));
    }
    if parts.len() == 1 {
        let certificates = list(state).await?;
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let counts:BTreeMap<String,i64>=client.query("SELECT certificate_id,count(*) FROM nginx_servers WHERE certificate_id IS NOT NULL GROUP BY certificate_id",&[]).await.map_err(|_|db_unavailable())?.into_iter().map(|r|(r.get(0),r.get(1))).collect();
        let items=certificates.iter().map(|c|json!({"id":c["id"],"domains":domains(&c["domains"]),"alias":c["alias"],"status":c["status"],"source":c["source"],"staging":c["staging"],"expiresAt":c["expiresAt"],"linkedCount":counts.get(c["id"].as_str().unwrap_or("")).copied().unwrap_or(0)})).collect::<Vec<_>>();
        return Ok(Some(json!({"data":{"items":items},"canWrite":can_write})));
    }
    let id = parts[1];
    let certificate = detail(state, id).await?;
    if parts.len() == 3 {
        match parts[2] {
            "replace" => {
                if certificate["source"] != "manual" {
                    return Err(ApiError::not_found());
                }
                return Ok(Some(json!({"id":id})));
            }
            "edit" => {
                if certificate["source"] != "acme" {
                    return Ok(Some(
                        json!({"redirectTo":format!("/certificates/{}/replace",crate::nodes::views::segment(id))}),
                    ));
                }
                return Ok(Some(
                    json!({"certificate":certificate,"isIssued":certificate["status"]=="issued","hasStoredToken":certificate["hasDnsToken"],"defaults":{"domains":certificate["domains"],"alias":certificate["alias"].as_str().unwrap_or(""),"email":certificate["acmeEmail"].as_str().or(session.email.as_deref()).unwrap_or(""),"staging":certificate["staging"],"challengeType":if certificate["challengeType"]=="dns-01"{"dns-01"}else{"http-01"},"dnsProvider":if certificate["dnsProvider"]=="cloudflare"{"cloudflare"}else{""}}}),
                ));
            }
            _ => return Ok(None),
        }
    }
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let servers=client.query("SELECT id,server_name,listen FROM nginx_servers WHERE certificate_id=$1 ORDER BY server_name,id",&[&id]).await.map_err(|_|db_unavailable())?.into_iter().map(|r|json!({"id":r.get::<_,String>(0),"serverName":r.get::<_,String>(1),"listen":r.get::<_,i32>(2)})).collect::<Vec<_>>();
    let mut data = certificate.clone();
    data["domains"] = domains(&certificate["domains"]);
    data["linkedServers"] = json!(servers);
    data["dnsOrder"] = if certificate["dnsSubmittedAt"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        json!({"phase":"verifying"})
    } else if certificate["dnsRecords"].is_array() {
        json!({"phase":"awaiting-txt","records":certificate["dnsRecords"]})
    } else {
        Value::Null
    };
    data["issueLog"]=json!(certificate["issueLog"].as_array().into_iter().flatten().map(|r|json!({"at":r["at"].as_str().unwrap_or("").replace('T'," ").chars().take(19).collect::<String>(),"level":r["level"],"msg":r["msg"]})).collect::<Vec<_>>());
    let uncovered = query
        .get("uncovered")
        .map(|s| s.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(Some(
        json!({"data":data,"canWrite":can_write,"uncovered":uncovered}),
    ))
}
