use nwui_api::{config::RuntimeConfig, server, state::AppState};

fn main() -> anyhow::Result<std::process::ExitCode> {
    if std::env::args().nth(1).as_deref()==Some("fdholder-passback") {
        let args=std::env::args().skip(2).collect::<Vec<_>>();
        return Ok(nwui_api::upgrade::fdholder_passback(&args));
    }
    // Docker HEALTHCHECK: decided before any configuration is read, because the runtime
    // secrets that configuration requires are not in the healthcheck's environment.
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(nwui_api::healthcheck::run());
    }
    if std::env::args().nth(1).as_deref() == Some("api-pre-exec") {
        nwui_api::nonroot::pre_exec::run()?;
        return Ok(std::process::ExitCode::SUCCESS);
    }
    run().map(|()|std::process::ExitCode::SUCCESS)
}

fn publisher_result(result:Result<(),nwui_api::upgrade::StepError>)->anyhow::Result<()> {
    if let Err(error)=result {
        eprintln!("[publisher] exiting: {}",error.code);
        // Runtime Drop waits for blocking writers; a fenced publisher must kill the whole process.
        std::process::exit(1);
    }
    Ok(())
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("access-probe") => return nwui_api::nonroot::access_probe::run().await.map_err(Into::into),
        Some("auto-status") => std::process::exit(nwui_api::upgrade::auto_status(&args[1..]).await),
        Some("upgrade-runner") => return nwui_api::upgrade::run().await.map_err(Into::into),
        Some("publisher") => return publisher_result(nwui_api::publisher::run().await),
        Some("gate-outcome") => {
            let selection = nwui_api::upgrade::read_selection()?;
            println!("{}", serde_json::to_value(selection.outcome)?.as_str().unwrap());
            return Ok(());
        },
        Some("db-upgrade") => {
            std::process::exit(nwui_api::db_upgrade::cli::run(args.into_iter().skip(1).collect()).await);
        },
        Some("storage" | "storage-secrets" | "storage-provision") => {
            std::process::exit(nwui_api::db_upgrade::storage::run(&args).await);
        },
        _ => {},
    }
    if std::env::args().nth(1).as_deref() == Some("dns-guard") {
        return nwui_api::dns_guard::run().await;
    }
    let verify_boot = boot_verification_args(&args)?;
    let config = RuntimeConfig::from_env()?;
    if verify_boot {
        nwui_api::db_upgrade::paths::validate_env()?;
        nwui_api::nonroot::selection::verify_boot(&config).await?;
        eprintln!("[api] boot verification complete");
        return Ok(());
    }
    // s6 restarts nginx without re-running boot: nginx/run calls this before every start
    // so managed files that no longer pass `nginx -t` cannot keep the admin UI down.
    if std::env::args().nth(1).as_deref() == Some("guard") {
        guard_result(nwui_api::nonroot::admit_guard(&config).await,true)?;
        let store = guard_result(nwui_api::publisher::PublisherStore::open(&config),true)?;
        let report = guard_result(nwui_api::publisher::boot::guard(&config, &store).await,false)?;
        if !report.removed_upstreams.is_empty() {
            eprintln!("[nginx-guard] removed {} upstream blocks that reach this container's internal ports: {} (upstreams.conf keeps the others; the original is in the recovery directory)",
                report.removed_upstreams.len(), report.removed_upstreams.join(", "));
        }
        if !report.rejected_files.is_empty() || !report.rejected_upstreams.is_empty() {
            eprintln!("[nginx-guard] nginx rejected managed files [{}] and upstream blocks [{}]", report.rejected_files.join(", "), report.rejected_upstreams.join(", "));
        }
        if report.reserved_files + report.internal_target_files + report.invalid_files > 0 || !report.removed_upstreams.is_empty() || !report.rejected_files.is_empty() || !report.rejected_upstreams.is_empty() {
            eprintln!("[nginx-guard] quarantined {} conflicting, {} internal-target and {} invalid managed configuration files",
                report.reserved_files, report.internal_target_files, report.invalid_files);
            for path in &report.quarantine_dirs {
                eprintln!("[nginx-guard] recovery directory: {}", path.display());
            }
        }
        return Ok(());
    }
    anyhow::ensure!(args.is_empty() || (args.len()==1 && args[0]=="serve"), "usage: nwui-api [boot [--verify]|serve|guard|dns-guard|healthcheck]");
    nwui_api::db_upgrade::paths::validate_env()?;
    let (selection,secrets)=nwui_api::nonroot::selection::load_for_serve(&config).await?;
    let state=AppState::new_with_secrets(config,secrets)?;
    nwui_api::nonroot::selection::install(&state,selection)?;
    let listener = prepare_service(&state).await?;
    let certificates = nwui_api::certificates::jobs::start(state.clone()).await?;
    let mut worker_failed = certificates.failure_receiver();
    let metrics = nwui_api::observability::start(state.clone());
    let retention = nwui_api::retention::start(state.clone());
    let notifications = nwui_api::notifications::start(state.clone());
    let runtime = state.runtime.clone();
    let shutdown = runtime.clone();
    nwui_api::runtime_boot::notify_ready()?;
    let serving = server::serve(listener, server::router(state), async move {
        tokio::select! {
            _ = server::shutdown_signal() => {},
            _ = async {
                while !*worker_failed.borrow() {
                    if worker_failed.changed().await.is_err() { break; }
                }
            } => { eprintln!("[api] certificate worker stopped unexpectedly"); },
        }
        shutdown.shutdown.send_replace(true);
    }).await;
    runtime.shutdown.send_replace(true);
    metrics.shutdown().await;
    notifications.shutdown().await;
    certificates.shutdown().await?;
    retention.await?;
    serving?;
    Ok(())
}


fn boot_verification_args(args:&[String]) -> anyhow::Result<bool> {
    if args.first().map(String::as_str)!=Some("boot") {return Ok(false);}
    anyhow::ensure!(args.len()==1 || (args.len()==2 && args[1]=="--verify"), "usage: nwui-api boot [--verify]");
    Ok(true)
}

async fn prepare_service(state: &AppState) -> anyhow::Result<tokio::net::TcpListener> {
    server::wait_for_database(state).await?;
    Ok(tokio::net::TcpListener::bind(state.config.listen).await?)
}


fn guard_result<T>(result: Result<T,nwui_api::upgrade::StepError>, fatal: bool) -> anyhow::Result<T> {
    result.map_err(|error| {
        if fatal || error.code!="guard_failed" {
            eprintln!("[nginx-guard] nginx start deferred: {}",error.code);
            std::process::exit(75);
        }
        anyhow::anyhow!("{}: {}",error.code,error.detail.as_deref().unwrap_or(error.message))
    })
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    fn failure(case:&str) -> std::process::Output {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","guard_tests::failure_child","--nocapture"])
            .env_clear().env("NWUI_GUARD_FAILURE_TEST",case).output().unwrap()
    }
    fn deferred(case:&str,code:&str) {
        let output=failure(case);
        assert_eq!(output.status.code(),Some(75),"{output:?}");
        let stderr=String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr,format!("[nginx-guard] nginx start deferred: {code}\n"));
    }
    #[test]
    fn store_open_failure_defers_with_one_content_free_line() { deferred("store","journal_write_failed"); }
    #[test]
    fn ownership_busy_still_defers() { deferred("busy","storage_busy"); }
    #[test]
    fn unreconciled_recovery_and_unknown_fatal_codes_defer() {
        deferred("recovery","publish_failed");deferred("future","future_recovery_failure");
    }
    #[test]
    fn publisher_capacity_refusal_defers_nginx_with_the_exact_code_only_line() { deferred("capacity","publisher_storage_full"); }
    #[test]
    fn managed_file_failure_keeps_its_best_effort_exit() {
        let output=failure("managed");assert_eq!(output.status.code(),Some(1));
        assert!(String::from_utf8(output.stderr).unwrap().contains("guard_failed: fixture-content"));
    }
    #[test]
    fn failure_child() {
        let Ok(case)=std::env::var("NWUI_GUARD_FAILURE_TEST") else{return;};
        let code=match case.as_str(){"store"=>"journal_write_failed","busy"=>"storage_busy","recovery"=>"publish_failed","future"=>"future_recovery_failure","managed"=>"guard_failed","capacity"=>"publisher_storage_full",_=>panic!("unknown fixture")};
        let error=nwui_api::upgrade::StepError{code,message:"fixture-content",detail:Some("fixture-content".into())};
        let result=if case=="store" {
            guard_result::<nwui_api::publisher::PublisherStore>(Err(error),true).map(|_|())
        }else{guard_result::<()>(Err(error),false)};
        if let Err(error)=result{eprintln!("{error:#}");std::process::exit(1);}
    }
}


#[cfg(test)]
mod i2_startup_tests {
    use super::*;
    use std::{path::Path,time::Duration};

    #[test]
    fn boot_is_only_verification_and_rejects_extra_actions() {
        for args in [&["boot"][..],&["boot","--verify"][..]] {
            assert!(boot_verification_args(&args.iter().map(|s|s.to_string()).collect::<Vec<_>>()).unwrap());
        }
        for args in [&["boot","--migrate"][..],&["boot","--verify","extra"][..]] {
            assert!(boot_verification_args(&args.iter().map(|s|s.to_string()).collect::<Vec<_>>()).is_err());
        }
        assert!(!boot_verification_args(&[]).unwrap());
        assert!(!boot_verification_args(&["serve".into()]).unwrap());
    }

    #[tokio::test]
    #[ignore = "requires disposable NWUI_N2_PRIVATE_DATABASE_URL with a Unix socket"]
    async fn service_readiness_does_not_migrate_seed_or_touch_nginx_files() {
        let Ok(raw)=std::env::var("NWUI_N2_PRIVATE_DATABASE_URL") else {
            eprintln!("I2 startup qualification requires NWUI_N2_PRIVATE_DATABASE_URL");return;
        };
        let pg:tokio_postgres::Config=raw.parse().unwrap();
        let Some(tokio_postgres::config::Host::Unix(socket))=pg.get_hosts().first() else{panic!("private Unix socket required");};
        let (admin,connection)=pg.connect(tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);
        let suffix=uuid::Uuid::new_v4().simple().to_string();let database=format!("n2_i2_{suffix}");let role=format!("n2_read_{suffix}");
        admin.batch_execute(&format!("CREATE ROLE {role} LOGIN; ALTER ROLE {role} SET default_transaction_read_only=on")).await.unwrap();
        admin.batch_execute(&format!("CREATE DATABASE {database}")).await.unwrap();
        admin.batch_execute(&format!("GRANT CREATE ON DATABASE {database} TO {role}")).await.unwrap();
        let mut selected=pg.clone();selected.dbname(&database);
        let (inspector,connection)=selected.connect(tokio_postgres::NoTls).await.unwrap();tokio::spawn(connection);
        inspector.batch_execute(&format!("GRANT CREATE ON SCHEMA public TO {role}")).await.unwrap();
        let mut url=url::Url::parse("postgres://localhost").unwrap();url.set_username(&role).unwrap();url.set_path(&database);
        url.query_pairs_mut().append_pair("host",socket.to_str().unwrap()).append_pair("port",&pg.get_ports()[0].to_string());
        let temp=tempfile::tempdir().unwrap();let key=temp.path().join("must-not-create-key");
        let mut config=RuntimeConfig::read(|name|match name {
            "DATABASE_URL"=>Some(url.to_string()),"AUTH_SECRET"=>Some("i2-startup-fixture-only".into()),
            "DATA_KEY_FILE"=>Some(key.display().to_string()),_=>None,
        }).unwrap();
        config.listen.set_port(0);
        config.migrations_dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
        config.nginx_conf_dir=temp.path().join("conf");config.nginx_stream_dir=temp.path().join("stream");config.certs_dir=temp.path().join("certs");
        config.nginx_status_url="http://127.0.0.1:1/status".into();
        let mut canaries=Vec::new();
        for dir in [&config.nginx_conf_dir,&config.nginx_stream_dir,&config.certs_dir] {
            std::fs::create_dir(dir).unwrap();let file=dir.join("canary");std::fs::write(&file,b"UNCHANGED").unwrap();canaries.push(file);
        }
        let state=AppState::new_with_secrets(config,nwui_api::crypto::SecretBox::from_key([23;32])).unwrap();
        let connection=state.db.get().await.unwrap();
        let readonly:String=connection.query_one("SHOW transaction_read_only",&[]).await.unwrap().get(0);assert_eq!(readonly,"on");
        let denied=connection.batch_execute("CREATE TABLE public.forbidden_boot_write(id integer)").await.unwrap_err();
        assert_eq!(denied.code(),Some(&tokio_postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION));drop(connection);
        let result=tokio::time::timeout(Duration::from_secs(10),prepare_service(&state)).await;
        let tables:i64=inspector.query_one("SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema')",&[]).await.unwrap().get(0);
        state.db.close();
        admin.batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)")).await.unwrap();
        admin.batch_execute(&format!("DROP ROLE {role}")).await.unwrap();
        let listener=result.expect("service readiness must not wait for nginx or run boot mutations").expect("read-only empty database must be sufficient for service readiness");
        assert_eq!(listener.local_addr().unwrap().ip(),std::net::IpAddr::from([127,0,0,1]));drop(listener);
        assert_eq!(tables,0,"service startup must not create a migration ledger or application schema");
        assert!(!key.exists(),"receipt-verified secrets must not be reopened or created");
        for file in canaries{assert_eq!(std::fs::read(file).unwrap(),b"UNCHANGED");}
    }
}

#[cfg(test)]
mod publisher_exit_tests {
    use super::*;
    use std::time::{Duration,Instant};
    #[test]
    fn failure_exits_without_waiting_for_a_sleeping_blocking_writer() {
        const CHILD:&str="NWUI_PUBLISHER_EXIT_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let runtime=tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let(tx,rx)=tokio::sync::oneshot::channel();
                tokio::task::spawn_blocking(move||{tx.send(()).unwrap();std::thread::sleep(Duration::from_secs(60));});
                rx.await.unwrap();
                publisher_result(Err(nwui_api::upgrade::StepError{code:"publish_failed",message:"must not log private detail",detail:Some("secret fixture".into())})).unwrap();
            });
            panic!("publisher error returned instead of exiting");
        }
        let start=Instant::now();
        let mut child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","publisher_exit_tests::failure_exits_without_waiting_for_a_sleeping_blocking_writer","--nocapture"]).env(CHILD,"1").stderr(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).spawn().unwrap();
        let status=loop {
            if let Some(status)=child.try_wait().unwrap(){break status;}
            if start.elapsed()>Duration::from_secs(5){child.kill().unwrap();let _=child.wait();panic!("publisher waited for a blocking writer");}
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stderr=String::new();std::io::Read::read_to_string(child.stderr.as_mut().unwrap(),&mut stderr).unwrap();
        assert_eq!(status.code(),Some(1));assert_eq!(stderr.trim(),"[publisher] exiting: publish_failed");
    }
}
