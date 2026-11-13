//! Notification text/HTML alternatives. Values are escaped before entering HTML.
use crate::notify::{NotificationEvent, RenderContext, resolve_link};
use serde_json::Value;

pub struct MailContent {
    pub subject: String,
    pub text: String,
    pub html: String,
}
pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn iso_display(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|d| {
            d.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| value.into())
}
pub fn mail_content(event: &NotificationEvent, ctx: RenderContext<'_>) -> MailContent {
    let empty = serde_json::Map::new();
    let data = event.data.as_ref().unwrap_or(&empty);
    let domains: Vec<&str> = data
        .get("domains")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let expires = data
        .get("expiresAt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let staging = data
        .get("staging")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let days = data.get("daysLeft").and_then(Value::as_i64);
    let link = resolve_link(event.path.as_deref(), ctx.base_url)
        .filter(|s| url::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https")));
    let mut subject = format!("[nginx-web-ui] {}", event.title);
    let mut lines = vec![
        event.message.as_deref().unwrap_or(&event.title).to_owned(),
        String::new(),
    ];
    if event.event == "cert.renewal_due"
        && expires.is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
    {
        subject = format!(
            "[nginx-web-ui] 인증서 갱신 필요: {}",
            domains.first().copied().unwrap_or("(도메인 없음)")
        );
        lines = vec![
            "DNS-01 방식으로 발급된 인증서의 갱신 시점이 되었습니다.".into(),
            String::new(),
            format!(
                "  도메인   : {}",
                if domains.is_empty() {
                    "(도메인 없음)".into()
                } else {
                    domains.join(", ")
                }
            ),
            format!(
                "  만료     : {} (약 {}일 남음)",
                expires.unwrap(),
                days.unwrap_or(0)
            ),
        ];
        if staging {
            lines.push("  구분     : STAGING (테스트 발급)".into());
        }
        lines.extend([
            String::new(),
            "DNS-01 인증서는 TXT 레코드를 다시 등록해야 하므로 자동으로 갱신되지 않습니다.".into(),
            "관리 화면의 Certificates → 해당 인증서 → 갱신에서 안내되는 TXT 레코드를 DNS에".into(),
            "추가한 뒤 검증을 진행하세요.".into(),
        ]);
    } else {
        if !domains.is_empty() {
            lines.push(format!("  도메인   : {}", domains.join(", ")));
        }
        if let Some(expires) = expires {
            lines.push(format!("  만료     : {expires}"));
        }
        if staging {
            lines.push("  구분     : STAGING (테스트 발급)".into());
        }
    }
    if let Some(link) = &link {
        lines.extend([String::new(), format!("  바로가기 : {link}")]);
    }
    if event.event == "cert.renewal_due" {
        lines.extend([
            String::new(),
            "갱신을 마치면 이 안내는 자동으로 멈춥니다.".into(),
        ]);
    }
    let (accent, badge) = match event.severity.as_str() {
        "success" => ("#10b981", "완료"),
        "warning" => ("#f59e0b", "주의"),
        "error" => ("#ef4444", "실패"),
        _ => ("#3b82f6", "안내"),
    };
    let mut details = String::new();
    let mut row = |label: &str, value: &str| {
        details.push_str(&format!("<tr><td class=\"k\" style=\"padding:10px 0;vertical-align:top;width:96px;color:#6b7280;font-size:13px;font-weight:500;\">{}</td><td class=\"v\" style=\"padding:10px 0;vertical-align:top;color:#111111;font-size:14px;\">{}</td></tr>",escape_html(label),escape_html(value).replace('\n',"<br>")));
    };
    if !domains.is_empty() {
        row("도메인", &domains.join("\n"));
    }
    if let Some(expires) = expires {
        row("만료", &iso_display(expires));
    }
    if staging {
        row("구분", "STAGING (테스트 발급)");
    }
    let details = if details.is_empty() {
        details
    } else {
        format!(
            "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" width=\"100%\" style=\"border-collapse:collapse;border-top:1px solid #e5e7eb;margin-top:20px;\">{details}</table>"
        )
    };
    let button=link.as_ref().map(|link|format!("<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin-top:24px;\"><tr><td class=\"btn\" style=\"background:#111111;border-radius:8px;\"><a class=\"btn-a\" href=\"{}\" style=\"display:inline-block;padding:11px 18px;color:#ffffff;font-size:14px;font-weight:600;text-decoration:none;\">인증서 열기</a></td></tr></table>",escape_html(link))).unwrap_or_default();
    let countdown=days.map(|days|format!("<div class=\"days\" style=\"font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:34px;line-height:1;font-weight:600;color:{accent};margin:0 0 10px;\">D-{days}</div>")).unwrap_or_default();
    let title = escape_html(&event.title);
    let preheader = escape_html(event.message.as_deref().unwrap_or(&event.title));
    let message=event.message.as_ref().filter(|s|!s.is_empty()).map(|s|format!("<p class=\"muted\" style=\"margin:8px 0 0;font-size:15px;line-height:1.6;color:#6b7280;\">{}</p>",escape_html(s))).unwrap_or_default();
    let html = format!(
        r##"<!doctype html>
<html lang="ko"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light dark"><meta name="supported-color-schemes" content="light dark"><title>{title}</title>
<style>@media (prefers-color-scheme: dark) {{.page {{background:#0b0b0c !important;}} .card {{background:#151517 !important;border-color:#2a2a2e !important;}} .ink,.v {{color:#f4f4f5 !important;}} .muted,.k {{color:#9ca3af !important;}} .btn {{background:#f4f4f5 !important;}} .btn-a {{color:#0b0b0c !important;}}}} @media (max-width:620px) {{.card {{padding:24px 20px !important;}} .days {{font-size:28px !important;}}}}</style></head>
<body class="page" style="margin:0;padding:0;background:#f7f7f8;"><div style="display:none;max-height:0;overflow:hidden;opacity:0;">{preheader}</div><table role="presentation" cellpadding="0" cellspacing="0" border="0" width="100%" style="background:#f7f7f8;"><tr><td align="center" style="padding:32px 16px;"><table role="presentation" cellpadding="0" cellspacing="0" border="0" width="600" style="width:600px;max-width:100%;border-collapse:collapse;"><tr><td class="card" style="background:#ffffff;border:1px solid #e5e7eb;border-top:3px solid {accent};border-radius:12px;padding:28px 28px 24px;"><table role="presentation" cellpadding="0" cellspacing="0" border="0" width="100%"><tr><td class="muted" style="font-size:12px;font-weight:600;letter-spacing:0.08em;text-transform:uppercase;color:#6b7280;">nginx-web-ui</td><td align="right" style="font-size:12px;font-weight:600;color:{accent};">{badge}</td></tr></table><div style="margin-top:22px;">{countdown}<h1 class="ink" style="margin:0;font-size:22px;line-height:1.3;font-weight:600;color:#111111;">{title}</h1>{message}</div>{details}{button}</td></tr></table></td></tr></table></body></html>"##
    );
    MailContent {
        subject,
        text: lines.join("\n"),
        html,
    }
}
