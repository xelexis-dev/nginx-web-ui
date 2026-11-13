//! P0/P1 observe an inactive PG15, including a byte-preserved stale PID file.
use super::{StorageLocks,StepError};
use crate::db_upgrade::{layout::Roots,control::Control};
use std::{fs::{self,OpenOptions},io::Read,os::{fd::{AsRawFd,FromRawFd,OwnedFd},unix::{ffi::OsStrExt,fs::{MetadataExt,OpenOptionsExt}}},path::{Path,PathBuf}};
fn error(code:&'static str)->StepError {StepError{code,message:"이전 PostgreSQL의 실행 여부를 안전하게 확인하지 못했습니다. 원본을 변경하지 않았습니다.",detail:None}}
pub(super) fn allowed_state(state:&str)->bool {matches!(state,"shut down"|"shut down in recovery"|"in production"|"in crash recovery")}
pub(crate) fn control(control:&Control)->Result<(),StepError> {control.validate().map_err(|_|error("unsupported_layout"))?;if !allowed_state(&control.cluster_state){return Err(error("unsupported_layout"));}Ok(())}
fn no_listener(path:&Path)->Result<(),StepError> {
    let bytes=path.as_os_str().as_bytes();let mut addr:libc::sockaddr_un=unsafe{std::mem::zeroed()};if !path.is_absolute() || bytes.len()>=addr.sun_path.len() || bytes.contains(&0){return Err(error("unsupported_layout"));}addr.sun_family=libc::AF_UNIX as _;for (to,from) in addr.sun_path.iter_mut().zip(bytes){*to=*from as _;}
    let raw=unsafe{libc::socket(libc::AF_UNIX,libc::SOCK_STREAM|libc::SOCK_NONBLOCK|libc::SOCK_CLOEXEC,0)};if raw<0{return Err(error("io_error"));}let fd=unsafe{OwnedFd::from_raw_fd(raw)};
    if unsafe{libc::connect(fd.as_raw_fd(),(&addr as *const libc::sockaddr_un).cast(),std::mem::size_of::<libc::sockaddr_un>()as _)}==0{return Err(error("storage_busy"));}
    match std::io::Error::last_os_error().raw_os_error(){Some(libc::ENOENT|libc::ECONNREFUSED)=>Ok(()),Some(libc::EINPROGRESS|libc::EAGAIN)=>Err(error("storage_busy")),_=>Err(error("unsupported_layout"))}
}
fn not_this_postmaster(pid:i32,data:&Path)->Result<(),StepError> {
    if unsafe{libc::kill(pid,0)}!=0 {return if std::io::Error::last_os_error().raw_os_error()==Some(libc::ESRCH){Ok(())}else{Err(error("storage_busy"))};}
    let process=PathBuf::from(format!("/proc/{pid}"));let comm=fs::read_to_string(process.join("comm")).map_err(|_|error("storage_busy"))?;
    if !matches!(comm.trim(),"postgres"|"postmaster"){return Ok(());}
    // Reading cwd as postgres also works when root lacks CAP_SYS_PTRACE in the image.
    let mut c=std::process::Command::new("/usr/sbin/runuser");c.args(["-u","postgres","--","/usr/bin/readlink","-z","--"]).arg(process.join("cwd")).env_clear().env("PATH","/usr/bin:/bin");
    let (out,_)=crate::db_upgrade::postgres::output(c,std::time::Duration::from_secs(3)).map_err(|_|error("storage_busy"))?;
    let cwd=out.strip_suffix('\0').ok_or_else(||error("storage_busy"))?;let actual=fs::metadata(cwd).map_err(|_|error("storage_busy"))?;let source=fs::metadata(data).map_err(|_|error("storage_busy"))?;
    if actual.dev()==source.dev() && actual.ino()==source.ino(){return Err(error("storage_busy"));}Ok(())
}
fn pidfile(data:&Path)->Result<Option<String>,StepError> {pidfile_with(data,false)}
/// defer_mode skips only the repairable g/o-write comparison; every other check is unchanged.
fn pidfile_with(data:&Path,defer_mode:bool)->Result<Option<String>,StepError> {
    let path=data.join("postmaster.pid");let file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path){Ok(f)=>Some(f),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>None,Err(_)=>return Err(error("unsupported_layout"))};
    let Some(file)=file else{return Ok(None)};
    let m=file.metadata().map_err(|_|error("unsupported_layout"))?;if !m.is_file() || m.nlink()!=1 || m.len()==0 || m.len()>16384 || !defer_mode&&m.mode()&0o022!=0 || !(m.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())){return Err(error("unsupported_layout"));}
    let mut bytes=String::new();file.take(16385).read_to_string(&mut bytes).map_err(|_|error("unsupported_layout"))?;if bytes.len()>16384{return Err(error("unsupported_layout"));}Ok(Some(bytes))
}
/// Read-only R-restore liveness check; stale PID files stay available for work disposal.
pub(crate) fn require_no_postmaster(data:&Path)->Result<(),StepError> {
    let Some(text)=pidfile(data)? else{return Ok(())};
    let pid=text.lines().next().and_then(|s|s.parse::<i32>().ok()).filter(|pid|*pid>0).ok_or_else(||error("unsupported_layout"))?;
    not_this_postmaster(pid,data)
}
pub(crate) fn require_idle(roots:&Roots,locks:&StorageLocks,data:&Path)->Result<(),StepError> {idle(roots,locks,data,false)}
/// 2.0.1 R1-1: storage_modes runs this before narrowing a g/o-writable postmaster.pid.
pub(crate) fn require_idle_before_modes(roots:&Roots,locks:&StorageLocks,data:&Path)->Result<(),StepError> {idle(roots,locks,data,true)}
fn idle(roots:&Roots,locks:&StorageLocks,data:&Path,defer_mode:bool)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !(data==roots.db || data==roots.db.join("pgdata")){return Err(error("storage_busy"));}
    let socket=if let Some(bytes)=pidfile_with(data,defer_mode)? {
        let lines=bytes.lines().collect::<Vec<_>>();
        let pid=lines.first().and_then(|s|s.parse::<i32>().ok()).filter(|p|*p>0).ok_or_else(||error("unsupported_layout"))?;let port=lines.get(3).and_then(|s|s.parse::<u16>().ok()).filter(|p|*p>0).ok_or_else(||error("unsupported_layout"))?;let directory=lines.get(4).ok_or_else(||error("unsupported_layout"))?;
        not_this_postmaster(pid,data)?;Path::new(directory).join(format!(".s.PGSQL.{port}"))
    }else{PathBuf::from("/run/postgresql/.s.PGSQL.5432")};no_listener(&socket)
}
#[cfg(test)]
mod tests {
    use super::*;use std::os::unix::fs::{PermissionsExt,symlink};
    #[test]
    fn w1_source_state_accepts_only_frozen_inactive_or_unclean_states() {for s in ["shut down","shut down in recovery","in production","in crash recovery"]{assert!(allowed_state(s));}for s in ["starting up","in archive recovery","unknown",""]{assert!(!allowed_state(s));}}
    #[test]
    fn w1_source_idle_rejects_unsafe_pid_files_and_any_accepting_local_socket_without_mutation() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let socket=t.path().join(".s.PGSQL.50432");let listener=std::os::unix::net::UnixListener::bind(&socket).unwrap();assert_eq!(no_listener(&socket).unwrap_err().code,"storage_busy");drop(listener);fs::remove_file(socket).unwrap();
        let path=roots.db.join("postmaster.pid");fs::write(&path,format!("{}\n{}\n0\n50432\n{}\n",std::process::id(),roots.db.display(),t.path().display())).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();let before=fs::read(&path).unwrap();require_idle(&roots,&locks,&roots.db).unwrap();assert_eq!(fs::read(&path).unwrap(),before);
        fs::remove_file(&path).unwrap();symlink(t.path().join("absent"),&path).unwrap();assert_eq!(require_idle(&roots,&locks,&roots.db).unwrap_err().code,"unsupported_layout");
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15: SIGKILL original, byte-exact P1, private recovery, R-restore, recovery again"]
    async fn w1_unclean_pg15_backup_recovery_restore_preserves_original_bytes_and_old_boot() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::{files,postgres};use super::super::{Branch,Journal,Step,Outcome,backup,backup_format::SourceLocation,journal::Candidate};use std::{os::unix::fs::PermissionsExt,sync::Arc};
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};postgres::directory(&roots.db).unwrap();files::private_dir(&roots.app).unwrap();let logs=t.path().join("logs");postgres::directory(&logs).unwrap();let init=logs.join("init");postgres::directory(&init).unwrap();let mut cmd=postgres::command(15,"initdb").unwrap();cmd.arg("-D").arg(&roots.db).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(cmd,&init,std::time::Duration::from_secs(300)).unwrap();
        let first=logs.join("first");postgres::directory(&first).unwrap();let server=postgres::Private::start(15,&roots.db,&first).unwrap();let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());assert_eq!(require_idle(&roots,&locks,&roots.db).unwrap_err().code,"storage_busy");
        let mut config=tokio_postgres::Config::new();config.host_path(&server.socket).port(50432).user("postgres").dbname("postgres");let(client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);client.batch_execute("CREATE TABLE public.crash_probe(id int PRIMARY KEY,value text); INSERT INTO public.crash_probe VALUES(1,'durable before kill')").await.unwrap();drop(client);driver.await.unwrap().unwrap();
        let pid:i32=fs::read_to_string(roots.db.join("postmaster.pid")).unwrap().lines().next().unwrap().parse().unwrap();assert!(pid>1);assert_eq!(unsafe{libc::kill(pid,libc::SIGKILL)},0);drop(server);let before=files::snapshot(&roots.db).unwrap();let control=postgres::read_control(15,&roots.db).unwrap();assert_eq!(control.cluster_state,"in production");require_idle(&roots,&locks,&roots.db).unwrap();
        let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};let names=fs::read_dir(&roots.db).unwrap().map(|e|e.unwrap().file_name().into_string().unwrap()).collect();let prepared=backup::prepare(&roots,&locks,&j.load().attempt_id,SourceLocation::Root,&names,&control,"2026-10-29T00:00:00Z").unwrap();assert_eq!(prepared.header.control_state,"in production");backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();let saved=roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id);assert_eq!(files::snapshot(&saved).unwrap(),before);assert!(saved.join("postmaster.pid").is_file());
        for recover in [false,true] {
            let log=logs.join(if recover{"restored"}else{"recovery"});postgres::directory(&log).unwrap();let server=postgres::Private::start_locked_legacy(&locks,&roots,&roots.db,&log).unwrap();let mut config=tokio_postgres::Config::new();config.host_path(&server.socket).port(50432).user("postgres").dbname("postgres");let(client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);assert_eq!(client.query_one("SELECT value FROM public.crash_probe WHERE id=1",&[]).await.unwrap().get::<_,String>(0),"durable before kill");drop(client);driver.await.unwrap().unwrap();server.stop().unwrap();
            if !recover {j=super::super::restore::recover(roots.clone(),locks.clone(),j,Step::P3Convert,"pg_upgrade_check_failed",super::super::boot_identity::current().ok()).await.unwrap();assert_eq!(j.load().outcome,Outcome::Restored);backup::verify_restored(&roots,&j).unwrap();assert_eq!(postgres::read_control(15,&roots.db).unwrap().cluster_state,"in production");assert!(roots.db.join("postmaster.pid").is_file());}
        }
    }
}
