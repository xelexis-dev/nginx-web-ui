//! Opt-in, task-owned mounted-volume proof using the approved PG15+17 tool image.
pub use nwui_api::{upgrade,nonroot,certificates};
#[path="../src/crypto.rs"] mod crypto;
#[path="../src/db_upgrade/mod.rs"] mod db_upgrade;
use db_upgrade::{postgres,inspect,engine::Engine,arguments::Arguments,files};
use std::{fs,path::Path,time::Duration,os::unix::fs::{PermissionsExt,MetadataExt}};
fn enabled()->bool{std::env::var("NWUI_TEST_PGUPGRADE_NATIVE").as_deref()==Ok("1")}
fn private(path:&Path,value:&[u8],mode:u32){fs::write(path,value).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(mode)).unwrap();}
fn work()->std::path::PathBuf{let path=std::env::temp_dir().join(format!("nwui-pg-{}",uuid::Uuid::new_v4()));postgres::directory(&path).unwrap();path}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires four disposable task-owned mounts and the approved native PG15/17 tool image"]
async fn prepare_private_pg15_fixture() {
    if !enabled(){return;}
    let db=Path::new("/source/db");let app=Path::new("/source/app");
    assert!(fs::read_dir(db).unwrap().next().is_none());assert!(fs::read_dir(app).unwrap().next().is_none());
    postgres::own(db).unwrap();let work=work();
    let mut init=postgres::command(15,"initdb").unwrap();init.arg("-D").arg(db).arg(format!("--locale={}",std::env::var("NWUI_TEST_PG_LOCALE").unwrap_or_else(|_|"C".into()))).args(["--encoding=UTF8","--auth-local=trust","--auth-host=scram-sha-256"]);
    postgres::capture(init,&work,Duration::from_secs(300)).unwrap();
    for name in ["secrets","tls"]{files::private_dir(&app.join(name)).unwrap();}
    let password=uuid::Uuid::new_v4().simple().to_string();
    private(&app.join("secrets/auth_secret"),uuid::Uuid::new_v4().simple().to_string().as_bytes(),0o644);
    private(&app.join("secrets/db_password"),password.as_bytes(),0o644);
    let key=crypto::SecretBox::load_or_create(&app.join("secrets/data_key")).unwrap();
    let sealed=key.seal("synthetic-roundtrip").unwrap();
    let tls=std::process::Command::new("/usr/bin/openssl").args(["req","-x509","-newkey","rsa:2048","-nodes","-days","1","-subj","/CN=unconfigured.invalid","-keyout","/source/app/tls/default.key","-out","/source/app/tls/default.crt"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();assert!(tls.success());
    fs::set_permissions(app.join("tls/default.key"),fs::Permissions::from_mode(0o600)).unwrap();
    let start=work.join("server");postgres::directory(&start).unwrap();let pg=postgres::Private::start(15,db,&start).unwrap();
    assert!(pg.socket.starts_with("/run")&&!pg.socket.starts_with(&start));
    let socket_metadata=fs::metadata(&pg.socket).unwrap();assert_eq!(socket_metadata.mode()&0o7777,0o700);assert_eq!(socket_metadata.uid(),fs::metadata(db).unwrap().uid());
    let root=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
    root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
    let quoted:String=root.client.query_one("SELECT quote_literal($1::text)",&[&password]).await.unwrap().get(0);
    root.client.batch_execute(&format!("CREATE ROLE nginx LOGIN PASSWORD {quoted}")).await.unwrap();
    let application=if std::env::var("NWUI_TEST_PG_LEGACY").as_deref()==Ok("1"){"nginx_control"}else{"nginx_web_ui"};
    root.client.batch_execute(&format!("CREATE DATABASE {application} OWNER nginx")).await.unwrap();
    drop(root);
    let connection=inspect::connect(&pg.socket,application,"nginx",Some(&password)).await.unwrap();
    connection.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
    connection.client.batch_execute("SET search_path=public").await.unwrap();
    connection.client.batch_execute("CREATE SCHEMA drizzle AUTHORIZATION nginx; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();
    let manifest:serde_json::Value=serde_json::from_str(include_str!("../src/db_upgrade/legacy-1.19.11.json")).unwrap();
    let sql_root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");
    let ledger_case=std::env::var("NWUI_TEST_LEDGER_CASE").unwrap_or_else(|_|"exact".into());
    for entry in manifest["signatures"].as_array().unwrap().iter().take(if ledger_case=="older"{11}else{28}) {
        let sql=fs::read(sql_root.join(format!("{}.sql",entry["tag"].as_str().unwrap()))).unwrap();
        use sha2::{Digest,Sha256};assert_eq!(hex::encode(Sha256::digest(&sql)),entry["hash"].as_str().unwrap());
        connection.client.batch_execute(std::str::from_utf8(&sql).unwrap()).await.unwrap();
        connection.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[&entry["hash"].as_str().unwrap(),&entry["createdAt"].as_i64().unwrap()]).await.unwrap();
    }
    match ledger_case.as_str() {
        "exact"|"older"=>{},
        "duplicate"=>{connection.client.batch_execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) SELECT hash,created_at FROM drizzle.__drizzle_migrations").await.unwrap();},
        "missing"=>{connection.client.batch_execute("DELETE FROM drizzle.__drizzle_migrations WHERE created_at=(SELECT max(created_at) FROM drizzle.__drizzle_migrations)").await.unwrap();},
        "unknown"=>{connection.client.batch_execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES('unknown',1)").await.unwrap();},
        _=>panic!("unknown test ledger case"),
    }
    connection.client.execute("INSERT INTO public.users(id,email,password_hash,role,totp_secret) VALUES('native-user','native@example.test','unused','admin',$1)",&[&sealed]).await.unwrap();
    connection.client.batch_execute("INSERT INTO public.app_settings(id) VALUES('native-fixture')").await.unwrap();drop(connection);
    if std::env::var("NWUI_TEST_TEMP_NAMESPACES").as_deref()==Ok("1") {
        for database in [application,"postgres"] {
            let other=inspect::connect(&pg.socket,database,"postgres",None).await.unwrap();
            other.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
            other.client.batch_execute("CREATE TEMP TABLE review_temporary(value text); DROP TABLE review_temporary").await.unwrap();
            let count:i64=other.client.query_one("SELECT count(*) FROM pg_namespace WHERE nspname ~ '^pg_(toast_)?temp_[0-9]+$'",&[]).await.unwrap().get(0);assert!(count>=2);
        }
    }
    let socket=pg.socket.clone();pg.stop().unwrap();assert!(!socket.exists(),"owned runtime socket directory leaked after stop");
    postgres::read_control(15,db).unwrap().require_cold().unwrap();
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires four disposable task-owned mounts and the approved native PG15/17 tool image"]
async fn convert_private_pg15_and_keep_diagnostic_state_unchanged() {
    if !enabled(){return;}
    let engine=Engine::fixed().unwrap();
    let arguments=|verb:&str|Arguments::parse([verb,"--backup-id","native-proof","--destination-db-volume","native-db","--destination-app-volume","native-app","--json"].map(String::from)).unwrap();
    let original_db=files::snapshot(&engine.source_db).unwrap();let original_app=files::snapshot(&engine.source_app).unwrap();
    let checked=engine.execute(&arguments("check")).await;
    assert_eq!(original_db,files::snapshot(&engine.source_db).unwrap());assert_eq!(original_app,files::snapshot(&engine.source_app).unwrap());
    let checked=checked.unwrap();assert!(!checked.state.ready);assert_eq!(checked.state.phase,db_upgrade::state::Phase::Checked);assert_work_sockets_removed(&engine);
    let ready=engine.execute(&arguments("run")).await.unwrap();assert!(ready.state.ready);assert_work_sockets_removed(&engine);
    assert!(!engine.roots.db.join(".nwui-work").join(ready.state.attempt_id.as_deref().unwrap()).join("pg15").exists(),"ready must remove only its PG15 working copy");
    assert_eq!(original_db,files::snapshot(&engine.source_db).unwrap());assert_eq!(original_app,files::snapshot(&engine.source_app).unwrap());
    let state_path=engine.roots.db.join(".nwui/migration-v1.json");let saved=fs::read(&state_path).unwrap();
    let verify=Arguments::parse(["verify".into(),"--json".into()]).unwrap();
    assert!(engine.execute(&verify).await.unwrap().notice.unwrap().contains("working copy removed"));assert_eq!(saved,fs::read(&state_path).unwrap());
    let copy=engine.roots.db.join(".nwui-work").join(ready.state.attempt_id.as_deref().unwrap()).join("pg15");
    files::copy_tree(&engine.source_db,&copy).unwrap();
    let status=Arguments::parse(["status".into(),"--json".into()]).unwrap();
    let before_status=files::snapshot(&engine.roots.db).unwrap();
    assert!(engine.execute(&status).await.unwrap().notice.unwrap().contains("working_copy_present"));assert_eq!(before_status,files::snapshot(&engine.roots.db).unwrap());
    assert!(engine.execute(&arguments("run")).await.unwrap().state.ready);assert!(!copy.exists());assert_eq!(saved,fs::read(&state_path).unwrap());
    assert!(engine.execute(&status).await.unwrap().notice.unwrap().contains("working copy removed"));
    std::os::unix::fs::symlink(&engine.source_db,&copy).unwrap();
    assert_eq!(engine.execute(&arguments("run")).await.unwrap_err().code,"unsupported_layout");assert_eq!(saved,fs::read(&state_path).unwrap());
    assert_eq!(original_db,files::snapshot(&engine.source_db).unwrap());fs::remove_file(&copy).unwrap();
    let start=work();let pg=postgres::Private::start(17,&engine.roots.db.join("17/data"),&start).unwrap();
    let connection=inspect::connect(&pg.socket,"nginx_web_ui","postgres",None).await.unwrap();
    let ciphertext:String=connection.client.query_one("SELECT totp_secret FROM public.users WHERE id='native-user'",&[]).await.unwrap().get(0);
    let key=crypto::SecretBox::load_existing(&engine.roots.app.join("secrets/data_key")).unwrap();assert_eq!(key.open(Some(&ciphertext)).as_deref(),Some("synthetic-roundtrip"));
    connection.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
    connection.client.batch_execute("INSERT INTO public.app_settings(id) VALUES('after-cutover')").await.unwrap();drop(connection);pg.stop().unwrap();
    assert!(engine.execute(&arguments("run")).await.unwrap().state.ready);
    let changed=engine.execute(&verify).await.unwrap();assert!(changed.state.ready);let notice=changed.notice.unwrap();assert!(notice.contains("changed_since_conversion")&&notice.contains("working copy removed"));assert_eq!(saved,fs::read(state_path).unwrap());
    assert_eq!(original_db,files::snapshot(&engine.source_db).unwrap());assert_eq!(original_app,files::snapshot(&engine.source_app).unwrap());
    assert_work_sockets_removed(&engine);assert!(!copy.exists(),"verify must not recreate the removed PG15 copy");
    for entry in fs::read_dir("/run").unwrap(){let entry=entry.unwrap();assert!(!(entry.file_name().to_string_lossy().starts_with("nwui-pg-")&&entry.file_type().unwrap().is_dir()),"owned runtime socket directory leaked");}
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires disposable native PG15 source/target mounts; prepare with missing or unknown ledger"]
async fn a_nonrelease_ledger_refuses_before_conversion_without_source_changes() {
    if !enabled(){return;}
    let engine=Engine::fixed().unwrap();let before=(files::snapshot(&engine.source_db).unwrap(),files::snapshot(&engine.source_app).unwrap());
    let args=Arguments::parse(["check","--backup-id","ledger-proof","--destination-db-volume","native-db","--destination-app-volume","native-app"].map(String::from)).unwrap();
    let error=engine.execute(&args).await.err().expect("a non-1.19.11 source ledger must refuse");
    assert_eq!(error.exit,2);assert!(error.message.contains("1.19.11"));
    let status=engine.execute(&Arguments::parse(["status".into()]).unwrap()).await.unwrap().state;
    assert!(!status.ready);assert!(!engine.roots.db.join("17/data").exists());
    assert!(!engine.roots.app.join(".nwui/pair-v1.json").exists());
    assert_eq!(before,(files::snapshot(&engine.source_db).unwrap(),files::snapshot(&engine.source_app).unwrap()));
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires four disposable task-owned mounts and the approved native PG15/17 tool image"]
async fn converted_diagnostic_never_promotes_or_changes_the_journal() {
    if !enabled(){return;}
    let engine=Engine::fixed().unwrap();let before=(files::snapshot(&engine.source_db).unwrap(),files::snapshot(&engine.source_app).unwrap());
    let args=Arguments::parse(["check","--backup-id","converted-proof","--destination-db-volume","native-db","--destination-app-volume","native-app"].map(String::from)).unwrap();
    let checked=engine.execute(&args).await.unwrap().state;
    let converted=db_upgrade::migration::stop_after_conversion_for_test(&engine,&args,checked).unwrap();assert!(!converted.ready);
    let path=engine.roots.db.join(".nwui/migration-v1.json");let journal=fs::read(&path).unwrap();
    let outcome=engine.execute(&Arguments::parse(["verify".into(),"--json".into()]).unwrap()).await.unwrap();
    assert_eq!(outcome.state.phase,db_upgrade::state::Phase::Converted);assert!(!outcome.state.ready);assert_eq!(journal,fs::read(&path).unwrap());
    assert!(!engine.roots.db.join(".nwui/pair-v1.json").exists());assert!(!engine.roots.app.join(".nwui/pair-v1.json").exists());
    let copy=engine.roots.db.join(".nwui-work").join(converted.attempt_id.as_deref().unwrap()).join("pg15");assert!(copy.is_dir());
    let run=Arguments::parse(["run","--backup-id","converted-proof","--destination-db-volume","native-db","--destination-app-volume","native-app"].map(String::from)).unwrap();
    assert_eq!(engine.execute(&run).await.unwrap_err().code,"migration_incomplete");
    assert!(copy.is_dir());assert_eq!(journal,fs::read(path).unwrap());assert_eq!(before,(files::snapshot(&engine.source_db).unwrap(),files::snapshot(&engine.source_app).unwrap()));
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="native PG17 and readiness double: NWUI_TEST_SOCKET_CLEANUP=1"]
async fn private_sockets_are_reclaimed_after_success_failures_timeout_and_sigterm(){
    if std::env::var("NWUI_TEST_SOCKET_CLEANUP").as_deref()!=Ok("1"){return;}
    use std::sync::{Arc,Mutex,atomic::Ordering};
    struct Reset;impl Drop for Reset{fn drop(&mut self){postgres::SIGNAL.store(0,Ordering::Relaxed);files::CANCELLED.store(false,Ordering::Relaxed);}}
    let _reset=Reset;
    let root=work();let data=root.join("data");postgres::directory(&data).unwrap();let init=work();
    let mut command=postgres::command(17,"initdb").unwrap();command.arg("-D").arg(&data).args(["--locale=C","--encoding=UTF8","--auth-local=trust","--auth-host=scram-sha-256"]);postgres::capture(command,&init,Duration::from_secs(60)).unwrap();
    let log=work();let pg=postgres::Private::start(17,&data,&log).unwrap();let socket=pg.socket.clone();pg.stop().unwrap();assert!(!socket.exists());
    for failure in ["config","spawn","startup"] {
        let log=work();let socket=postgres::socket_path(&log,"s17");
        if failure=="config"{fs::write(log.join("private.conf"),"already exists").unwrap();}
        if failure=="startup"{fs::write(data.join("PG_VERSION"),"16\n").unwrap();}
        let runuser=Path::new("/usr/sbin/runuser");let hidden=runuser.with_file_name(format!("runuser-d11-{}",uuid::Uuid::new_v4()));
        if failure=="spawn"{fs::rename(runuser,&hidden).unwrap();}
        let result=postgres::Private::start(17,&data,&log);
        if failure=="spawn"{fs::rename(&hidden,runuser).unwrap();}
        if failure=="startup"{fs::write(data.join("PG_VERSION"),"17\n").unwrap();}
        assert!(result.is_err(),"{failure}");assert!(!socket.exists(),"leaked after {failure}");assert!(!data.join("postmaster.pid").exists());
    }
    let log=work();let socket=postgres::socket_path(&log,"s17");postgres::private_file(&socket.with_extension("probe-mode"),b"always-no-response").unwrap();postgres::private_file(&socket.with_extension("probe-count"),b"0").unwrap();
    let observed=data.join("postmaster.pid");let watch=std::thread::spawn(move||{let end=std::time::Instant::now()+Duration::from_secs(5);while std::time::Instant::now()<end{if let Ok(value)=fs::read_to_string(&observed){if let Some(pid)=value.lines().next().and_then(|s|s.parse::<i32>().ok()){return Some(pid);}}std::thread::sleep(Duration::from_millis(2));}None});
    assert!(postgres::Private::start_with_limits(17,&data,&log,Duration::from_secs(1),Duration::from_secs(2)).is_err());let pid=watch.join().unwrap().expect("real postmaster never started");assert_eq!(unsafe{libc::kill(pid,0)},-1);assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));assert!(!socket.exists());assert!(!data.join("postmaster.pid").exists());
    for suffix in ["probe-mode","probe-count"]{fs::remove_file(socket.with_extension(suffix)).unwrap();}
    let ready=Arc::new(Mutex::new(None));let controller=ready.clone();
    let signal=std::thread::spawn(move||{let end=std::time::Instant::now()+Duration::from_secs(10);let mut started=false;while std::time::Instant::now()<end{if controller.lock().unwrap().is_some(){started=true;break;}std::thread::sleep(Duration::from_millis(5));}assert_eq!(unsafe{libc::kill(std::process::id()as i32,libc::SIGTERM)},0);started});
    let result=db_upgrade::cli::cancellable(async {
        let log=work();let pg=postgres::Private::start(17,&data,&log)?;let pid=fs::read_to_string(data.join("postmaster.pid")).unwrap().lines().next().unwrap().parse::<i32>().unwrap();*ready.lock().unwrap()=Some((pg.socket.clone(),pid));
        let held=pg;let result=std::future::pending::<db_upgrade::error::Result<()>>().await;drop(held);result
    }).await;
    assert!(signal.join().unwrap());assert_eq!(result.unwrap_err().exit,143);
    let (socket,pid)=ready.lock().unwrap().clone().unwrap();assert!(!socket.exists());assert!(!data.join("postmaster.pid").exists());assert_eq!(unsafe{libc::kill(pid,0)},-1);assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
}

fn assert_work_sockets_removed(engine:&Engine){
    for attempt in fs::read_dir(engine.roots.db.join(".nwui-work")).unwrap(){for log in fs::read_dir(attempt.unwrap().path()).unwrap(){let path=log.unwrap().path();if path.is_dir(){for label in ["s15","s17","upgrade"]{let socket=postgres::socket_path(&path,label);assert!(socket.starts_with("/run")&&!socket.starts_with(&path));assert!(!socket.exists(),"runtime socket leaked for {}",path.display());}}}}
}
