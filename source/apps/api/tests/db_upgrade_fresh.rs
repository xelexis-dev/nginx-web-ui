#![cfg(unix)]
#![allow(dead_code)]
#[path="../src/db_upgrade/files.rs"] mod files;
#[path="../src/db_upgrade/state.rs"] mod state;
#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/pair.rs"] mod pair;
#[path="../src/db_upgrade/mounts.rs"] mod mounts;
#[path="../src/db_upgrade/layout.rs"] mod layout;
#[path="../src/db_upgrade/control.rs"] mod control;
#[path="../src/db_upgrade/settings.rs"] mod settings;
#[path="../src/db_upgrade/shared_folder.rs"] mod shared_folder;
#[path="../src/db_upgrade/postgres.rs"] mod postgres;
#[path="../src/db_upgrade/catalog.rs"] mod catalog;
#[path="../src/db_upgrade/inspect.rs"] mod inspect;
#[path="../src/db_upgrade/cluster.rs"] mod cluster;
#[path="../src/db_upgrade/fresh.rs"] mod fresh;
pub use nwui_api::db_upgrade::{paths,key_file};
pub use nwui_api::{upgrade,crypto,certificates};
use std::{fs,path::{Path,PathBuf}};
use std::os::unix::fs::{PermissionsExt,MetadataExt,symlink};
use layout::Roots;
fn fixture()->(tempfile::TempDir,Roots,PathBuf) {
 let temp=tempfile::tempdir_in("/run").unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o755)).unwrap();
 let roots=Roots{db:temp.path().join("db"),app:temp.path().join("app"),architecture:"arm64".into()};
 for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
 let env=temp.path().join("env");fs::create_dir(&env).unwrap();(temp,roots,env)
}
fn current(r:&Roots)->pair::Pair{let p:pair::Pair=files::read_json(&r.db.join(".nwui/pair-v1.json")).unwrap();assert_eq!(p,files::read_json(&r.app.join(".nwui/pair-v1.json")).unwrap());p}
fn snapshot(r:&Roots)->(String,String){(files::snapshot(&r.db).unwrap(),files::snapshot(&r.app).unwrap())}
#[test]
fn secrets_publish_paired_intent_and_strict_material_without_an_encryption_key() {
 let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();
 let p=current(&r);assert_eq!(p.fresh_step.as_deref(),Some("initdb"));assert!(!p.ready);assert!(!p.encryption_key_required);
 for name in ["auth_secret","db_password"]{let path=r.app.join("secrets").join(name);assert!(!fs::read(&path).unwrap().is_empty());assert_eq!(fs::metadata(path).unwrap().mode()&0o777,0o600);}
 assert!(!r.app.join("secrets/data_key").exists());assert!(!r.db.join("17").exists());assert!(!env.join("ADMIN_PASSWORD").exists());
 assert!(fs::read_to_string(env.join("DATABASE_URL")).unwrap().starts_with("postgres://nginx:"));
 certificates::pem::parse_certificate_pem(&fs::read_to_string(r.app.join("tls/default.crt")).unwrap(),&fs::read_to_string(r.app.join("tls/default.key")).unwrap()).unwrap();
 let cert=fs::read(r.app.join("tls/default.crt")).unwrap();let (_,pem)=x509_parser::pem::parse_x509_pem(&cert).unwrap();let (_,cert)=x509_parser::parse_x509_certificate(&pem.contents).unwrap();assert_eq!(cert.public_key().algorithm.algorithm.to_id_string(),"1.2.840.113549.1.1.1","preserve the shipped RSA catch-all certificate policy");
}
#[test]
fn a_secrets_restart_preserves_values_even_if_operator_inputs_change() {
 let (_t,r,env)=fixture();let inputs=fresh::Inputs{auth_secret:Some("first-auth".into()),admin_password:Some("first-admin".into())};
 fresh::secrets(&r,&inputs,&env).unwrap();let before=snapshot(&r);
 fresh::secrets(&r,&fresh::Inputs{auth_secret:Some("new-auth".into()),admin_password:Some("new-admin".into())},&env).unwrap();assert_eq!(before,snapshot(&r));
 assert_eq!(fs::read_to_string(env.join("AUTH_SECRET")).unwrap(),"first-auth");assert_eq!(fs::read_to_string(env.join("ADMIN_PASSWORD")).unwrap(),"first-admin");
}
#[test]
fn foreign_pg15_or_unpaired_app_is_refused_before_any_secret_write() {
 for kind in ["pg15","app","partial"] {let (_t,r,env)=fixture();match kind {"pg15"=>fs::write(r.db.join("PG_VERSION"),"15").unwrap(),"app"=>fs::write(r.app.join("keep"),"untouched").unwrap(),_=>{layout::prepare(&r,&|_|Ok("123456".into())).unwrap();fs::create_dir_all(r.db.join("17/data")).unwrap();fs::write(r.db.join("17/data/partial"),"keep").unwrap();}}
 let before=snapshot(&r);assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err(),"{kind}");assert_eq!(before,snapshot(&r));assert_eq!(fs::read_dir(&env).unwrap().count(),0);}
}
#[test]
fn malformed_material_and_secret_modes_are_never_repaired() {
 for kind in ["mode","empty","link","tls"] {let (_t,r,env)=fixture();layout::prepare(&r,&|_|Ok("123456".into())).unwrap();files::private_dir(&r.app.join("secrets")).unwrap();let path=r.app.join("secrets/auth_secret");
 match kind {"link"=>symlink("/etc/passwd",&path).unwrap(),"tls"=>{files::private_dir(&r.app.join("tls")).unwrap();fs::write(r.app.join("tls/default.crt"),"partial").unwrap();},_=>{fs::write(&path,if kind=="empty"{""}else{"valid"}).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(if kind=="mode"{0o644}else{0o600})).unwrap();}}
 assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err(),"{kind}");assert!(!r.app.join("secrets/db_password").exists());}
}
#[test]
fn scripts_gate_before_consuming_runner_storage_and_do_not_initialize_or_rotate_secrets() {
 let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker/scripts");
 for name in ["secrets-init.sh","pg-init.sh"] {
  let s=fs::read_to_string(root.join(name)).unwrap();
  assert!(s.starts_with("#!/bin/sh\nset -eu\n. /etc/s6-overlay/scripts/nwui-gate.sh\nnwui_gate_oneshot\n"),"{name}");
  for bad in ["storage-secrets","storage-provision","initdb","rand_hex","CREATE USER","chown -hR postgres"]{assert!(!s.contains(bad),"{name}: {bad}");}
  if name=="pg-init.sh" {
   assert!(s.find("nwui_gate_oneshot").unwrap()<s.find(". /etc/s6-overlay/scripts/pg-env.sh").unwrap());
   assert!(s.contains("cat \"$PGDATA/PG_VERSION\""));assert!(s.contains("storage_selection_invalid"));
  }
 }
}

#[test]
fn scripts_stop_before_other_mutations_when_the_binary_refuses() {
 let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker/scripts");
 for name in ["secrets-init.sh","pg-init.sh"] {
  let t=tempfile::tempdir().unwrap();let bin=t.path().join("bin");fs::create_dir(&bin).unwrap();let api=bin.join("api");
  for (path,text) in [(api.clone(),r#"#!/bin/sh
printf 'binary:%s\n' "$*" >> "$TRACE"
[ "$GATE_OUTCOME" != reader-failed ] || exit 2
printf '%s\n' "$GATE_OUTCOME"
"#),(bin.join("install"),"#!/bin/sh\necho sideeffect >> \"$TRACE\"\n"),(bin.join("chown"),"#!/bin/sh\necho sideeffect >> \"$TRACE\"\n")] {
   fs::write(&path,text).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();
  }
  let data=t.path().join("data");fs::create_dir(&data).unwrap();fs::write(data.join("PG_VERSION"),"17\n").unwrap();
  let env=t.path().join("pg-env");fs::write(&env,format!("echo resolve >> \"$TRACE\"\nPGDATA='{}'\n",data.display())).unwrap();
  let gate=t.path().join("gate");fs::write(&gate,fs::read_to_string(root.join("nwui-gate.sh")).unwrap().replace("/usr/local/bin/nwui-api",&format!("'{}'",api.display()))).unwrap();
  let source=fs::read_to_string(root.join(name)).unwrap().replace("/etc/s6-overlay/scripts/nwui-gate.sh",gate.to_str().unwrap()).replace("/etc/s6-overlay/scripts/pg-env.sh",env.to_str().unwrap());
  let script=t.path().join("run");fs::write(&script,source).unwrap();
  for (outcome,code) in [("pg17",0),("refused-fresh",0),("reader-failed",2)] {
   let trace=t.path().join(format!("trace-{outcome}"));
   let result=std::process::Command::new("sh").arg(&script).env_clear().env("PATH",format!("{}:/usr/bin:/bin",bin.display())).env("TRACE",&trace).env("GATE_OUTCOME",outcome).output().unwrap();
   assert_eq!(result.status.code(),Some(code),"{name}: {outcome}");let calls=fs::read_to_string(trace).unwrap();
   assert_eq!(calls,if outcome=="pg17"&&name=="pg-init.sh"{"binary:gate-outcome\nresolve\n"}else{"binary:gate-outcome\n"});
   assert!(!calls.contains("sideeffect"));assert_eq!(fs::read_to_string(data.join("PG_VERSION")).unwrap(),"17\n");
  }
 }
}

fn native_selected()->bool{match std::env::var("NWUI_TEST_FRESH_PG17"){Err(std::env::VarError::NotPresent)=>{eprintln!("native PG17 qualification not selected; run the separate approved-image gate");false},value=>{assert_eq!(value.as_deref(),Ok("1"));true}}}

fn save(r:&Roots,p:&pair::Pair){for root in [&r.db,&r.app]{files::atomic_json(&root.join(".nwui/pair-v1.json"),p).unwrap();}}
fn work()->PathBuf{let p=Path::new("/run").join(format!("nwui-fresh-test-{}",uuid::Uuid::new_v4()));postgres::directory(&p).unwrap();p}
async fn seed(r:&Roots,step:&str,role:bool,database:bool,extra:&str){
 postgres::own(&r.db).unwrap();postgres::directory(&r.db.join("17")).unwrap();postgres::directory(&r.db.join("17/data")).unwrap();let w=work();
 let mut command=postgres::command(17,"initdb").unwrap();command.arg("-D").arg(r.db.join("17/data")).args(["--encoding=UTF8","--locale=C","--auth-local=trust","--auth-host=scram-sha-256"]);postgres::capture(command,&w,std::time::Duration::from_secs(30)).unwrap();
 let id=postgres::read_control(17,&r.db.join("17/data")).unwrap().system_identifier;
 let mut p=current(r);p.system_identifier=Some(id);p.fresh_step=Some(step.into());save(r,&p);
 let pg=postgres::Private::start(17,&r.db.join("17/data"),&work()).unwrap();let root=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();root.client.batch_execute("SET default_transaction_read_only=off; SET standard_conforming_strings=on").await.unwrap();
 if role{let password=fs::read_to_string(r.app.join("secrets/db_password")).unwrap();root.client.batch_execute(&format!("CREATE ROLE nginx LOGIN PASSWORD '{}'",password.replace('\'',"''"))).await.unwrap();}
 if database{root.client.batch_execute("CREATE DATABASE nginx_web_ui OWNER nginx").await.unwrap();}
 if !extra.is_empty(){if extra.starts_with("APP:"){let app=inspect::connect(&pg.socket,"nginx_web_ui","postgres",None).await.unwrap();app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute(&extra[4..]).await.unwrap();drop(app);}else{root.client.batch_execute(extra).await.unwrap();}}
 drop(root);pg.stop().unwrap();
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_pg17_provisions_and_restarts_without_reinitializing_or_creating_a_key(){
 if !native_selected(){return;}let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();let material=files::snapshot(&r.app.join("secrets")).unwrap();fresh::provision(&r).await.unwrap();
 let ready=current(&r);assert!(ready.ready);assert!(ready.fresh_step.is_none());assert!(!ready.encryption_key_required);assert!(!r.app.join("secrets/data_key").exists());assert!(r.db.join("17/data/.db-random-pw").exists());assert!(!r.db.join(".nwui-work").exists());
 fresh::provision(&r).await.unwrap();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();assert_eq!(ready,current(&r));assert_eq!(material,files::snapshot(&r.app.join("secrets")).unwrap());
 let pg=postgres::Private::start(17,&r.db.join("17/data"),&work()).unwrap();let password=fs::read_to_string(r.app.join("secrets/db_password")).unwrap();let c=inspect::connect(&pg.socket,"nginx_web_ui","nginx",Some(&password)).await.unwrap();assert_eq!(c.client.query_one("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'",&[]).await.unwrap().get::<_,i64>(0),0);drop(c);pg.stop().unwrap();
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_pg17_resumes_each_recorded_step_and_preserves_an_existing_password(){
 if !native_selected(){return;}
 for (step,role,database)in[("initdb",false,false),("role",false,false),("role",true,false),("database",true,false),("database",true,true),("verify",true,true)]{
  let (_t,r,env)=fixture();layout::prepare(&r,&|_|Ok("123456".into())).unwrap();files::private_dir(&r.app.join("secrets")).unwrap();let password=r.app.join("secrets/db_password");fs::write(&password,"special'@:/#?pass\\word").unwrap();fs::set_permissions(&password,fs::Permissions::from_mode(0o600)).unwrap();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();
  seed(&r,step,role,database,"").await;let before=fs::read(&password).unwrap();fresh::provision(&r).await.unwrap();assert!(current(&r).ready,"{step} {role} {database}");assert!(before==fs::read(&password).unwrap());assert!(!r.app.join("secrets/data_key").exists());
 }
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_pg17_refuses_foreign_databases_objects_roles_and_wrong_password(){
 if !native_selected(){return;}
 for extra in ["CREATE DATABASE foreign_db", "CREATE ROLE foreign_role", "APP:CREATE TABLE public.keep(value text)", "APP:CREATE SCHEMA foreign_schema", "APP:CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id int)", "APP:CREATE EXTENSION hstore", "ALTER ROLE nginx PASSWORD 'different'", "APP:CREATE FUNCTION public.keep() RETURNS int LANGUAGE sql AS 'SELECT 1'"] {
  let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();seed(&r,"verify",true,true,extra).await;let before=current(&r);let password=fs::read(r.app.join("secrets/db_password")).unwrap();assert!(fresh::provision(&r).await.is_err(),"{extra}");assert_eq!(before,current(&r));assert!(password==fs::read(r.app.join("secrets/db_password")).unwrap());assert!(!current(&r).ready);
 }
}
#[test]
fn interrupted_or_disagreeing_intent_never_generates_material(){
 for kind in ["one-sided","wrong-id","bad-step"] {let (_t,r,env)=fixture();let p=layout::prepare(&r,&|_|Ok("123456".into())).unwrap();let app=r.app.join(".nwui/pair-v1.json");match kind {"one-sided"=>fs::remove_file(&app).unwrap(),"wrong-id"=>{let mut other=p;other.pair_id=uuid::Uuid::new_v4().to_string();files::atomic_json(&app,&other).unwrap();},_=>{let mut other=p;other.fresh_step=Some("unknown".into());save(&r,&other);}}let before=snapshot(&r);assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err());assert_eq!(before,snapshot(&r));}
}
#[test]
fn material_secret_hardlinks_and_a_foreign_encryption_key_are_refused(){
 for kind in ["hardlink","key"]{let (t,r,env)=fixture();layout::prepare(&r,&|_|Ok("123456".into())).unwrap();files::private_dir(&r.app.join("secrets")).unwrap();let name=if kind=="key"{"data_key"}else{"auth_secret"};let path=r.app.join("secrets").join(name);fs::write(&path,"0a".repeat(32)).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();if kind=="hardlink"{fs::hard_link(&path,t.path().join("alias")).unwrap();}assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err());assert!(!r.app.join("secrets/db_password").exists());}
}
#[test]
fn invalid_operator_secret_is_refused_before_writing_intent(){
 for input in [fresh::Inputs{auth_secret:Some("bad\0auth".into()),admin_password:None},fresh::Inputs{auth_secret:None,admin_password:Some("a".repeat(65537))}]{let (_t,r,env)=fixture();let before=snapshot(&r);assert!(fresh::secrets(&r,&input,&env).is_err());assert_eq!(before,snapshot(&r));assert_eq!(fs::read_dir(&env).unwrap().count(),0);}
}
#[test]
fn empty_root_lost_found_is_preserved_and_invalid_forms_do_not_write(){
 let (_t,r,env)=fixture();fs::create_dir(r.db.join("lost+found")).unwrap();fs::create_dir(r.app.join("lost+found")).unwrap();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();assert!(r.db.join("lost+found").is_dir());assert!(r.app.join("lost+found").is_dir());
 for bad in ["nonempty","link","file","owner"]{let (_t,r,env)=fixture();let path=r.app.join("lost+found");match bad{"link"=>symlink("/run",path).unwrap(),"file"=>fs::write(path,"keep").unwrap(),_=>{fs::create_dir(&path).unwrap();if bad=="owner"{std::os::unix::fs::chown(&path,Some(65534),Some(65534)).unwrap();}else{fs::write(path.join("keep"),"keep").unwrap();}}}assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err());assert!(!r.db.join(".nwui").exists());}
}
#[test]
#[ignore="native storage fault mounts: NWUI_TEST_FRESH_RO and NWUI_TEST_FRESH_FULL"]
fn native_readonly_and_enospc_never_publish_ready_or_repair_an_incomplete_pair(){
 if std::env::var_os("NWUI_TEST_FRESH_RO").is_none() && std::env::var_os("NWUI_TEST_FRESH_FULL").is_none(){eprintln!("native fault mounts not selected");return;}
 for variable in ["NWUI_TEST_FRESH_RO","NWUI_TEST_FRESH_FULL"]{let (_t,mut r,env)=fixture();r.app=std::env::var(variable).expect("isolated driver mount required").into();assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err());assert_eq!(fs::read_dir(&env).unwrap().count(),0);assert!(!r.app.join("secrets/data_key").exists());let path=r.db.join(".nwui/pair-v1.json");if path.exists(){let p:pair::Pair=files::read_json(&path).unwrap();assert!(!p.ready);}assert!(fresh::secrets(&r,&fresh::Inputs::default(),&env).is_err());}
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_running_postmaster_is_refused_without_stopping_its_owner(){
 if !native_selected(){return;}let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();seed(&r,"verify",true,true,"").await;
 let pg=postgres::Private::start(17,&r.db.join("17/data"),&work()).unwrap();let before=current(&r);assert!(fresh::provision(&r).await.is_err());assert_eq!(before,current(&r));
 let c=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();c.client.query_one("SELECT 1",&[]).await.unwrap();drop(c);pg.stop().unwrap();
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_empty_cluster_directory_resumes_after_interrupted_ownership_setup(){
 if !native_selected(){return;}let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();
 fs::create_dir(r.db.join("17")).unwrap();fs::set_permissions(r.db.join("17"),fs::Permissions::from_mode(0o700)).unwrap();fs::create_dir(r.db.join("17/data")).unwrap();fs::set_permissions(r.db.join("17/data"),fs::Permissions::from_mode(0o700)).unwrap();
 fresh::provision(&r).await.unwrap();assert!(current(&r).ready);assert!(!r.app.join("secrets/data_key").exists());
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_writable_mode_is_a_separate_batch_before_role_and_database_creation(){
 if !native_selected(){return;}let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();seed(&r,"role",false,false,"").await;
 let pg=postgres::Private::start(17,&r.db.join("17/data"),&work()).unwrap();
 let connection=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
 let error=connection.client.batch_execute("SET default_transaction_read_only=off; CREATE ROLE combined_form_should_fail").await.unwrap_err();
 assert_eq!(error.code(),Some(&tokio_postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION));
 let error=connection.client.batch_execute("SET default_transaction_read_only=off; CREATE DATABASE combined_form_should_fail").await.unwrap_err();
 assert_eq!(error.code(),Some(&tokio_postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION));
 assert!(connection.client.query_one("SHOW default_transaction_read_only",&[]).await.unwrap().get::<_,String>(0)=="on");
 drop(connection);pg.stop().unwrap();
 // Exercises the real creation path; combining SET with CREATE there must fail this test.
 fresh::provision(&r).await.unwrap();assert!(current(&r).ready);
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_STARTUP=1 with the controlled readiness wrapper"]
async fn native_provisioning_bounds_recovery_and_cleans_up_every_timed_out_postmaster(){
 if std::env::var_os("NWUI_TEST_FRESH_STARTUP").is_none(){eprintln!("native startup limits not selected");return;}
 assert_eq!(std::env::var("NWUI_TEST_FRESH_STARTUP").as_deref(),Ok("1"));
 use std::sync::{Arc,Mutex};use std::time::{Duration,Instant};
 for mode in ["recover-later","no-response","recovery-timeout","recovery-then-silent"]{
  let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();seed(&r,"role",false,false,"").await;
  let data=r.db.join("17/data");let pid_path=data.join("postmaster.pid");
  let watch=std::thread::spawn(move||{let deadline=Instant::now()+Duration::from_secs(3);while Instant::now()<deadline{if let Ok(text)=fs::read_to_string(&pid_path){if let Some(pid)=text.lines().next().and_then(|s|s.parse::<i32>().ok()){return pid;}}std::thread::sleep(Duration::from_millis(2));}panic!("native postmaster never started");});
  let timing=Arc::new(Mutex::new(None));let record=timing.clone();
  let result=fresh::provision_using(&r,|major,data,work|{
   let socket=postgres::socket_path(work,"s17");postgres::private_file(&socket.with_extension("probe-mode"),mode.as_bytes()).unwrap();postgres::private_file(&socket.with_extension("probe-count"),b"0").unwrap();*record.lock().unwrap()=Some((Instant::now(),work.to_owned()));
   postgres::Private::start_with_limits(major,data,work,Duration::from_millis(180),Duration::from_millis(750))
  }).await;
  let pid=watch.join().unwrap();let (started,work)=timing.lock().unwrap().clone().unwrap();let elapsed=started.elapsed();
  eprintln!("native startup mode={mode}, elapsed_ms={}",elapsed.as_millis());
  assert!(pid>1);assert!(!data.join("postmaster.pid").exists());assert_eq!(unsafe{libc::kill(pid,0)},-1,"postmaster survived its owner");assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
  if mode=="recover-later"{result.unwrap();assert!(current(&r).ready);assert!(elapsed>=Duration::from_millis(350),"recovery did not pass the ordinary budget");}
  else{assert!(result.is_err(),"{mode} incorrectly reached ready");assert!(!current(&r).ready);assert!(elapsed<Duration::from_secs(3),"timeout or cleanup escaped its bound");let probes=fs::read_to_string(postgres::socket_path(&work,"s17").with_extension("probe-count")).unwrap().parse::<usize>().unwrap();if mode=="no-response"{assert!(probes<5,"silent failures inherited the recovery budget");}else if mode=="recovery-timeout"{assert!(elapsed>=Duration::from_millis(650),"recovery used the ordinary failure budget");}else{assert_eq!(probes,5,"silence after recovery inherited its extended allowance");}}
  let socket=postgres::socket_path(&work,"s17");assert!(!socket.exists());for suffix in ["probe-mode","probe-count"]{fs::remove_file(socket.with_extension(suffix)).unwrap();}
 }
}

#[test]
fn private_postgres_builder_has_exactly_one_empty_listen_address(){
 let work=Path::new("/run/nwui-bind-check");let command=postgres::server_command(17,Path::new("/var/lib/postgresql/data/17/data"),&work.join("s17"),&work.join("private.conf")).unwrap();
 let addresses=command.get_args().filter_map(|arg|arg.to_str()).filter(|arg|arg.starts_with("listen_addresses=")).collect::<Vec<_>>();assert_eq!(addresses,["listen_addresses="]);
}
#[tokio::test]
#[ignore="native PG17: set NWUI_TEST_FRESH_PG17=1 in the approved disposable Trixie image"]
async fn native_user_collations_and_text_search_objects_never_promote_fresh_storage(){
 if !native_selected(){return;}
 let mut adopted=Vec::new();
 for (database,statement,step) in [
  ("nginx_web_ui","CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","verify"),
  ("postgres","CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","verify"),
  ("nginx_web_ui","CREATE TEXT SEARCH CONFIGURATION public.review_foreign (COPY=pg_catalog.simple)","verify"),
  ("template1","CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","verify"),
  ("template0","CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","verify"),
  ("postgres","CREATE COLLATION public.review_foreign FROM pg_catalog.\"C\"","initdb"),
  ("postgres","ALTER DATABASE template0 IS_TEMPLATE false","verify"),
  ("postgres","ALTER DATABASE template0 ALLOW_CONNECTIONS true","verify"),
  ("postgres","CREATE ACCESS METHOD review_foreign TYPE TABLE HANDLER pg_catalog.heap_tableam_handler","verify"),
  ("postgres","CREATE DOMAIN public.review_foreign AS integer CHECK (VALUE > 0)","verify"),
  ("nginx_web_ui","CREATE TEMP TABLE review_temporary(value text); DROP TABLE review_temporary","verify"),
  ("postgres","CREATE TEMP TABLE review_temporary(value text); DROP TABLE review_temporary","verify"),
 ]{
  let (_t,r,env)=fixture();fresh::secrets(&r,&fresh::Inputs::default(),&env).unwrap();seed(&r,step,step!="initdb",step!="initdb","").await;
  let pg=postgres::Private::start(17,&r.db.join("17/data"),&work()).unwrap();let root=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
  if database=="template0"{root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS true").await.unwrap();}
  let c=inspect::connect(&pg.socket,database,"postgres",None).await.unwrap();c.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();c.client.batch_execute(statement).await.unwrap();drop(c);
  if database=="template0"{root.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS false").await.unwrap();}drop(root);pg.stop().unwrap();
  let pair=current(&r);let secrets=files::snapshot(&r.app.join("secrets")).unwrap();
  if fresh::provision(&r).await.is_ok(){adopted.push(format!("{database}/{step}: {statement}"));continue;}
  assert_eq!(pair,current(&r),"refusal advanced {database}/{step}");assert_eq!(secrets,files::snapshot(&r.app.join("secrets")).unwrap());assert!(!current(&r).ready);
 }
 assert!(adopted.is_empty(),"foreign objects adopted: {adopted:?}");
}

#[cfg(target_os="linux")]
#[test]
fn root_lock_release_ends_ownership_despite_a_cloexec_inherited_descriptor() {
 use std::os::fd::FromRawFd;
 let (_t,r,_env)=fixture();let owner=layout::lock_roots(&r).unwrap();
 let fd=fs::read_dir("/proc/self/fd").unwrap().filter_map(|e|e.ok()).find_map(|e|{
  (fs::read_link(e.path()).ok().as_deref()==Some(r.db.as_path())).then(||e.file_name().to_str().unwrap().parse::<i32>().unwrap())
 }).expect("owned database directory descriptor");
 // dup and fork reference the same flock; CLOEXEC does not close it until exec.
 let copied=unsafe{libc::fcntl(fd,libc::F_DUPFD_CLOEXEC,0)};assert!(copied>=0);
 let inherited=unsafe{fs::File::from_raw_fd(copied)};drop(owner);
 let reacquired=layout::lock_roots(&r);assert!(reacquired.is_ok(),"parent ownership ended, got {:?}",reacquired.err());
 drop(inherited);
}
#[test]
fn root_lock_release_keeps_an_independent_postmaster_owner_locked() {
 let (_t,r,_env)=fixture();let postmaster=fs::File::open(&r.db).unwrap();postmaster.try_lock_shared().unwrap();
 let key_owner=layout::lock_key_roots(&r).unwrap();drop(key_owner);
 assert_eq!(layout::lock_roots(&r).err().unwrap().code,"storage_busy");
 postmaster.unlock().unwrap();assert!(layout::lock_roots(&r).is_ok());
}
