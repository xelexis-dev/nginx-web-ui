//! Explicit binaries and private, socket-only PostgreSQL children running as postgres.
use super::{control::Control,files,error::{Error,Result}};
use std::{fs::{self,File,OpenOptions},io::Read,path::{Path,PathBuf},process::{Command,Child,Stdio},sync::atomic::{AtomicI32,Ordering},time::{Duration,Instant}};
use std::os::fd::AsRawFd;
use std::os::unix::{fs::{DirBuilderExt,OpenOptionsExt,MetadataExt},process::CommandExt};
pub static SIGNAL:AtomicI32=AtomicI32::new(0);
fn failed()->Error{Error{code:"postgres_failed",message:"전용 PostgreSQL 작업을 완료하지 못했습니다. 원본과 실패한 작업은 보존했습니다. 제한된 작업 로그를 확인하세요.",exit:1,detail:None}}
pub fn interrupted()->Option<Error>{match SIGNAL.load(Ordering::Relaxed){signal@ (2|15)=>Some(Error{code:"interrupted",message:"이전 작업이 중단됐습니다. 원본과 현재 작업을 보존했습니다.",exit:128+signal,detail:None}),_=>None}}
pub fn command(major:u8,tool:&str)->Result<Command> {
    if ![15,17].contains(&major) || !["postgres","pg_ctl","pg_controldata","pg_isready","pg_upgrade","initdb","vacuumdb","pg_dump","psql"].contains(&tool){return Err(Error::refusal("unsupported_layout"));}
    let mut c=Command::new("/usr/sbin/runuser");
    c.args(["-u","postgres","--"]).arg(format!("/usr/lib/postgresql/{major}/bin/{tool}"));
    c.env_clear().env("HOME","/var/lib/postgresql").env("PATH","/usr/bin:/bin").env("LC_ALL","C").env("TZ","UTC");
    c.stdin(Stdio::null()).process_group(0);Ok(c)
}
pub fn server_command(major:u8,data:&Path,socket:&Path,config:&Path)->Result<Command> {
    use std::os::unix::ffi::OsStrExt;
    if socket.as_os_str().as_bytes().len()+"/.s.PGSQL.50432".len()>=108{return Err(Error::refusal("unsupported_layout"));}
    let mut c=command(major,"postgres")?;c.arg("-D").arg(data);
    for setting in [
        format!("data_directory={}",data.display()),format!("config_file={}",config.display()),
        format!("hba_file={}",config.with_file_name("hba.conf").display()),
        format!("ident_file={}",config.with_file_name("ident.conf").display()),
        format!("unix_socket_directories={}",socket.display()),
        "listen_addresses=".into(),"port=50432".into(),"unix_socket_permissions=0700".into(),
        "shared_preload_libraries=".into(),"session_preload_libraries=".into(),"local_preload_libraries=".into(),
        "archive_mode=off".into(),"archive_command=".into(),"restore_command=".into(),"primary_conninfo=".into(),
        "ssl=off".into(),"logging_collector=off".into(),"log_destination=stderr".into(),"external_pid_file=".into(),
        "log_statement=none".into(),"log_min_error_statement=panic".into(),"log_parameter_max_length_on_error=0".into(),
        "max_logical_replication_workers=0".into(),"max_sync_workers_per_subscription=0".into(),
        "max_connections=10".into(),"shared_buffers=16MB".into(),"max_worker_processes=0".into(),"autovacuum=off".into(),
    ]{c.arg("-c").arg(setting);}Ok(c)
}
pub fn wait(child:Child,timeout:Duration)->Result<()> {wait_inner(child,timeout,true)}
fn wait_inner(mut child:Child,timeout:Duration,cancellable:bool)->Result<()> {
    let start=Instant::now();
    loop {
        match child.try_wait(){Ok(Some(status))=>return if status.success(){Ok(())}else{Err(failed())},Err(_)=>{let _=child.kill();let _=child.wait();return Err(failed());},_=>{}}
        let cancel=if cancellable{interrupted()}else{None};
        if cancel.is_some() || start.elapsed()>=timeout {
            unsafe{libc::kill(-(child.id()as i32),libc::SIGKILL);}
            let _=child.kill();let _=child.wait();return Err(cancel.unwrap_or_else(failed));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub(crate) fn postgres_ids()->Result<(u32,u32)> {
    let passwd=fs::read_to_string("/etc/passwd").map_err(|_|failed())?;
    let fields=passwd.lines().find(|line|line.starts_with("postgres:")).ok_or_else(failed)?.split(':').collect::<Vec<_>>();
    Ok((fields.get(2).and_then(|s|s.parse().ok()).ok_or_else(failed)?,fields.get(3).and_then(|s|s.parse().ok()).ok_or_else(failed)?))
}
fn owner_matches(actual:(u32,u32),expected:(u32,u32),kind:Option<super::shared_folder::SharedKind>)->bool{
    actual==expected || actual==(0,0)&&kind==Some(super::shared_folder::SharedKind::OrbStackMac)
}
fn owned(file:&File,uid:u32,gid:u32)->bool{
    file.metadata().is_ok_and(|m|owner_matches((m.uid(),m.gid()),(uid,gid),if (m.uid(),m.gid())==(uid,gid){None}else{super::shared_folder::kind(file)}))
}
fn ownership_after_chown(success:bool,owned:impl Fn()->bool,fallback:impl FnOnce()->Result<()>)->Result<()>{
    if !success||!owned(){fallback()?;}
    if !owned(){return Err(failed());}Ok(())
}
pub fn own(path:&Path)->Result<()> {
    files::real_path(path).map_err(|_|failed())?;let(uid,gid)=postgres_ids()?;
    let before=fs::symlink_metadata(path).map_err(|_|failed())?;
    if !before.is_file()&&!before.is_dir(){return Err(failed());}
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path).map_err(|_|failed())?;
    let same=||file.metadata().ok().zip(fs::symlink_metadata(path).ok()).is_some_and(|(m,p)|(m.dev(),m.ino())==(before.dev(),before.ino())&&(p.dev(),p.ino())==(m.dev(),m.ino()));
    if !same(){return Err(failed());}
    let changed=unsafe{libc::fchown(file.as_raw_fd(),uid,gid)};
    ownership_after_chown(changed==0,||same()&&owned(&file,uid,gid),||{
        let mut c=Command::new("/usr/sbin/runuser");c.args(["-u","postgres","--","/usr/bin/chown","-h","postgres:postgres"]).arg(path).env_clear().env("PATH","/usr/bin:/bin").stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
        wait(c.spawn().map_err(|_|failed())?,Duration::from_secs(30))
    })
}
// Only the independent DB copy is normalized; application ownership stays unchanged.
pub fn own_tree(path:&Path)->Result<()> {
    fn walk(path:&Path,depth:u8,count:&mut u64)->Result<()> {
        *count+=1;if depth>64||*count>1_000_000{return Err(failed());}
        let metadata=fs::symlink_metadata(path).map_err(|_|failed())?;
        if !metadata.is_file()&&!metadata.is_dir(){return Err(failed());}
        own(path)?;
        if metadata.is_dir(){for child in fs::read_dir(path).map_err(|_|failed())?{walk(&child.map_err(|_|failed())?.path(),depth+1,count)?;}}Ok(())
    }
    files::real_path(path).map_err(|_|failed())?;walk(path,0,&mut 0)
}
// The copy never contains a source PID file; only our tools may publish this one.
pub fn stop_working(major:u8,data:&Path)->Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path=data.join("postmaster.pid");
    let metadata=match fs::symlink_metadata(&path){Ok(m)=>m,Err(e)if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(failed())};
    files::real_path(&path).map_err(|_|failed())?;
    if !metadata.is_file()||metadata.len()>65536{return Err(failed());}
    let pid=text(&path)?;let mut lines=pid.lines();let pid:i32=lines.next().ok_or_else(failed)?.parse().map_err(|_|failed())?;
    if pid<=1||lines.next()!=data.to_str(){return Err(failed());}
    if unsafe{libc::kill(pid,0)}!=0 && std::io::Error::last_os_error().raw_os_error()==Some(libc::ESRCH){return Ok(());}
    // Mac bind mounts remap file owners; bind the live process to postgres and this data dir.
    let process=Path::new("/proc").join(pid.to_string());let status=text(&process.join("status"))?;
    let uid=postgres_ids()?.0;let mut uids=status.lines().find_map(|line|line.strip_prefix("Uid:")).ok_or_else(failed)?.split_whitespace();
    if uids.next().and_then(|s|s.parse::<u32>().ok())!=Some(uid)||uids.next().and_then(|s|s.parse::<u32>().ok())!=Some(uid){return Err(failed());}
    let mut read=Command::new("/usr/sbin/runuser");read.args(["-u","postgres","--","/usr/bin/readlink","-z","--"]).arg(process.join("cwd"))
        .env_clear().env("PATH","/usr/bin:/bin").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
    let mut child=read.spawn().map_err(|_|failed())?;let stdout=child.stdout.take().ok_or_else(failed)?;
    // Cleanup must still inspect and stop the child when SIGNAL is already set.
    wait_inner(child,Duration::from_secs(5),false)?;
    let mut cwd=Vec::new();stdout.take(65537).read_to_end(&mut cwd).map_err(|_|failed())?;
    if cwd.len()>65536||cwd.strip_suffix(&[0])!=Some(fs::canonicalize(data).map_err(|_|failed())?.as_os_str().as_bytes()){return Err(failed());}
    let mut c=command(major,"pg_ctl")?;
    c.arg("-D").arg(data).args(["-m","fast","-t","30","-w","stop"]).stdout(Stdio::null()).stderr(Stdio::null());
    wait_inner(c.spawn().map_err(|_|failed())?,Duration::from_secs(40),false)
}
pub fn directory(path:&Path)->Result<()> {directory_io(path).map_err(|_|failed())}
pub(crate) fn directory_io(path:&Path)->std::io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    own(path).map_err(std::io::Error::other)?;
    File::open(path.parent().ok_or_else(||std::io::Error::other(failed()))?)?.sync_all()
}
pub fn private_file(path:&Path,bytes:&[u8])->Result<()> {
    use std::io::Write;
    let mut f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|_|failed())?;
    f.write_all(bytes).and_then(|_|f.sync_all()).map_err(|_|failed())?;own(path)?;f.sync_all().map_err(|_|failed())?;File::open(path.parent().ok_or_else(failed)?).and_then(|f|f.sync_all()).map_err(|_|failed())
}
fn logfile(path:&Path)->Result<File> {let f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|_|failed())?;own(path)?;Ok(f)}
fn text(path:&Path)->Result<String> {let mut s=String::new();File::open(path).map_err(|_|failed())?.take(65537).read_to_string(&mut s).map_err(|_|failed())?;if s.len()>65536{return Err(failed());}Ok(s)}
pub fn capture(mut command:Command,logdir:&Path,timeout:Duration)->Result<(String,String)> {
    if let Some(e)=interrupted(){return Err(e);}
    let id=uuid::Uuid::new_v4();let out=logdir.join(format!("{id}.stdout"));let err=logdir.join(format!("{id}.stderr"));
    command.stdout(logfile(&out)?).stderr(logfile(&err)?);
    wait(command.spawn().map_err(|_|failed())?,timeout)?;
    Ok((text(&out)?,text(&err)?))
}
// Control/status reads leave no log or temporary file on either storage volume.
pub fn read_control(major:u8,data:&Path)->Result<Control> {
    let mut c=command(major,"pg_controldata")?;c.arg(data);
    let(out,err)=output(c,Duration::from_secs(30))?;Control::parse(&out,&err)
}
pub fn output(mut command:Command,timeout:Duration)->Result<(String,String)> {
    if let Some(e)=interrupted(){return Err(e);}
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child=command.spawn().map_err(|_|failed())?;
    let stdout=child.stdout.take().ok_or_else(failed)?;let stderr=child.stderr.take().ok_or_else(failed)?;
    fn read(pipe:impl Read)->std::io::Result<Vec<u8>>{let mut bytes=Vec::new();pipe.take(65537).read_to_end(&mut bytes)?;Ok(bytes)}
    let out=std::thread::spawn(move||read(stdout));let err=std::thread::spawn(move||read(stderr));
    let status=wait(child,timeout);
    let out=out.join().map_err(|_|failed())?.map_err(|_|failed())?;
    let err=err.join().map_err(|_|failed())?.map_err(|_|failed())?;
    status?;
    if out.len()>65536 || err.len()>65536{return Err(failed());}
    Ok((String::from_utf8(out).map_err(|_|failed())?,String::from_utf8(err).map_err(|_|failed())?))
}
// VirtioFS cannot host PostgreSQL socket chmod; only data/logs belong on the volume.
pub(crate) fn socket_path(work:&Path,label:&str)->PathBuf {
    use std::os::unix::ffi::OsStrExt;
    use sha2::Digest;
    let id=hex::encode(sha2::Sha256::digest(work.as_os_str().as_bytes()));
    Path::new("/run").join(format!("nwui-pg-{}-{label}",&id[..32]))
}
pub(super) struct SocketDirectory {pub path:PathBuf,directory:File,owner:u32,creator:u32}
impl SocketDirectory {
    pub fn new(work:&Path,label:&str)->Result<Self> {
        let (uid,gid)=postgres_ids()?;Self::create(socket_path(work,label),uid,gid)
    }
    fn create(path:PathBuf,uid:u32,gid:u32)->Result<Self> {
        use std::os::fd::AsRawFd;
        let parent=path.parent().ok_or_else(failed)?;files::real_path(parent).map_err(|_|failed())?;
        fs::DirBuilder::new().mode(0o700).create(&path).map_err(|_|failed())?;
        let directory=match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW).open(&path) {
            Ok(directory)=>directory,
            // Nothing has used this new directory yet; rmdir cannot remove contents or follow links.
            Err(_)=>{let _=fs::remove_dir(&path);return Err(failed());},
        };
        let creator=unsafe{libc::geteuid()};
        let guard=Self{path,directory,owner:uid,creator};
        if unsafe{libc::fchown(guard.directory.as_raw_fd(),uid,gid)}!=0{return Err(failed());}
        let meta=guard.directory.metadata().map_err(|_|failed())?;
        if meta.uid()!=uid||meta.gid()!=gid||meta.mode()&0o7777!=0o700{return Err(failed());}
        File::open(guard.path.parent().ok_or_else(failed)?).and_then(|f|f.sync_all()).map_err(|_|failed())?;Ok(guard)
    }
}
impl Drop for SocketDirectory {
    fn drop(&mut self) {
        let Ok(original)=self.directory.metadata() else{return;};
        let Ok(current)=fs::symlink_metadata(&self.path) else{return;};
        // Keep the original inode open so a replacement cannot reuse its identity.
        if !current.is_dir()||current.dev()!=original.dev()||current.ino()!=original.ino()||![self.owner,self.creator].contains(&current.uid())||current.mode()&0o7777!=0o700{return;}
        let Ok(entries)=fs::read_dir(&self.path) else{return;};
        for entry in entries {
            let Ok(entry)=entry else{return;};let name=entry.file_name();let name=name.as_encoded_bytes();
            if !name.starts_with(b".s.PGSQL.")||!name.ends_with(b".lock"){continue;}
            let file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(entry.path()) {
                Ok(file)=>file,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>continue,Err(_)=>return,
            };
            let Ok(meta)=file.metadata() else{return;};if !meta.is_file()||meta.nlink()!=1||meta.len()>65536{return;}
            let mut value=String::new();if file.take(65537).read_to_string(&mut value).is_err()||value.len()>65536{return;}
            let Some(pid)=value.lines().next().and_then(|s|s.parse::<i32>().ok()).filter(|pid|*pid>1) else{return;};
            if unsafe{libc::kill(pid,0)}==0||std::io::Error::last_os_error().raw_os_error()!=Some(libc::ESRCH){return;}
        }
        // remove_dir_all does not follow child symlinks; data and logs never live here.
        if fs::remove_dir_all(&self.path).is_err(){eprintln!("전용 PostgreSQL 소켓 디렉터리를 정리하지 못했습니다.");}
    }
}

#[cfg(test)]
mod socket_cleanup_tests {
    use super::*;
    fn create(path:PathBuf)->SocketDirectory{SocketDirectory::create(path,unsafe{libc::geteuid()},unsafe{libc::getegid()}).unwrap()}
    #[test]
    fn removes_owned_runtime_entries_but_never_follows_child_symlinks(){
        let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let outside=root.join("outside");fs::create_dir(&outside).unwrap();fs::write(outside.join("keep"),"keep").unwrap();
        let guard=create(root.join("socket"));fs::write(guard.path.join("stale"),"owned").unwrap();std::os::unix::fs::symlink(&outside,guard.path.join("link")).unwrap();let path=guard.path.clone();drop(guard);
        assert!(!path.exists());assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(),"keep");
    }
    #[test]
    fn never_removes_a_replaced_directory_or_root_symlink(){
        for symlink in [false,true]{
            let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let guard=create(root.join("socket"));fs::rename(&guard.path,root.join("original")).unwrap();
            let foreign=root.join("foreign");fs::create_dir(&foreign).unwrap();fs::write(foreign.join("keep"),"keep").unwrap();
            if symlink{std::os::unix::fs::symlink(&foreign,&guard.path).unwrap();}else{fs::DirBuilder::new().mode(0o700).create(&guard.path).unwrap();fs::write(guard.path.join("keep"),"keep").unwrap();}
            let path=guard.path.clone();drop(guard);assert!(fs::symlink_metadata(path).is_ok());assert_eq!(fs::read_to_string(foreign.join("keep")).unwrap(),"keep");assert!(root.join("original").exists());
        }
    }
    #[test]
    fn an_unlisted_port_with_a_live_lock_is_not_removed(){
        let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let guard=create(root.join("socket"));fs::write(guard.path.join(".s.PGSQL.55444.lock"),format!("{}\n",std::process::id())).unwrap();let path=guard.path.clone();drop(guard);assert!(path.exists());
    }
    #[test]
    fn fifo_lock_files_do_not_block_or_get_adopted_for_cleanup(){
        use std::os::unix::ffi::OsStrExt;
        let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let guard=create(root.join("socket"));let path=guard.path.clone();let lock=std::ffi::CString::new(path.join(".s.PGSQL.50432.lock").as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(lock.as_ptr(),0o600)},0);
        let (send,receive)=std::sync::mpsc::channel();let thread=std::thread::spawn(move||{drop(guard);send.send(()).unwrap();});receive.recv_timeout(Duration::from_secs(5)).expect("FIFO lock blocked cleanup");thread.join().unwrap();assert!(path.exists());
    }
    #[test]
    fn leaves_a_live_postmaster_socket_directory_in_place(){
        let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let guard=create(root.join("socket"));fs::write(guard.path.join(".s.PGSQL.50432.lock"),format!("{}\n",std::process::id())).unwrap();let path=guard.path.clone();drop(guard);assert!(path.exists());
    }
}

// Only the already-locked ready path delegates stale-PID handling to PostgreSQL.
fn ready_pid(data:&Path)->Result<()> {inactive_pid(data,false)}
fn pid_metadata(m:&fs::Metadata,uid:u32,source:bool,kind:Option<super::shared_folder::SharedKind>)->bool{
    m.is_file()&&m.nlink()==1&&m.len()>0&&m.len()<=16384&&m.mode()&0o7777==0o600
        &&(m.uid()==uid || source&&m.uid()==0 || !source&&m.uid()==0&&m.gid()==0&&kind==Some(super::shared_folder::SharedKind::OrbStackMac))
}
fn inactive_pid(data:&Path,source:bool)->Result<()> {
    let path=data.join("postmaster.pid");
    let file=match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&path) {
        Ok(file)=>file,Err(e)if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),Err(_)=>return Err(Error::refusal("unsupported_layout")),
    };
    let m=file.metadata().map_err(|_|failed())?;
    if !pid_metadata(&m,postgres_ids()?.0,source,super::shared_folder::kind(&file)){return Err(Error::refusal("unsupported_layout"));}
    Ok(())
}
pub struct Private {child:Child,pub socket:PathBuf,data:PathBuf,major:u8,_socket_directory:SocketDirectory}
impl Private {
    pub fn start(major:u8,data:&Path,work:&Path)->Result<Self> {
        Self::start_with_limits(major,data,work,Duration::from_secs(30),Duration::from_secs(900))
    }
    /// The PG17 ready branch already holds both root locks. PostgreSQL itself adjudicates a
    /// leftover postmaster.pid and performs crash recovery; root never removes that file.
    pub(crate) fn start_locked_ready(locks:&crate::upgrade::StorageLocks,roots:&super::layout::Roots,work:&Path)->Result<Self> {
        if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(Error::refusal("storage_busy"));}
        Self::start_inner(17,&roots.db.join("17/data"),work,Duration::from_secs(30),Duration::from_secs(900),true)
    }
    /// P0/P1 already proved this source inactive and backed up its exact unclean bytes.
    /// Only PostgreSQL may adjudicate/remove the stale PID and recover the original.
    pub(crate) fn start_locked_legacy(locks:&crate::upgrade::StorageLocks,roots:&super::layout::Roots,data:&Path,work:&Path)->Result<Self> {
        if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !(data==roots.db || data==roots.db.join("pgdata")){return Err(Error::refusal("storage_busy"));}
        Self::start_inner(15,data,work,Duration::from_secs(30),Duration::from_secs(900),true)
    }
    pub(crate) fn start_with_limits(major:u8,data:&Path,work:&Path,normal:Duration,recovery:Duration)->Result<Self> {
        Self::start_inner(major,data,work,normal,recovery,false)
    }
    fn start_inner(major:u8,data:&Path,work:&Path,normal:Duration,recovery:Duration,ready:bool)->Result<Self> {
        if normal.is_zero() || recovery<normal{return Err(Error::refusal("unsupported_layout"));}
        files::real_path(data).map_err(|_|failed())?;files::real_path(work).map_err(|_|failed())?;
        // Never honor ALTER SYSTEM settings from a source, or adopt a live postmaster.
        if ready {if major==15{inactive_pid(data,true)?;}else{ready_pid(data)?;}}else if data.join("postmaster.pid").exists(){return Err(Error::refusal("unsupported_layout"));}
        let auto=data.join("postgresql.auto.conf");files::real_path(&auto).map_err(|_|failed())?;
        if text(&auto)?.lines().any(|line|!line.trim().is_empty()&&!line.trim_start().starts_with('#')){return Err(Error::refusal("unsupported_layout"));}
        let socket_directory=SocketDirectory::new(work,&format!("s{major}"))?;let socket=socket_directory.path.clone();
        let config=work.join("private.conf");private_file(&config,b"# Dedicated offline postmaster\n")?;
        private_file(&work.join("hba.conf"),b"local all postgres trust\nlocal all nginx scram-sha-256\nlocal all all reject\n")?;
        private_file(&work.join("ident.conf"),b"")?;
        let mut command=server_command(major,data,&socket,&config)?;
        command.stdout(logfile(&work.join("server.stdout"))?).stderr(logfile(&work.join("server.stderr"))?);
        let child=command.spawn().map_err(|_|failed())?;let mut pg=Self{child,socket,data:data.into(),major,_socket_directory:socket_directory};
        let started=Instant::now();
        let normal_deadline=started+normal;let recovery_deadline=started+recovery;let mut deadline=normal_deadline;
        loop {
            if let Some(error)=interrupted(){return Err(error);}
            if pg.child.try_wait().map_err(|_|failed())?.is_some() || Instant::now()>=deadline{return Err(failed());}
            match probe_before(major,&pg.socket,deadline)? {
                0=>{if Instant::now()>=deadline{return Err(failed());}return Ok(pg);},
                1=>deadline=recovery_deadline,
                2=>deadline=normal_deadline,
                _=>return Err(failed()),
            }
            // Silence after recovery does not inherit the extended recovery allowance.
            if Instant::now()>=deadline{return Err(failed());}
            std::thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())));
        }
    }
    pub fn stop(mut self)->Result<()> {self.shutdown()}
    fn shutdown(&mut self)->Result<()> {
        if let Some(status)=self.child.try_wait().map_err(|_|failed())?{return if status.success(){Ok(())}else{Err(failed())};}
        let mut c=command(self.major,"pg_ctl")?;
        c.arg("-D").arg(&self.data).args(["-m","fast","-t","30","-w","stop"]).stdout(Stdio::null()).stderr(Stdio::null());
        let result=wait_inner(c.spawn().map_err(|_|failed())?,Duration::from_secs(40),false);
        if result.is_err(){unsafe{libc::kill(-(self.child.id()as i32),libc::SIGKILL);}let _=self.child.kill();}
        let _=self.child.wait();result
    }
}
fn command_probe(major:u8,socket:&Path)->Result<Command>{let mut c=command(major,"pg_isready")?;c.env("PGOPTIONS",super::settings::ROOT_OPTIONS);c.arg("-h").arg(socket).args(["-p","50432","-U","postgres","-d","postgres","-t","1"]).stdout(Stdio::null()).stderr(Stdio::null());Ok(c)}
// Bound the probe process as well as the wait loop; failed starts drop/stop the owned server.
fn probe_before(major:u8,socket:&Path,deadline:Instant)->Result<i32> {
    if Instant::now()>=deadline{return Err(failed());}
    let mut child=command_probe(major,socket)?.spawn().map_err(|_|failed())?;
    loop {
        let error=match child.try_wait() {
            Ok(Some(status))=>{if Instant::now()>=deadline{return Err(failed());}return status.code().ok_or_else(failed);},
            Ok(None)=>interrupted().or_else(||(Instant::now()>=deadline).then(failed)),
            Err(_)=>Some(failed()),
        };
        if let Some(error)=error {
            unsafe{libc::kill(-(child.id()as i32),libc::SIGKILL);}
            let _=child.kill();let _=child.wait();return Err(error);
        }
        std::thread::sleep(Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())));
    }
}
impl Drop for Private{fn drop(&mut self){let _=self.shutdown();}}

#[cfg(test)]
mod ready_pid_tests {
    use super::*;
    #[test]
    fn w1_ready_pid_never_passes_special_or_aliased_files_to_postgres() {
        use std::os::unix::ffi::OsStrExt;
        for kind in ["dir","symlink","fifo","empty","mode","hardlink"] {
            let t=tempfile::tempdir().unwrap();let path=t.path().join("postmaster.pid");
            match kind {
                "dir"=>fs::create_dir(&path).unwrap(),
                "symlink"=>std::os::unix::fs::symlink("foreign",&path).unwrap(),
                "fifo"=>{let p=std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(p.as_ptr(),0o600)},0);},
                "empty"=>{fs::write(&path,b"").unwrap();},
                "hardlink"=>{fs::write(t.path().join("other"),b"123\n").unwrap();fs::hard_link(t.path().join("other"),&path).unwrap();},
                _=>fs::write(&path,b"123\n").unwrap(),
            }
            assert!(ready_pid(t.path()).is_err(),"{kind}");
        }
        let t=tempfile::tempdir().unwrap();ready_pid(t.path()).unwrap();
    }
}

/// Validated original root and manifest-owned top-level entries still pending P6.
#[derive(Clone,Copy)]
pub struct LinkWindow<'a> {pub l_root:&'a Path,pub pending:&'a std::collections::BTreeSet<String>}

/// Caller holds the exclusive D lock after validating its selector and pair.
pub fn repair_cluster_ownership(d17:&Path,link_window:Option<LinkWindow<'_>>)->Result<u64> {
    use std::{ffi::{CStr,CString},os::{fd::{AsRawFd,FromRawFd},unix::ffi::OsStrExt}};
    fn invalid()->Error {Error::refusal("unsupported_layout")}
    fn ownership()->Error {Error{code:"unsupported_layout",message:"ownership: PostgreSQL 데이터 소유자를 postgres로 맞추지 못했습니다. 호스트 폴더 공유와 권한을 확인하세요(Docker Desktop for Mac은 root의 소유자 변경을 무시할 수 있습니다).",exit:2,detail:None}}
    fn fdpath(file:&File)->PathBuf {Path::new("/proc/self/fd").join(file.as_raw_fd().to_string())}
    fn open(parent:&File,name:&CStr,directory:bool)->Result<File> {
        let flags=libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC|if directory{libc::O_DIRECTORY}else{0};
        let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),flags)};
        if fd<0{return Err(invalid());}Ok(unsafe{File::from_raw_fd(fd)})
    }
    fn inactive(data:&File)->Result<()> {
        let fd=unsafe{libc::openat(data.as_raw_fd(),c"postmaster.pid".as_ptr(),libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};
        if fd<0{return if std::io::Error::last_os_error().kind()==std::io::ErrorKind::NotFound{Ok(())}else{Err(invalid())};}
        let f=unsafe{File::from_raw_fd(fd)};let m=f.metadata().map_err(|_|invalid())?;if !m.is_file()||m.len()==0||m.len()>16384{return Err(invalid());}
        let mut text=String::new();f.take(16385).read_to_string(&mut text).map_err(|_|invalid())?;
        let pid=text.lines().next().and_then(|s|s.parse::<i32>().ok()).filter(|p|*p>0).ok_or_else(invalid)?;
        if text.len()>16384{return Err(invalid());}
        if unsafe{libc::kill(pid,0)}==0||std::io::Error::last_os_error().raw_os_error()!=Some(libc::ESRCH){return Err(Error::refusal("storage_busy"));}Ok(())
    }
    fn hardlink()->Error {Error{code:"unsupported_layout",message:"PG17 데이터 파일이 다른 위치와 하드 링크로 연결돼 있어 소유자를 고치지 않았습니다. 연결된 파일을 확인하세요.",exit:2,detail:None}}
    struct Scope<'a> {device:u64,uid:u32,gid:u32,data_root:PathBuf,link_window:Option<(File,&'a std::collections::BTreeSet<String>)>}
    fn paired(path:&Path,m:&std::fs::Metadata,scope:&Scope<'_>)->Result<()> {
        use std::path::Component;
        if m.nlink()!=2{return Err(hardlink());}
        let(root,pending)=scope.link_window.as_ref().ok_or_else(hardlink)?;
        let relative=path.strip_prefix(&scope.data_root).map_err(|_|hardlink())?;
        let top=match relative.components().next(){Some(Component::Normal(name))=>name.to_str().ok_or_else(hardlink)?,_=>return Err(hardlink())};
        if !pending.contains(top){return Err(hardlink());}
        let mut parent=root.try_clone().map_err(|_|hardlink())?;let mut parts=relative.components().peekable();
        while let Some(part)=parts.next() {
            let Component::Normal(name)=part else{return Err(hardlink())};let name=CString::new(name.as_bytes()).map_err(|_|hardlink())?;
            let file=open(&parent,&name,parts.peek().is_some()).map_err(|_|hardlink())?;
            if parts.peek().is_none(){let peer=file.metadata().map_err(|_|hardlink())?;return if peer.is_file()&&peer.nlink()==2&&(peer.dev(),peer.ino())==(m.dev(),m.ino()){Ok(())}else{Err(hardlink())};}
            parent=file;
        }
        Err(hardlink())
    }
    fn walk(parent:&File,name:&CStr,path:&Path,scope:&Scope<'_>,depth:u8,seen:&mut u64,repair:bool)->Result<u64> {
        *seen+=1;if depth>64||*seen>1_000_000{return Err(invalid());}
        let before=fs::symlink_metadata(fdpath(parent).join(std::ffi::OsStr::from_bytes(name.to_bytes()))).map_err(|_|invalid())?;
        if (!before.is_file()&&!before.is_dir())||before.dev()!=scope.device{return Err(invalid());}
        let file=open(parent,name,before.is_dir())?;let m=file.metadata().map_err(|_|invalid())?;
        if (m.dev(),m.ino(),m.mode())!=(before.dev(),before.ino(),before.mode()){return Err(invalid());}
        let mismatched=m.uid()!=scope.uid||m.gid()!=scope.gid;
        if mismatched&&m.is_file() {
            if m.mode()&(libc::S_ISUID|libc::S_ISGID)!=0{return Err(Error{code:"unsupported_layout",message:"PG17 데이터에 특수 권한(setuid/setgid)이 붙은 파일이 있어 소유자를 바꾸지 않았습니다. 해당 파일을 확인하세요.",exit:2,detail:None});}
            if m.nlink()>1{paired(path,&m,scope)?;}
        }
        let mut changed=0;
        if repair&&mismatched {
            let result=unsafe{libc::fchownat(parent.as_raw_fd(),name.as_ptr(),scope.uid,scope.gid,libc::AT_SYMLINK_NOFOLLOW)};
            let owned=||owned(&file,scope.uid,scope.gid);
            // Reuse the owner-context fallback only after the descriptor-relative root attempt.
            ownership_after_chown(result==0,owned,||own(path)).map_err(|_|ownership())?;
            if !owned()||file.metadata().map_err(|_|ownership())?.mode()&0o7777!=m.mode()&0o7777{return Err(ownership());}changed=1;
        }
        if m.is_dir() {
            if depth==0 {changed+=walk(&file,c"data",&path.join("data"),scope,depth+1,seen,repair)?;}
            else {for entry in fs::read_dir(fdpath(&file)).map_err(|_|invalid())? {
                let name=entry.map_err(|_|invalid())?.file_name();let c=CString::new(name.as_bytes()).map_err(|_|invalid())?;
                changed+=walk(&file,&c,&path.join(name),scope,depth+1,seen,repair)?;
            }}
        }
        Ok(changed)
    }
    if !d17.is_absolute()||d17.file_name()!=Some(std::ffi::OsStr::new("17")){return Err(invalid());}
    files::real_path(d17).map_err(|_|invalid())?;
    let parent=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(d17.parent().ok_or_else(invalid)?).map_err(|_|invalid())?;
    let root=open(&parent,c"17",true)?;let data=open(&root,c"data",true)?;let(uid,gid)=postgres_ids()?;
    let link_window=match link_window {
        Some(window)=>{files::real_path(window.l_root).map_err(|_|hardlink())?;let l=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(window.l_root).map_err(|_|hardlink())?;Some((l,window.pending))},
        None=>None,
    };
    let scope=Scope{device:root.metadata().map_err(|_|invalid())?.dev(),uid,gid,data_root:d17.join("data"),link_window};
    inactive(&data)?;walk(&parent,c"17",d17,&scope,0,&mut 0,false)?;inactive(&data)?;
    let changed=walk(&parent,c"17",d17,&scope,0,&mut 0,true)?;
    if changed>0{eprintln!("[db-upgrade] ownership repaired: {changed} entries under {}",d17.display());}Ok(changed)
}

#[cfg(all(test,target_os="linux"))]
mod ownership_repair_tests {
    use super::*;
    use std::os::unix::{ffi::OsStrExt,fs::{PermissionsExt,symlink}};
    fn available()->bool {(unsafe{libc::geteuid()==0}) && postgres_ids().is_ok()}
    fn fixture()->(tempfile::TempDir,PathBuf) {
        let t=tempfile::tempdir().unwrap();let d17=t.path().join("17");fs::create_dir_all(d17.join("data/nested")).unwrap();
        fs::set_permissions(d17.join("data"),fs::Permissions::from_mode(0o700)).unwrap();fs::write(d17.join("data/nested/probe"),b"keep").unwrap();
        std::os::unix::fs::chown(d17.join("data/nested/probe"),Some(12345),Some(12345)).unwrap();(t,d17)
    }
    fn metadata_tree(root:&Path)->Vec<(PathBuf,u32,u32,u32,u64,i64,i64,i64,i64)> {
        fn walk(root:&Path,path:&Path,out:&mut Vec<(PathBuf,u32,u32,u32,u64,i64,i64,i64,i64)>) {
            let m=fs::symlink_metadata(path).unwrap();out.push((path.strip_prefix(root).unwrap().into(),m.uid(),m.gid(),m.mode(),m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec()));
            if m.is_dir(){let mut children=fs::read_dir(path).unwrap().map(|x|x.unwrap().path()).collect::<Vec<_>>();children.sort();for p in children{walk(root,&p,out);}}
        }
        let mut out=Vec::new();walk(root,root,&mut out);out
    }
    #[test]
    fn repairs_only_the_pg17_data_scope_and_is_idempotent_without_mode_changes() {
        if !available(){return;}
        let(t,d17)=fixture();for n in ["pgdata",".nwui-pg15-backup",".nwui-discard",".nwui-auto-work","app","17/other"]{let p=t.path().join(n);fs::create_dir_all(&p).unwrap();fs::write(p.join("keep"),n).unwrap();}
        let outside=["pgdata",".nwui-pg15-backup",".nwui-discard",".nwui-auto-work","app","17/other"].map(|n|(t.path().join(n),metadata_tree(&t.path().join(n))));
        let modes=metadata_tree(&d17.join("data")).into_iter().map(|r|(r.0,r.3)).collect::<Vec<_>>();let(uid,gid)=postgres_ids().unwrap();
        assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),4);
        for p in [&d17,&d17.join("data"),&d17.join("data/nested"),&d17.join("data/nested/probe")]{let m=fs::metadata(p).unwrap();assert_eq!((m.uid(),m.gid()),(uid,gid));}
        assert_eq!(metadata_tree(&d17.join("data")).into_iter().map(|r|(r.0,r.3)).collect::<Vec<_>>(),modes);
        for (p,before)in outside{assert_eq!(metadata_tree(&p),before);}
        let repaired=metadata_tree(&d17);assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),0);assert_eq!(metadata_tree(&d17),repaired);
    }
    #[test]
    fn rejects_symlinks_fifos_and_sockets_before_chowning_any_entry() {
        if !available(){return;}
        for kind in ["symlink","fifo","socket"] {
            let(t,d17)=fixture();let bad=d17.join("data/nested/bad");let mut listener=None;
            match kind {"symlink"=>symlink(t.path().join("external"),&bad).unwrap(),"fifo"=>{let p=std::ffi::CString::new(bad.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(p.as_ptr(),0o600)},0);},_=>listener=Some(std::os::unix::net::UnixListener::bind(&bad).unwrap())}
            let before=metadata_tree(&d17);assert_eq!(repair_cluster_ownership(&d17,None).unwrap_err().code,"unsupported_layout","{kind}");assert_eq!(metadata_tree(&d17),before);drop(listener);
        }
    }
    #[test]
    fn refuses_live_pid_and_unsafe_root_without_ownership_changes() {
        if !available(){return;}
        let(t,d17)=fixture();fs::write(d17.join("data/postmaster.pid"),format!("{}\n",std::process::id())).unwrap();let before=metadata_tree(&d17);
        assert_eq!(repair_cluster_ownership(&d17,None).unwrap_err().code,"storage_busy");assert_eq!(metadata_tree(&d17),before);
        let alias=t.path().join("alias");symlink(&d17,&alias).unwrap();assert_eq!(repair_cluster_ownership(&alias,None).unwrap_err().code,"unsupported_layout");assert_eq!(metadata_tree(&d17),before);
    }
    #[test]
    fn depth_limit_is_checked_before_any_repair() {
        if !available(){return;}
        let(_t,d17)=fixture();let mut p=d17.join("data");for _ in 0..65{p.push("x");fs::create_dir(&p).unwrap();}
        let before=metadata_tree(&d17);assert_eq!(repair_cluster_ownership(&d17,None).unwrap_err().code,"unsupported_layout");assert_eq!(metadata_tree(&d17),before);
    }
    #[test]
    fn foreign_hardlinks_are_refused_without_mutation_outside_the_link_window() {
        if !available(){return;}
        let(t,d17)=fixture();let alias=t.path().join("outside");fs::hard_link(d17.join("data/nested/probe"),&alias).unwrap();let before=metadata_tree(t.path());
        let e=repair_cluster_ownership(&d17,None).unwrap_err();assert_eq!(e.code,"unsupported_layout");assert_eq!(e.message,"PG17 데이터 파일이 다른 위치와 하드 링크로 연결돼 있어 소유자를 고치지 않았습니다. 연결된 파일을 확인하세요.");assert_eq!(metadata_tree(t.path()),before);
    }
    #[test]
    fn even_an_active_window_must_not_reown_a_foreign_app_hardlink(){
        if !available(){return;}
        let(t,d17)=fixture();let l=t.path().join("pgdata");let app=t.path().join("app");fs::create_dir(&l).unwrap();fs::create_dir(&app).unwrap();let outside=app.join("data_key");fs::hard_link(d17.join("data/nested/probe"),&outside).unwrap();let before=metadata_tree(&d17);let app_before=metadata_tree(&app);
        let pending=std::collections::BTreeSet::from(["nested".into()]);let result=repair_cluster_ownership(&d17,Some(LinkWindow{l_root:&l,pending:&pending}));assert_eq!(metadata_tree(&app),app_before,"the outside owner's metadata must remain unchanged");assert_eq!(result.unwrap_err().code,"unsupported_layout");assert_eq!(metadata_tree(&d17),before);
    }
    #[test]
    fn every_shared_file_requires_the_exact_pending_L_pair_and_exactly_two_links(){
        if !available(){return;}
        for kind in ["missing","different_inode","not_pending","third_link","parent_symlink","leaf_symlink","fifo"] {
            let(t,d17)=fixture();let l=t.path().join("pgdata");let app=t.path().join("app");fs::create_dir_all(l.join("nested")).unwrap();fs::create_dir(&app).unwrap();let data=d17.join("data/nested/probe");let peer=l.join("nested/probe");
            match kind {
                "not_pending"=>fs::hard_link(&data,&peer).unwrap(),
                "third_link"=>{fs::hard_link(&data,&peer).unwrap();fs::hard_link(&data,app.join("key")).unwrap();},
                "parent_symlink"=>{fs::remove_dir(l.join("nested")).unwrap();symlink(&app,l.join("nested")).unwrap();fs::hard_link(&data,app.join("probe")).unwrap();},
                "leaf_symlink"=>{fs::hard_link(&data,app.join("probe")).unwrap();symlink(app.join("probe"),&peer).unwrap();},
                _=>{fs::hard_link(&data,app.join("key")).unwrap();if kind=="different_inode"{fs::write(&peer,b"not the linked inode").unwrap();}if kind=="fifo"{let p=std::ffi::CString::new(peer.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(p.as_ptr(),0o600)},0);}},
            }
            let pending=std::collections::BTreeSet::from([if kind=="not_pending"{"other".into()}else{"nested".into()}]);let before=metadata_tree(t.path());let result=repair_cluster_ownership(&d17,Some(LinkWindow{l_root:&l,pending:&pending}));assert_eq!(result.unwrap_err().code,"unsupported_layout","{kind}");assert_eq!(metadata_tree(t.path()),before,"{kind}");
        }
    }
    #[test]
    fn the_link_window_repairs_shared_inodes_and_closes_cleanly_after_retirement() {
        if !available(){return;}
        let(t,d17)=fixture();let l=t.path().join("pgdata");fs::create_dir_all(l.join("nested")).unwrap();let alias=l.join("nested/probe");fs::hard_link(d17.join("data/nested/probe"),&alias).unwrap();let pending=std::collections::BTreeSet::from(["nested".into()]);
        assert_eq!(repair_cluster_ownership(&d17,Some(LinkWindow{l_root:&l,pending:&pending})).unwrap(),4);let(uid,gid)=postgres_ids().unwrap();let m=fs::metadata(&alias).unwrap();assert_eq!((m.uid(),m.gid()),(uid,gid));
        fs::remove_file(alias).unwrap();assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),0);
    }
    #[test]
    fn special_regular_permissions_refuse_before_any_repair_but_setgid_directories_survive() {
        if !available(){return;}
        for bit in [0o4000,0o2000]{let(_t,d17)=fixture();fs::set_permissions(d17.join("data/nested/probe"),fs::Permissions::from_mode(bit|0o600)).unwrap();let before=metadata_tree(&d17);
            let e=repair_cluster_ownership(&d17,None).unwrap_err();assert_eq!(e.code,"unsupported_layout");assert_eq!(e.message,"PG17 데이터에 특수 권한(setuid/setgid)이 붙은 파일이 있어 소유자를 바꾸지 않았습니다. 해당 파일을 확인하세요.");assert_eq!(metadata_tree(&d17),before);}
        let(_t,d17)=fixture();fs::set_permissions(d17.join("data"),fs::Permissions::from_mode(0o2700)).unwrap();assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),4);assert_eq!(fs::metadata(d17.join("data")).unwrap().mode()&0o7777,0o2700);
    }
    #[test]
    #[ignore="native PG17 ownership repair; NWUI_TEST_PROFILE_NATIVE=1"]
    fn native_repair_allows_pg17_start_preserves_the_journal_and_refuses_a_live_postmaster() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        for link_window in [false,true] {
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();
        let db=t.path().join("db");let app=t.path().join("app");directory(&db).unwrap();files::private_dir(&app).unwrap();let d17=db.join("17");directory(&d17).unwrap();let data=d17.join("data");
        let init=t.path().join("init");directory(&init).unwrap();let mut cmd=command(17,"initdb").unwrap();cmd.arg("-D").arg(&data).args(["--no-locale","--encoding=UTF8","--auth=trust"]);capture(cmd,&init,Duration::from_secs(120)).unwrap();
        files::private_dir(&app.join(".nwui-auto")).unwrap();
        fs::write(app.join(".nwui-auto/journal-v1.json"),b"{\"ownershipRepairFixture\":true}\n").unwrap();
        let outside=[db.join("pgdata"),db.join(".nwui-pg15-backup"),app.clone()];for p in &outside[..2]{fs::create_dir(p).unwrap();fs::write(p.join("keep"),b"unchanged").unwrap();}
        let mut linked=None;
        if link_window {let relation=fs::read_dir(data.join("base/5")).unwrap().map(|e|e.unwrap().path()).find(|p|p.file_name().unwrap().as_bytes().iter().all(u8::is_ascii_digit)&&p.is_file()).unwrap();let relative=relation.strip_prefix(&data).unwrap().to_path_buf();let peer=outside[0].join(&relative);fs::create_dir_all(peer.parent().unwrap()).unwrap();fs::hard_link(&relation,&peer).unwrap();std::os::unix::fs::chown(relation,Some(12345),Some(12345)).unwrap();linked=Some((peer,relative));}
        let pending=std::collections::BTreeSet::from(["base".into()]);let window=link_window.then_some(LinkWindow{l_root:&outside[0],pending:&pending});
        let before=outside.iter().map(|p|metadata_tree(p)).collect::<Vec<_>>();let journal=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();
        std::os::unix::fs::chown(&d17,Some(0),Some(0)).unwrap();std::os::unix::fs::chown(&data,Some(0),Some(0)).unwrap();std::os::unix::fs::chown(data.join("global/pg_control"),Some(12345),Some(12345)).unwrap();
        let failed_work=t.path().join("before-repair");directory(&failed_work).unwrap();assert!(Private::start(17,&data,&failed_work).is_err(),"unrepaired cluster must reproduce startup failure");
        assert_eq!(repair_cluster_ownership(&d17,window).unwrap(),3+u64::from(link_window));assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),0);
        let work=t.path().join("repaired");directory(&work).unwrap();let pg=Private::start(17,&data,&work).unwrap();
        assert_eq!(repair_cluster_ownership(&d17,None).unwrap_err().code,"storage_busy");pg.stop().unwrap();
        assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),journal);
        for (p,mut b)in outside.iter().zip(before){let mut after=metadata_tree(p);if p==&outside[0]{if let Some((_,relative))=&linked{b.retain(|r|&r.0!=relative);after.retain(|r|&r.0!=relative);}}assert_eq!(after,b);}
        if let Some((linked,_))=linked {let m=fs::metadata(&linked).unwrap();assert_eq!((m.uid(),m.gid()),postgres_ids().unwrap());fs::remove_file(linked).unwrap();assert_eq!(repair_cluster_ownership(&d17,None).unwrap(),0);}
        }
    }
}

#[cfg(test)]mod orbstack_owner_tests{
 use super::*;
 use super::super::shared_folder::SharedKind;
 #[test]fn postgres_postcondition_accepts_only_the_exact_pair_or_orbstack_projection(){
  for kind in [None,Some(SharedKind::DockerDesktop),Some(SharedKind::OrbStackMac)]{
   assert!(owner_matches((100,102),(100,102),kind));
   assert_eq!(owner_matches((0,0),(100,102),kind),kind==Some(SharedKind::OrbStackMac));
   for pair in [(0,102),(100,0),(12345,0),(0,12345),(12345,12345)]{assert!(!owner_matches(pair,(100,102),kind));}
  }
 }
 #[test]fn successful_chown_of_projected_entry_does_not_spawn_fallback(){
  let calls=std::cell::Cell::new(0);
  ownership_after_chown(true,||owner_matches((0,0),(100,102),Some(SharedKind::OrbStackMac)),||{calls.set(calls.get()+1);Ok(())}).unwrap();
  assert_eq!(calls.get(),0);
  for (success,postcondition)in [(false,true),(true,false),(false,false)]{
   let calls=std::cell::Cell::new(0);let result=ownership_after_chown(success,||postcondition,||{calls.set(calls.get()+1);Ok(())});
   assert_eq!(calls.get(),1);assert_eq!(result.is_ok(),postcondition);
  }
  assert!(ownership_after_chown(false,||true,||Err(failed())).is_err());
 }
}

#[cfg(all(test,target_os="linux"))]mod orbstack_pid_tests{
 use super::*;
 use super::super::shared_folder::SharedKind;
 use std::os::unix::fs::PermissionsExt;
 #[test]fn projected_stale_pid_keeps_shape_checks_and_source_policy(){
  if unsafe{libc::geteuid()}!=0{return;}
  let t=tempfile::tempdir().unwrap();let p=t.path().join("pid");fs::write(&p,b"123\n").unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o600)).unwrap();
  let m=fs::metadata(&p).unwrap();assert!(pid_metadata(&m,100,false,Some(SharedKind::OrbStackMac)));assert!(pid_metadata(&m,100,true,None));
  for kind in [None,Some(SharedKind::DockerDesktop)]{assert!(!pid_metadata(&m,100,false,kind));}
  for mode in [0o644,0o660,0o1600]{fs::set_permissions(&p,fs::Permissions::from_mode(mode)).unwrap();assert!(!pid_metadata(&fs::metadata(&p).unwrap(),100,false,Some(SharedKind::OrbStackMac)));}
  fs::set_permissions(&p,fs::Permissions::from_mode(0o600)).unwrap();
  for bytes in [vec![],vec![b'x';16385]]{fs::write(&p,bytes).unwrap();assert!(!pid_metadata(&fs::metadata(&p).unwrap(),100,false,Some(SharedKind::OrbStackMac)));}
  fs::write(&p,b"123\n").unwrap();fs::hard_link(&p,t.path().join("peer")).unwrap();assert!(!pid_metadata(&fs::metadata(&p).unwrap(),100,false,Some(SharedKind::OrbStackMac)));
  fs::remove_file(t.path().join("peer")).unwrap();std::os::unix::fs::chown(&p,Some(0),Some(12345)).unwrap();assert!(!pid_metadata(&fs::metadata(&p).unwrap(),100,false,Some(SharedKind::OrbStackMac)));
  assert!(!pid_metadata(&fs::metadata(t.path()).unwrap(),100,false,Some(SharedKind::OrbStackMac)));
 }
}
