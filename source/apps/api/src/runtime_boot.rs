//! Runner boot cores and API readiness. Boot publication belongs to the publisher.
use crate::state::AppState;
use anyhow::{Context, Result, ensure};

/// Explicit runner input: no AppState construction or background workers.
pub struct BootCtx<'a> {
    pub config: &'a crate::config::RuntimeConfig,
    pub secrets: &'a crate::crypto::SecretBox,
    pub pool: &'a deadpool_postgres::Pool,
}

fn step_error(config: &crate::config::RuntimeConfig, code: &'static str, step: &str, error: impl std::fmt::Display) -> crate::upgrade::StepError {
    let mut detail = format!("{step}: {error:#}");
    for secret in std::iter::once(&config.database_url).chain(config.auth_secrets.iter()) {
        if !secret.is_empty() { detail = detail.replace(secret, "[redacted]"); }
    }
    let detail = detail.chars().map(|c| if c.is_control() {' '} else {c}).take(512).collect();
    crate::upgrade::StepError {code, message: "API 시작 사전 검사를 완료하지 못했습니다.", detail: Some(detail)}
}
fn original_error(error: crate::upgrade::StepError) -> anyhow::Error {
    let detail = error.detail.unwrap_or_else(||error.message.to_owned());
    anyhow::anyhow!(detail.split_once(": ").map_or(detail.as_str(),|(_,original)|original).to_owned())
}

/// P0b needs no database connection, AppState, or fresh secrets.
/// Callers apply config.rs's port parsing/nonzero rules before this collision check.
pub fn preflight_ports(api_port: u16, web_port: u16, db_port: u16) -> Result<(),crate::upgrade::StepError> {
    if api_port == db_port || api_port == web_port
        || crate::control::reserved_ports::NGINX_BASE_LISTEN_PORTS.contains(&api_port) {
        return Err(crate::upgrade::StepError {
            code: "api_port_conflict", message: "다른 내부 수신 포트와 겹치지 않는 API_PORT를 선택하세요.",
            detail: Some(format!("API_PORT={api_port} conflicts with an internal web, database, or nginx listener; choose a different API_PORT")),
        });
    }
    Ok(())
}

pub fn preflight_api_port_env(config: &crate::config::RuntimeConfig) -> Result<(),crate::upgrade::StepError> {
    let db_port = url::Url::parse(&config.database_url).map_err(|error|crate::upgrade::StepError {
        code: "api_port_conflict", message: "DATABASE_URL의 형식을 확인하세요.", detail: Some(error.to_string()),
    })?.port().unwrap_or(5432);
    preflight_ports(config.listen.port(),config.web_port,db_port)
}

/// Disabled saved listeners count too: enabling one later must not take the API port.
pub async fn preflight_api_port_rows(ctx: &BootCtx<'_>) -> Result<(), crate::upgrade::StepError> {
    let port = ctx.config.listen.port();
    async {
    let db = ctx.pool.get().await?;
    let mut conflicts = Vec::new();
    for row in db
        .query("SELECT id,listen,config,server_name,enabled FROM nginx_servers", &[])
        .await?
    {
        let id: String = row.get(0);
        let listen: i32 = row.get(1);
        let config: Option<serde_json::Value> = row.get(2);
        let name: String = row.get(3);
        let state = if row.get::<_, bool>(4) { "" } else { ", disabled" };
        let tls = config
            .as_ref()
            .and_then(|c| c.get("tlsPort"))
            .and_then(serde_json::Value::as_u64);
        if listen == i32::from(port) || tls == Some(u64::from(port)) {
            conflicts.push(format!("reverse proxy {name} ({id}{state})"));
        }
    }
    for row in db
        .query(
            "SELECT id,proxy_pass,enabled FROM nginx_streams WHERE protocol='tcp' AND listen=$1",
            &[&i32::from(port)],
        )
        .await?
    {
        let state = if row.get::<_, bool>(2) { "" } else { ", disabled" };
        conflicts.push(format!(
            "TCP stream -> {} ({}{state})",
            row.get::<_, String>(1),
            row.get::<_, String>(0)
        ));
    }
    ensure!(
        conflicts.is_empty(),
        "API_PORT={port} conflicts with saved configuration: {}. Disabled rules count too, because enabling one later would take the API's port. Choose a different API_PORT or move these listeners before upgrading; no saved rule was changed.",
        conflicts.join(", ")
    );
    Ok(())
    }.await.map_err(|error:anyhow::Error|step_error(ctx.config,"api_port_conflict","preflight_api_port_rows",error))
}

pub async fn preflight_api_port(state: &AppState) -> Result<()> {
    preflight_api_port_env(&state.config).map_err(original_error)?;
    preflight_api_port_rows(&BootCtx::from(state)).await.map_err(original_error)
}
impl<'a> From<&'a AppState> for BootCtx<'a> {
    fn from(state: &'a AppState) -> Self { Self {config: &state.config, secrets: &state.secrets, pool: &state.db} }
}

pub async fn run_migrations(ctx: &BootCtx<'_>) -> Result<usize,crate::upgrade::StepError> {
    crate::migrations::run(ctx.pool,&ctx.config.migrations_dir).await
        .map_err(|error| {
            if error.downcast_ref::<crate::migrations::NewerDatabase>().is_some() {
                let mut failure=step_error(ctx.config,"database_newer","run_migrations",error);
                failure.message="이 DB는 더 새 버전의 이미지가 사용했습니다. 이 이미지로는 시작하지 않습니다.";
                failure
            } else {
                step_error(ctx.config,"migration_failed","run_migrations",error)
            }
        })
}
pub async fn ensure_instance(ctx: &BootCtx<'_>) -> Result<String,crate::upgrade::StepError> {
    crate::nodes::service::ensure_instance_id_with_pool(ctx.pool).await
        .map_err(|error|step_error(ctx.config,"boot_step_failed","ensure_instance",error))
}
pub async fn seed_admin(ctx: &BootCtx<'_>, email: Option<&str>, password: Option<&str>) -> Result<crate::auth::setup::SeedOutcome,crate::upgrade::StepError> {
    let passwords = crate::auth::password::PasswordHasher::new(ctx.config.password_concurrency);
    crate::auth::setup::seed_admin_with(ctx.pool,&passwords,email,password).await
        .map_err(|error|step_error(ctx.config,"boot_step_failed","seed_admin",error))
}
pub async fn dns_preflight(ctx: &BootCtx<'_>) -> Result<crate::dns_guard::Descriptor,crate::upgrade::StepError> {
    crate::dns_guard::preflight::prepare_with(ctx.config,ctx.pool).await
        .map_err(|error|step_error(ctx.config,"boot_step_failed","dns_preflight",error))
}
pub async fn backfill(ctx: &BootCtx<'_>, requested: Option<&str>) -> Result<Option<crate::runtime_backfill::BackfillResult>,crate::upgrade::StepError> {
    Ok(crate::runtime_backfill::run_with(ctx.pool,ctx.secrets,requested).await)
}

/// Native s6 readiness notification, enabled only by the API service wrapper.
pub fn notify_ready() -> Result<()> {
    if std::env::var("NWUI_READY_FD").ok().as_deref() != Some("3") {
        return Ok(());
    }
    loop {
        // Reopening a root-owned pipe rechecks permissions after the API uid drop.
        let written=unsafe{libc::write(3,b"\n".as_ptr().cast(),1)};
        if written==1 {return Ok(());}
        let error=if written<0 {std::io::Error::last_os_error()}else{std::io::Error::from(std::io::ErrorKind::WriteZero)};
        if error.kind()==std::io::ErrorKind::Interrupted {continue;}
        return Err(error).context("Notify s6 readiness");
    }
}

#[cfg(test)]
mod boot_step_tests {
    use super::*;
    #[test]
    fn runner_error_details_are_bounded_sanitized_and_do_not_expose_startup_secrets() {
        let config = crate::config::RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://fixture:private-password@localhost/test".into()),
            "AUTH_SECRET"=>Some("private-auth-fixture".into()),_=>None,
        }).unwrap();
        let raw = format!("{} {}\n\0{}",config.database_url,config.auth_secrets[0],"가".repeat(700));
        let error=step_error(&config,"boot_step_failed","dns_preflight",anyhow::anyhow!(raw));
        let detail=error.detail.unwrap();
        assert!(detail.starts_with("dns_preflight: "));
        assert_eq!(detail.chars().count(),512);
        assert!(!detail.contains("private-password"));assert!(!detail.contains("private-auth-fixture"));
        assert!(!detail.chars().any(char::is_control));
    }
}

#[cfg(test)]
mod port_preflight_tests {
    use super::*;
    #[test]
    fn parsed_ports_preserve_every_internal_listener_reservation() {
        preflight_ports(3001,3000,5432).unwrap();
        preflight_ports(3101,3100,5544).unwrap();
        for port in [80,443,8080,3000,5432] {
            let error=preflight_ports(port,3000,5432).unwrap_err();
            assert_eq!(error.code,"api_port_conflict");
            assert_eq!(error.detail.unwrap(),format!("API_PORT={port} conflicts with an internal web, database, or nginx listener; choose a different API_PORT"));
        }
        assert!(preflight_ports(5544,3100,5544).is_err());
    }
    #[test]
    fn config_wrapper_delegates_and_refuses_a_malformed_database_url() {
        let mut config=crate::config::RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://test@localhost/test".into()),
            "AUTH_SECRET"=>Some("fixture-only".into()),_=>None,
        }).unwrap();
        preflight_api_port_env(&config).unwrap();
        config.listen.set_port(5432);
        assert_eq!(preflight_api_port_env(&config).unwrap_err().code,"api_port_conflict");
        config.listen.set_port(8080);
        let wrapped=preflight_api_port_env(&config).unwrap_err();
        let direct=preflight_ports(8080,config.web_port,5432).unwrap_err();
        assert_eq!(wrapped.code,direct.code);assert_eq!(wrapped.detail,direct.detail);
        config.listen.set_port(5544);config.database_url="postgres://test@localhost:5544/test".into();
        assert_eq!(preflight_api_port_env(&config).unwrap_err().code,"api_port_conflict");
        config.database_url="relative-not-a-url".into();
        assert_eq!(preflight_api_port_env(&config).unwrap_err().code,"api_port_conflict");
    }
}

#[cfg(all(test,target_os="linux"))]
mod readiness_tests {
    use super::*;
    use std::{fs::File,io::Read,os::{fd::{AsRawFd,FromRawFd},unix::process::CommandExt},process::{Command,Stdio}};

    #[test]
    fn inherited_readiness_child() {
        if std::env::var("NWUI_READY_TEST_CHILD").as_deref()!=Ok("1") {return;}
        assert_eq!(unsafe{libc::geteuid()},10001);
        assert!(std::fs::OpenOptions::new().write(true).open("/proc/self/fd/3").is_err(),"reopening the root-owned pipe must be denied");
        notify_ready().unwrap();
        assert!(unsafe{libc::fcntl(3,libc::F_GETFD)}>=0,"notification must not close the inherited descriptor");
    }
    #[test]
    fn readiness_writes_the_root_owned_inherited_pipe_after_uid_drop() {
        assert_eq!(unsafe{libc::geteuid()},0,"run in the root API test container");
        let mut fds=[-1;2];assert_eq!(unsafe{libc::pipe2(fds.as_mut_ptr(),libc::O_CLOEXEC)},0);
        let mut read=unsafe{File::from_raw_fd(fds[0])};let write=unsafe{File::from_raw_fd(fds[1])};let fd=write.as_raw_fd();
        let mut command=Command::new(std::env::current_exe().unwrap());command.args(["--exact","runtime_boot::readiness_tests::inherited_readiness_child","--nocapture"])
            .env_clear().env("NWUI_READY_TEST_CHILD","1").env("NWUI_READY_FD","3").stdout(Stdio::piped()).stderr(Stdio::piped());
        unsafe {command.pre_exec(move|| {
            if libc::dup2(fd,3)<0 || libc::fcntl(3,libc::F_SETFD,0)<0 {return Err(std::io::Error::last_os_error());}
            if fd!=3 {libc::close(fd);}
            if libc::setgroups(0,std::ptr::null())!=0 || libc::setresgid(10001,10001,10001)!=0 || libc::setresuid(10001,10001,10001)!=0 {return Err(std::io::Error::last_os_error());}Ok(())
        });}
        let child=command.spawn().unwrap();drop(write);let output=child.wait_with_output().unwrap();
        assert!(output.status.success(),"{} {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        let mut bytes=Vec::new();read.read_to_end(&mut bytes).unwrap();assert_eq!(bytes,b"\n");
    }
}
