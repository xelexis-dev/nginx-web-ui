//! Schema-only rehearsal transport. Caller owns the FDs/scratch and has verified the source profile.
use super::StepError;
use crate::db_upgrade::postgres;
use std::{fs::File,io::{Seek,SeekFrom},os::{fd::AsRawFd,unix::{fs::MetadataExt,process::CommandExt}},path::Path,process::{Command,Stdio},time::Duration};
const MAX_SCHEMA_BYTES:u64=64*1024*1024;
fn failed()->StepError {StepError{code:"rehearsal_failed",message:"스키마 사전 검증용 사본을 준비하지 못했습니다. 원본과 작업 기록을 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {StepError{code:match e.raw_os_error(){Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EROFS)=>"storage_read_only",Some(libc::EINTR)=>"interrupted",_=>"io_error"},message:"스키마 검증용 파일 작업을 완료하지 못했습니다. 기존 파일은 보존했습니다.",detail:None}}
fn process_error(e:crate::db_upgrade::error::Error)->StepError {if e.code=="interrupted"{StepError{code:e.code,message:e.message,detail:None}}else{failed()}}
fn regular(file:&File,output:bool)->Result<(),StepError> {
    let m=file.metadata().map_err(|_|failed())?;let flags=unsafe{libc::fcntl(file.as_raw_fd(),libc::F_GETFL)};
    if !m.is_file() || m.nlink()!=1 || m.mode()&0o7777!=0o600 || !(m.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|m.uid()==uid))
        || flags<0 || m.len()>MAX_SCHEMA_BYTES || output && (m.len()!=0 || flags&libc::O_ACCMODE==libc::O_RDONLY) || !output && flags&libc::O_ACCMODE==libc::O_WRONLY {return Err(failed());}Ok(())
}
fn distinct(a:&File,b:&File)->Result<(),StepError> {let a=a.metadata().map_err(|_|failed())?;let b=b.metadata().map_err(|_|failed())?;if a.dev()==b.dev() && a.ino()==b.ino(){return Err(failed());}Ok(())}
fn command(major:u8,tool:&str,socket:&Path,database:&str)->Result<Command,StepError> {
    if !matches!((major,tool),(15,"pg_dump")|(17,"psql")) || !socket.is_absolute() || !matches!(database,"nginx_control"|"nginx_web_ui"){return Err(failed());}
    let mut c=postgres::command(major,tool).map_err(|_|failed())?;c.arg("--host").arg(socket).args(["--port","50432","--username","postgres","--no-password","--dbname",database]);if tool=="pg_dump"{c.arg("--schema-only");}else{c.args(["--no-psqlrc","--set=ON_ERROR_STOP=1"]);}Ok(c)
}
fn bounded(mut c:Command)->Command {
    // Bound file output in the child too; no unbounded pipe/Vec or disk-filling schema export.
    unsafe{c.pre_exec(||{let limit=libc::rlimit{rlim_cur:MAX_SCHEMA_BYTES as libc::rlim_t,rlim_max:MAX_SCHEMA_BYTES as libc::rlim_t};if libc::setrlimit(libc::RLIMIT_FSIZE,&limit)<0{return Err(std::io::Error::last_os_error());}Ok(())});}c
}
/// Read-only PG15 export; pg_dump --schema-only carries no table/ledger rows or role passwords.
pub(crate) fn dump(socket:&Path,database:&str,output:&mut File,stderr:&File)->Result<(),StepError> {
    regular(output,true)?;regular(stderr,true)?;distinct(output,stderr)?;output.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut c=command(15,"pg_dump",socket,database)?;c.stdout(output.try_clone().map_err(io_error)?).stderr(stderr.try_clone().map_err(io_error)?);
    postgres::wait(bounded(c).spawn().map_err(io_error)?,Duration::from_secs(300)).map_err(process_error)?;output.sync_all().map_err(io_error)?;
    if output.metadata().map_err(|_|failed())?.len()==0{return Err(failed());}Ok(())
}
/// Empty private PG17 app only. Roles exist first; caller seeds the verified ledger afterwards.
pub(crate) fn restore(socket:&Path,database:&str,schema:&mut File,stderr:&File)->Result<(),StepError> {
    regular(schema,false)?;regular(stderr,true)?;distinct(schema,stderr)?;if schema.metadata().map_err(|_|failed())?.len()==0{return Err(failed());}schema.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut c=command(17,"psql",socket,database)?;c.stdin(schema.try_clone().map_err(io_error)?).stdout(Stdio::null()).stderr(stderr.try_clone().map_err(io_error)?);
    postgres::wait(bounded(c).spawn().map_err(io_error)?,Duration::from_secs(300)).map_err(process_error)
}
#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::{OpenOptionsExt,PermissionsExt}};
    fn file(path:&Path)->File {fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(path).unwrap()}
    #[test]
    fn w1_rehearsal_transport_uses_fixed_clients_and_no_inherited_credentials_or_psqlrc() {
        for (major,tool) in [(15,"pg_dump"),(17,"psql")] {let c=command(major,tool,Path::new("/run/owned-socket"),"nginx_control").unwrap();let args=c.get_args().map(|v|v.to_str().unwrap()).collect::<Vec<_>>();assert_eq!(args[3],format!("/usr/lib/postgresql/{major}/bin/{tool}"));assert!(args.windows(2).any(|v|v==["--host","/run/owned-socket"]));assert!(args.contains(&"--no-password"));if tool=="pg_dump"{assert!(args.contains(&"--schema-only"));assert!(!args.contains(&"--create"));}else{assert!(args.contains(&"--no-psqlrc") && args.contains(&"--set=ON_ERROR_STOP=1"));}assert!(c.get_envs().all(|(key,_)|["HOME","PATH","LC_ALL","TZ"].iter().any(|name|key==*name)));}
        assert!(command(15,"pg_dump",Path::new("relative"),"nginx_web_ui").is_err());assert!(command(15,"pg_dump",Path::new("/run/owned"),"postgres").is_err());
    }
    #[test]
    fn w1_rehearsal_kernel_bound_stops_real_file_output_at_the_schema_limit() {
        let t=tempfile::tempdir().unwrap();let output=file(&t.path().join("bounded"));let mut c=Command::new("/usr/bin/dd");c.args(["if=/dev/zero","bs=1048576","count=65","status=none"]).env_clear().stdin(Stdio::null()).stdout(output.try_clone().unwrap()).stderr(Stdio::null()).process_group(0);
        assert!(postgres::wait(bounded(c).spawn().unwrap(),Duration::from_secs(30)).is_err());assert_eq!(output.metadata().unwrap().len(),MAX_SCHEMA_BYTES);
        assert_eq!(io_error(std::io::Error::from_raw_os_error(libc::ENOSPC)).code,"insufficient_space");assert_eq!(process_error(crate::db_upgrade::error::Error{code:"interrupted",message:"fixture",exit:143,detail:None}).code,"interrupted");
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17: real schema-only dump/restore transport"]
    async fn w1_rehearsal_native_schema_transport_keeps_schema_owner_but_copies_no_business_rows() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::inspect;let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();let mut servers=Vec::new();
        for major in [15,17] {let data=t.path().join(format!("data{major}"));let log=t.path().join(format!("log{major}"));postgres::directory(&data).unwrap();postgres::directory(&log).unwrap();let mut init=postgres::command(major,"initdb").unwrap();init.arg("-D").arg(&data).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&log,Duration::from_secs(300)).unwrap();let server=postgres::Private::start(major,&data,&log).unwrap();
            let root=inspect::connect(&server.socket,"postgres","postgres",None).await.unwrap();root.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();root.client.batch_execute("CREATE ROLE nginx LOGIN PASSWORD 'fixture-only'").await.unwrap();root.client.batch_execute("CREATE DATABASE nginx_control OWNER nginx").await.unwrap();drop(root);servers.push(server);
        }
        let source=inspect::connect(&servers[0].socket,"nginx_control","postgres",None).await.unwrap();source.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();source.client.batch_execute("CREATE TABLE public.probe(id integer PRIMARY KEY,secret text); INSERT INTO public.probe VALUES(1,'do-not-dump-business'); ALTER TABLE public.probe OWNER TO nginx").await.unwrap();drop(source);
        let mut schema=file(&t.path().join("schema.sql"));let dump_error=file(&t.path().join("dump.stderr"));tokio::task::block_in_place(||dump(&servers[0].socket,"nginx_control",&mut schema,&dump_error)).unwrap();let sql=fs::read_to_string(t.path().join("schema.sql")).unwrap();assert!(sql.contains("CREATE TABLE public.probe"));assert!(!sql.contains("do-not-dump-business"));let restore_error=file(&t.path().join("restore.stderr"));tokio::task::block_in_place(||restore(&servers[1].socket,"nginx_control",&mut schema,&restore_error)).unwrap();
        let target=inspect::connect(&servers[1].socket,"nginx_control","postgres",None).await.unwrap();assert_eq!(target.client.query_one("SELECT count(*)::bigint FROM public.probe",&[]).await.unwrap().get::<_,i64>(0),0);assert_eq!(target.client.query_one("SELECT pg_get_userbyid(relowner) FROM pg_class WHERE oid='public.probe'::regclass",&[]).await.unwrap().get::<_,String>(0),"nginx");drop(target);
        let mut config=crate::config::RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@must-not-contact.invalid/nginx_control".into()),_=>None}).unwrap();config.migrations_dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle");let migrations=crate::migrations::read_migrations(&config.migrations_dir).await.unwrap();let candidate=super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)};let rows=crate::db_upgrade::profiles::released().unwrap().signatures.into_iter().map(|s|(s.hash,s.created_at)).collect::<Vec<_>>();let mislabeled=super::super::PrivatePg{socket_dir:servers[0].socket.clone(),port:50432,dbname:"nginx_control".into(),role:"nginx".into(),major:17};
        assert_eq!(super::super::rehearsal::seed_and_migrate(&mislabeled,&config,&candidate,&rows).await.unwrap_err().detail.as_deref(),Some("target_major"));let source=inspect::connect(&servers[0].socket,"nginx_control","postgres",None).await.unwrap();assert_eq!(source.client.query_one("SELECT count(*)::bigint FROM public.probe",&[]).await.unwrap().get::<_,i64>(0),1);drop(source);for server in servers{server.stop().unwrap();}
    }
    #[test]
    fn w1_rehearsal_fd_validation_refuses_aliases_special_permissions_and_nonempty_outputs_before_spawn() {
        for case in ["alias","mode","populated","hardlink","directory"] {let t=tempfile::tempdir().unwrap();let path=t.path().join("schema");let mut output=file(&path);let mut error=file(&t.path().join("error"));
            match case{"alias"=>error=output.try_clone().unwrap(),"mode"=>fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap(),"populated"=>fs::write(&path,b"keep").unwrap(),"hardlink"=>fs::hard_link(&path,t.path().join("second-name")).unwrap(),"directory"=>output=File::open(t.path()).unwrap(),_=>{}}
            if case=="alias"{assert!(distinct(&output,&error).is_err());}else{assert!(regular(&output,true).is_err());}
            assert_eq!(dump(Path::new("/not-contacted"),"nginx_web_ui",&mut output,&error).unwrap_err().code,"rehearsal_failed");if case=="populated"{assert_eq!(fs::read(path).unwrap(),b"keep");}
        }
    }
}
