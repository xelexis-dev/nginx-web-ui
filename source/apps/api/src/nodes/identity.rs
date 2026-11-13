//! Prove possession of the paired token without sending that token to a new address.
use crate::{auth::principal, error::ApiError, request_context::RequestContext, state::AppState};
use axum::{Extension, Json, extract::{Request, State}, http::StatusCode};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::hmac::{Hmac, Mac as _};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::BTreeMap;

pub fn refused() -> ApiError {
    let message = "노드 주소의 프로토콜·호스트·포트는 바꿀 수 없습니다. 연결을 해제한 뒤 새 주소로 다시 연결하세요.";
    let mut error = ApiError::new(StatusCode::BAD_REQUEST, "validation_failed", message);
    error.field_errors = Some(BTreeMap::from([("baseUrl".into(), message.into())]));
    error
}

// The stored lowercase SHA-256 hex string is the HMAC key; the nonce is raw bytes.
fn mac(digest: &str, nonce: &[u8; 32], instance_id: &str) -> Hmac<Sha256> {
    let mut mac = <Hmac<Sha256> as hkdf::hmac::KeyInit>::new_from_slice(digest.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(nonce);
    mac.update(instance_id.as_bytes());
    mac
}

pub fn proof(digest: &str, nonce: &[u8; 32], instance_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(mac(digest, nonce, instance_id).finalize().into_bytes())
}

pub fn verify(digest: &str, nonce: &[u8; 32], instance_id: &str, value: &Value) -> bool {
    let Some(tag) = value["proof"].as_str().and_then(|v| URL_SAFE_NO_PAD.decode(v).ok()) else { return false };
    value["instanceId"].as_str() == Some(instance_id)
        && mac(digest, nonce, instance_id).verify_slice(&tag).is_ok()
}

pub async fn challenge(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let now = principal::now_unix_ms();
    principal::check_pairing_ip_lock(&state, &ctx, now)?;
    // Count every anonymous challenge, including successful ones, before reading its body.
    if let Some(ip) = &ctx.ip {
        state.runtime.login_throttle.begin_ip(ip, now).ok_or_else(|| ApiError::new(
            StatusCode::TOO_MANY_REQUESTS, "rate_limited", "요청이 잦아 잠시 차단되었습니다. 15분 뒤에 다시 시도하세요."
        ))?.keep();
    }
    let bytes = axum::body::to_bytes(request.into_body(), 4096).await.map_err(|_| refused())?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|_| refused())?;
    let id = input["gatewayTokenId"].as_str().filter(|v| !v.is_empty() && v.len() <= 128).ok_or_else(refused)?;
    let nonce: [u8; 32] = input["nonce"].as_str().and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
        .and_then(|v| v.try_into().ok()).ok_or_else(refused)?;
    let client = state.db.get().await.map_err(|_| principal::db_unavailable())?;
    let row = client.query_opt("SELECT token_hash FROM gateway_tokens WHERE id=$1 AND revoked_at IS NULL", &[&id])
        .await.map_err(|_| principal::db_unavailable())?.ok_or_else(refused)?;
    let digest: String = row.get(0);
    drop(client);
    let instance_id = super::service::ensure_instance_id(&state).await?;
    Ok(Json(json!({"instanceId":instance_id,"proof":proof(&digest,&nonce,&instance_id)})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proof_binds_token_nonce_and_instance_and_rejects_truncation() {
        let nonce = [7; 32];
        let digest = crate::auth::token_digest("secret");
        let value = json!({"instanceId":"node","proof":proof(&digest,&nonce,"node")});
        assert_eq!(value["proof"],"t3GEhu9AJU9KIBG-aF_u95Q-hCXUFGjlYx1ag1a4reA");
        assert!(verify(&digest,&nonce,"node",&value));
        assert!(!verify("other",&nonce,"node",&value));
        assert!(!verify(&digest,&[8;32],"node",&value));
        assert!(!verify(&digest,&nonce,"other",&value));
        assert!(!verify(&digest,&nonce,"node",&json!({"instanceId":"node","proof":"AA"})));
    }
}
