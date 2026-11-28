//! Native combined A.25(a) fixture; phase-1 authority is synthetic, PG17 and N1 are real.
use super::{*, backup_format::SourceLocation, journal::{Candidate,Handoff}, selector::{Selector,GenerationManifest,KeyRecord,Origin,StoredKeySource}};
use crate::{publisher::{boot,test_support}, db_upgrade::{files,layout::Roots,control::{Control,COMPATIBILITY_FIELDS}}};
use serde_json::{json,Value};
use std::{collections::BTreeSet,fs,os::unix::fs::PermissionsExt,path::{Path,PathBuf},process::{Child,Command,Stdio},time::{Duration,Instant}};
#[path="../../tests/auth_support/mod.rs"] mod auth_support;
const SERVER:&str="11111111-1111-4111-8111-111111111111";
fn write(p:&Path,bytes:impl AsRef<[u8]>){fs::write(p,bytes).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o600)).unwrap();}
fn private(p:&Path){fs::create_dir_all(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
fn read(p:&Path)->Value{serde_json::from_slice(&fs::read(p).unwrap()).unwrap()}
struct Master(Child);
impl Drop for Master{fn drop(&mut self){unsafe{libc::kill(self.0.id()as i32,libc::SIGQUIT);}let until=Instant::now()+Duration::from_secs(3);while Instant::now()<until{if self.0.try_wait().unwrap().is_some(){return}std::thread::sleep(Duration::from_millis(20));}let _=self.0.kill();let _=self.0.wait();}}
struct Fixture{startup:runner::Startup,db:auth_support::TestDb,store:crate::publisher::PublisherStore,l:PathBuf,g:String,derived:String,_master:Master,_managed:crate::publisher::test_support::RegisteredMaster,services:Vec<tokio::task::JoinHandle<()>>}
impl Drop for Fixture{fn drop(&mut self){for service in &self.services{service.abort();}}}
impl Fixture{
 async fn new()->Self{
  assert_eq!(std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref(),Ok("1"));
  let root=PathBuf::from(std::env::var_os("NWUI_G18_ROOT").expect("isolated native fixture root"));
  let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()};private(&roots.app);
  // Each native row is an isolated process; the test-support store uses this fixture root.
  unsafe{std::env::set_var("NWUI_TEST_PUBLISHER_ROOT",&roots.app);std::env::set_var("DATA_KEY_FILE",roots.app.join("data_key"));}
  let l=roots.db.join("pgdata");private(&l);private(&l.join("base"));private(&l.join("global"));write(&l.join("PG_VERSION"),b"15\n");write(&l.join("base/owned"),b"synthetic old PG15 entry");write(&l.join("global/pg_control"),b"synthetic cold control");
  let mut locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
  let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());let control=Control::parse(&text,"").unwrap();
  let prepared=backup::prepare(&roots,&locks,&j.load().attempt_id,SourceLocation::Pgdata,&BTreeSet::from(["PG_VERSION".into(),"base".into(),"global".into()]),&control,"2026-10-30T00:00:00Z").unwrap();backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();
  let control=crate::db_upgrade::postgres::read_control(17,&roots.db.join("17/data")).unwrap();let id=control.system_identifier;
  let generation=uuid::Uuid::new_v4().to_string();let pair=crate::db_upgrade::pair::Pair{schema_version:1,pair_id:uuid::Uuid::new_v4().to_string(),mode:"migration".into(),ready:true,major:17,architecture:roots.architecture.clone(),system_identifier:Some(id.clone()),encryption_key_required:true,fresh_step:None};
  write(&roots.app.join("data_key"),"A5".repeat(32));
  let key=KeyRecord{version:1,key_binding:KeyBinding::Verified{pair_id:pair.pair_id.clone()},key_source:StoredKeySource::Canonical,key_sha256:selector::digest("A5".repeat(32).as_bytes())};let pair_bytes=serde_json::to_vec(&pair).unwrap();let key_bytes=serde_json::to_vec(&key).unwrap();
  let identity=profile::historical_identity().unwrap();let manifest=GenerationManifest{version:1,generation:generation.clone(),system_identifier:id.clone(),origin:Origin::Auto,profile_id:Some(identity.profile_id),profile_sha256:Some(identity.profile_sha256),db_pair_sha256:selector::digest(&pair_bytes),app_pair_sha256:selector::digest(&pair_bytes),key_binding_sha256:selector::digest(&key_bytes),created_at:"2026-10-30T00:00:00Z".into()};let manifest_bytes=serde_json::to_vec(&manifest).unwrap();
  let generation_root=roots.app.join(".nwui-auto/gen").join(&generation);private(&generation_root);for(name,bytes)in[("manifest.json",&manifest_bytes),("db-pair.json",&pair_bytes),("app-pair.json",&pair_bytes),("key-binding.json",&key_bytes)]{write(&generation_root.join(name),bytes);}
  let selected=Selector{version:1,active_major:17,generation:Some(generation.clone()),system_identifier:id,origin:Origin::Auto,backup_id:Some(j.load().attempt_id.clone()),profile_id:manifest.profile_id.clone(),profile_sha256:manifest.profile_sha256.clone(),manifest_sha256:Some(selector::digest(&manifest_bytes)),operator_reason:None};write(&roots.app.join(".nwui-auto/active-v1.json"),serde_json::to_vec(&selected).unwrap());private(&roots.app.join(".nwui"));write(&roots.app.join(".nwui/pair-v1.json"),&pair_bytes);
  j.record(Step::P5Commit,StepState::Intent,Some(json!({"authorityEvent":"selector-intent","generation":generation,"selectorSha256":selector::digest(&serde_json::to_vec(&selected).unwrap())}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
  private(&roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("linked"));
  fs::rename(l.join("PG_VERSION"),l.join("PG_VERSION.nwui-pre17")).unwrap();write(&l.join("UPGRADED-TO-PG17.txt"),format!("NWUI: PostgreSQL 17 upgrade. Do not start an older image on these volumes.\nBackup: {}\nGeneration: {generation}\nStop the container and follow the documented PG15 restore procedure. Keep both volumes and the backup.\n",j.load().attempt_id));
  let main=root.join("nginx.conf");let conf=root.join("conf.d");let stream=root.join("stream.d");private(&conf);private(&stream);write(&conf.join(".nwui-generation"),b"add_header X-NWUI-Config-Generation \"22222222-2222-4222-8222-222222222222\";\n");let bin=root.join("nginx");
  write(&main,format!("pid {}; error_log {} notice; events {{}} http {{access_log off; include {}/*.conf; server {{ listen 127.0.0.1:8080; location = /api/health {{ proxy_pass http://127.0.0.1:3001; }} location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}/.nwui-generation; }} }} }}\n",root.join("nginx.pid").display(),root.join("nginx.log").display(),conf.display(),conf.display()));
  write(&bin,format!("#!/bin/sh\nif [ \"$1\" = -V ]; then /usr/sbin/nginx -V 2>&1 | sed 's#--conf-path=[^ ]*#--conf-path={}#' >&2; exit 0; fi\nhas=0; for arg do [ \"$arg\" != -c ] || has=1; done; if [ \"$has\" = 0 ]; then set -- \"$@\" -c '{}'; fi\nexec /usr/sbin/nginx \"$@\"\n",main.display(),main.display()));fs::set_permissions(&bin,fs::Permissions::from_mode(0o700)).unwrap();
  let values=[bin.display().to_string(),main.display().to_string(),conf.display().to_string(),stream.display().to_string(),root.join("certs").display().to_string(),roots.app.join("data_key").display().to_string()];
  let db=auth_support::test_db_with(&[("NGINX_BIN",&values[0]),("NGINX_CONF",&values[1]),("NGINX_CONF_D",&values[2]),("NGINX_STREAM_D",&values[3]),("CERTS_DIR",&values[4]),("DATA_KEY_FILE",&values[5]),("NGINX_STATUS_URL","http://127.0.0.1:8080/nginx_status"),("PORT","3000")]).await;
  fs::set_permissions(&db.state.config.data_key_file,fs::Permissions::from_mode(0o600)).unwrap();
  db.client.execute("INSERT INTO nginx_servers(id,server_name,listen,config) VALUES($1,'g18.example.test',18081,$2)",&[&SERVER,&json!({"proxyPass":"http://127.0.0.1:19000"})]).await.unwrap();
  let candidate=StorageSelection{version:1,epoch:1,outcome:Outcome::Pg17,branch:Branch::Upgrade,pgdata:roots.db.join("17/data"),backup_id:Some(j.load().attempt_id.clone()),schema_profile:"legacy-28".into(),key_binding:KeyBinding::Verified{pair_id:pair.pair_id}};
  for record in crate::nonroot::fixture_prepare_records(&candidate,&j.load().attempt_id,j.load().records.last().unwrap().seq+1,false){j.record(record.step,record.state,record.data).unwrap();}
  private(&roots.app.join(".nwui-publisher"));let store=crate::publisher::PublisherStore::open(&db.state.config).unwrap();
  let stopped=test_support::RegisteredMaster::stopped(&root.join("nginx.pid"),Path::new("/usr/sbin/nginx"));
  let staged=boot::prepare_payload(&db.state.config,&db.state.secrets,boot::PgSource::Pool(&db.state.db),&store,crate::publisher::boot::BootMode::Conversion).await.unwrap();boot::publish_live(&db.state.config,&store,&staged.id).await.unwrap_or_else(|error|panic!("publication: {error:?}; nginx={}",fs::read_to_string(root.join("nginx.log")).unwrap_or_default()));
  let publisher=roots.app.join(".nwui-publisher");let g=staged.id;let original=files::snapshot(&publisher.join("payloads").join(&g)).unwrap();let applied:Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);
  let live=conf.join(format!("server-{SERVER}.conf"));write(&live,fs::read_to_string(&live).unwrap().replace("listen 18081","listen 3001"));
  let report=boot::guard(&db.state.config,&store).await.unwrap();assert_eq!(report.reserved_files,1);let derived=read(&publisher.join("manifest.json"))["current"].as_str().unwrap().to_owned();assert_ne!(derived,g);assert_eq!(read(&publisher.join("payloads").join(&derived).join("manifest.json"))["derivedFrom"],g);assert_eq!(files::snapshot(&publisher.join("payloads").join(&g)).unwrap(),original);assert!(!live.exists());
  drop(stopped);
  let master=Master(Command::new("/usr/sbin/nginx").arg("-c").arg(&main).args(["-g","daemon off;"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
  let until=Instant::now()+Duration::from_secs(5);loop{if reqwest::get("http://127.0.0.1:8080/nginx_status").await.is_ok(){break}assert!(Instant::now()<until);tokio::time::sleep(Duration::from_millis(20)).await;}
  let managed=test_support::RegisteredMaster::new(&root.join("nginx.pid"),Path::new("/usr/sbin/nginx"),master.0.id());
  test_support::adopt_current(&db.state,&store).await.unwrap();assert!(crate::control::confirmation::uncertain(&db.state.config).await);assert!(!crate::control::pending::status(&db.state).await.unwrap().has_baseline);let after:Value=db.client.query_one("SELECT data FROM applied_config WHERE id='current'",&[]).await.unwrap().get(0);assert_eq!(after,applied);
  let api=tokio::net::TcpListener::bind("127.0.0.1:3001").await.unwrap();let router=crate::server::router(db.state.clone());
  let api=tokio::spawn(async move{axum::serve(api,router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();});
  let web=tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();let web=tokio::spawn(async move{axum::serve(web,axum::Router::new().route("/",axum::routing::get(||async{"G18 web-smoke fixture"}))).await.unwrap();});
  let identity=boot_identity::current().unwrap();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&identity).unwrap();locks.release_d();j.handoff(Handoff::Done,&identity).unwrap();drop(locks);
  let custody=custody::Custody::acquire(roots.clone(),false).await.unwrap();let startup=runner::Startup{roots,custody,journal:Some(j),identity:Some(identity),plan:runner::BootPlan::Phase2,unrecorded:None};
  Self{startup,db,store,l,g,derived,_master:master,_managed:managed,services:vec![api,web]}
 }
}
#[tokio::test]
#[ignore="G18 native PG17 + real nginx + owned s6 custody; NWUI_TEST_PROFILE_NATIVE=1"]
async fn native_g18_guard_derivation_keeps_a8_and_the_original_entries(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new().await;assert_ne!(f.g,f.derived);assert!(f.l.join("base/owned").is_file());assert!(f.startup.custody.locks.holds_a());assert!(!f.startup.custody.locks.holds_d());assert!(!f.startup.journal.as_ref().unwrap().load().records.iter().any(|r|r.step==Step::P6Cleanup));
 println!("G18 scaffold: real PG17 / real fdholder custody / real nginx G -> guard-derived G' / A8 kept; no phase2 gate claim");
}

#[tokio::test]
#[ignore="A24 amended G18; isolated native PG17 + s6 fixture"]
async fn native_g18_kept_a8_defers_p6_without_activation_failure(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;
 assert!(service::phase2_ready(Duration::from_secs(2)).unwrap(),"A24(a) service-ready fixture");
 crate::healthcheck::check(&crate::healthcheck::Targets{api:"http://127.0.0.1:8080/api/health".into(),web:"http://127.0.0.1:3000/".into(),timeout:Duration::from_secs(2)}).await.unwrap();
 crate::control::confirmation::observe_current(&f.db.state.config,Instant::now()+Duration::from_secs(2)).await.unwrap();
 let before=files::snapshot(&f.l).unwrap();let result=runner::monitor_after_handoff(&mut f.startup,&f.db.state.config,Instant::now()+Duration::from_secs(2)).await;
 let j=f.startup.journal.as_ref().unwrap();assert_eq!(j.load().outcome,Outcome::Pg17,"kept A8 is cleanupPending, not activation-failed; monitor={result:?}");
 assert_eq!(result.unwrap(),true);let status=status::read().unwrap();assert!(status.cleanup_pending);assert!(status.failure.is_none());assert!(!j.load().records.iter().any(|r|matches!(r.state,StepState::Failed{..})||r.step==Step::Phase2Monitor&&r.state==StepState::Done||r.step==Step::P6Cleanup));assert_eq!(files::snapshot(&f.l).unwrap(),before);assert!(f.startup.roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).is_dir());
}

impl Fixture{
 async fn apply_saved(&self){
  use crate::publisher::protocol::{Request,Actor,Op,OpResult};
  let result=test_support::dispatch(&self.db.state,Request{v:1,id:uuid::Uuid::new_v4().to_string(),op:Op::ApplySaved{label:Some("G18 confirmed saved configuration".into()),expected_saved_digest:None},deadline_ms:280000,actor:Actor{via: None,user_id:None,label:Some("G18 native fixture".into()),ip:None}}).await;
  assert!(matches!(result,OpResult::Ok{apply:Some(ref result),uncertain:false,..}if result.ok),"ApplySaved={}",serde_json::to_string(&result).unwrap());
  assert!(!crate::control::confirmation::uncertain(&self.db.state.config).await);assert!(crate::control::pending::status(&self.db.state).await.unwrap().has_baseline);
 }
 async fn assert_activation_failure(&mut self){
  let before=files::snapshot(&self.l).unwrap();let error=runner::monitor_after_handoff(&mut self.startup,&self.db.state.config,Instant::now()+Duration::from_secs(2)).await.unwrap_err();assert_eq!(error.code,"activation_timeout");
  let j=self.startup.journal.as_ref().unwrap();assert_eq!(j.load().outcome,Outcome::ActivationFailed);assert!(j.load().records.iter().any(|r|r.step==Step::Phase2Monitor&&matches!(&r.state,StepState::Failed{code}if code=="activation_timeout")));assert!(!j.load().records.iter().any(|r|r.step==Step::P6Cleanup));assert_eq!(files::snapshot(&self.l).unwrap(),before);assert!(self.startup.roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).is_dir());
 }
}
#[tokio::test]
#[ignore="G18 native isolated service (a) failure control"]
async fn native_g18_unready_service_is_activation_failed(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;assert!(Command::new("/command/s6-svc").args(["-d","/run/service/api"]).status().unwrap().success());assert!(Command::new("/command/s6-svwait").args(["-D","-t","5000","/run/service/api"]).status().unwrap().success());assert!(!service::phase2_ready(Duration::from_secs(2)).unwrap());f.assert_activation_failure().await;
}
#[tokio::test]
#[ignore="G18 native isolated HTTP (b) failure control"]
async fn native_g18_failed_http_is_activation_failed(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;f.services[0].abort();tokio::task::yield_now().await;assert!(service::phase2_ready(Duration::from_secs(2)).unwrap());assert!(crate::healthcheck::check(&crate::healthcheck::Targets{api:"http://127.0.0.1:8080/api/health".into(),web:"http://127.0.0.1:3000/".into(),timeout:Duration::from_secs(1)}).await.is_err());f.assert_activation_failure().await;
}
#[tokio::test]
#[ignore="G18 native isolated generation observer (c) failure control"]
async fn native_g18_invalid_generation_is_activation_failed(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;write(&f.db.state.config.nginx_conf_dir.join(".nwui-generation"),b"invalid marker\n");assert!(crate::control::confirmation::observe_current(&f.db.state.config,Instant::now()+Duration::from_secs(1)).await.is_err());f.assert_activation_failure().await;
}
#[tokio::test]
#[ignore="G18 native confirmed ApplySaved transition; not a P6 proof"]
async fn native_g18_apply_saved_clears_only_the_derived_a8(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new().await;assert!(crate::control::confirmation::uncertain(&f.db.state.config).await);f.apply_saved().await;assert!(f.l.join("base/owned").exists());assert!(!f.startup.journal.as_ref().unwrap().load().records.iter().any(|r|r.step==Step::P6Cleanup));
}

impl Fixture{
 fn assert_p6_complete(&self){
  let status=status::read().unwrap();assert!(!status.cleanup_pending);assert!(status.failure.is_none());
  let j=self.startup.journal.as_ref().unwrap();assert_eq!(j.load().outcome,Outcome::Pg17);assert!(j.load().records.iter().any(|r|r.step==Step::Phase2Monitor&&r.state==StepState::Done));assert!(j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.state==StepState::Done&&r.data.is_none()));
  for name in ["base","global","PG_VERSION","PG_VERSION.nwui-pre17"]{assert!(!self.l.join(name).exists(),"owned L entry remains: {name}");}
  assert!(self.l.join("UPGRADED-TO-PG17.txt").is_file());assert!(self.startup.roots.db.join("17/data/PG_VERSION").is_file());assert!(self.startup.roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).is_dir());
 }
 async fn defer(&mut self){assert!(runner::monitor_after_handoff(&mut self.startup,&self.db.state.config,Instant::now()+Duration::from_secs(2)).await.unwrap());let status=status::read().unwrap();assert!(status.cleanup_pending);assert!(status.failure.is_none());assert_eq!(self.startup.journal.as_ref().unwrap().load().outcome,Outcome::Pg17);assert!(self.l.join("base/owned").is_file());assert!(!self.startup.journal.as_ref().unwrap().load().records.iter().any(|r|r.step==Step::P6Cleanup));}
}
#[tokio::test]
#[ignore="G18 real ApplySaved followed by once-only 15-minute injected scheduler"]
async fn native_g18_apply_then_due_retry_proves_and_cleans_only_owned_entries(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;f.defer().await;let backup=f.startup.roots.db.join(".nwui-pg15-backup").join(&f.startup.journal.as_ref().unwrap().load().attempt_id);let before=files::snapshot(&backup).unwrap();let mut retry=runner::CleanupRetry::new(Instant::now());let due=retry.due;
 assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,due-Duration::from_nanos(1),Instant::now()+Duration::from_secs(2)).await.unwrap(),None);assert!(f.l.join("base/owned").exists());
 f.apply_saved().await;assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,due,Instant::now()+Duration::from_secs(5)).await.unwrap(),Some(false));f.assert_p6_complete();assert_eq!(files::snapshot(&backup).unwrap(),before);
 let before=fs::read(f.startup.roots.app.join(".nwui-auto/journal-v1.json")).unwrap();assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,due+Duration::from_secs(900),Instant::now()+Duration::from_secs(2)).await.unwrap(),None);assert_eq!(fs::read(f.startup.roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
}
#[tokio::test]
#[ignore="G18 durable re-entry after a later confirmed ApplySaved; real custody reacquired"]
async fn native_g18_later_apply_then_phase2_reentry_proves_and_cleans(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;f.defer().await;f.apply_saved().await;let roots=f.startup.roots.clone();let identity=boot_identity::current().unwrap();
 let placeholder=custody::Custody::acquire(roots.clone(),false).await.unwrap();let old=std::mem::replace(&mut f.startup.custody,placeholder);drop(old);
 let journal=Journal::open(&roots.app,&f.startup.custody.locks).unwrap();assert_eq!(runner::boot_plan(journal.load(),Some(&identity)),runner::BootPlan::Phase2);f.startup.journal=Some(journal);f.startup.identity=Some(identity);f.startup.plan=runner::BootPlan::Phase2;
 assert!(!runner::monitor_after_handoff(&mut f.startup,&f.db.state.config,Instant::now()+Duration::from_secs(5)).await.unwrap());f.assert_p6_complete();
}

#[tokio::test]
#[ignore="A33 G34 replacement: real guard-derived A8 and real ApplySaved dispatch"]
async fn native_g34_real_guard_a8_apply_saved_and_once_only_retry(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let mut f=Fixture::new().await;f.defer().await;
 let j=f.startup.journal.as_ref().unwrap();let offline=commands::offline_status(&f.startup.roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();assert_eq!(offline.outcome,Outcome::Pg17);assert!(offline.cleanup_pending);assert!(offline.failure.is_none());
 let now=Instant::now();let mut retry=runner::CleanupRetry::new(now);let due=retry.due;let journal=f.startup.roots.app.join(".nwui-auto/journal-v1.json");let before=fs::read(&journal).unwrap();
 assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,now,Instant::now()+Duration::from_secs(2)).await.unwrap(),None);assert_eq!(fs::read(&journal).unwrap(),before);
 f.apply_saved().await;let nginx_tree=||(fs::read(&f.db.state.config.nginx_main_conf).unwrap(),[f.db.state.config.nginx_conf_dir.clone(),f.db.state.config.nginx_stream_dir.clone()].into_iter().map(|p|(p.clone(),files::snapshot(&p).unwrap())).collect::<Vec<_>>());let nginx=nginx_tree();
 assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,due,Instant::now()+Duration::from_secs(5)).await.unwrap(),Some(false));f.assert_p6_complete();
 let before=fs::read(&journal).unwrap();assert_eq!(runner::retry_cleanup_after_handoff(&mut f.startup,&f.db.state.config,&mut retry,due+Duration::from_secs(900),Instant::now()+Duration::from_secs(2)).await.unwrap(),None);assert_eq!(fs::read(&journal).unwrap(),before);assert_eq!(nginx_tree(),nginx);
 println!("G34_NATIVE_ROW real guard-derived A8, real ApplySaved via test_support::dispatch; live+offline pending, early retry none, due proof+P6, second retry none; nginx unchanged");
}

#[tokio::test]
#[ignore="G18 native generation observer with a live worker and foreign PID"]
async fn native_g18_foreign_pid_never_completes_phase2_or_p6() {
    if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref() != Ok("1") { return; }
    let mut f = Fixture::new().await;
    f.apply_saved().await;
    let foreign = Master(Command::new("/bin/sleep").arg("30").spawn().unwrap());
    let pid_file = f.db.state.config.nginx_main_conf.with_file_name("nginx.pid");
    write(&pid_file, foreign.0.id().to_string());
    let reply = reqwest::Client::builder().local_address(std::net::IpAddr::from([127,0,0,3]))
        .no_proxy().build().unwrap().get("http://127.0.0.1:8080/nginx_status").send().await.unwrap();
    assert_eq!(reply.status(), reqwest::StatusCode::OK);
    let marker = fs::read_to_string(f.db.state.config.nginx_conf_dir.join(".nwui-generation")).unwrap();
    assert_eq!(reply.headers()["x-nwui-config-generation"], marker.split('"').nth(1).unwrap());
    f.assert_activation_failure().await;
    assert_eq!(unsafe { libc::kill(foreign.0.id() as i32, 0) }, 0);
}
