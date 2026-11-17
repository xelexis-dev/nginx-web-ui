//! Migrated PG17 retains the legacy credential policy; this fixture runs only PG17 tools.
use super::*;
use crate::db_upgrade::{inspect,layout::Roots,postgres};
use std::{fs,os::unix::fs::PermissionsExt,path::{Path,PathBuf},process::Stdio,sync::Arc,time::Duration};
const STORED:&str="g37-generated-fixture-password-not-nginx";
fn private(p:&Path){fs::create_dir_all(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
fn write(p:&Path,s:&str,mode:u32){fs::write(p,s).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(mode)).unwrap();}
struct Fixture{server:Option<postgres::Private>,locks:Arc<super::super::StorageLocks>,roots:Roots,data:PathBuf,_temp:tempfile::TempDir}
impl Fixture{
 async fn new(major:u8,stored:Option<&str>,actual:&str)->Self{
  assert_eq!(std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref(),Ok("1"));
  let temp=tempfile::tempdir().unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:temp.path().join("db"),app:if std::env::var_os("NWUI_TEST_API_BIN").is_some(){crate::db_upgrade::paths::APPROOT.into()}else{temp.path().join("app")},architecture:crate::db_upgrade::paths::architecture().unwrap().into()};private(&roots.db);private(&roots.app);unsafe{std::env::set_var("NWUI_TEST_PUBLISHER_ROOT",&roots.app);}assert_eq!(major,17);postgres::own(&roots.db).unwrap();postgres::directory(&roots.db.join("17")).unwrap();
  let data=roots.db.join("17/data");postgres::directory(&data).unwrap();let log=temp.path().join("logs");postgres::directory(&log).unwrap();
  let mut init=postgres::command(major,"initdb").unwrap();init.arg("-D").arg(&data).args(["--no-locale","--encoding=UTF8","--auth-local=trust","--auth-host=reject"]).stdout(Stdio::null()).stderr(Stdio::null());postgres::wait(init.spawn().unwrap(),Duration::from_secs(60)).unwrap();let server=postgres::Private::start(major,&data,&log).unwrap();
  let root=inspect::connect(&server.socket,"postgres","postgres",None).await.unwrap();root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();assert!(actual.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'));root.client.batch_execute(&format!("CREATE ROLE nginx LOGIN PASSWORD '{actual}'")).await.unwrap();root.client.batch_execute("CREATE DATABASE nginx_web_ui OWNER nginx").await.unwrap();drop(root);
  private(&roots.app.join("secrets"));private(&roots.app.join("tls"));write(&roots.app.join("secrets/auth_secret"),"g37-stored-auth-secret",0o644);if let Some(password)=stored{write(&roots.app.join("secrets/db_password"),password,0o644);}
  let cert=rcgen::generate_simple_self_signed(vec!["g37.invalid".into()]).unwrap();write(&roots.app.join("tls/default.crt"),&cert.cert.pem(),0o644);write(&roots.app.join("tls/default.key"),&cert.signing_key.serialize_pem(),0o600);
  let mut pair=crate::db_upgrade::pair::Pair::fresh(&roots.architecture);pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(postgres::read_control(17,&data).unwrap().system_identifier);for root in [&roots.db,&roots.app]{private(&root.join(".nwui"));crate::db_upgrade::files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).unwrap();}
  let locks=Arc::new(super::super::StorageLocks::acquire(&roots.db,&roots.app).unwrap());Self{_temp:temp,roots,locks,server:Some(server),data}
 }
 fn publish(&self){private(Path::new("/run/s6/container_environment"));crate::db_upgrade::fresh::ready_environment_locked(&self.locks,&self.roots,Path::new("/run/s6/container_environment")).unwrap();}
}
#[tokio::test]
#[ignore="A37 native migrated PG17 authentication; isolated container with image fallback DATABASE_URL"]
async fn native_g37_stored_password_authenticates_after_legacy_publication(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();
 let config=read_config().unwrap();let credentials:tokio_postgres::Config=config.database_url.parse().unwrap();let password=std::str::from_utf8(credentials.get_password().unwrap()).unwrap();
 let connected=inspect::connect(&f.server.as_ref().unwrap().socket,"nginx_web_ui","nginx",Some(password)).await;
 assert!(connected.is_ok(),"PG17 must authenticate using the published stored credential, not the image fallback; refusal={:?}",connected.as_ref().err().map(|e|e.code));
 let connected=connected.unwrap();let user:String=connected.client.query_one("SELECT current_user",&[]).await.unwrap().get(0);assert_eq!(user,"nginx");
}

#[tokio::test]
#[ignore="A37 native migrated PG17 pre-random-password legacy compatibility"]
async fn native_g37_legacy_missing_password_uses_nginx_without_rewriting_secrets(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,None,"nginx").await;let before=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();f.publish();
 let config=read_config().unwrap();let credentials:tokio_postgres::Config=config.database_url.parse().unwrap();let password=std::str::from_utf8(credentials.get_password().unwrap()).unwrap();
 let connected=inspect::connect(&f.server.as_ref().unwrap().socket,"nginx_web_ui","nginx",Some(password)).await.unwrap();assert_eq!(connected.client.query_one("SELECT 1",&[]).await.unwrap().get::<_,i32>(0),1);
 assert!(!f.roots.app.join("secrets/db_password").exists());assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),before);
}
#[tokio::test]
#[ignore="A37 native random-marker refusal before publishing or connecting"]
async fn native_g37_missing_password_with_random_marker_never_falls_back(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,None,STORED).await;write(&f.data.join(".db-random-pw"),"",0o600);private(Path::new("/run/s6/container_environment"));
 let before=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();let error=crate::db_upgrade::fresh::ready_environment_locked(&f.locks,&f.roots,Path::new("/run/s6/container_environment")).unwrap_err();assert_eq!(error.code,"pair_mismatch");assert_eq!(error.detail,Some("db_password_missing_random_marker"));assert!(!Path::new("/run/s6/container_environment/DATABASE_URL").exists());assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),before);
}
#[tokio::test]
#[ignore="A37 native migrated PG17 wrong stored password produces only a static auth diagnostic"]
async fn native_g37_wrong_stored_password_has_sanitized_auth_detail(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some("g37-wrong-fixture-password"),STORED).await;f.publish();let config=read_config().unwrap();let credentials:tokio_postgres::Config=config.database_url.parse().unwrap();let password=std::str::from_utf8(credentials.get_password().unwrap()).unwrap();
 let error=inspect::connect(&f.server.as_ref().unwrap().socket,"nginx_web_ui","nginx",Some(password)).await.err().unwrap();assert_eq!(error.code,"pair_mismatch");assert_eq!(error.detail,Some("db_auth_failed"));assert!(!error.to_string().contains(STORED));
}
#[tokio::test]
#[ignore="A37 native connection diagnostic classes without PostgreSQL text"]
async fn native_g37_other_connection_diagnostics_are_static(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;let connected=inspect::connect(&f.server.as_ref().unwrap().socket,"nginx_web_ui","nginx",Some(STORED)).await.unwrap();
 let sql=connected.client.query_one("SELECT 1/0",&[]).await.unwrap_err();assert_eq!(inspect::connection_error(&sql).detail,Some("db_connect_failed"));drop(connected);f.server.unwrap().stop().unwrap();
 let error=inspect::connect(Path::new("/run/nwui-g37-no-such-socket"),"nginx_web_ui","nginx",Some(STORED)).await.err().unwrap();assert_eq!(error.code,"pair_mismatch");assert_eq!(error.detail,Some("db_unreachable"));
}
#[tokio::test]
#[ignore="A37 native trusted reader and process-only non-credential override"]
async fn native_g37_reader_never_falls_back_and_keeps_data_key_override(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();let env=Path::new("/run/s6/container_environment");write(&env.join("DATA_KEY_FILE"),"/not-the-process-key",0o600);
 assert_eq!(crate::nonroot::startup_value("DATA_KEY_FILE").as_deref(),Some("/tmp/g37-process-key-dir/data_key"));assert_eq!(read_config().unwrap().data_key_file,Path::new("/tmp/g37-process-key-dir/data_key"));
 for field in ["AUTH_SECRET","DATABASE_URL"]{let path=env.join(field);let original=fs::read(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o666)).unwrap();assert!(read_config().is_err());fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();fs::remove_file(&path).unwrap();assert!(read_config().is_err());fs::write(&path,&original).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();}
}
#[tokio::test]
#[ignore="A37 native PG17 ready/repeated BootCtx uses republished stored credentials"]
async fn native_g37_ready_restart_bootctx_connects_with_stored_password(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 ready_restart_with_seed(false).await;
}
#[tokio::test]
#[ignore="A37 operator admin seed/persistence and existing-user skip on real PG17"]
async fn native_g37_operator_admin_seed_persists_and_skips_existing(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 ready_restart_with_seed(true).await;
}
async fn ready_restart_with_seed(seed:bool){
 assert!(std::env::var_os("NWUI_TEST_API_BIN").is_some_and(|path|Path::new(&path).is_file()),"Ready native tests require NWUI_TEST_API_BIN pointing to the built real nwui-api for the actual-UID access probe");
 let original_password=std::env::var("ADMIN_PASSWORD").ok();
 let mut f=Fixture::new(17,Some(STORED),STORED).await;let server=f.server.take().unwrap();server.stop().unwrap();
 unsafe{std::env::set_var("DATA_KEY_FILE",f.roots.app.join("secrets/data_key"));}
 let mut pair=crate::db_upgrade::pair::Pair::fresh(&f.roots.architecture);pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(postgres::read_control(17,&f.data).unwrap().system_identifier);
 for root in [&f.roots.db,&f.roots.app]{private(&root.join(".nwui"));crate::db_upgrade::files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).unwrap();}
 let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&dir).await.unwrap();let candidate=super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)};private(Path::new("/run/s6/container_environment"));
 for name in ["auth_secret","db_password"]{fs::set_permissions(f.roots.app.join("secrets").join(name),fs::Permissions::from_mode(0o600)).unwrap();}
 let mut previous=None;
 for turn in 0..2 {
  if seed && turn==1{unsafe{std::env::set_var("ADMIN_PASSWORD","g37-later-admin-password2");}}
  let j=super::super::ready::enter(f.roots.clone(),f.locks.clone(),previous,candidate.clone(),crate::db_upgrade::fresh::Inputs::default(),"/run/s6/container_environment".into()).await.unwrap();
  let mut config=read_config().unwrap();let credentials:tokio_postgres::Config=config.database_url.parse().unwrap();assert_eq!(credentials.get_password(),Some(STORED.as_bytes()));config.data_key_file=f.roots.app.join("secrets/data_key");config.target_dns.descriptor_path=f._temp.path().join("target-dns.json");config.migrations_dir=dir.clone();
  let prepared=super::super::ready::prepare(f.roots.clone(),f.locks.clone(),j,candidate.clone(),config.clone(),super::super::boot::Inputs::startup()).await.unwrap();
  assert!(prepared.journal.load().records.iter().any(|r|r.data.as_ref().and_then(|d|d.get("bootStep")).and_then(|s|s.as_str())==Some("run_migrations")&&r.state==StepState::Done));previous=Some(prepared.journal);
  let log=f._temp.path().join(format!("observe-{turn}"));postgres::directory(&log).unwrap();let server=postgres::Private::start(17,&f.data,&log).unwrap();let session=inspect::connect(&server.socket,"nginx_web_ui","nginx",Some(STORED)).await.unwrap();let count:i64=session.client.query_one("SELECT count(*) FROM public.users",&[]).await.unwrap().get(0);assert_eq!(count,if seed{1}else{0});
  if seed{let hash:String=session.client.query_one("SELECT password_hash FROM public.users",&[]).await.unwrap().get(0);assert!(crate::auth::password::verify_password(original_password.as_deref().unwrap(),&hash));assert!(fs::read_to_string(f.roots.app.join("secrets/admin_password")).unwrap()==std::env::var("ADMIN_PASSWORD").unwrap());assert_eq!(super::super::boot::Inputs::startup().admin_password,std::env::var("ADMIN_PASSWORD").ok());}
  else{assert!(!f.roots.app.join("secrets/admin_password").exists());assert!(crate::nonroot::envdir_value("ADMIN_PASSWORD").is_none());assert!(super::super::boot::Inputs::startup().admin_password.is_none());}
  if seed && turn==0{assert_rotated_login(&f,&server,config,original_password.as_deref().unwrap()).await;}
  drop(session);server.stop().unwrap();
 }
}

#[tokio::test]
#[ignore="A37 operator AUTH_SECRET rotation on ready PG17; isolated native container"]
async fn native_g37_operator_auth_rotates_ready_and_restarts_without_writes(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 use std::os::unix::fs::MetadataExt;
 let mut f=Fixture::new(17,Some(STORED),STORED).await;f.server.take().unwrap().stop().unwrap();
 let mut pair=crate::db_upgrade::pair::Pair::fresh(&f.roots.architecture);pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(postgres::read_control(17,&f.data).unwrap().system_identifier);
 for root in [&f.roots.db,&f.roots.app]{private(&root.join(".nwui"));crate::db_upgrade::files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).unwrap();}
 for name in ["auth_secret","db_password"]{fs::set_permissions(f.roots.app.join("secrets").join(name),fs::Permissions::from_mode(0o600)).unwrap();}
 private(Path::new("/run/s6/container_environment"));let publish=||crate::db_upgrade::fresh::ready_environment_locked(&f.locks,&f.roots,Path::new("/run/s6/container_environment")).unwrap();
 let claims=serde_json::json!({"sub":"g37-user","id":"g37-user"}).as_object().unwrap().clone();let now=chrono::Utc::now().timestamp();let cookie=crate::auth::session::COOKIE_NAME_PLAIN;let old_token=crate::auth::session::encode(&claims,"g37-stored-auth-secret",cookie,now,3600).unwrap();
 publish();let path=f.roots.app.join("secrets/auth_secret");let supplied=std::env::var("AUTH_SECRET").unwrap();
 assert!(fs::read_to_string(&path).unwrap()==supplied,"operator AUTH_SECRET must replace stored bytes");assert_eq!(fs::metadata(&path).unwrap().mode()&0o7777,0o600);assert_eq!(crate::nonroot::envdir_value("AUTH_SECRET"),Some(supplied.clone()));
 assert!(crate::auth::session::decode(&old_token,&[&supplied],cookie,now).is_err());let token=crate::auth::session::encode(&claims,&supplied,cookie,now,3600).unwrap();assert!(crate::auth::session::decode(&token,&[&supplied],cookie,now).is_ok());
 let before=fs::metadata(&path).unwrap();publish();let after=fs::metadata(&path).unwrap();assert_eq!((before.ino(),before.mtime(),before.mtime_nsec()),(after.ino(),after.mtime(),after.mtime_nsec()));
 // This native test runs alone in its own container on a current-thread runtime.
 unsafe{std::env::remove_var("AUTH_SECRET");}publish();assert_eq!(fs::read_to_string(&path).unwrap(),supplied);assert_eq!(crate::nonroot::envdir_value("AUTH_SECRET"),Some(supplied));
}

#[tokio::test]
#[ignore="A37 legacy equal operator secret retains inode/mtime and 0644"]
async fn native_g37_operator_equal_legacy_secret_is_not_written(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 use std::os::unix::fs::MetadataExt;
 let f=Fixture::new(17,Some(STORED),STORED).await;let path=f.roots.app.join("secrets/auth_secret");let before=fs::metadata(&path).unwrap();let app=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();f.publish();let after=fs::metadata(&path).unwrap();assert_eq!((before.ino(),before.mtime(),before.mtime_nsec(),before.mode()),(after.ino(),after.mtime(),after.mtime_nsec(),after.mode()));assert_eq!(after.mode()&0o7777,0o644);assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),app);
}
#[tokio::test]
#[ignore="A37 empty operator AUTH is absent; invalid bytes/text or stored file still refuse"]
async fn native_g37_operator_invalid_and_empty_values_preserve_material(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 use std::os::unix::ffi::OsStringExt;
 let f=Fixture::new(17,Some(STORED),STORED).await;let before=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();f.publish();assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),before);
 for value in [std::ffi::OsString::from_vec(vec![0xff]),std::ffi::OsString::from("x".repeat(65537))]{unsafe{std::env::set_var("AUTH_SECRET",value);}let error=crate::db_upgrade::fresh::ready_environment_locked(&f.locks,&f.roots,Path::new("/run/s6/container_environment")).unwrap_err();assert_eq!(error.code,"pair_mismatch");assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),before);}
 unsafe{std::env::set_var("AUTH_SECRET","g37-operator-auth-secret");}let path=f.roots.app.join("secrets/auth_secret");fs::remove_file(&path).unwrap();std::os::unix::fs::symlink("db_password",&path).unwrap();let before=fs::read(f.roots.app.join("secrets/db_password")).unwrap();let error=crate::db_upgrade::fresh::ready_environment_locked(&f.locks,&f.roots,Path::new("/run/s6/container_environment")).unwrap_err();assert_eq!(error.code,"pair_mismatch");assert_eq!(fs::read_link(&path).unwrap(),Path::new("db_password"));assert_eq!(fs::read(f.roots.app.join("secrets/db_password")).unwrap(),before);
}

#[tokio::test]
#[ignore="A37 first legacy boot admin seed uses supplied and persisted credentials"]
async fn native_g37_operator_admin_seeds_legacy_database(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();let inputs=super::super::boot::Inputs::startup();
 let mut config:tokio_postgres::Config=read_config().unwrap().database_url.parse().unwrap();config.host_path(&f.server.as_ref().unwrap().socket).port(50432);
 let manager=deadpool_postgres::Manager::new(config,tokio_postgres::NoTls);let pool=deadpool_postgres::Pool::builder(manager).max_size(2).runtime(deadpool_postgres::Runtime::Tokio1).build().unwrap();
 {let client=pool.get().await.unwrap();client.batch_execute("SET default_transaction_read_only=off; CREATE TABLE users(id text PRIMARY KEY,email text,password_hash text,role text)").await.unwrap();}
 let passwords=crate::auth::password::PasswordHasher::new(1);let seeded=crate::auth::setup::seed_admin_with(&pool,&passwords,inputs.admin_email.as_deref(),inputs.admin_password.as_deref()).await.unwrap();assert_eq!(seeded,crate::auth::setup::SeedOutcome::Seeded);
 let client=pool.get().await.unwrap();let hash:String=client.query_one("SELECT password_hash FROM public.users",&[]).await.unwrap().get(0);assert!(crate::auth::password::verify_password(&std::env::var("ADMIN_PASSWORD").unwrap(),&hash));assert!(fs::read_to_string(f.roots.app.join("secrets/admin_password")).unwrap()==std::env::var("ADMIN_PASSWORD").unwrap());drop(client);pool.close();
}

async fn assert_rotated_login(f:&Fixture,server:&postgres::Private,config:crate::config::RuntimeConfig,password:&str){
 use tower::ServiceExt;
 use axum::{body::Body,http::{Request,StatusCode},extract::ConnectInfo};
 let mut state=crate::state::AppState::new(config).unwrap();let pg=inspect::connection_config(&server.socket,"nginx_web_ui","nginx",Some(STORED));let mut pg=pg;pg.options("-c default_transaction_read_only=off");state.db=deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(pg,tokio_postgres::NoTls)).max_size(2).runtime(deadpool_postgres::Runtime::Tokio1).build().unwrap();
 let id:String=state.db.get().await.unwrap().query_one("SELECT id FROM public.users",&[]).await.unwrap().get(0);let claims=serde_json::json!({"sub":id,"id":id}).as_object().unwrap().clone();let old=crate::auth::session::encode(&claims,"g37-stored-auth-secret",crate::auth::session::COOKIE_NAME_PLAIN,chrono::Utc::now().timestamp(),3600).unwrap();
 let app=crate::app::with_request_context(crate::auth::router().with_state(state.clone()),None);
 let request=|method:&str,path:&str|Request::builder().method(method).uri(path).header("host","admin.example.test").header("x-real-ip","203.0.113.20").extension(ConnectInfo(std::net::SocketAddr::from(([127,0,0,1],40000))));
 let reply=app.clone().oneshot(request("GET","/api/ui/v1/session").header("cookie",format!("authjs.session-token={old}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(reply.status(),StatusCode::UNAUTHORIZED);
 let body=url::form_urlencoded::Serializer::new(String::new()).append_pair("email","g37-admin@example.test").append_pair("password",password).append_pair("totp","").finish();let reply=app.clone().oneshot(request("POST","/api/ui/v1/forms/login.loginAction").header("origin","http://admin.example.test").header("content-type","application/x-www-form-urlencoded").body(Body::from(body)).unwrap()).await.unwrap();assert_eq!(reply.status(),StatusCode::OK);
 let cookie=reply.headers().get_all("set-cookie").iter().filter_map(|v|v.to_str().ok()).find(|v|!v.contains("Max-Age=0")).expect("successful login issues a session").split(';').next().unwrap().to_owned();let reply=app.oneshot(request("GET","/api/ui/v1/session").header("cookie",cookie).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(reply.status(),StatusCode::OK);state.db.close();assert!(f.roots.app.join("secrets/auth_secret").exists());
}

#[tokio::test]
#[ignore="G41 native PG17 N2 config must ignore the inherited image DB fallback"]
async fn native_g41_n2_config_authenticates_published_password(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();
 let inherited:tokio_postgres::Config=std::env::var("DATABASE_URL").unwrap().parse().unwrap();
 assert!(inherited.get_password()==Some(b"nginx".as_slice()),"fixture must carry the image fallback");
 let config=crate::nonroot::config().unwrap();let credentials:tokio_postgres::Config=config.database_url.parse().unwrap();
 let password=std::str::from_utf8(credentials.get_password().unwrap()).unwrap();
 let session=inspect::connect(&f.server.as_ref().unwrap().socket,"nginx_web_ui","nginx",Some(password)).await;
 assert!(session.is_ok(),"N2 config must authenticate with the published stored credential");
 println!("G41_N2_AUTHENTICATES true");
}

#[tokio::test]
#[ignore="G41 native runner and N2 agree on published credentials; isolated process"]
async fn native_g41_runner_and_n2_credentials_agree(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();
 write(Path::new("/run/s6/container_environment/ADMIN_PASSWORD"),"g41-published-admin-only",0o600);
 // Every native row runs alone in a fresh container/current-thread test process.
 unsafe{std::env::set_var("AUTH_SECRET","g41-stale-process-auth");std::env::set_var("ADMIN_PASSWORD","g41-stale-process-admin");}
 let runner=read_config().unwrap();let n2=crate::nonroot::config().unwrap();
 let auth_agrees=runner.auth_secrets==n2.auth_secrets && runner.auth_secrets.last()==crate::nonroot::envdir_value("AUTH_SECRET").as_ref();
 let database_agrees=runner.database_url==n2.database_url && Some(runner.database_url.clone())==crate::nonroot::envdir_value("DATABASE_URL");
 // ADMIN_PASSWORD is a BootInputs value, not a RuntimeConfig field.
 let admin=crate::nonroot::envdir_value("ADMIN_PASSWORD");
 let admin_agrees=super::super::boot::Inputs::startup().admin_password==admin && crate::nonroot::runner_value("ADMIN_PASSWORD")==admin;
 assert!(auth_agrees && database_agrees && admin_agrees,"runner and N2 must share the three published credential sources");
 println!("G41_CREDENTIAL_AGREEMENT auth=true database=true admin=true");
}

#[tokio::test]
#[ignore="G41 native N2 config preserves process-first DATA_KEY_FILE"]
async fn native_g41_n2_data_key_override_stays_process_first(){
 if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
 let f=Fixture::new(17,Some(STORED),STORED).await;f.publish();
 write(Path::new("/run/s6/container_environment/DATA_KEY_FILE"),"/tmp/g41-not-the-process-key",0o600);
 unsafe{std::env::set_var("DATA_KEY_FILE","/tmp/g41-process-key-dir/data_key");}
 let n2=crate::nonroot::config().unwrap();let runner=read_config().unwrap();let expected=Path::new("/tmp/g41-process-key-dir/data_key");
 assert!(n2.data_key_file==expected && runner.data_key_file==expected,"non-credential startup precedence changed");
 println!("G41_DATA_KEY_PROCESS_FIRST true");
}
