pub use nwui_api::{upgrade,nonroot,certificates};
#[path="../src/crypto.rs"] mod crypto;
#[path="../src/db_upgrade/mod.rs"] mod db_upgrade;
use db_upgrade::{postgres,files};use std::{fs,path::Path,time::Duration};
fn work()->std::path::PathBuf{let path=std::env::temp_dir().join(format!("nwui-pg-{}",uuid::Uuid::new_v4()));postgres::directory(&path).unwrap();path}
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
    let result=db_upgrade::storage::cancellable(async {
        let log=work();let pg=postgres::Private::start(17,&data,&log)?;let pid=fs::read_to_string(data.join("postmaster.pid")).unwrap().lines().next().unwrap().parse::<i32>().unwrap();*ready.lock().unwrap()=Some((pg.socket.clone(),pid));
        let held=pg;let result=std::future::pending::<db_upgrade::error::Result<()>>().await;drop(held);result
    }).await;
    assert!(signal.join().unwrap());assert_eq!(result.unwrap_err().exit,143);
    let (socket,pid)=ready.lock().unwrap().clone().unwrap();assert!(!socket.exists());assert!(!data.join("postmaster.pid").exists());assert_eq!(unsafe{libc::kill(pid,0)},-1);assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
}
