#![cfg(unix)]
use nwui_api::{crypto::SecretBox,db_upgrade::{self,files,layout::{self,Roots},pair::Pair,key_binding}};
use std::{fs,process::{Child,Command,Stdio},time::{Duration,Instant}};
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self){if self.0.try_wait().ok().flatten().is_some(){return;}unsafe{libc::kill(self.0.id() as i32,libc::SIGTERM);};let end=Instant::now()+Duration::from_secs(10);while Instant::now()<end{if self.0.try_wait().ok().flatten().is_some(){return;}std::thread::sleep(Duration::from_millis(20));}let _=self.0.kill();let _=self.0.wait();}
}
#[tokio::test]
#[ignore="native PG17: NWUI_TEST_W5_LOCK=1 in disposable roots, no host ports"]
async fn production_lock_is_shared_inherited_and_released_after_direct_sigterm(){
    if std::env::var("NWUI_TEST_W5_LOCK").as_deref()!=Ok("1"){return;}
    let roots=Roots{db:"/var/lib/postgresql/data".into(),app:"/var/lib/nginx-web-ui".into(),architecture:db_upgrade::paths::architecture().unwrap().into()};
    fs::create_dir_all("/run/s6/container_environment").unwrap();
    db_upgrade::fresh::secrets(&roots,&db_upgrade::fresh::Inputs::default(),std::path::Path::new("/run/s6/container_environment")).unwrap();
    db_upgrade::fresh::provision(&roots).await.unwrap();
    let fixture=tempfile::tempdir_in("/run").unwrap();let gate=fixture.path().join("nwui-gate.sh");
    // Selection is already established by this fixture; keep the real postmaster/lock assertions below.
    fs::write(&gate,"nwui_gate_longrun() { :; }\n").unwrap();
    let source=include_str!("../../../docker/s6-rc.d/postgres/run");
    assert!(source.find("nwui_gate_longrun").unwrap()<source.find("exec 9<").unwrap());
    let script=fixture.path().join("postgres-run");fs::write(&script,source.replace("/etc/s6-overlay/scripts/nwui-gate.sh",gate.to_str().unwrap())).unwrap();
    let admission=fs::File::open(&roots.db).unwrap();admission.try_lock().unwrap();
    let mut blocked=Running(Command::new("sh").arg(&script).stdout(Stdio::null()).stderr(fs::File::create("/run/w5-blocked.log").unwrap()).spawn().unwrap());
    let end=Instant::now()+Duration::from_secs(3);loop{if let Some(status)=blocked.0.try_wait().unwrap(){assert_eq!(status.code(),Some(1));break;}assert!(Instant::now()<end,"production did not refuse a held exclusive lock");tokio::time::sleep(Duration::from_millis(20)).await;}
    assert!(fs::read_to_string("/run/w5-blocked.log").unwrap().contains("storage_busy"));assert!(!roots.db.join("17/data/postmaster.pid").exists());drop(admission);
    let log=fs::File::create("/run/w5-postgres.log").unwrap();
    let mut pg=Running(Command::new("sh").arg(script).stdout(Stdio::null()).stderr(log).spawn().unwrap());
    let end=Instant::now()+Duration::from_secs(15);
    loop{assert!(pg.0.try_wait().unwrap().is_none(),"production exited before readiness: {}",fs::read_to_string("/run/w5-postgres.log").unwrap());if Command::new("/usr/lib/postgresql/17/bin/pg_isready").args(["-h","127.0.0.1","-U","postgres"]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success(){break;}assert!(Instant::now()<end,"startup timed out");tokio::time::sleep(Duration::from_millis(30)).await;}
    let exclusive=fs::File::open(&roots.db).unwrap();assert!(exclusive.try_lock().is_err(),"running postmaster did not retain PGROOT lock");
    let shared=fs::File::open(&roots.db).unwrap();shared.try_lock_shared().unwrap();drop(shared);
    assert_eq!(fs::read_to_string(format!("/proc/{}/comm",pg.0.id())).unwrap().trim(),"postgres");
    let path=roots.app.join("secrets/data_key");fs::write(&path,"A5".repeat(32)).unwrap();
    fs::set_permissions(&path,std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let key=SecretBox::load_existing(&path).unwrap();
    key_binding::mark_key_required(&roots,&|data|Ok(db_upgrade::postgres::read_control(17,data)?.system_identifier),&key).unwrap();
    assert_eq!(layout::prepare(&roots,&|data|Ok(db_upgrade::postgres::read_control(17,data)?.system_identifier)).unwrap_err().code,"storage_busy");
    assert_eq!(unsafe{libc::kill(pg.0.id() as i32,libc::SIGTERM)},0);
    let end=Instant::now()+Duration::from_secs(10);loop{if let Some(status)=pg.0.try_wait().unwrap(){assert!(status.success());break;}assert!(Instant::now()<end,"SIGTERM did not stop supervised postmaster");tokio::time::sleep(Duration::from_millis(25)).await;}
    assert!(!roots.db.join("17/data/postmaster.pid").exists());exclusive.try_lock().unwrap();drop(exclusive);
    assert!(layout::prepare(&roots,&|data|Ok(db_upgrade::postgres::read_control(17,data)?.system_identifier)).is_ok());
}
