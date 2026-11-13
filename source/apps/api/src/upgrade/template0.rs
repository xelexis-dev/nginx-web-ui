//! V-P7: complete read-only template0 inspection through the stopped PG15 backend.
use crate::{db_upgrade::{layout::Roots,inspect as shared,inspect::DatabaseSnapshot,cluster::Collation,postgres,files},crypto::SecretBox};
use super::{StorageLocks,StepError,backup_format::SourceLocation};
use std::{path::{Path,PathBuf},fs::{File,OpenOptions},io::{self,Read,BufRead,Write},process::{Child,ChildStdin,Stdio},sync::{Arc,atomic::{AtomicBool,Ordering},mpsc},thread,time::{Duration,Instant},os::unix::fs::{MetadataExt,OpenOptionsExt}};
use serde_json::Value;
use std::os::fd::AsRawFd;
// shortcut: strict non-application metadata fits 64 KiB; requalify this bound if its query set grows.
const CAP:usize=65_536;
const TIMEOUT:Duration=Duration::from_secs(300);
fn failed()->StepError{StepError{code:"postgres_failed",message:"읽기 전용 template0 검사를 완료하지 못했습니다. 원본과 제한된 진단 로그를 보존했습니다.",detail:None}}
fn unsafe_layout()->StepError{StepError{code:"unsupported_layout",message:"template0 검사 경로나 객체 구성을 지원하지 않습니다. 원본을 보존했습니다.",detail:None}}
fn mapped(e:crate::db_upgrade::error::Error)->StepError{StepError{code:e.code,message:e.message,detail:None}}
fn diagnostic_metadata(m:&std::fs::Metadata,device:u64,uid:u32,kind:Option<crate::db_upgrade::shared_folder::SharedKind>)->bool{
    m.is_dir()&&m.dev()==device&&m.mode()&0o7777==0o700
        &&(m.uid()==uid||m.uid()==0&&m.gid()==0&&kind==Some(crate::db_upgrade::shared_folder::SharedKind::OrbStackMac))
}
fn inputs(roots:&Roots,locks:&StorageLocks,location:SourceLocation,log:&Path)->Result<PathBuf,StepError>{
    if !locks.holds_a()||!locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app){return Err(unsafe_layout());}
    let j=super::Journal::open(&roots.app,locks)?;let id=&j.load().attempt_id;
    let name=j.load().records.iter().rev().find(|r|r.step==super::Step::P2Preflight).filter(|r|r.state==super::StepState::Intent).and_then(|r|r.data.as_ref()?.get("logName")?.as_str()).ok_or_else(unsafe_layout)?;
    let uuid=name.strip_prefix("check-").ok_or_else(unsafe_layout)?;if !super::types::canonical_generation(uuid){return Err(unsafe_layout());}
    let logs=roots.db.join(".nwui-auto-work/logs");let attempt=logs.join(id);if log!=attempt.join(name){return Err(unsafe_layout());}
    files::real_path(&roots.db).map_err(|_|unsafe_layout())?;files::real_path(log).map_err(|_|unsafe_layout())?;
    let device=std::fs::symlink_metadata(&roots.db).map_err(|_|unsafe_layout())?.dev();let (uid,_)=postgres::postgres_ids().map_err(mapped)?;
    for path in [&logs,&attempt,log]{
        let file=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|_|unsafe_layout())?;
        let m=file.metadata().map_err(|_|unsafe_layout())?;let p=std::fs::symlink_metadata(path).map_err(|_|unsafe_layout())?;
        if (p.dev(),p.ino())!=(m.dev(),m.ino())||!diagnostic_metadata(&m,device,uid,crate::db_upgrade::shared_folder::kind(&file)){return Err(unsafe_layout());}
    }
    let data=match location{SourceLocation::Root=>roots.db.clone(),SourceLocation::Pgdata=>roots.db.join("pgdata")};files::real_path(&data).map_err(|_|unsafe_layout())?;super::source::require_idle(roots,locks,&data)?;Ok(data)
}
/// No SQL write, datallowconn toggle, key publication, journal update or W diagnostic.
pub(crate) fn inspect(roots:&Roots,locks:&StorageLocks,location:SourceLocation,log_dir:&Path,_key:Option<&SecretBox>)->Result<(DatabaseSnapshot,Vec<Collation>,u64),StepError>{
    let data=inputs(roots,locks,location,log_dir)?;postgres::read_control(15,&data).and_then(|c|{c.validate()?;c.require_cold()}).map_err(mapped)?;
    let mut session=Session::start(&data,log_dir)?;
    let state=session.query("SELECT jsonb_build_array(current_setting('transaction_read_only'),pg_backend_pid())")?;
    if state.get(0).and_then(Value::as_str)!=Some("on"){return Err(failed());}
    #[cfg(test)]pause(log_dir,state.get(1).and_then(Value::as_i64).ok_or_else(failed)?);
    let catalogs=session.query(&format!("SELECT to_jsonb(ARRAY({}))",shared::OBJECT_CATALOGS_QUERY))?;
    let catalogs=catalogs.as_array().ok_or_else(failed)?.iter().map(|v|v.as_str().map(str::to_owned).ok_or_else(failed)).collect::<Result<Vec<_>,_>>()?;
    for query in shared::non_application_checks(&catalogs).map_err(mapped)?{if session.query(&format!("SELECT to_jsonb(({query}))"))?.as_bool().ok_or_else(failed)?{session.finish()?;return Err(unsafe_layout());}}
    let mut groups=Vec::new();for query in shared::SCHEMA_QUERIES{groups.push(session.query(&format!("SELECT to_jsonb(ARRAY({query}))"))?);}
    let snapshot=shared::non_application_snapshot(&groups).map_err(mapped)?;
    let bytes=session.query(&format!("SELECT to_jsonb(({}))",shared::LARGEST_INDEX_QUERY))?.as_u64().ok_or_else(failed)?;
    let query=format!("SELECT COALESCE(jsonb_agg(jsonb_build_array(q.schema,q.name,q.provider,q.locale,q.recorded,q.actual) ORDER BY q.schema,q.name),'[]'::jsonb) FROM ({}) AS q(schema,name,provider,locale,recorded,actual)",shared::AUTO_COLLATIONS_QUERY);
    let collations=shared::used_collations(&session.query(&query)?).map_err(mapped)?;
    session.finish()?;Ok((snapshot,collations,bytes))
}
struct Session{
    child:Child,input:Option<ChildStdin>,frames:mpsc::Receiver<Vec<u8>>,readers:Vec<thread::JoinHandle<io::Result<()>>>,broken:Arc<AtomicBool>,deadline:Instant,sequence:u32,reaped:bool,
}
impl Session{
    fn start(data:&Path,log:&Path)->Result<Self,StepError>{
        // Reuse every safe house server override, with single-user as PostgreSQL's FIRST argument.
        let safe=postgres::server_command(15,data,Path::new("/run"),Path::new("/dev/null")).map_err(mapped)?;
        let mut c=postgres::command(15,"postgres").map_err(mapped)?;c.arg("--single").args(safe.get_args().skip(4));
        for setting in ["hba_file=/dev/null","ident_file=/dev/null","default_transaction_read_only=on","search_path=pg_catalog","TimeZone=UTC","DateStyle=ISO,YMD","extra_float_digits=3","statement_timeout=300000","lock_timeout=10000","exit_on_error=on"]{c.arg("-c").arg(setting);}
        c.arg("template0").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let path=log.join(format!("template0-{}.stderr",uuid::Uuid::new_v4()));let error_log=OpenOptions::new().create_new(true).write(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path).map_err(|_|failed())?;postgres::own(&path).map_err(mapped)?;
        let mut child=c.spawn().map_err(|_|failed())?;let input=child.stdin.take();let stdout=child.stdout.take().ok_or_else(failed)?;let stderr=child.stderr.take().ok_or_else(failed)?;
        let (send,frames)=mpsc::channel();let broken=Arc::new(AtomicBool::new(false));let out_failed=broken.clone();let err_failed=broken.clone();
        let mut session=Self{child,input,frames,readers:Vec::new(),broken,deadline:Instant::now()+TIMEOUT,sequence:0,reaped:false};
        let fd=session.input.as_ref().ok_or_else(failed)?.as_raw_fd();let flags=unsafe{libc::fcntl(fd,libc::F_GETFL)};
        if flags<0||unsafe{libc::fcntl(fd,libc::F_SETFL,flags|libc::O_NONBLOCK)}<0{return Err(failed());}
        session.readers.push(thread::Builder::new().spawn(move||{let result=read_frames(stdout,send);if result.is_err(){out_failed.store(true,Ordering::Release);}result}).map_err(|_|failed())?);
        session.readers.push(thread::Builder::new().spawn(move||{let result=read_errors(stderr,error_log);if result.is_err(){err_failed.store(true,Ordering::Release);}result}).map_err(|_|failed())?);
        Ok(session)
    }
    fn query(&mut self,query:&str)->Result<Value,StepError>{
        self.sequence+=1;let marker=format!("NWUI_TEMPLATE0_{}_",self.sequence);
        let statement=format!("COPY (SELECT '{marker}'||encode(convert_to(({query})::text,'UTF8'),'hex')) TO STDOUT;\n");
        let mut remaining=statement.as_bytes();while !remaining.is_empty(){
            if let Some(e)=postgres::interrupted(){return Err(mapped(e));}if self.broken.load(Ordering::Acquire)||Instant::now()>=self.deadline{return Err(failed());}
            match self.input.as_mut().ok_or_else(failed)?.write(remaining){Ok(0)=>return Err(failed()),Ok(n)=>remaining=&remaining[n..],Err(e)if matches!(e.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::Interrupted)=>thread::sleep(Duration::from_millis(10)),Err(_)=>return Err(failed())}
        }
        loop{
            if let Some(e)=postgres::interrupted(){return Err(mapped(e));}if self.broken.load(Ordering::Acquire)||Instant::now()>=self.deadline{return Err(failed());}
            match self.frames.recv_timeout(Duration::from_millis(50)){
                Ok(frame)=>{let bytes=frame.strip_prefix(marker.as_bytes()).ok_or_else(failed)?;let bytes=hex::decode(bytes).map_err(|_|failed())?;return serde_json::from_slice(&bytes).map_err(|_|failed());},
                Err(mpsc::RecvTimeoutError::Disconnected)=>return Err(failed()),Err(mpsc::RecvTimeoutError::Timeout)=>{},
            }
            if self.child.try_wait().map_err(|_|failed())?.is_some(){self.reaped=true;return Err(failed());}
        }
    }
    fn finish(mut self)->Result<(),StepError>{
        self.input.take();loop{if self.broken.load(Ordering::Acquire)||Instant::now()>=self.deadline{return Err(failed());}if let Some(e)=postgres::interrupted(){return Err(mapped(e));}
            if let Some(status)=self.child.try_wait().map_err(|_|failed())?{self.reaped=true;if !status.success(){return Err(failed());}break;}thread::sleep(Duration::from_millis(10));}
        let mut good=true;for reader in self.readers.drain(..){good &= reader.join().is_ok_and(|r|r.is_ok());}if !good||self.broken.load(Ordering::Acquire){return Err(failed());}Ok(())
    }
}
impl Drop for Session{
    fn drop(&mut self){self.input.take();if !self.reaped{if matches!(self.child.try_wait(),Ok(Some(_))){self.reaped=true;}else{unsafe{libc::kill(-(self.child.id()as i32),libc::SIGKILL);}let _=self.child.kill();let _=self.child.wait();self.reaped=true;}}for reader in self.readers.drain(..){let _=reader.join();}}
}
fn read_frames(pipe:impl Read,send:mpsc::Sender<Vec<u8>>)->io::Result<()>{
    let mut reader=io::BufReader::new(pipe.take((CAP+1)as u64));let mut total=0;let mut frames=0;
    loop{let mut line=Vec::new();let n=reader.read_until(b'\n',&mut line)?;if n==0{return Ok(());}total+=n;if total>CAP{return Err(io::Error::other("template0 stdout bound"));}
        if line.last()==Some(&b'\n'){line.pop();}let mut line=line.as_slice();while let Some(rest)=line.strip_prefix(b"backend> "){line=rest;}
        if line.is_empty()||line.starts_with(b"PostgreSQL stand-alone backend 15."){continue;}
        if !line.starts_with(b"NWUI_TEMPLATE0_"){return Err(io::Error::other("template0 frame"));}frames+=1;if frames>32{return Err(io::Error::other("template0 frame count"));}if send.send(line.to_owned()).is_err(){return Ok(());}
    }
}
fn read_errors(mut pipe:impl Read,mut log:File)->io::Result<()>{
    let mut bytes=[0;4096];let mut total=0;loop{let n=pipe.read(&mut bytes)?;if n==0{return log.sync_all();}let keep=n.min(CAP-total);log.write_all(&bytes[..keep])?;total+=keep;if keep<n{log.sync_all()?;return Err(io::Error::other("template0 stderr bound"));}}
}
#[cfg(test)]fn pause(log:&Path,pid:i64){if std::env::var("NWUI_TEMPLATE0_PAUSE").as_deref()==Ok("1"){std::fs::write(log.join("backend.pid.tmp"),pid.to_string()).unwrap();std::fs::rename(log.join("backend.pid.tmp"),log.join("backend.pid")).unwrap();let until=Instant::now()+Duration::from_secs(30);while !log.join("continue").exists(){assert!(Instant::now()<until,"owned pause deadline");thread::sleep(Duration::from_millis(10));}}}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::db_upgrade::{postgres,files,inspect as ordinary};
    use std::{fs,path::PathBuf,time::Duration,os::unix::fs::PermissionsExt};
    // Byte-identical existing cluster queries; extraction will expose these without changing them.
    const INDEX:&str="SELECT COALESCE(max(pg_relation_size(indexrelid)),0) FROM pg_index";
    const COLLATIONS:&str="SELECT DISTINCT n.nspname,c.collname,c.collprovider::text,COALESCE(to_jsonb(c)->>'colllocale',to_jsonb(c)->>'colliculocale',c.collcollate),c.collversion,CASE WHEN c.collprovider='d' THEN NULL ELSE pg_collation_actual_version(c.oid) END FROM pg_index i JOIN pg_class t ON t.oid=i.indrelid JOIN pg_namespace tn ON tn.oid=t.relnamespace JOIN LATERAL unnest(i.indcollation) u(oid) ON true JOIN pg_collation c ON c.oid=u.oid JOIN pg_namespace n ON n.oid=c.collnamespace WHERE tn.nspname IN('public','drizzle') ORDER BY n.nspname,c.collname";
    fn root()->PathBuf{PathBuf::from(std::env::var_os("NWUI_TEST_TEMPLATE0_ROOT").expect("task-owned fixture volume"))}
    fn seed(root:&Path){
        fs::set_permissions(root,fs::Permissions::from_mode(0o711)).unwrap();
        if root.join("seed").exists(){return;}
        postgres::directory(&root.join("seed")).unwrap();postgres::directory(&root.join("init-logs")).unwrap();
        let mut c=postgres::command(15,"initdb").unwrap();c.arg("-D").arg(root.join("seed")).args(["--no-locale","--encoding=UTF8","--auth-local=trust","--auth-host=reject"]);postgres::capture(c,&root.join("init-logs"),Duration::from_secs(120)).unwrap();
    }
    async fn ordinary_copy(root:&Path,auto:bool)->(DatabaseSnapshot,Vec<Collation>,u64){
        let id=uuid::Uuid::new_v4().to_string();let data=root.join(format!("trusted-{id}"));files::copy_tree(&root.join("seed"),&data).unwrap();let log=root.join(format!("trusted-log-{id}"));postgres::directory(&log).unwrap();
        let pg=postgres::Private::start(15,&data,&log).unwrap();let admin=ordinary::connect(&pg.socket,"postgres","postgres",None).await.unwrap();
        admin.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();admin.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS true").await.unwrap();
        let c=ordinary::connect(&pg.socket,"template0","postgres",None).await.unwrap();let snapshot=ordinary::snapshot_database(&c.client,None,false).await.unwrap();let size:i64=c.client.query_one(INDEX,&[]).await.unwrap().get(0);let collations=c.client.query(if auto{ordinary::AUTO_COLLATIONS_QUERY}else{COLLATIONS},&[]).await.unwrap().into_iter().map(|r|Collation{schema:r.get(0),name:r.get(1),provider:r.get(2),locale:r.get(3),recorded:r.get(4),actual:r.get(5)}).collect();drop(c);
        admin.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS false").await.unwrap();drop(admin);pg.stop().unwrap();(snapshot,collations,size.try_into().unwrap())
    }
    fn source(root:&Path,location:SourceLocation)->(Roots,StorageLocks,super::super::Journal,PathBuf){
        let case=root.join(format!("case-{}",uuid::Uuid::new_v4()));fs::create_dir(&case).unwrap();fs::set_permissions(&case,fs::Permissions::from_mode(0o711)).unwrap();let roots=Roots{db:case.join("db"),app:case.join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};
        match location{SourceLocation::Root=>files::copy_tree(&root.join("seed"),&roots.db).unwrap(),SourceLocation::Pgdata=>{postgres::directory(&roots.db).unwrap();files::copy_tree(&root.join("seed"),&roots.db.join("pgdata")).unwrap();}}
        files::private_dir(&roots.app).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=super::super::Journal::begin(&roots.app,&locks,super::super::Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();super::super::reserves::ensure(&roots,&locks,&j).unwrap();
        let parent=roots.db.join(".nwui-auto-work/logs");postgres::directory(&parent).unwrap();let attempt=parent.join(&j.load().attempt_id);postgres::directory(&attempt).unwrap();let name=format!("check-{}",uuid::Uuid::new_v4());j.record(super::super::Step::P2Preflight,super::super::StepState::Intent,Some(serde_json::json!({"logName":name}))).unwrap();let log=attempt.join(name);postgres::directory(&log).unwrap();(roots,locks,j,log)
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="pinned PG15 fixture COPY; NWUI_TEST_TEMPLATE0_ROOT"]
    async fn native_baseline_before_extraction_and_complete_single_user_parity(){
        if std::env::var_os("NWUI_TEST_TEMPLATE0_ROOT").is_none(){return;}
        let root=root();seed(&root);let baseline=root.join("baseline-before.json");let actual=ordinary_copy(&root,false).await;
        if baseline.exists(){assert_eq!(serde_json::to_vec(&actual).unwrap(),fs::read(&baseline).unwrap(),"ordinary Client false path drifted after extraction");}else{fs::write(&baseline,serde_json::to_vec(&actual).unwrap()).unwrap();}
        let expected=ordinary_copy(&root,true).await;assert_eq!(expected.0,actual.0,"A.22 snapshot must stay frozen");assert_eq!(expected.2,actual.2);
        for location in [SourceLocation::Root,SourceLocation::Pgdata]{let (roots,locks,j,log)=source(&root,location);let frame=serde_json::to_vec(j.load()).unwrap();let got=inspect(&roots,&locks,location,&log,None).unwrap();assert_eq!(serde_json::to_vec(&got).unwrap(),serde_json::to_vec(&expected).unwrap(),"single-user and ordinary auto collation maps must match");assert_eq!(serde_json::to_vec(j.load()).unwrap(),frame);}
    }
    #[test]
    #[ignore="pinned PG15 task-owned authority fixture"]
    fn native_input_authority_refuses_before_starting_a_backend(){
        if std::env::var_os("NWUI_TEST_TEMPLATE0_ROOT").is_none(){return;}
        let root=root();seed(&root);let(roots,mut locks,_j,log)=source(&root,SourceLocation::Root);
        assert!(inputs(&roots,&locks,SourceLocation::Root,&log).is_ok());
        let unchanged=fs::read_dir(&log).unwrap().count();
        assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log.join("extra"),None).unwrap_err().code,"unsupported_layout");
        fs::set_permissions(&log,fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"unsupported_layout");
        fs::set_permissions(&log,fs::Permissions::from_mode(0o700)).unwrap();
        let moved=log.with_extension("saved");fs::rename(&log,&moved).unwrap();std::os::unix::fs::symlink(&moved,&log).unwrap();
        assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"unsupported_layout");
        fs::remove_file(&log).unwrap();fs::rename(&moved,&log).unwrap();
        let other=Roots{db:roots.db.clone(),app:root.join("wrong-app"),architecture:roots.architecture.clone()};
        assert_eq!(inspect(&other,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"unsupported_layout");
        locks.release_d();assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"unsupported_layout");
        assert_eq!(fs::read_dir(&log).unwrap().count(),unchanged,"refusals must not spawn or create diagnostics");
    }
    #[test]
    fn framed_output_is_bounded_and_never_decodes_developer_row_quotes(){
        let (send,receive)=mpsc::channel();read_frames(io::Cursor::new(b"\nPostgreSQL stand-alone backend 15.19\nbackend> NWUI_TEMPLATE0_1_74727565\nbackend> "),send).unwrap();assert_eq!(receive.recv().unwrap(),b"NWUI_TEMPLATE0_1_74727565");
        for bytes in [b"backend> garbage\n".to_vec(),vec![b'x';CAP+1]]{let(send,_)=mpsc::channel();assert!(read_frames(io::Cursor::new(bytes),send).is_err());}
        let log=tempfile::tempfile().unwrap();assert!(read_errors(io::Cursor::new(vec![b'x';CAP+1]),log.try_clone().unwrap()).is_err());assert_eq!(log.metadata().unwrap().len(),CAP as u64);
    }
    #[test]
    fn shared_decoder_requires_all_ten_ordered_schema_groups(){
        assert!(shared::non_application_snapshot(&[]).is_err());let mut groups=vec![serde_json::json!([]);shared::SCHEMA_QUERIES.len()];groups[0]=serde_json::json!([1]);assert!(shared::non_application_snapshot(&groups).is_err());
        assert!(shared::used_collations(&serde_json::json!([["schema","name","?",null,null,null]])).is_err());
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="pinned PG15 unexpected-object COPY"]
    async fn native_unexpected_template0_table_refuses_without_changing_it(){
        if std::env::var_os("NWUI_TEST_TEMPLATE0_ROOT").is_none(){return;}
        let root=root();seed(&root);let(roots,locks,_j,log)=source(&root,SourceLocation::Root);let setup=roots.db.parent().unwrap().join("mutant-setup");postgres::directory(&setup).unwrap();let pg=postgres::Private::start(15,&roots.db,&setup).unwrap();
        let admin=ordinary::connect(&pg.socket,"postgres","postgres",None).await.unwrap();admin.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();admin.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS true").await.unwrap();
        let c=ordinary::connect(&pg.socket,"template0","postgres",None).await.unwrap();c.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();c.client.batch_execute("CREATE TABLE public.unexpected_template0(id integer); INSERT INTO public.unexpected_template0 VALUES(42)").await.unwrap();let relation:String=c.client.query_one("SELECT pg_relation_filepath('public.unexpected_template0')",&[]).await.unwrap().get(0);drop(c);admin.client.batch_execute("ALTER DATABASE template0 ALLOW_CONNECTIONS false").await.unwrap();drop(admin);pg.stop().unwrap();
        let before=fs::read(roots.db.join(&relation)).unwrap();assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"unsupported_layout");assert_eq!(fs::read(roots.db.join(relation)).unwrap(),before);
    }
    #[test]
    fn single_user_kill_child(){
        let Ok(case)=std::env::var("NWUI_TEMPLATE0_CHILD_CASE")else{return};let roots=Roots{db:Path::new(&case).join("db"),app:Path::new(&case).join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let log=PathBuf::from(std::env::var_os("NWUI_TEMPLATE0_CHILD_LOG").unwrap());
        assert_eq!(inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap_err().code,"postgres_failed");
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="real SIGKILL of the owned PG15 single-user backend; source COPY boots afterward"]
    async fn native_sigkill_during_single_user_inspection_keeps_source_bootable(){
        if std::env::var_os("NWUI_TEST_TEMPLATE0_ROOT").is_none(){return;}
        let root=root();seed(&root);let(roots,locks,_j,log)=source(&root,SourceLocation::Root);
        let before=serde_json::to_vec(&inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap()).unwrap();drop(locks);
        let mut child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::template0::tests::single_user_kill_child","--nocapture","--test-threads=1"]).env("NWUI_TEMPLATE0_CHILD_CASE",roots.db.parent().unwrap()).env("NWUI_TEMPLATE0_CHILD_LOG",&log).env("NWUI_TEMPLATE0_PAUSE","1").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let until=Instant::now()+Duration::from_secs(30);while !log.join("backend.pid").exists(){if let Some(status)=child.try_wait().unwrap(){let output=child.wait_with_output().unwrap();panic!("child {status}: {}",String::from_utf8_lossy(&output.stderr));}assert!(Instant::now()<until,"backend ready deadline");thread::sleep(Duration::from_millis(10));}
        let pid:i32=fs::read_to_string(log.join("backend.pid")).unwrap().parse().unwrap();let status=fs::read_to_string(format!("/proc/{pid}/status")).unwrap();let uid=postgres::postgres_ids().unwrap().0;assert!(status.lines().find(|l|l.starts_with("Uid:")).unwrap().split_whitespace().skip(1).all(|v|v.parse::<u32>().unwrap()==uid));
        let mut read=std::process::Command::new("/usr/sbin/runuser");read.args(["-u","postgres","--","/usr/bin/readlink","-z","--"]).arg(format!("/proc/{pid}/cwd")).env_clear().env("PATH","/usr/bin:/bin");let(cwd,_)=postgres::output(read,Duration::from_secs(3)).unwrap();assert_eq!(Path::new(cwd.strip_suffix('\0').unwrap()),roots.db.canonicalize().unwrap());
        assert_eq!(unsafe{libc::kill(pid,libc::SIGKILL)},0);fs::write(log.join("continue"),b"resume after actual backend SIGKILL").unwrap();let output=child.wait_with_output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));assert_eq!(unsafe{libc::kill(pid,0)},-1);
        let restart=roots.db.parent().unwrap().join("after-kill");postgres::directory(&restart).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let pg=postgres::Private::start_locked_legacy(&locks,&roots,&roots.db,&restart).unwrap();let c=ordinary::connect(&pg.socket,"postgres","postgres",None).await.unwrap();assert_eq!(c.client.query_one("SELECT 42",&[]).await.unwrap().get::<_,i32>(0),42);assert!(!c.client.query_one("SELECT datallowconn FROM pg_database WHERE datname='template0'",&[]).await.unwrap().get::<_,bool>(0));drop(c);pg.stop().unwrap();
        let after=inspect(&roots,&locks,SourceLocation::Root,&log,None).unwrap();
        assert_eq!(serde_json::to_vec(&after).unwrap(),before,"complete template0 snapshot/collations/index bytes changed after SIGKILL recovery");
    }

}

#[cfg(all(test,target_os="linux"))]mod orbstack_metadata_tests{
 use super::*;
 use crate::db_upgrade::shared_folder::SharedKind;
 use std::os::unix::fs::PermissionsExt;
 #[test]fn projected_diagnostics_keep_type_device_and_exact_mode(){
  if unsafe{libc::geteuid()}!=0{return;}
  let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let m=std::fs::metadata(t.path()).unwrap();
  assert!(diagnostic_metadata(&m,m.dev(),100,Some(SharedKind::OrbStackMac)));
  for kind in [None,Some(SharedKind::DockerDesktop)]{assert!(!diagnostic_metadata(&m,m.dev(),100,kind));}
  assert!(!diagnostic_metadata(&m,m.dev()+1,100,Some(SharedKind::OrbStackMac)));
  std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o750)).unwrap();assert!(!diagnostic_metadata(&std::fs::metadata(t.path()).unwrap(),m.dev(),100,Some(SharedKind::OrbStackMac)));
  std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();std::os::unix::fs::chown(t.path(),Some(0),Some(12345)).unwrap();assert!(!diagnostic_metadata(&std::fs::metadata(t.path()).unwrap(),m.dev(),100,Some(SharedKind::OrbStackMac)));
  let p=t.path().join("file");std::fs::write(&p,b"x").unwrap();std::fs::set_permissions(&p,std::fs::Permissions::from_mode(0o700)).unwrap();assert!(!diagnostic_metadata(&std::fs::metadata(p).unwrap(),m.dev(),100,Some(SharedKind::OrbStackMac)));
 }
}
