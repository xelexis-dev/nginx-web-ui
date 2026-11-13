//! Stored-secrets health (`apps/web/app/admin/secrets/page.tsx`).
//!
//! Every sealed column opens with the same data key; losing it silently stops mail, certificate
//! renewal and OAuth sign-in at once, so the states are gathered in one place. Only the state is
//! reported — never a value. The explanatory copy (impact, recovery) belongs to the screen and is
//! keyed by `kind`: `oauth`, `certificate`, `acme`, `smtp`, `webhook`, `totp`.

use serde::Serialize;
use serde_json::Value;

use super::{AdminResult, unavailable};
use crate::crypto::{SealedState, SecretBox};
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SecretHealth {
    pub kind: &'static str,
    pub label: String,
    /// `sealed-ok` | `sealed-broken` | `plaintext` | `absent` (TS `SealedState`).
    pub state: &'static str,
    pub href: Option<String>,
}

fn state_str(state: SealedState) -> &'static str {
    match state {
        SealedState::SealedOk => "sealed-ok",
        SealedState::SealedBroken => "sealed-broken",
        SealedState::Plaintext => "plaintext",
        SealedState::Absent => "absent",
    }
}

fn row(kind: &'static str, label: String, state: SealedState, href: Option<String>) -> SecretHealth {
    SecretHealth { kind, label, state: state_str(state), href }
}

pub async fn view(state: &AppState) -> AdminResult<Vec<SecretHealth>> {
    let client = state.db.get().await.map_err(unavailable)?;
    let secrets: &SecretBox = &state.secrets;
    let check = |stored: Option<&str>| secrets.sealed_state(stored);
    let mut rows = Vec::new();

    for r in client.query("SELECT id, name, client_secret_enc FROM oauth_providers ORDER BY sort_order, name, id", &[]).await.map_err(unavailable)? {
        let id: String = r.get("id");
        rows.push(row("oauth", r.get("name"), check(r.get("client_secret_enc")), Some(format!("/admin/oauth/{id}/edit"))));
    }
    for r in client.query("SELECT id, domains, key_pem FROM certificates ORDER BY id", &[]).await.map_err(unavailable)? {
        let id: String = r.get("id");
        let domains: Value = r.get("domains");
        let label = domains
            .as_array()
            .map(|d| d.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| id.clone());
        rows.push(row("certificate", label, check(r.get("key_pem")), Some(format!("/certificates/{id}"))));
    }
    for r in client.query("SELECT id, directory_url, account_key_pem FROM acme_accounts ORDER BY id", &[]).await.map_err(unavailable)? {
        rows.push(row("acme", r.get("directory_url"), check(r.get("account_key_pem")), None));
    }
    if let Some(r) = client.query_opt("SELECT host, password_enc FROM smtp_settings LIMIT 1", &[]).await.map_err(unavailable)? {
        rows.push(row("smtp", r.get("host"), check(r.get("password_enc")), Some("/admin/settings/smtp".into())));
    }
    for r in client.query("SELECT id, name, url_enc, headers_enc FROM webhook_targets ORDER BY name, id", &[]).await.map_err(unavailable)? {
        let id: String = r.get("id");
        let url = check(r.get("url_enc"));
        let headers = check(r.get("headers_enc"));
        // The worse of the two: a broken header stops delivery as surely as a broken URL.
        let worst = if url == SealedState::SealedBroken || headers == SealedState::SealedBroken { SealedState::SealedBroken } else { url };
        rows.push(row("webhook", r.get("name"), worst, Some(format!("/admin/notifications/webhooks/{id}/edit"))));
    }
    // Only accounts that use 2FA: an unused secret is absent or meaningless.
    for r in client.query("SELECT id, email, totp_secret FROM users WHERE totp_enabled ORDER BY created_at, id", &[]).await.map_err(unavailable)? {
        let id: String = r.get("id");
        let label = r.get::<_, Option<String>>("email").unwrap_or_else(|| id.clone());
        rows.push(row("totp", label, check(r.get("totp_secret")), Some(format!("/admin/users/{id}"))));
    }
    Ok(rows)
}
