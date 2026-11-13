//! Offline conversion of independent copies under one validated attempt directory.
use super::{engine::Engine,arguments::Arguments,files,postgres,control::Control,cluster::{self,Snapshot},layout,pair::Pair,state::{self,State,Phase,Operation,Source,Destinations,LastError},error::{Error,Result}};
use crate::crypto::SecretBox;
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use std::{fs::{self,File},io::Read,path::{Path,PathBuf},time::Duration};
use std::os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt};
fn invalid()->Error{Error::refusal("unsupported_layout")}
fn failed()->Error{Error{code:"migration_failed",message:"이전 작업을 완료하지 못했습니다. 원본과 실패한 대상은 보존했습니다. 상태와 제한된 작업 로그를 확인하세요.",exit:1,detail:None}}
fn mismatch()->Error{Error::refusal("pair_mismatch")}
fn digest(value:&impl Serialize)->Result<String>{Ok(hex::encode(Sha256::digest(serde_json::to_vec(value).map_err(|_|failed())?)))}
#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Checked {workspace_sha256:String,app_sha256:String,target_identifier:String}
#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Inputs {
    schema_version:u8,architecture:String,backup_id:String,destinations:Destinations,
    db_sha256:String,app_sha256:String,db_bytes:u64,app_bytes:u64,db_files:u64,app_files:u64,
    old_version:String,new_version:String,source_control:Control,snapshot:Option<Snapshot>,checked:Option<Checked>,
}
fn log_name(label:&str)->String{format!("{label}-{:08x}",uuid::Uuid::new_v4().as_u128()as u32)}
struct Work {root:PathBuf}
impl Work {
    fn path(engine:&Engine,state:&State)->Result<Self>{let id=state.attempt_id.as_deref().filter(|id|state::uuid(id)).ok_or_else(invalid)?;Ok(Self{root:engine.roots.db.join(".nwui-work").join(id)})}
    fn create(engine:&Engine,state:&State)->Result<Self>{let work=Self::path(engine,state)?;postgres::directory(&engine.roots.db.join(".nwui-work"))?;postgres::directory(&work.root)?;Ok(work)}
    fn old(&self)->PathBuf{self.root.join("pg15")}
    fn new(&self)->PathBuf{self.root.join("pg17")}
    fn log(&self,label:&str)->Result<PathBuf>{let path=self.root.join(log_name(label));postgres::directory(&path)?;Ok(path)}
}
fn control_path(engine:&Engine,name:&str)->PathBuf{engine.roots.db.join(".nwui").join(name)}
fn journal(engine:&Engine,state:&State)->Result<()> {state.validate().map_err(|_|invalid())?;files::atomic_json(&control_path(engine,"migration-v1.json"),state).map_err(|_|failed())}
fn begin(engine:&Engine,state:&mut State,phase:Phase)->Result<()> {if let Some(e)=postgres::interrupted(){return Err(e);}state.begin(phase).map_err(|_|invalid())?;journal(engine,state)}
fn complete(engine:&Engine,state:&mut State,phase:Phase)->Result<()> {state.complete(phase).map_err(|_|invalid())?;journal(engine,state)}
fn save_inputs(engine:&Engine,state:&mut State,inputs:&Inputs)->Result<()> {files::atomic_json(&control_path(engine,"inputs-v1.json"),inputs).map_err(|_|failed())?;state.inputs_sha256=Some(digest(inputs)?);journal(engine,state)}
fn load_inputs(engine:&Engine,state:&State)->Result<Inputs> {
    let input:Inputs=files::read_json(&control_path(engine,"inputs-v1.json")).map_err(|_|invalid())?;
    if input.schema_version!=1||input.architecture!=state.target.architecture||state.inputs_sha256.as_deref()!=Some(digest(&input)?.as_str())||!state::sha256(&input.db_sha256)||!state::sha256(&input.app_sha256){return Err(invalid());}
    input.source_control.validate()?;input.source_control.require_cold()?;
    let source=state.source.as_ref().ok_or_else(invalid)?;
    if source.system_identifier!=input.source_control.system_identifier||source.backup_id!=input.backup_id||source.manifest_sha256!=digest(&(&input.db_sha256,&input.app_sha256))?{return Err(invalid());}
    if let Some(checked)=&input.checked {if !state::sha256(&checked.workspace_sha256)||!state::sha256(&checked.app_sha256)||state.target.system_identifier.as_deref()!=Some(checked.target_identifier.as_str()){return Err(invalid());}}
    Ok(input)
}
fn arguments_match(input:&Inputs,args:&Arguments)->Result<()> {
    if args.backup_id.as_deref()!=Some(input.backup_id.as_str())||digest(&args.destinations)?!=digest(&Some(&input.destinations))?{return Err(Error::refusal("invalid_arguments"));}Ok(())
}
fn version(major:u8)->Result<String>{let mut c=postgres::command(major,"pg_controldata")?;c.arg("--version");let(out,err)=postgres::output(c,Duration::from_secs(30))?;if !err.trim().is_empty()||!out.contains(&format!("(PostgreSQL) {major}.")){return Err(invalid());}Ok(out.trim().into())}
fn bytes(path:&Path,max:u64)->Result<Vec<u8>> {
    files::real_path(path).map_err(|_|invalid())?;
    let file=fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(|_|invalid())?;let meta=file.metadata().map_err(|_|invalid())?;
    if !meta.is_file()||meta.len()>max{return Err(invalid());}
    let mut out=Vec::new();file.take(max+1).read_to_end(&mut out).map_err(|_|failed())?;if out.len()as u64>max{return Err(invalid());}Ok(out)
}
fn exists(path:&Path)->Result<bool>{match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(invalid())}}
fn source_layout(engine:&Engine)->Result<()> {
    if std::str::from_utf8(&bytes(&engine.source_db.join("PG_VERSION"),16)?).map_err(|_|invalid())?.trim()!="15"||exists(&engine.source_db.join("postmaster.pid"))?||exists(&engine.source_db.join("standby.signal"))?||exists(&engine.source_db.join("recovery.signal"))?||exists(&engine.source_app.join(".nwui"))?{return Err(invalid());}
    for name in ["pg_tblspc","pg_twophase","pg_replslot"]{let path=engine.source_db.join(name);files::real_path(&path).map_err(|_|invalid())?;if fs::read_dir(path).map_err(|_|invalid())?.next().is_some(){return Err(invalid());}}
    let app=fs::metadata(&engine.source_app).map_err(|_|invalid())?;if app.uid()!=0||app.mode()&0o022!=0{return Err(mismatch());}
    layout::genuinely_empty(&engine.source_db)?;layout::genuinely_empty(&engine.source_app)?;Ok(())
}
fn source_unchanged(engine:&Engine,input:&Inputs)->Result<()> {if files::snapshot(&engine.source_db).map_err(|_|invalid())?!=input.db_sha256||files::snapshot(&engine.source_app).map_err(|_|invalid())?!=input.app_sha256{return Err(Error::refusal("source_changed"));}Ok(())}
fn payload(root:&Path)->Result<String> {
    let mut records=std::collections::BTreeMap::new();layout::genuinely_empty(root)?;
    for entry in fs::read_dir(root).map_err(|_|invalid())?{let entry=entry.map_err(|_|invalid())?;let name=entry.file_name().into_string().map_err(|_|invalid())?;if name==".nwui"||name=="lost+found"{continue;}records.insert(name,files::snapshot(&entry.path()).map_err(|_|invalid())?);}
    digest(&records)
}
fn copy_app(engine:&Engine)->Result<()> {
    for entry in fs::read_dir(&engine.source_app).map_err(|_|invalid())? {
        let entry=entry.map_err(|_|invalid())?;if entry.file_name()=="lost+found"{continue;}
        files::copy_tree(&entry.path(),&engine.roots.app.join(entry.file_name())).map_err(|_|failed())?;
    }
    let meta=fs::metadata(&engine.source_app).map_err(|_|invalid())?;
    std::os::unix::fs::chown(&engine.roots.app,Some(meta.uid()),Some(meta.gid())).map_err(|_|failed())?;
    fs::set_permissions(&engine.roots.app,fs::Permissions::from_mode(meta.mode()&0o7777)).map_err(|_|failed())?;File::open(&engine.roots.app).and_then(|d|d.sync_all()).map_err(|_|failed())?;
    if payload(&engine.source_app)?!=payload(&engine.roots.app)?{return Err(mismatch());}Ok(())
}
fn secret(root:&Path,name:&str)->Result<String> {
    let parent=root.join("secrets");files::real_path(&parent).map_err(|_|mismatch())?;let meta=fs::metadata(&parent).map_err(|_|mismatch())?;
    if !meta.is_dir()||meta.uid()!=0||meta.mode()&0o7777!=0o700{return Err(mismatch());}
    let path=parent.join(name);let meta=fs::symlink_metadata(&path).map_err(|_|mismatch())?;
    // Released 1.19 writes auth/db files as 0644 inside the private directory.
    if meta.uid()!=0||![0o600,0o644].contains(&(meta.mode()&0o7777)){return Err(mismatch());}
    let value=String::from_utf8(bytes(&path,65536)?).map_err(|_|mismatch())?;let value=value.trim_end_matches('\n');
    if value.is_empty()||value.contains('\0'){return Err(mismatch());}Ok(value.into())
}
pub(crate) fn key(root:&Path)->Result<Option<SecretBox>> {
    let path=super::paths::startup_data_key_file(root)?;
    match super::key_file::read_key_file(&path,super::key_file::canonical_policy(root,&path)) {
        Ok(key)=>Ok(Some(key.secrets)),Err(super::key_file::KeyFileError::Missing)=>Ok(None),Err(_)=>Err(mismatch()),
    }
}
fn space(root:&Path,required:u64,files:u64)->Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path=std::ffi::CString::new(root.as_os_str().as_bytes()).map_err(|_|invalid())?;let mut stats=std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe{libc::statvfs(path.as_ptr(),stats.as_mut_ptr())}!=0{return Err(failed());}let stats=unsafe{stats.assume_init()};
    let available=(stats.f_bavail as u64).checked_mul(stats.f_frsize as u64).ok_or_else(invalid)?;
    if available<required||(stats.f_files>0&&(stats.f_favail as u64)<files){return Err(Error::refusal("insufficient_space"));}Ok(available)
}
fn raw_copy_control(engine:&Engine,work:&Work)->Result<Control> {
    let root=work.root.join("control15");postgres::directory(&root)?;postgres::directory(&root.join("global"))?;
    postgres::private_file(&root.join("PG_VERSION"),b"15\n")?;postgres::private_file(&root.join("global/pg_control"),&bytes(&engine.source_db.join("global/pg_control"),65536)?)?;
    let control=postgres::read_control(15,&root)?;control.require_cold()?;Ok(control)
}
fn capacity(engine:&Engine,db:u64,app:u64,index:u64,db_files:u64,app_files:u64)->Result<(u64,u64)> {
    let required=super::engine::required_bytes(db,app,index)?;
    let db_inodes=db_files.checked_mul(2).and_then(|n|n.checked_add(1024)).ok_or_else(invalid)?;
    if fs::metadata(&engine.roots.db).map_err(|_|invalid())?.dev()==fs::metadata(&engine.roots.app).map_err(|_|invalid())?.dev(){Ok((required,space(&engine.roots.db,required,db_inodes.checked_add(app_files).ok_or_else(invalid)?)?))}
    else{let available=space(&engine.roots.db,required.checked_sub(app).ok_or_else(invalid)?,db_inodes)?;let app_available=space(&engine.roots.app,app,app_files.checked_add(1024).ok_or_else(invalid)?)?;Ok((required,available.checked_add(app_available).ok_or_else(invalid)?))}
}
fn checkpoint_layout(engine:&Engine,state:&State)->Result<()> {
    for entry in fs::read_dir(&engine.roots.db).map_err(|_|invalid())?{let name=entry.map_err(|_|invalid())?.file_name();if ![".nwui",".nwui-work","lost+found"].iter().any(|v|name==*v){return Err(invalid());}}
    let id=state.attempt_id.as_deref().ok_or_else(invalid)?;
    for entry in fs::read_dir(engine.roots.db.join(".nwui-work")).map_err(|_|invalid())?{if entry.map_err(|_|invalid())?.file_name()!=id{return Err(invalid());}}
    Ok(())
}
fn blank_copy_configuration(data:&Path)->Result<()> {
    let path=data.join("postgresql.auto.conf");files::real_path(&path).map_err(|_|invalid())?;
    let mut file=fs::OpenOptions::new().write(true).truncate(true).custom_flags(libc::O_NOFOLLOW).open(&path).map_err(|_|failed())?;
    use std::io::Write;file.write_all(b"# Offline working copy; source settings remain unchanged.\n").and_then(|_|file.sync_all()).map_err(|_|failed())
}
fn init_target(work:&Work,control:&Control,snapshot:&Snapshot)->Result<()> {
    let command=init_target_command(&work.new(),control,snapshot)?;
    postgres::directory(&work.new())?;
    postgres::capture(command,&work.log("init17")?,Duration::from_secs(300))?;Ok(())
}
/// Shared with the automatic converter; caller owns and journals the target directory.
pub(crate) fn init_target_command(data:&Path,control:&Control,snapshot:&Snapshot)->Result<std::process::Command> {
    let template=snapshot.databases.iter().find(|db|db.name=="template0").ok_or_else(invalid)?;
    let postgres=snapshot.databases.iter().find(|db|db.name=="postgres").ok_or_else(invalid)?;
    if (template.encoding.as_str(),template.collate.as_str(),template.ctype.as_str(),template.provider.as_str(),&template.icu)!=(postgres.encoding.as_str(),postgres.collate.as_str(),postgres.ctype.as_str(),postgres.provider.as_str(),&postgres.icu){return Err(invalid());}
    let wal:u64=control.setting("Bytes per WAL segment")?.parse().map_err(|_|invalid())?;let mb=wal/1_048_576;
    if wal%1_048_576!=0||!mb.is_power_of_two()||!(1..=1024).contains(&mb){return Err(invalid());}
    let checksum=control.setting("Data page checksum version")?;if !["0","1"].contains(&checksum){return Err(invalid());}
    let mut command=postgres::command(17,"initdb")?;
    command.arg("-D").arg(data).arg(format!("--encoding={}",template.encoding)).arg(format!("--lc-collate={}",template.collate)).arg(format!("--lc-ctype={}",template.ctype)).arg(format!("--wal-segsize={mb}")).args(["--auth-local=trust","--auth-host=scram-sha-256"]);
    match template.provider.as_str(){"c"=>{command.arg("--locale-provider=libc");},"i"=>{command.arg("--locale-provider=icu").arg(format!("--icu-locale={}",template.icu.as_deref().ok_or_else(invalid)?));},_=>return Err(invalid())}
    if checksum=="1"{command.arg("--data-checksums");}
    Ok(command)
}
// Paths are generated from the frozen roots and a validated UUID, never host metadata.
fn upgrade_options(data:&Path,config:&Path)->Result<String> {
    let values=[format!("data_directory={}",data.display()),format!("config_file={}",config.display()),format!("hba_file={}",config.with_file_name("hba.conf").display()),format!("ident_file={}",config.with_file_name("ident.conf").display()),"listen_addresses=".into(),"unix_socket_permissions=0700".into(),"shared_preload_libraries=".into(),"session_preload_libraries=".into(),"local_preload_libraries=".into(),"archive_mode=off".into(),"archive_command=".into(),"restore_command=".into(),"primary_conninfo=".into(),"ssl=off".into(),"max_logical_replication_workers=0".into(),"max_worker_processes=0".into(),"autovacuum=off".into(),"shared_buffers=16MB".into(),"max_connections=10".into(),"log_statement=none".into(),"log_min_error_statement=panic".into(),"log_parameter_max_length_on_error=0".into()];
    if values.iter().any(|v|!v.bytes().all(|b|b.is_ascii_alphanumeric()||b"_./=-".contains(&b))){return Err(invalid());}
    Ok(values.into_iter().map(|v|format!("-c {v}")).collect::<Vec<_>>().join(" "))
}
struct UpgradeServers{old:PathBuf,new:PathBuf,link:bool}
impl Drop for UpgradeServers{fn drop(&mut self){for(major,data)in[(15,&self.old),(17,&self.new)]{
    // Link conversion deliberately disables the stopped old cluster; its control is checked below.
    if major==15 && self.link && matches!(exists(&data.join("postmaster.pid")),Ok(false)) && matches!(exists(&data.join("global/pg_control")),Ok(false)) && matches!(exists(&data.join("global/pg_control.old")),Ok(true)){continue;}
    if postgres::stop_working(major,data).is_err(){eprintln!("이전 전용 PostgreSQL 종료를 확인하지 못했습니다. 작업 사본은 보존했습니다.");}
}}}
fn disabled_control(old:&Path,log:&Path)->Result<Control> {
    if exists(&old.join("global/pg_control"))? || exists(&old.join("postmaster.pid"))?{return Err(invalid());}
    let path=old.join("global/pg_control.old");files::real_path(&path).map_err(|_|invalid())?;let meta=fs::symlink_metadata(&path).map_err(|_|failed())?;
    if !meta.is_file() || meta.nlink()!=1 || meta.mode()&0o022!=0 || !(meta.uid()==0 || postgres::postgres_ids().is_ok_and(|(uid,_)|uid==meta.uid())){return Err(invalid());}
    let bytes=bytes(&path,65536)?;if bytes.is_empty(){return Err(invalid());}
    // pg_controldata accepts a datadir, not an alternate control filename. A diagnostic copy
    // preserves the disabled source and still checks the pinned tool's CRC/format/state.
    let copy=log.join("disabled-control15");postgres::directory(&copy)?;postgres::directory(&copy.join("global"))?;
    postgres::private_file(&copy.join("PG_VERSION"),b"15\n")?;postgres::private_file(&copy.join("global/pg_control"),&bytes)?;
    let control=postgres::read_control(15,&copy)?;control.require_cold()?;Ok(control)
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub(crate) enum UpgradeMode {Link,Copy}
fn upgrade(work:&Work,check:bool)->Result<()> {
    for data in [work.old(),work.new()]{if exists(&data.join("postmaster.pid"))?{return Err(invalid());}}
    let log=work.log(if check{"check"}else{"convert"})?;
    upgrade_at(&work.old(),&work.new(),&log,UpgradeMode::Copy,check)
}
fn upgrade_command(old:&Path,new:&Path,log:&Path,socket:&Path,mode:UpgradeMode,check:bool)->Result<std::process::Command> {
    let mut command=postgres::command(17,"pg_upgrade")?;
    command.env("PGOPTIONS",super::settings::ROOT_OPTIONS);
    command.current_dir(&log).arg("--old-bindir=/usr/lib/postgresql/15/bin").arg("--new-bindir=/usr/lib/postgresql/17/bin").arg("--old-datadir").arg(old).arg("--new-datadir").arg(new).arg("--socketdir").arg(socket).args(["--old-port=50432","--new-port=50433","--username=postgres","--jobs=2","--retain"]);
    command.arg(format!("--old-options={}",upgrade_options(old,&log.join("old.conf"))?)).arg(format!("--new-options={}",upgrade_options(new,&log.join("new.conf"))?));
    command.arg(match mode{UpgradeMode::Link=>"--link",UpgradeMode::Copy=>"--copy"});
    if check{command.arg("--check");}
    Ok(command)
}
/// Caller holds A+D, has verified P1, and journals may-be-linked before a real conversion.
/// No retry or link-to-copy fallback is safe here after pg_upgrade may have changed the source.
pub(crate) fn upgrade_at(old:&Path,new:&Path,log:&Path,mode:UpgradeMode,check:bool)->Result<()> {
    upgrade_paths(old,new,log,mode,check,false)
}
/// The automatic root layout necessarily nests W beneath L. Permit that only through
/// the fixed journal-owned W/data + W/<phase>-UUID mapping, never arbitrary nested paths.
/// The caller has verified the full P1 manifest before creating/converting this target.
pub(crate) fn upgrade_auto_at(roots:&layout::Roots,locks:&crate::upgrade::StorageLocks,j:&crate::upgrade::Journal,log_name:&str,mode:UpgradeMode,check:bool)->Result<()> {
    use crate::upgrade::{Branch,Step,StepState};
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || j.load().branch!=Branch::Upgrade{return Err(mismatch());}
    let current=crate::upgrade::Journal::open(&roots.app,locks).map_err(|_|mismatch())?;
    if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    let id=&j.load().attempt_id;if !(state::uuid(id) && uuid::Uuid::parse_str(id).is_ok_and(|u|u.hyphenated().to_string()==*id)){return Err(mismatch());}
    let prefix=if check{"check-"}else{"convert-"};let suffix=log_name.strip_prefix(prefix).ok_or_else(invalid)?;
    if !(state::uuid(suffix) && uuid::Uuid::parse_str(suffix).is_ok_and(|u|u.hyphenated().to_string()==suffix)){return Err(invalid());}
    let header=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done).and_then(|r|r.data.as_ref()).and_then(|d|d.get("header")).ok_or_else(mismatch)?;
    if header.get("backupId").and_then(|v|v.as_str())!=Some(id.as_str()){return Err(mismatch());}
    let old=match header.get("sourceLocation").and_then(|v|v.as_str()){Some("root")=>roots.db.clone(),Some("pgdata")=>roots.db.join("pgdata"),_=>return Err(mismatch())};
    crate::upgrade::precommit_selector(&roots.app,j).map_err(|_|mismatch())?;
    if header.get("systemIdentifier").and_then(|v|v.as_str())!=Some(postgres::read_control(15,&old)?.system_identifier.as_str()){return Err(mismatch());}
    let work=roots.db.join(".nwui-auto-work").join(id);let new=work.join("data");let log=crate::upgrade::diagnostic_log_path(&roots.db,j,log_name,check).map_err(|_|mismatch())?;
    upgrade_paths(&old,&new,&log,mode,check,true)
}
// The stopped upgrade's report survives check-target removal and R-restore.
fn archive_upgrade_output(new:&Path,log:&Path)->Result<()> {
    use std::os::fd::AsRawFd;
    let source=new.join("pg_upgrade_output.d");if !exists(&source)?{return Ok(());}
    for path in [new,log]{files::real_path(path).map_err(|_|invalid())?;}
    let directory=|path:&Path|fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_DIRECTORY).open(path).map_err(|_|invalid());
    let from=directory(new)?;let to=directory(log)?;let report=directory(&source)?;
    let original=report.metadata().map_err(|_|invalid())?;
    if original.dev()!=from.metadata().map_err(|_|invalid())?.dev() || original.dev()!=to.metadata().map_err(|_|invalid())?.dev(){return Err(invalid());}
    let name=c"pg_upgrade_output.d";let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe{libc::fstatat(to.as_raw_fd(),name.as_ptr(),stat.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}==0 || std::io::Error::last_os_error().kind()!=std::io::ErrorKind::NotFound{return Err(invalid());}
    if unsafe{libc::renameat(from.as_raw_fd(),name.as_ptr(),to.as_raw_fd(),name.as_ptr())}!=0{return Err(failed());}
    from.sync_all().and_then(|_|to.sync_all()).map_err(|_|failed())?;
    let moved=directory(&log.join("pg_upgrade_output.d"))?.metadata().map_err(|_|invalid())?;
    if moved.dev()!=original.dev() || moved.ino()!=original.ino(){return Err(invalid());}Ok(())
}
fn upgrade_paths(old:&Path,new:&Path,log:&Path,mode:UpgradeMode,check:bool,owned_auto:bool)->Result<()> {
    for path in [old,new,log] {files::real_path(path).map_err(|_|invalid())?;if !path.is_absolute() || !fs::symlink_metadata(path).map_err(|_|invalid())?.is_dir(){return Err(invalid());}}
    let a=fs::metadata(old).map_err(|_|invalid())?;let b=fs::metadata(new).map_err(|_|invalid())?;
    if a.dev()==b.dev() && a.ino()==b.ino() || old.starts_with(new) || !owned_auto && (new.starts_with(old) || log.starts_with(old)) || log.starts_with(new)
        || mode==UpgradeMode::Link && a.dev()!=b.dev() {return Err(invalid());}
    for data in [old,new]{if exists(&data.join("postmaster.pid"))?{return Err(invalid());}}
    let before_old=postgres::read_control(15,old)?;before_old.require_cold()?;
    let before_new=postgres::read_control(17,new)?;before_new.require_cold()?;
    let socket_directory=postgres::SocketDirectory::new(&log,"upgrade")?;let socket=&socket_directory.path;
    let old_config=log.join("old.conf");let new_config=log.join("new.conf");postgres::private_file(&old_config,b"# Offline upgrade\n")?;postgres::private_file(&new_config,b"# Offline upgrade\n")?;
    postgres::private_file(&log.join("hba.conf"),b"local all postgres trust\nlocal all nginx scram-sha-256\nlocal all all reject\n")?;postgres::private_file(&log.join("ident.conf"),b"")?;
    let command=upgrade_command(old,new,log,socket,mode,check)?;
    // Declared after the socket guard: stop postmasters before reclaiming sockets on every exit.
    let servers=UpgradeServers{old:old.to_owned(),new:new.to_owned(),link:mode==UpgradeMode::Link && !check};
    let result=postgres::capture(command,&log,Duration::from_secs(86400));
    drop(servers);
    archive_upgrade_output(new,log)?;
    // Retain all data and diagnostics, never the destructive convenience script.
    let script=log.join("delete_old_cluster.sh");if exists(&script)?{files::real_path(&script).map_err(|_|invalid())?;if !fs::symlink_metadata(&script).map_err(|_|invalid())?.is_file(){return Err(invalid());}fs::remove_file(script).map_err(|_|failed())?;File::open(&log).and_then(|d|d.sync_all()).map_err(|_|failed())?;}
    result?;
    let after_old=if mode==UpgradeMode::Link && !check{disabled_control(old,log)?}else{let c=postgres::read_control(15,old)?;c.require_cold()?;c};
    let after_new=postgres::read_control(17,new)?;after_new.require_cold()?;
    if before_old.system_identifier!=after_old.system_identifier || before_new.system_identifier!=after_new.system_identifier{return Err(mismatch());}
    before_old.compatible_with(&after_old)?;before_new.compatible_with(&after_new)?;
    for data in [old,new]{if exists(&data.join("postmaster.pid"))?{return Err(failed());}}
    Ok(())
}
pub(super) async fn check_new(engine:&Engine,args:&Arguments)->Result<State> {
    source_layout(engine)?;
    if !layout::genuinely_empty(&engine.roots.db)?||!layout::genuinely_empty(&engine.roots.app)?{return Err(invalid());}
    let db=files::tree_info(&engine.source_db).map_err(|_|invalid())?;let app=files::tree_info(&engine.source_app).map_err(|_|invalid())?;
    let(required,available)=capacity(engine,db.bytes,app.bytes,0,db.files,app.files)?;
    secret(&engine.source_app,"auth_secret")?;let password=secret(&engine.source_app,"db_password")?;let source_key=key(&engine.source_app)?;
    for root in [&engine.roots.db,&engine.roots.app]{files::private_dir(&root.join(".nwui")).map_err(|_|failed())?;}
    let mut state=State::not_started(&engine.roots.architecture);state.attempt_id=Some(uuid::Uuid::new_v4().to_string());state.pair_id=Some(uuid::Uuid::new_v4().to_string());state.destinations=args.destinations.clone();
    state.summary.source_db_bytes=Some(db.bytes);state.summary.source_app_bytes=Some(app.bytes);state.summary.required_bytes=Some(required);state.summary.available_bytes=Some(available);
    let result=async {
        begin(engine,&mut state,Phase::Validated)?;let work=Work::create(engine,&state)?;
        let source_control=raw_copy_control(engine,&work)?;
        let mut input=Inputs{schema_version:1,architecture:engine.roots.architecture.clone(),backup_id:args.backup_id.clone().ok_or_else(invalid)?,destinations:args.destinations.clone().ok_or_else(invalid)?,db_sha256:db.sha256,app_sha256:app.sha256,db_bytes:db.bytes,app_bytes:app.bytes,db_files:db.files,app_files:app.files,old_version:version(15)?,new_version:version(17)?,source_control,snapshot:None,checked:None};
        state.source=Some(Source{major:15,architecture:"unrecorded".into(),system_identifier:input.source_control.system_identifier.clone(),manifest_sha256:digest(&(&input.db_sha256,&input.app_sha256))?,backup_id:input.backup_id.clone()});
        save_inputs(engine,&mut state,&input)?;complete(engine,&mut state,Phase::Validated)?;
        begin(engine,&mut state,Phase::Copied)?;
        files::copy_tree(&engine.source_db,&work.old()).map_err(|_|failed())?;postgres::own_tree(&work.old())?;copy_app(engine)?;blank_copy_configuration(&work.old())?;
        let log=work.log("inspect15")?;let pg=postgres::Private::start(15,&work.old(),&log)?;
        let snapshot=cluster::inspect_source(&pg.socket,&password,source_key.as_ref()).await?;pg.stop()?;
        let required=super::engine::required_bytes(input.db_bytes,input.app_bytes,snapshot.largest_index_bytes)?;
        let remaining=required.checked_sub(input.db_bytes).and_then(|n|n.checked_sub(input.app_bytes)).ok_or_else(invalid)?;
        space(&engine.roots.db,remaining,input.db_files.checked_add(1024).ok_or_else(invalid)?)?;
        if fs::metadata(&engine.roots.db).map_err(|_|invalid())?.dev()!=fs::metadata(&engine.roots.app).map_err(|_|invalid())?.dev(){space(&engine.roots.app,65536,16)?;}
        state.summary.largest_index_bytes=Some(snapshot.largest_index_bytes);state.summary.required_bytes=Some(required);state.summary.database_count=Some(snapshot.databases.len()as u64);state.summary.table_counts=Some(snapshot.table_counts()?);
        init_target(&work,&input.source_control,&snapshot)?;let target=postgres::read_control(17,&work.new())?;input.source_control.compatible_with(&target)?;
        if target.system_identifier==input.source_control.system_identifier{return Err(invalid());}state.target.system_identifier=Some(target.system_identifier.clone());
        input.snapshot=Some(snapshot);save_inputs(engine,&mut state,&input)?;complete(engine,&mut state,Phase::Copied)?;
        begin(engine,&mut state,Phase::Checked)?;upgrade(&work,true)?;source_unchanged(engine,&input)?;
        input.checked=Some(Checked{workspace_sha256:files::snapshot(&work.root).map_err(|_|failed())?,app_sha256:payload(&engine.roots.app)?,target_identifier:target.system_identifier});
        save_inputs(engine,&mut state,&input)?;complete(engine,&mut state,Phase::Checked)?;
        if let Some(e)=postgres::interrupted(){return Err(e);}Ok(state.clone())
    }.await;
    if let Err(error)=&result{record_failure(engine,&mut state,error);}result
}
fn record_failure(engine:&Engine,state:&mut State,error:&Error) {
    let signal=postgres::interrupted();let error=signal.as_ref().unwrap_or(error);
    state.operation=Operation::Failed;state.next_phase=None;state.ready=false;state.last_error=Some(LastError{code:error.code.into(),message:error.message.into()});
    if journal(engine,state).is_err(){eprintln!("이전 실패 상태를 저장하지 못했습니다. 대상을 전환하지 마세요.");}
}
pub(super) fn validate_arguments(engine:&Engine,state:&State,args:&Arguments)->Result<()> {arguments_match(&load_inputs(engine,state)?,args)}
pub(super) fn resume_checked(engine:&Engine,state:&State,args:&Arguments)->Result<()> {
    if !state.can_continue()||state.phase!=Phase::Checked{return Err(Error::refusal("migration_incomplete"));}
    source_layout(engine)?;checkpoint_layout(engine,state)?;let input=load_inputs(engine,state)?;arguments_match(&input,args)?;source_unchanged(engine,&input)?;
    let work=Work::path(engine,state)?;files::real_path(&work.root).map_err(|_|invalid())?;
    let checked=input.checked.as_ref().ok_or_else(invalid)?;
    if input.snapshot.is_none()||version(15)?!=input.old_version||version(17)?!=input.new_version||files::snapshot(&work.root).map_err(|_|invalid())?!=checked.workspace_sha256||payload(&engine.roots.app)?!=checked.app_sha256{return Err(invalid());}
    let old=postgres::read_control(15,&work.old())?;let new=postgres::read_control(17,&work.new())?;old.require_cold()?;new.require_cold()?;old.compatible_with(&new)?;
    if old.system_identifier!=input.source_control.system_identifier||new.system_identifier!=checked.target_identifier{return Err(mismatch());}Ok(())
}
fn sync_tree(path:&Path,depth:u8,count:&mut u64)->Result<()> {
    *count+=1;if depth>64||*count>1_000_000{return Err(invalid());}files::real_path(path).map_err(|_|invalid())?;let meta=fs::symlink_metadata(path).map_err(|_|invalid())?;
    if meta.is_dir(){for child in fs::read_dir(path).map_err(|_|failed())?{sync_tree(&child.map_err(|_|failed())?.path(),depth+1,count)?;}}
    else if !meta.is_file(){return Err(invalid());}
    File::open(path).and_then(|f|f.sync_all()).map_err(|_|failed())
}
async fn inspect_target(engine:&Engine,input:&Inputs,data:&Path,work:&Work,postprocess:bool,rename_legacy:bool,informational:bool)->Result<bool> {
    let baseline=input.snapshot.as_ref().ok_or_else(invalid)?;let password=secret(&engine.roots.app,"db_password")?;let target_key=key(&engine.roots.app)?;
    if exists(&data.join("postmaster.pid"))?{return Err(invalid());}
    let log=work.log("verify17")?;let pg=postgres::Private::start(17,data,&log)?;
    if postprocess{cluster::postprocess(&pg.socket,baseline).await?;}
    let comparison=match cluster::inspect_cluster(&pg.socket,&password,target_key.as_ref(),false).await {
        Ok(snapshot)=>baseline.require_equivalent(&snapshot),
        Err(error) if informational&&error.code=="unsupported_layout"=>Err(Error{code:"baseline_changed",message:"이전 기준과 데이터가 달라졌습니다. 전환 후 데이터 변경은 정상일 수 있습니다.",exit:1,detail:None}),
        Err(error)=>Err(error),
    };
    let changed=diagnostic_result(informational,comparison)?;
    if rename_legacy&&cluster::rename_legacy_application(&pg.socket,baseline).await? {
        let snapshot=cluster::inspect_cluster(&pg.socket,&password,target_key.as_ref(),false).await?;baseline.require_equivalent(&snapshot)?;
    }
    pg.stop()?;let control=postgres::read_control(17,data)?;control.require_cold()?;
    if input.checked.as_ref().map(|c|c.target_identifier.as_str())!=Some(control.system_identifier.as_str()){return Err(mismatch());}Ok(changed)
}
fn publish_pair(engine:&Engine,state:&mut State,input:&Inputs,work:&Work)->Result<()> {
    let baseline=input.snapshot.as_ref().ok_or_else(invalid)?;
    let pair=Pair{schema_version:1,pair_id:state.pair_id.clone().ok_or_else(invalid)?,mode:"migration".into(),ready:true,major:17,architecture:state.target.architecture.clone(),system_identifier:state.target.system_identifier.clone(),encryption_key_required:baseline.key_required(),fresh_step:None};pair.validate()?;
    if input.checked.as_ref().map(|checked|checked.app_sha256.as_str())!=Some(payload(&engine.roots.app)?.as_str()){return Err(mismatch());}
    // Check all material before any ready marker, including the existing permissions policy.
    layout::ready_material(&engine.roots,&pair)?;
    source_unchanged(engine,input)?;
    let parent=engine.roots.db.join("17");if exists(&parent)?{return Err(invalid());}postgres::directory(&parent)?;
    let data=parent.join("data");fs::rename(work.new(),&data).map_err(|_|failed())?;
    File::open(&work.root).and_then(|f|f.sync_all()).map_err(|_|failed())?;File::open(&parent).and_then(|f|f.sync_all()).map_err(|_|failed())?;
    if exists(&engine.source_db.join(".db-random-pw"))?{postgres::private_file(&data.join(".db-random-pw"),&bytes(&engine.source_db.join(".db-random-pw"),65536)?)?;}
    postgres::private_file(&data.join(".migrated-nginx-web-ui"),b"")?;
    sync_tree(&data,0,&mut 0)?;sync_tree(&engine.roots.app,0,&mut 0)?;
    let control=postgres::read_control(17,&data)?;control.require_cold()?;
    if pair.system_identifier.as_deref()!=Some(control.system_identifier.as_str()){return Err(mismatch());}
    for root in [&engine.roots.db,&engine.roots.app]{files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).map_err(|_|failed())?;}
    for root in [&engine.roots.db,&engine.roots.app]{let saved:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).map_err(|_|failed())?;if saved!=pair{return Err(mismatch());}}
    state.checks.pair_agreement=true;Ok(())
}
fn upgrade_copy(engine:&Engine,state:&mut State,work:&Work)->Result<()> {
    begin(engine,state,Phase::Converted)?;upgrade(work,false)?;complete(engine,state,Phase::Converted)
}
#[cfg(test)]
pub(crate) fn stop_after_conversion_for_test(engine:&Engine,args:&Arguments,mut state:State)->Result<State> {
    engine.validate_mounts()?;let _locks=lock(&[&engine.source_db,&engine.source_app,&engine.roots.db,&engine.roots.app])?;
    resume_checked(engine,&state,args)?;let work=Work::path(engine,&state)?;upgrade_copy(engine,&mut state,&work)?;Ok(state)
}
pub(super) async fn convert(engine:&Engine,args:&Arguments,mut state:State)->Result<State> {
    resume_checked(engine,&state,args)?;let input=load_inputs(engine,&state)?;let work=Work::path(engine,&state)?;
    let result=async {
        upgrade_copy(engine,&mut state,&work)?;
        begin(engine,&mut state,Phase::Verified)?;
        inspect_target(engine,&input,&work.new(),&work,true,true,false).await?;source_unchanged(engine,&input)?;
        state.checks.source_unchanged=true;state.checks.role_password=true;state.checks.encrypted_data=true;state.checks.logical_data=true;state.checks.collations=true;state.checks.clean_shutdown=true;
        complete(engine,&mut state,Phase::Verified)?;
        begin(engine,&mut state,Phase::Ready)?;publish_pair(engine,&mut state,&input,&work)?;complete(engine,&mut state,Phase::Ready)?;Ok(state.clone())
    }.await;
    if let Err(error)=&result{record_failure(engine,&mut state,error);}result
}
fn diagnostic_result(ready:bool,result:Result<()>)->Result<bool> {
    match result{Ok(())=>Ok(false),Err(error) if ready&&error.code=="baseline_changed"=>Ok(true),Err(error)=>Err(error)}
}
pub(super) async fn diagnostic(engine:&Engine,state:&State)->Result<bool> {
    if state.operation!=Operation::Idle||!matches!(state.phase,Phase::Converted|Phase::Verified|Phase::Ready){return Err(Error::refusal("migration_incomplete"));}
    let input=load_inputs(engine,state)?;let work=Work::path(engine,state)?;
    let data=if state.ready{engine.roots.db.join("17/data")}else{work.new()};
    inspect_target(engine,&input,&data,&work,false,false,state.ready).await
}
pub(super) fn lock(roots:&[&Path])->Result<Vec<File>> {
    let mut files=Vec::new();for path in roots{let file=File::open(path).map_err(|_|invalid())?;file.try_lock().map_err(|_|Error::refusal("storage_busy"))?;files.push(file);}Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_output_is_archived_with_identity_and_foreign_destinations_preserved() {
        use std::os::unix::fs::symlink;
        for case in ["valid","absent","symlink","file","destination"] {
            let t=tempfile::tempdir().unwrap();let new=t.path().join("new");let log=t.path().join("log");fs::create_dir(&new).unwrap();fs::create_dir(&log).unwrap();let report=new.join("pg_upgrade_output.d");
            match case {"absent"=>{},"symlink"=>symlink(&log,&report).unwrap(),"file"=>fs::write(&report,b"preserve").unwrap(),_=>{fs::create_dir(&report).unwrap();fs::write(report.join("report"),b"diagnostic").unwrap();}}
            if case=="destination"{fs::create_dir(log.join("pg_upgrade_output.d")).unwrap();}
            let before=if case=="symlink"{None}else{Some(files::snapshot(t.path()).unwrap())};
            if matches!(case,"valid"|"absent"){archive_upgrade_output(&new,&log).unwrap();if case=="valid"{assert!(!report.exists());assert_eq!(fs::read(log.join("pg_upgrade_output.d/report")).unwrap(),b"diagnostic");}}
            else{assert!(archive_upgrade_output(&new,&log).is_err());if let Some(before)=before{assert_eq!(files::snapshot(t.path()).unwrap(),before);}else{assert_eq!(fs::read_link(&report).unwrap(),log);assert!(fs::read_dir(&log).unwrap().next().is_none());}}
        }
    }
    #[test]
    fn ready_differences_are_informational_but_converted_and_operational_failures_are_not() {
        let changed=||Error{code:"baseline_changed",message:"changed",exit:1,detail:None};
        assert!(diagnostic_result(true,Err(changed())).unwrap());
        assert!(diagnostic_result(false,Err(changed())).is_err());
        assert!(diagnostic_result(true,Err(Error::refusal("pair_mismatch"))).is_err());
        assert!(!diagnostic_result(true,Ok(())).unwrap());
    }
    #[test]
    #[ignore="native PG17: NWUI_TEST_SOCKET_CLEANUP=1"]
    fn failed_upgrade_reclaims_its_runtime_socket_directory(){
        if std::env::var("NWUI_TEST_SOCKET_CLEANUP").as_deref()!=Ok("1"){return;}
        let root=std::env::temp_dir().join(format!("d11-upgrade-{}",uuid::Uuid::new_v4()));postgres::directory(&root).unwrap();let work=Work{root:root.clone()};postgres::directory(&work.old()).unwrap();postgres::directory(&work.new()).unwrap();
        assert!(upgrade(&work,true).is_err());
        for log in fs::read_dir(&root).unwrap(){let path=log.unwrap().path();assert!(!postgres::socket_path(&path,"upgrade").exists());}
        assert!(work.old().exists()&&work.new().exists());
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17: isolated physical transport, not A22 qualification"]
    async fn w1_conversion_native_link_and_copy_preserve_rows_and_use_the_requested_transport() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        for mode in [UpgradeMode::Link,UpgradeMode::Copy] {
            let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();
            let old=t.path().join("old");let new=t.path().join("new");let init=t.path().join("init");let source_log=t.path().join("source");let check=t.path().join("check");let convert=t.path().join("convert");let target_log=t.path().join("target");
            for path in [&old,&new,&init,&source_log,&check,&convert,&target_log]{postgres::directory(path).unwrap();}
            let mut command=postgres::command(15,"initdb").unwrap();command.arg("-D").arg(&old).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(command,&init,Duration::from_secs(300)).unwrap();
            let source=postgres::Private::start(15,&old,&source_log).unwrap();
            let mut config=tokio_postgres::Config::new();config.host_path(&source.socket).port(50432).user("postgres").dbname("postgres");
            let (client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);
            client.batch_execute("CREATE TABLE public.transport_probe(id integer PRIMARY KEY,value text NOT NULL); INSERT INTO public.transport_probe VALUES(1,'first'),(2,'second'),(42,'fixture')").await.unwrap();
            let source_file:String=client.query_one("SELECT pg_relation_filepath('public.transport_probe')",&[]).await.unwrap().get(0);
            let (databases,roles)=super::super::inspect::read_catalog(&client).await.unwrap();drop(client);driver.await.unwrap().unwrap();source.stop().unwrap();
            let control=postgres::read_control(15,&old).unwrap();control.require_cold().unwrap();let snapshot=Snapshot{databases,roles,data:Default::default(),collations:Default::default(),largest_index_bytes:0};
            postgres::capture(init_target_command(&new,&control,&snapshot).unwrap(),&init,Duration::from_secs(300)).unwrap();
            upgrade_at(&old,&new,&check,mode,true).unwrap();let result=upgrade_at(&old,&new,&convert,mode,false);
            if result.is_err() {
                eprintln!("transport {mode:?}: old control={}, disabled control={}, target cold={}",old.join("global/pg_control").exists(),old.join("global/pg_control.old").exists(),postgres::read_control(17,&new).and_then(|c|c.require_cold()).is_ok());
                for entry in fs::read_dir(&convert).unwrap(){let path=entry.unwrap().path();if path.extension().is_some_and(|ext|ext=="stdout" || ext=="stderr"){eprintln!("synthetic transport tool output: {}",String::from_utf8_lossy(&bytes(&path,65536).unwrap()));}}
            }
            result.unwrap();let target=postgres::Private::start(17,&new,&target_log).unwrap();
            // Build a new config: host_path appends, so reusing the source config would add a host.
            let mut config=tokio_postgres::Config::new();config.host_path(&target.socket).port(50432).user("postgres").dbname("postgres");
            let (client,driver)=config.connect(tokio_postgres::NoTls).await.unwrap();let driver=tokio::spawn(driver);
            let rows=client.query("SELECT id,value FROM public.transport_probe ORDER BY id",&[]).await.unwrap().into_iter().map(|r|(r.get::<_,i32>(0),r.get::<_,String>(1))).collect::<Vec<_>>();assert_eq!(rows,[(1,"first".into()),(2,"second".into()),(42,"fixture".into())]);
            let target_file:String=client.query_one("SELECT pg_relation_filepath('public.transport_probe')",&[]).await.unwrap().get(0);
            assert_eq!(fs::metadata(old.join(source_file)).unwrap().ino()==fs::metadata(new.join(target_file)).unwrap().ino(),mode==UpgradeMode::Link);
            drop(client);driver.await.unwrap().unwrap();target.stop().unwrap();
        }
    }
    #[test]
    fn w1_conversion_disabled_control_refuses_special_or_ambiguous_material_before_copying() {
        for case in ["both","symlink","empty","hardlink"] {
            let t=tempfile::tempdir().unwrap();let old=t.path().join("old");let log=t.path().join("log");fs::create_dir(&old).unwrap();fs::create_dir(old.join("global")).unwrap();fs::create_dir(&log).unwrap();let path=old.join("global/pg_control.old");
            match case {"both"=>{fs::write(old.join("global/pg_control"),b"keep").unwrap();fs::write(&path,b"disabled").unwrap();},"symlink"=>std::os::unix::fs::symlink("/not-followed",&path).unwrap(),"empty"=>fs::write(&path,b"").unwrap(),_=>{fs::write(old.join("other"),b"keep").unwrap();fs::hard_link(old.join("other"),&path).unwrap();}}
            assert!(disabled_control(&old,&log).is_err(),"{case}");assert!(!log.join("disabled-control15").exists());assert!(fs::symlink_metadata(path).is_ok());
        }
    }
    #[test]
    fn w1_conversion_initdb_preserves_source_wal_checksum_and_template_locale() {
        let control=Control::parse(&format!("Database system identifier: 123\nDatabase cluster state: shut down\n{}",super::super::control::COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: {}\n",if *f=="Bytes per WAL segment"{"16777216"}else{"1"})).collect::<String>()),"").unwrap();
        let db=|name:&str|super::super::catalog::Database{name:name.into(),owner:"postgres".into(),encoding:"UTF8".into(),collate:"C.UTF-8".into(),ctype:"C.UTF-8".into(),provider:"c".into(),icu:None};
        let mut snapshot=Snapshot{databases:vec![db("template0"),db("postgres")],roles:Vec::new(),data:Default::default(),collations:Default::default(),largest_index_bytes:0};
        let command=init_target_command(Path::new("/owned/work/data"),&control,&snapshot).unwrap();let args=command.get_args().map(|a|a.to_str().unwrap()).collect::<Vec<_>>();
        for value in ["--encoding=UTF8","--lc-collate=C.UTF-8","--lc-ctype=C.UTF-8","--locale-provider=libc","--wal-segsize=16","--data-checksums","--auth-local=trust","--auth-host=scram-sha-256"]{assert!(args.contains(&value),"{value}");}
        snapshot.databases[1].collate="C".into();assert!(init_target_command(Path::new("/owned/work/data"),&control,&snapshot).is_err());
        for db in &mut snapshot.databases{db.collate="en-US".into();db.ctype="en-US".into();db.provider="i".into();db.icu=Some("en-US".into());}
        let command=init_target_command(Path::new("/owned/work/data"),&control,&snapshot).unwrap();let args=command.get_args().map(|a|a.to_str().unwrap()).collect::<Vec<_>>();assert!(args.contains(&"--locale-provider=icu") && args.contains(&"--icu-locale=en-US"));
        snapshot.databases[0].provider="x".into();snapshot.databases[1].provider="x".into();assert!(init_target_command(Path::new("/owned/work/data"),&control,&snapshot).is_err());
    }
    #[test]
    fn w1_conversion_command_selects_one_mode_and_keeps_private_server_limits() {
        for mode in [UpgradeMode::Link,UpgradeMode::Copy] {for check in [false,true] {
            let command=upgrade_command(Path::new("/fixed/old"),Path::new("/fixed/new"),Path::new("/fixed/log"),Path::new("/run/private-socket"),mode,check).unwrap();
            let args=command.get_args().map(|s|s.to_str().unwrap()).collect::<Vec<_>>();
            assert_eq!(args.iter().filter(|arg|matches!(**arg,"--link"|"--copy")).count(),1);
            assert!(args.contains(&if mode==UpgradeMode::Link{"--link"}else{"--copy"}));assert_eq!(args.contains(&"--check"),check);
            for arg in ["--jobs=2","--retain","--old-port=50432","--new-port=50433","/run/private-socket"] {assert!(args.contains(&arg));}
            assert!(args.iter().any(|arg|arg.contains("listen_addresses=")));assert!(args.iter().any(|arg|arg.contains("shared_preload_libraries=")));
        }}
    }
    #[test]
    fn w1_conversion_refuses_aliased_nested_or_live_roots_before_creating_tool_files() {
        for kind in ["same","nested","log-in-source","pid","symlink"] {
            let t=tempfile::tempdir().unwrap();let old=t.path().join("old");let mut new=t.path().join("new");let mut log=t.path().join("log");
            for p in [&old,&new,&log]{fs::create_dir(p).unwrap();}
            match kind {
                "same"=>new=old.clone(),"nested"=>{new=old.join("nested");fs::create_dir(&new).unwrap();},
                "log-in-source"=>{log=old.join("log");fs::create_dir(&log).unwrap();},
                "pid"=>fs::write(old.join("postmaster.pid"),b"1\n").unwrap(),
                "symlink"=>{fs::remove_dir(&new).unwrap();std::os::unix::fs::symlink(&old,&new).unwrap();},_=>unreachable!(),
            }
            let before=files::snapshot(t.path());assert!(upgrade_at(&old,&new,&log,UpgradeMode::Link,false).is_err());
            if let Ok(before)=before{assert_eq!(files::snapshot(t.path()).unwrap(),before);}assert!(!log.join("old.conf").exists());
        }
    }
    #[test]
    fn upgrade_postmasters_use_private_auth_and_no_source_or_operator_shell_values() {
        let options=upgrade_options(Path::new("/destination/db/.nwui-work/id/pg15"),Path::new("/destination/db/.nwui-work/id/log/old.conf")).unwrap();
        for setting in ["-c listen_addresses=","-c hba_file=/destination/db/.nwui-work/id/log/hba.conf","-c ident_file=/destination/db/.nwui-work/id/log/ident.conf","-c max_logical_replication_workers=0","-c log_min_error_statement=panic"]{assert!(options.contains(setting));}
        assert!(upgrade_options(Path::new("/unsafe path"),Path::new("/safe/config")).is_err());
    }
}
