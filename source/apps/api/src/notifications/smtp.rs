//! SMTP uses lettre's RFC message builder and transport. Credentials never leave over plaintext.
use crate::state::AppState;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart},
    transport::smtp::{
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
};
use std::time::Duration;

/// Secret-bearing runtime configuration deliberately has no Debug or Serialize implementation.
pub struct MailerConfig {
    pub host: String,
    pub port: u16,
    pub secure: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_email: String,
    pub from_name: Option<String>,
    pub reject_unauthorized: bool,
}
pub struct MailInput {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    pub message_id: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct SendResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    pub retryable: bool,
}
impl SendResult {
    pub fn success() -> Self {
        Self {
            ok: true,
            error: None,
            retryable: false,
        }
    }
    pub fn failure(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            retryable,
        }
    }
}
const TIMEOUT: Duration = Duration::from_secs(10);

pub async fn mailer_config(state: &AppState) -> Result<MailerConfig, String> {
    let client = state
        .db
        .get()
        .await
        .map_err(|_| "SMTP 설정을 읽을 수 없습니다.")?;
    let row=client.query_opt("SELECT enabled,host,port,secure,username,password_enc,from_email,from_name,reject_unauthorized FROM smtp_settings WHERE id='current'",&[]).await.map_err(|_|"SMTP 설정을 읽을 수 없습니다.")?.ok_or("SMTP가 설정되지 않았습니다. 관리자 패널 → 사이트 설정 → SMTP 설정에서 설정하세요.")?;
    if !row.get::<_, bool>("enabled") {
        return Err(
            "SMTP가 꺼져 있습니다. 관리자 패널 → 사이트 설정 → SMTP 설정에서 켜세요.".into(),
        );
    }
    let sealed: Option<String> = row.get("password_enc");
    let password=match sealed.filter(|s|!s.is_empty()){Some(value)=>Some(state.secrets.open(Some(&value)).ok_or("저장된 SMTP 비밀번호를 복호화할 수 없습니다(암호화 키 유실). 비밀번호를 다시 입력하세요.")?),None=>None};
    let port = u16::try_from(row.get::<_, i32>("port"))
        .ok()
        .filter(|p| *p > 0)
        .ok_or("SMTP 포트가 올바르지 않습니다.")?;
    Ok(MailerConfig {
        host: row.get("host"),
        port,
        secure: row.get("secure"),
        username: row.get("username"),
        password,
        from_email: row.get("from_email"),
        from_name: row.get("from_name"),
        reject_unauthorized: row.get("reject_unauthorized"),
    })
}
fn resolution_failure(error: std::io::Error) -> SendResult {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::PermissionDenied | ErrorKind::InvalidInput => SendResult::failure("SMTP 서버 설정이 올바르지 않습니다.", false),
        ErrorKind::TimedOut => SendResult::failure("SMTP 서버 응답 시간이 초과되었습니다.", true),
        _ => SendResult::failure("SMTP 서버 연결 또는 인증에 실패했습니다.", true),
    }
}
async fn transport(config: &MailerConfig,runtime:&crate::config::RuntimeConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, SendResult> {
    if config.host.is_empty() || config.port == 0 || config.host.chars().any(char::is_control) {
        return Err(SendResult::failure(
            "SMTP 서버 설정이 올바르지 않습니다.",
            false,
        ));
    }
    let addresses=crate::outbound::resolve_host(&config.host,config.port,runtime).await
        .map_err(resolution_failure)?;
    let tls = TlsParameters::builder(config.host.clone())
        .dangerous_accept_invalid_certs(!config.reject_unauthorized)
        .build()
        .map_err(|_| SendResult::failure("SMTP TLS 설정을 사용할 수 없습니다.", false))?;
    let authenticated = config.username.as_ref().is_some_and(|s| !s.is_empty());
    let security = if config.secure {
        Tls::Wrapper(tls)
    } else if authenticated {
        Tls::Required(tls)
    } else {
        Tls::Opportunistic(tls)
    };
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(addresses[0].ip().to_string())
        .port(config.port)
        .timeout(Some(TIMEOUT))
        .tls(security);
    if let Some(username) = config.username.as_ref().filter(|s| !s.is_empty()) {
        builder = builder.credentials(Credentials::new(
            username.clone(),
            config.password.clone().unwrap_or_default(),
        ));
    }
    Ok(builder.build())
}
fn failed(error: lettre::transport::smtp::Error) -> SendResult {
    let message = if error.is_timeout() {
        "SMTP 서버 응답 시간이 초과되었습니다.".into()
    } else if error.is_tls() {
        "SMTP TLS 연결을 확인하세요.".into()
    } else if let Some(code) = error.status() {
        format!("SMTP 서버가 발송을 거절했습니다 ({code}).")
    } else {
        "SMTP 서버 연결 또는 인증에 실패했습니다.".into()
    };
    SendResult::failure(message, !error.is_permanent())
}
fn message(config: &MailerConfig, input: &MailInput) -> Result<Message, SendResult> {
    let invalid = || SendResult::failure("메일 주소 또는 헤더가 올바르지 않습니다.", false);
    for value in [&input.to, &input.subject, &config.from_email] {
        if value.contains(['\r', '\n']) {
            return Err(invalid());
        }
    }
    if config
        .from_name
        .as_ref()
        .is_some_and(|s| s.contains(['\r', '\n']))
        || input
            .message_id
            .as_ref()
            .is_some_and(|s| s.contains(['\r', '\n']))
    {
        return Err(invalid());
    }
    let from = Mailbox::new(
        config.from_name.clone(),
        config.from_email.parse().map_err(|_| invalid())?,
    );
    let to: Mailbox = input.to.parse().map_err(|_| invalid())?;
    let builder = Message::builder()
        .from(from)
        .to(to)
        .subject(&input.subject)
        .message_id(input.message_id.clone());
    match &input.html {
        Some(html) => builder.multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(input.text.clone()))
                .singlepart(SinglePart::html(html.clone())),
        ),
        None => builder.singlepart(SinglePart::plain(input.text.clone())),
    }
    .map_err(|_| invalid())
}
pub async fn send_with_config(config: &MailerConfig, input: &MailInput,runtime:&crate::config::RuntimeConfig) -> SendResult {
    let message = match message(config, input) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let smtp = match transport(config,runtime).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    match tokio::time::timeout(TIMEOUT, smtp.send(message)).await {
        Ok(Ok(_)) => SendResult::success(),
        Ok(Err(e)) => failed(e),
        Err(_) => SendResult::failure("SMTP 서버 응답 시간이 초과되었습니다.", true),
    }
}
pub async fn verify_config(config: &MailerConfig,runtime:&crate::config::RuntimeConfig) -> SendResult {
    let smtp = match transport(config,runtime).await {
        Ok(t) => t,
        Err(e) => return e,
    };
    match tokio::time::timeout(TIMEOUT, smtp.test_connection()).await {
        Ok(Ok(true)) => SendResult::success(),
        Ok(Ok(false)) => SendResult::failure("SMTP 연결을 확인할 수 없습니다.", true),
        Ok(Err(e)) => failed(e),
        Err(_) => SendResult::failure("SMTP 서버 응답 시간이 초과되었습니다.", true),
    }
}

#[cfg(test)]
mod outbound_policy_tests {
    use super::*;
    #[test]
    fn unsafe_hosts_are_permanent_but_dns_outages_remain_retryable() {
        for (kind,retryable) in [(std::io::ErrorKind::PermissionDenied,false),(std::io::ErrorKind::TimedOut,true),(std::io::ErrorKind::NotFound,true)] {
            assert_eq!(resolution_failure(std::io::Error::new(kind,"fixture")).retryable,retryable);
        }
    }
}
