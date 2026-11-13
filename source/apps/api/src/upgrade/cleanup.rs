//! A.24 P6: retire only manifest-owned PG15 entries after this boot's activation.
use super::{Journal,StepError,StorageLocks,selector::SelectedGeneration};
use crate::db_upgrade::layout::Roots;

/// Phase 2 supplies A-only custody; production PG17 keeps its shared D lock.
pub(crate) fn cleanup(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&SelectedGeneration)->Result<(),StepError>{
    if !admitted(j,selected,super::boot_identity::current().ok().as_deref()){return Ok(());}
    staged_key_state(j,selected.selector.generation.as_deref().ok_or_else(mismatch)?)?;
    delete_closed_set(roots,locks,j,selected)?;
    remove_staged_key(roots,j,selected)?;
    remove_work_dir(roots,locks,j)?;
    super::reserves::drain(roots,locks,j)?;
    if !j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.state==StepState::Done&&r.data.is_none()){j.record(Step::P6Cleanup,StepState::Done,None)?;}
    Ok(())
}
/// Operator restore supplies its own offline admission, not phase-2 activation proof.
pub(crate) fn delete_closed_set(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&SelectedGeneration)->Result<(),StepError>{
    if !locks.holds_a()||!locks.matches_app(&roots.app)||!j.belongs_to(&roots.app)||j.load().branch!=Branch::Upgrade{return Err(mismatch());}
    let selector=&selected.selector;selector.validate()?;
    if selector.active_major!=17||selector.origin!=super::selector::Origin::Auto||selector.backup_id.as_deref()!=Some(j.load().attempt_id.as_str())||super::selector::read_selector(&roots.app)?!=*selector{return Err(mismatch());}
    let manifest=super::backup::open_verified_manifest(roots,j)?;let names=closed_names(&manifest.top_names,manifest.header.source_location)?;
    let (states,_)=entry_states(j,&names)?;let pending=names.iter().filter(|name|states.get(*name)!=Some(&StepState::Done)).collect::<Vec<_>>();
    // Operator resume may already have removed the marker; an entirely Done set has no work.
    if pending.is_empty(){return Ok(());}
    let l=match manifest.header.source_location{SourceLocation::Root=>roots.db.clone(),SourceLocation::Pgdata=>roots.db.join("pgdata")};
    let parent=super::target::directory(&l)?;super::guard::verify_marker(&parent,&manifest.header.backup_id,selector.generation.as_deref().ok_or_else(mismatch)?)?;
    let scope=boundary(&parent)?;let mut seen=0;
    // No entry is mutated until every pending subtree passes the same device/mount checks.
    for name in &pending{let name=CString::new(name.as_str()).map_err(|_|mismatch())?;visit(&parent,&name,&scope,0,&mut seen,false,"")?;}
    let mut seen=0;
    for name in pending {
        let data=Some(serde_json::json!({"entry":name}));
        if states.get(name).is_none(){j.record(Step::P6Cleanup,StepState::Intent,data.clone())?;}
        checkpoint("intent",name);
        let leaf=CString::new(name.as_str()).map_err(|_|mismatch())?;visit(&parent,&leaf,&scope,0,&mut seen,true,name)?;
        parent.sync_all().map_err(io_error)?;checkpoint("after-fsync",name);checkpoint("before-done",name);
        j.record(Step::P6Cleanup,StepState::Done,data)?;
    }Ok(())
}

use super::{Outcome,Step,StepState,Branch,journal::Handoff,backup_format::SourceLocation};
use std::{ffi::{CStr,CString},fs::File,io::Read,os::{fd::{AsRawFd,FromRawFd,IntoRawFd},unix::fs::MetadataExt}};
fn mismatch()->StepError{StepError{code:"pair_mismatch",message:"이전 데이터 정리의 근거와 실제 위치가 다릅니다. 남은 데이터와 백업을 보존했습니다.",detail:Some("cleanup authority".into())}}
fn io_error(error:std::io::Error)->StepError{super::backup_format::io_error(error)}
fn admitted(j:&Journal,selected:&SelectedGeneration,identity:Option<&str>)->bool {
    let Some(identity)=identity else{return false};let selector=&selected.selector;
    if j.load().branch!=Branch::Upgrade||j.load().outcome!=Outcome::Pg17||j.load().handoff!=Handoff::Done||selector.active_major!=17||selector.validate().is_err(){return false;}
    let Some(record)=j.load().records.iter().rev().find(|r|r.step==Step::Phase2Monitor)else{return false};
    let Some(data)=record.data.as_ref()else{return false};
    let handoff=j.load().records.iter().rev().find(|r|r.step==Step::Handoff);
    record.state==StepState::Done&&data.get("generation").and_then(|v|v.as_str())==selector.generation.as_deref()
        &&data.get("bootIdentity").and_then(|v|v.as_str())==Some(identity)
        &&data.get("observedAt").and_then(|v|v.as_str()).is_some_and(|s|chrono::DateTime::parse_from_rfc3339(s).is_ok())
        &&handoff.is_some_and(|r|r.state==StepState::Done&&r.data.as_ref().and_then(|v|v.get("bootIdentity")).and_then(|v|v.as_str())==Some(identity))
}
fn reserved(name:&str,location:SourceLocation)->bool {
    matches!(name,"UPGRADED-TO-PG17.txt"|"NWUI-RESTORE-IN-PROGRESS.txt"|"lost+found"|".nwui")||name.starts_with(".nwui-")||name=="17"&&location==SourceLocation::Root
}
fn closed_names(names:&std::collections::BTreeSet<String>,location:SourceLocation)->Result<Vec<String>,StepError>{
    let mut result=Vec::new();for name in names {
        if name.is_empty()||name.len()>255||matches!(name.as_str(),"."|"..")||name.contains(['/', '\0']){return Err(mismatch());}
        if !reserved(name,location){result.push(name.clone());}
    }
    if !result.iter().any(|n|n=="PG_VERSION.nwui-pre17"){result.push("PG_VERSION.nwui-pre17".into());}Ok(result)
}
/// Manifest names bound to this attempt define completion, not the final reserve-drain receipt.
pub(super) fn pending_entries(roots:&Roots,j:&Journal)->Result<(std::path::PathBuf,std::collections::BTreeSet<String>),StepError>{
    let manifest=super::backup::open_verified_manifest(roots,j)?;let names=closed_names(&manifest.top_names,manifest.header.source_location)?;
    let (states,_)=entry_states(j,&names)?;
    let l=match manifest.header.source_location{SourceLocation::Root=>roots.db.clone(),SourceLocation::Pgdata=>roots.db.join("pgdata")};
    Ok((l,names.into_iter().filter(|name|states.get(name)!=Some(&StepState::Done)).collect()))
}
fn entry_states(j:&Journal,names:&[String])->Result<(std::collections::BTreeMap<String,StepState>,bool),StepError>{
    let mut states=std::collections::BTreeMap::new();let mut complete=false;
    for record in j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&!work_record(r,j)){
        if staged_key_record(record)?{
            let generation=j.load().records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Intent).and_then(|r|r.data.as_ref()?.get("generation")?.as_str()).ok_or_else(mismatch)?;
            if record.data.as_ref().and_then(|d|d.get("stagedKey")).and_then(|v|v.as_str())!=Some(generation){return Err(mismatch());}continue;
        }
        if complete{return Err(mismatch());}
        let Some(data)=record.data.as_ref()else{
            if record.state!=StepState::Done||!names.iter().all(|n|states.get(n)==Some(&StepState::Done)){return Err(mismatch());}complete=true;continue;
        };
        let object=data.as_object().ok_or_else(mismatch)?;let entry=object.get("entry").and_then(|v|v.as_str()).ok_or_else(mismatch)?;
        if object.len()!=1||!names.iter().any(|name|name==entry){return Err(mismatch());}
        match &record.state {
            StepState::Intent if states.get(entry).is_none_or(|s|s==&StepState::Intent)=>{},
            StepState::Done if states.get(entry)==Some(&StepState::Intent)=>{},
            _=>return Err(mismatch()),
        }
        states.insert(entry.to_owned(),record.state.clone());
    }Ok((states,complete))
}
fn staged_key_record(record:&super::journal::Record)->Result<bool,StepError>{
    let Some(data)=record.data.as_ref().filter(|v|v.get("stagedKey").is_some())else{return Ok(false)};
    if !data.as_object().is_some_and(|o|o.len()==1&&o.get("stagedKey").and_then(|v|v.as_str()).is_some_and(super::types::canonical_generation)){return Err(mismatch());}Ok(true)
}
fn staged_key_state(j:&Journal,generation:&str)->Result<Option<StepState>,StepError>{
    let mut state=None;
    for r in j.load().records.iter().filter(|r|r.step==Step::P6Cleanup){
        if !staged_key_record(r)?{continue;}
        if r.data.as_ref().and_then(|d|d.get("stagedKey")).and_then(|v|v.as_str())!=Some(generation){return Err(mismatch());}
        match (&state,&r.state){(None,StepState::Intent)|(Some(StepState::Intent),StepState::Done)=>{},_=>return Err(mismatch())}state=Some(r.state.clone());
    }Ok(state)
}
/// Only the staged leaf is mutable; immutable generation snapshots and the canonical key stay.
fn remove_staged_key(roots:&Roots,j:&mut Journal,selected:&SelectedGeneration)->Result<(),StepError>{
    let generation=selected.selector.generation.as_deref().ok_or_else(mismatch)?;let state=staged_key_state(j,generation)?;
    if state==Some(StepState::Done){return Ok(());}
    let path=roots.app.join(".nwui-auto/gen").join(generation).join("data_key");
    let parent=crate::db_upgrade::key_file::validate_parent(&path,crate::db_upgrade::key_file::KeyPolicy::Strict).map_err(|_|mismatch())?;
    let app=super::target::directory(&roots.app)?;check_boundary(&parent,&boundary(&app)?)?;
    let file=opened(&parent,c"data_key",libc::O_RDONLY)?;
    if file.is_none()&&state.is_none(){return Ok(());}
    let data=Some(serde_json::json!({"stagedKey":generation}));
    if state.is_none(){j.record(Step::P6Cleanup,StepState::Intent,data.clone())?;}
    checkpoint("staged-key-intent","data_key");
    if let Some(mut file)=file{
        let metadata=file.metadata().map_err(io_error)?;
        if !metadata.is_file()||metadata.uid()!=0||metadata.mode()&0o7777!=0o600||metadata.nlink()!=1||metadata.len()==0||metadata.len()>4096{return Err(mismatch());}
        let mut bytes=Vec::new();(&mut file).take(4097).read_to_end(&mut bytes).map_err(io_error)?;
        if bytes.len()>4096||super::selector::digest(&bytes)!=selected.key_record.key_sha256{return Err(mismatch());}
        same_leaf(&parent,c"data_key",&file)?;
        if unsafe{libc::unlinkat(parent.as_raw_fd(),c"data_key".as_ptr(),0)}!=0{return Err(io_error(std::io::Error::last_os_error()));}
    }
    checkpoint("staged-key-unlinked","data_key");parent.sync_all().map_err(io_error)?;
    j.record(Step::P6Cleanup,StepState::Done,data)
}
fn work_record(record:&super::journal::Record,j:&Journal)->bool{
    record.data.as_ref().and_then(|v|v.as_object()).is_some_and(|o|o.len()==1&&o.get("workDir").and_then(|v|v.as_str())==Some(j.load().attempt_id.as_str()))
}
/// Operator restore shares this A-only primitive; activation and reserve drain belong to callers.
pub(crate) fn remove_work_dir(roots:&Roots,locks:&StorageLocks,j:&mut Journal)->Result<(),StepError>{
    if !locks.holds_a()||!locks.matches_app(&roots.app)||!j.belongs_to(&roots.app)||j.load().branch!=Branch::Upgrade||!super::types::canonical_generation(&j.load().attempt_id){return Err(mismatch());}
    let state=j.work_dir_state(&j.load().attempt_id);
    if state==super::journal::WorkDirState::Done{return Ok(());}
    let db=super::target::directory(&roots.db)?;let scope=boundary(&db)?;
    let parent=super::target::directory(&roots.db.join(".nwui-auto-work"))?;check_boundary(&parent,&scope)?;
    let name=CString::new(j.load().attempt_id.as_str()).map_err(|_|mismatch())?;
    let work=opened(&parent,&name,libc::O_PATH)?;
    if state==super::journal::WorkDirState::None&&work.is_none(){return Ok(());}
    let data=Some(serde_json::json!({"workDir":j.load().attempt_id}));
    if state==super::journal::WorkDirState::None{j.record(Step::P6Cleanup,StepState::Intent,data.clone())?;}
    let mut linked=None;
    if let Some(work)=&work{
        check_boundary(work,&scope)?;let directory=opened(work,c".",libc::O_RDONLY|libc::O_DIRECTORY)?.ok_or_else(mismatch)?;
        let mut seen=0;let entries=children(&directory,&mut seen)?;if entries.iter().any(|n|n.as_c_str()!=c"linked"){return Err(mismatch());}
        if let Some(child)=opened(&directory,c"linked",libc::O_PATH)?{
            check_boundary(&child,&scope)?;let inner=opened(&child,c".",libc::O_RDONLY|libc::O_DIRECTORY)?.ok_or_else(mismatch)?;
            if !children(&inner,&mut seen)?.is_empty(){return Err(mismatch());}linked=Some((directory,child));
        }
    }
    if let Some((directory,child))=&linked{same_leaf(directory,c"linked",child)?;if unsafe{libc::unlinkat(directory.as_raw_fd(),c"linked".as_ptr(),libc::AT_REMOVEDIR)}!=0{return Err(io_error(std::io::Error::last_os_error()));}}
    if let Some(work)=&work{same_leaf(&parent,&name,work)?;if unsafe{libc::unlinkat(parent.as_raw_fd(),name.as_ptr(),libc::AT_REMOVEDIR)}!=0{return Err(io_error(std::io::Error::last_os_error()));}}
    checkpoint("work-removed","work");parent.sync_all().map_err(io_error)?;j.record(Step::P6Cleanup,StepState::Done,data)
}
/// Caller owns authority and journal/locks; this only removes one validated leaf beneath its fd.
pub(crate) fn delete_entry(parent:&File,name:&str)->Result<(),StepError>{
    if name.is_empty()||name.len()>255||matches!(name,"."|"..")||name.contains(['/', '\0']){return Err(mismatch());}
    let parent=opened(parent,c".",libc::O_RDONLY|libc::O_DIRECTORY)?.ok_or_else(mismatch)?;
    let name_c=CString::new(name).map_err(|_|mismatch())?;let scope=boundary(&parent)?;let mut seen=0;
    visit(&parent,&name_c,&scope,0,&mut seen,false,name)?;let mut seen=0;
    visit(&parent,&name_c,&scope,0,&mut seen,true,name)?;parent.sync_all().map_err(io_error)
}
struct Boundary{device:u64,mount:u64}
fn mount_id(file:&File)->Result<u64,StepError>{
    let mut text=String::new();File::open(format!("/proc/self/fdinfo/{}",file.as_raw_fd())).map_err(io_error)?.take(4097).read_to_string(&mut text).map_err(io_error)?;
    if text.len()>4096{return Err(mismatch());}let mut ids=text.lines().filter_map(|s|s.strip_prefix("mnt_id:")).map(str::trim);
    let id=ids.next().and_then(|s|s.parse::<u64>().ok()).ok_or_else(mismatch)?;if ids.next().is_some(){return Err(mismatch());}Ok(id)
}
fn opened(parent:&File,name:&CStr,flags:i32)->Result<Option<File>,StepError>{
    let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),flags|libc::O_NOFOLLOW|libc::O_CLOEXEC|libc::O_NONBLOCK)};
    if fd>=0{return Ok(Some(unsafe{File::from_raw_fd(fd)}));}let error=std::io::Error::last_os_error();if error.kind()==std::io::ErrorKind::NotFound{Ok(None)}else{Err(io_error(error))}
}
fn boundary(file:&File)->Result<Boundary,StepError>{Ok(Boundary{device:file.metadata().map_err(io_error)?.dev(),mount:mount_id(file)?})}
fn check_boundary(file:&File,scope:&Boundary)->Result<(),StepError>{
    if file.metadata().map_err(io_error)?.dev()!=scope.device||mount_id(file)?!=scope.mount{return Err(mismatch());}Ok(())
}
fn count(count:&mut u64)->Result<(),StepError>{*count=count.checked_add(1).ok_or_else(mismatch)?;if *count>super::backup_format::MAX_ENTRIES{return Err(mismatch());}Ok(())}
struct Directory(*mut libc::DIR);
impl Drop for Directory{fn drop(&mut self){unsafe{libc::closedir(self.0);}}}
fn children(directory:&File,seen:&mut u64)->Result<Vec<CString>,StepError>{
    let file=opened(directory,c".",libc::O_RDONLY|libc::O_DIRECTORY)?.ok_or_else(mismatch)?;let fd=file.into_raw_fd();let stream=unsafe{libc::fdopendir(fd)};
    if stream.is_null(){let error=std::io::Error::last_os_error();unsafe{libc::close(fd);};return Err(io_error(error));}let stream=Directory(stream);let mut names=Vec::new();
    loop {
        unsafe{*libc::__errno_location()=0;};let entry=unsafe{libc::readdir(stream.0)};
        if entry.is_null(){let error=std::io::Error::last_os_error();if error.raw_os_error()!=Some(0){return Err(io_error(error));}break;}
        let name=unsafe{CStr::from_ptr((*entry).d_name.as_ptr())};if matches!(name.to_bytes(),b"."|b".."){continue;}count(seen)?;names.push(name.to_owned());
    }
    names.sort();Ok(names)
}
fn same_leaf(parent:&File,name:&CStr,file:&File)->Result<(),StepError>{
    let current=opened(parent,name,libc::O_PATH)?.ok_or_else(mismatch)?;let a=current.metadata().map_err(io_error)?;let b=file.metadata().map_err(io_error)?;
    if a.dev()!=b.dev()||a.ino()!=b.ino(){return Err(mismatch());}Ok(())
}
// Preflight is read-only; all pending top-level entries pass it before the first Intent.
fn visit(parent:&File,name:&CStr,scope:&Boundary,depth:u8,seen:&mut u64,remove:bool,entry:&str)->Result<(),StepError>{
    if depth>64{return Err(mismatch());}let Some(file)=opened(parent,name,libc::O_PATH)?else{return Ok(())};if depth==0{count(seen)?;}check_boundary(&file,scope)?;
    let is_directory=file.metadata().map_err(io_error)?.is_dir();
    if is_directory {
        let directory=opened(&file,c".",libc::O_RDONLY|libc::O_DIRECTORY)?.ok_or_else(mismatch)?;check_boundary(&directory,scope)?;
        for child in children(&directory,seen)?{visit(&directory,&child,scope,depth+1,seen,remove,entry)?;}
        if remove{directory.sync_all().map_err(io_error)?;}
    }
    if remove {
        same_leaf(parent,name,&file)?;if unsafe{libc::unlinkat(parent.as_raw_fd(),name.as_ptr(),if is_directory{libc::AT_REMOVEDIR}else{0})}!=0{return Err(io_error(std::io::Error::last_os_error()));}
        checkpoint("mid-unlink",entry);
    }Ok(())
}
#[cfg(test)]fn checkpoint(point:&str,entry:&str){if std::env::var("NWUI_CLEANUP_KILL_POINT").as_deref()==Ok(point)&&std::env::var("NWUI_CLEANUP_KILL_ENTRY").as_deref()==Ok(entry){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn checkpoint(_point:&str,_entry:&str){}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,Outcome,Step,StepState,KeyBinding,backup,backup_format::SourceLocation,journal::{Candidate,Handoff},selector::{self,Selector,GenerationManifest,KeyRecord,Origin,StoredKeySource}};
    use crate::db_upgrade::{pair::Pair,control::{Control,COMPATIBILITY_FIELDS}};
    use std::{fs::{self,OpenOptions},path::{Path,PathBuf},os::unix::fs::{PermissionsExt,OpenOptionsExt,symlink}};
    struct Fixture { _temp:tempfile::TempDir,roots:Roots,locks:StorageLocks,j:Journal,selected:SelectedGeneration,l:PathBuf }
    fn write(path:&Path,bytes:&[u8]) {fs::write(path,bytes).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();}
    fn fixture(location:SourceLocation)->Fixture {fixture_with_proof(location,true)}
    fn fixture_with_proof(location:SourceLocation,proof:bool)->Fixture {fixture_with_original_work(location,proof,false)}
    fn fixture_with_original_work(location:SourceLocation,proof:bool,literal_work:bool)->Fixture {fixture_build(location,proof,literal_work,false)}
    fn fixture_build(location:SourceLocation,proof:bool,literal_work:bool,native_pg17:bool)->Fixture {
        let temp=tempfile::tempdir().unwrap();let roots=Roots{db:temp.path().join("db"),app:temp.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        if native_pg17{fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o711)).unwrap();crate::db_upgrade::postgres::own(&roots.db).unwrap();}
        let l=match location{SourceLocation::Root=>roots.db.clone(),SourceLocation::Pgdata=>{let p=roots.db.join("pgdata");fs::create_dir(&p).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o700)).unwrap();p}};
        write(&l.join("PG_VERSION"),b"15\n");fs::create_dir(l.join("base")).unwrap();write(&l.join("base/one"),b"obsolete PG15 one");write(&l.join("base/two"),b"obsolete PG15 two");fs::create_dir(l.join("global")).unwrap();write(&l.join("global/pg_control"),b"cold fixture control");
        let mut locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        super::super::reserves::ensure(&roots,&locks,&j).unwrap();
        let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());let control=Control::parse(&text,"").unwrap();
        if literal_work{write(&l.join("work"),b"original L/work");}
        let mut names=std::collections::BTreeSet::from(["PG_VERSION".into(),"base".into(),"global".into()]);if literal_work{names.insert("work".into());}let prepared=backup::prepare(&roots,&locks,&j.load().attempt_id,location,&names,&control,"2026-10-29T00:00:00Z").unwrap();backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();
        let mut system_identifier="987654".to_owned();let mut key_file_bytes=vec![7;32];
        if native_pg17{
            use crate::db_upgrade::postgres;let parent=roots.db.join("17");postgres::directory(&parent).unwrap();let data=parent.join("data");postgres::directory(&data).unwrap();let logs=temp.path().join("init17");postgres::directory(&logs).unwrap();let mut init=postgres::command(17,"initdb").unwrap();init.arg("-D").arg(&data).args(["--locale=C","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&logs,std::time::Duration::from_secs(120)).unwrap();system_identifier=postgres::read_control(17,&data).unwrap().system_identifier;
            key_file_bytes=hex::encode([7u8;32]).into_bytes();fs::create_dir(roots.app.join("secrets")).unwrap();fs::set_permissions(roots.app.join("secrets"),fs::Permissions::from_mode(0o700)).unwrap();write(&roots.app.join("secrets/data_key"),&key_file_bytes);
        }
        let generation=uuid::Uuid::new_v4().to_string();let pair=Pair{schema_version:1,pair_id:uuid::Uuid::new_v4().to_string(),mode:"migration".into(),ready:true,major:17,architecture:roots.architecture.clone(),system_identifier:Some(system_identifier.clone()),encryption_key_required:true,fresh_step:None};
        let key=KeyRecord{version:1,key_binding:KeyBinding::Verified{pair_id:pair.pair_id.clone()},key_source:StoredKeySource::Canonical,key_sha256:selector::digest(&key_file_bytes)};
        let pair_bytes=serde_json::to_vec(&pair).unwrap();let key_bytes=serde_json::to_vec(&key).unwrap();let manifest=GenerationManifest{version:1,generation:generation.clone(),system_identifier:system_identifier.clone(),origin:Origin::Auto,profile_id:Some("legacy-28".into()),profile_sha256:Some(super::super::profile::packaged_identity().unwrap().profile_sha256),db_pair_sha256:selector::digest(&pair_bytes),app_pair_sha256:selector::digest(&pair_bytes),key_binding_sha256:selector::digest(&key_bytes),created_at:"2026-10-29T00:00:00Z".into()};let manifest_bytes=serde_json::to_vec(&manifest).unwrap();
        let root=roots.app.join(".nwui-auto/gen").join(&generation);fs::create_dir_all(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
        for (name,bytes) in [("manifest.json",&manifest_bytes),("db-pair.json",&pair_bytes),("app-pair.json",&pair_bytes),("key-binding.json",&key_bytes)]{write(&root.join(name),bytes);}
        let selected=Selector{version:1,active_major:17,generation:Some(generation.clone()),system_identifier:system_identifier.clone(),origin:Origin::Auto,backup_id:Some(j.load().attempt_id.clone()),profile_id:manifest.profile_id.clone(),profile_sha256:manifest.profile_sha256.clone(),manifest_sha256:Some(selector::digest(&manifest_bytes)),operator_reason:None};write(&roots.app.join(".nwui-auto/active-v1.json"),&serde_json::to_vec(&selected).unwrap());
        j.record(Step::P3Convert,StepState::Intent,Some(serde_json::json!({"mayBeLinked":true,"mode":"link"}))).unwrap();
        j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"authorityEvent":"selector-intent","generation":generation,"selectorSha256":selector::digest(&serde_json::to_vec(&selected).unwrap())}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
        fs::create_dir(roots.app.join(".nwui")).unwrap();fs::set_permissions(roots.app.join(".nwui"),fs::Permissions::from_mode(0o700)).unwrap();write(&roots.app.join(".nwui/pair-v1.json"),&pair_bytes);
        let selected=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected,&system_identifier,&key_file_bytes).unwrap();
        fs::rename(l.join("PG_VERSION"),l.join("PG_VERSION.nwui-pre17")).unwrap();
        let marker=format!("NWUI: PostgreSQL 17 upgrade. Do not start an older image on these volumes.\nBackup: {}\nGeneration: {}\nStop the container and follow the documented PG15 restore procedure. Keep both volumes and the backup.\n",j.load().attempt_id,generation);write(&l.join("UPGRADED-TO-PG17.txt"),marker.as_bytes());
        fs::create_dir_all(roots.db.join("17/data")).unwrap();write(&roots.db.join("17/data/preserve"),b"live PG17");if native_pg17{crate::db_upgrade::postgres::own(&roots.db.join("17/data/preserve")).unwrap();}
        j.set_outcome(Outcome::Pg17).unwrap();let identity=super::super::boot_identity::current().unwrap();j.handoff(Handoff::Intent,&identity).unwrap();locks.release_d();j.handoff(Handoff::Done,&identity).unwrap();
        if proof {j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"bootIdentity":identity,"generation":generation,"observedAt":chrono::Utc::now().to_rfc3339()}))).unwrap();}
        Fixture{_temp:temp,roots,locks,j,selected,l}
    }
    async fn g34_native_row(roots:Roots,mut locks:StorageLocks,mut j:Journal,selected:SelectedGeneration){
        use super::super::{runner,fdholder,custody};use crate::db_upgrade::postgres;
        let config=crate::config::RuntimeConfig::read(|key|std::env::var(key).ok()).unwrap();let identity=super::super::boot_identity::current().unwrap();let generation=selected.selector.generation.as_ref().unwrap().clone();
        let bootstrap=if locks.holds_d(){None}else{let fd=File::open(&roots.db).unwrap();fd.try_lock().unwrap();Some(fd)};
        let work=PathBuf::from("/run/g34").join(format!("pg-{}",uuid::Uuid::new_v4()));postgres::directory(&work).unwrap();let server=postgres::Private::start(17,&roots.db.join("17/data"),&work).unwrap();
        fs::write("/run/g34/db.json",serde_json::to_vec(&serde_json::json!({"socket":server.socket})).unwrap()).unwrap();
        if locks.holds_d(){j.handoff(Handoff::Intent,&identity).unwrap();locks.release_d();j.handoff(Handoff::Done,&identity).unwrap();}drop(bootstrap);let shared=File::open(&roots.db).unwrap();shared.lock_shared().unwrap();
        assert!(fdholder::list().unwrap().is_empty());fdholder::store(fdholder::CustodyId::A,locks.a_fd()).unwrap();drop(locks);let custody=custody::Custody::acquire(roots.clone(),false).await.unwrap();let mut startup=runner::Startup{roots:roots.clone(),custody,journal:Some(j),identity:Some(identity.clone()),plan:runner::BootPlan::Phase2};
        let deadline=||std::time::Instant::now()+std::time::Duration::from_secs(5);
        assert!(crate::control::confirmation::uncertain(&config).await);
        let nginx_before=crate::db_upgrade::files::snapshot(&config.nginx_conf_dir).unwrap();let before=startup.journal.as_ref().unwrap().load().records.len();
        assert!(runner::monitor_after_handoff(&mut startup,&config,deadline()).await.unwrap());let j=startup.journal.as_ref().unwrap();assert_eq!(j.load().outcome,Outcome::Pg17);assert!(!runner::activation_proof(j,&identity,&generation));assert!(j.load().records[before..].iter().all(|r|!matches!(r.state,StepState::Failed{..})&&r.step!=Step::P6Cleanup));assert!(!super::super::boot::notices(&roots,j).unwrap().iter().any(|n|n.kind=="system.db_upgrade_activation_failed"));assert_eq!(nginx_before,crate::db_upgrade::files::snapshot(&config.nginx_conf_dir).unwrap());
        let status=super::super::status::read().unwrap();assert_eq!(status.outcome,Outcome::Pg17);assert!(status.cleanup_pending);assert!(status.failure.is_none());assert!(roots.db.join("base/one").is_file());
        let offline=super::super::commands::offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();assert!(offline.cleanup_pending);assert!(offline.failure.is_none());
        println!("G34_NATIVE_ROW {}",serde_json::json!({"row":"kept-a8","phase":"A8-kept","outcome":"pg17","cleanupPending":true,"failure":null,"proof":false,"p6":false,"bootIdentity":identity}));
        let now=std::time::Instant::now();let mut retry=runner::CleanupRetry::new(now);let records=startup.journal.as_ref().unwrap().load().records.len();assert_eq!(runner::retry_cleanup_after_handoff(&mut startup,&config,&mut retry,now,deadline()).await.unwrap(),None);assert_eq!(startup.journal.as_ref().unwrap().load().records.len(),records);
        fdholder::delete(fdholder::CustodyId::A).unwrap();drop(startup);drop(shared);server.stop().unwrap();
    }
    // A.33 eecdb398: s6/health/generation negatives → G18 rows3–5; new-A.19 identity → W1 79cfa261 native evidence; real ApplySaved retry → G18 Fixture.
    #[tokio::test(flavor="multi_thread",worker_threads=4)]
    #[ignore="native G34 real kept-A8 observation; explicitly enabled with NWUI_TEST_PROFILE_NATIVE=1"]
    async fn g34_native_kept_a8_real_observation(){
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        assert_eq!(std::env::var("NWUI_G34_NATIVE_PHASE").as_deref(),Ok("first"),"persist/next-boot retired on I2; see W1 79cfa261");
        let Fixture{_temp,roots,locks,j,selected,..}=fixture_build(SourceLocation::Root,false,false,true);g34_native_row(roots,locks,j,selected).await;drop(_temp);
    }
    fn assert_protected(f:&Fixture){
        assert_eq!(fs::read(f.roots.db.join("17/data/preserve")).unwrap(),b"live PG17");assert!(f.l.join("UPGRADED-TO-PG17.txt").exists());
        assert_eq!(fs::read(f.roots.db.join(".nwui-pg15-backup").join(&f.j.load().attempt_id).join("base/one")).unwrap(),b"obsolete PG15 one");
    }
    #[test]
    fn link_window_closes_when_manifest_entries_are_done_before_final_p6_receipt(){
        let mut f=fixture(SourceLocation::Root);assert!(super::super::ready::link_window(&f.roots,&f.j).unwrap().is_some());
        delete_closed_set(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();
        assert!(!f.j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.state==StepState::Done&&r.data.is_none()));
        assert!(super::super::ready::link_window(&f.roots,&f.j).unwrap().is_none());
    }
    fn staged_key(f:&Fixture)->PathBuf {f.roots.app.join(".nwui-auto/gen").join(f.selected.selector.generation.as_deref().unwrap()).join("data_key")}
    #[test]
    fn staged_key_is_removed_after_activation_while_generation_records_and_canonical_key_stay(){
        let mut f=fixture(SourceLocation::Root);let path=staged_key(&f);write(&path,&[7;32]);
        fs::create_dir(f.roots.app.join("secrets")).unwrap();write(&f.roots.app.join("secrets/data_key"),b"canonical preserved");
        let records=["manifest.json","db-pair.json","app-pair.json","key-binding.json"].map(|n|(n,fs::read(path.parent().unwrap().join(n)).unwrap()));
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!path.exists());
        for(n,before)in records{assert_eq!(fs::read(path.parent().unwrap().join(n)).unwrap(),before);}
        let reread=selector::load_generation(&f.roots.db,&f.roots.app,&f.roots.architecture,f.selected.selector.clone(),"987654",&[7;32]).unwrap();assert_eq!(reread.selector,f.selected.selector,"ready reader must not require the removed staged copy");
        assert_eq!(fs::read(f.roots.app.join("secrets/data_key")).unwrap(),b"canonical preserved");
        let states=f.j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&r.data==Some(serde_json::json!({"stagedKey":f.selected.selector.generation}))).map(|r|r.state.clone()).collect::<Vec<_>>();assert_eq!(states,vec![StepState::Intent,StepState::Done]);
        let count=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),count);
    }
    #[test]
    fn staged_key_invalid_leaf_is_preserved_and_does_not_release_reserves(){
        for bad in ["digest","mode","owner","nlink","symlink","directory"]{let mut f=fixture(SourceLocation::Root);let path=staged_key(&f);write(&path,&[7;32]);
            match bad{"digest"=>write(&path,&[8;32]),"mode"=>fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap(),"owner"=>std::os::unix::fs::chown(&path,Some(10001),None).unwrap(),"nlink"=>fs::hard_link(&path,path.with_extension("other")).unwrap(),"symlink"=>{fs::rename(&path,path.with_extension("outside")).unwrap();symlink(path.with_extension("outside"),&path).unwrap();},_=>{fs::remove_file(&path).unwrap();fs::create_dir(&path).unwrap();}}
            let before=super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap();assert!(cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).is_err(),"{bad}");assert!(fs::symlink_metadata(&path).is_ok());assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),before);
            assert!(!f.j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.state==StepState::Done&&r.data==Some(serde_json::json!({"stagedKey":f.selected.selector.generation}))));
        }
    }
    #[test]
    fn keyed_source_has_no_staged_key_cleanup_record(){let mut f=fixture(SourceLocation::Root);cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!f.j.load().records.iter().any(|r|r.data.as_ref().is_some_and(|d|d.get("stagedKey").is_some())));}
    fn assert_staged_key_scanner_refusal(kind:&str){
        let mut f=fixture(SourceLocation::Root);let path=staged_key(&f);write(&path,&[7;32]);
        let data=match kind{"generation"=>serde_json::json!({"stagedKey":uuid::Uuid::new_v4().to_string()}),"extra"=>serde_json::json!({"stagedKey":f.selected.selector.generation,"entry":"base"}),_=>serde_json::json!({"stagedKey":"not-a-uuid"})};
        let before=serde_json::to_vec(f.j.load()).unwrap();
        match f.j.record(Step::P6Cleanup,StepState::Intent,Some(data)){
            Err(error)=>{assert_eq!(error.code,"pair_mismatch");assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);},
            Ok(())=>{let names=closed_names(&backup::open_verified_manifest(&f.roots,&f.j).unwrap().top_names,SourceLocation::Root).unwrap();let recorded=serde_json::to_vec(f.j.load()).unwrap();assert_eq!(entry_states(&f.j,&names).unwrap_err().code,"pair_mismatch","{kind}");assert!(cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).is_err());assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),recorded);},
        }
        assert_eq!(fs::read(path).unwrap(),[7;32]);assert_eq!(fs::read(f.l.join("base/one")).unwrap(),b"obsolete PG15 one");
    }
    #[test]fn staged_key_scanner_refuses_wrong_generation(){assert_staged_key_scanner_refusal("generation");}
    #[test]fn staged_key_scanner_refuses_extra_keys(){assert_staged_key_scanner_refusal("extra");}
    #[test]fn staged_key_scanner_refuses_non_uuid(){assert_staged_key_scanner_refusal("uuid");}
    #[test]
    fn staged_key_stays_before_activation_without_a_cleanup_record(){
        let mut f=fixture_with_proof(SourceLocation::Root,false);let path=staged_key(&f);write(&path,&[7;32]);let before=serde_json::to_vec(f.j.load()).unwrap();
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(fs::read(path).unwrap(),[7;32]);assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);
    }
    #[test]
    fn staged_key_kill_child(){
        let Ok(root)=std::env::var("NWUI_STAGED_KEY_CHILD")else{return};let roots=Roots{db:Path::new(&root).join("db"),app:Path::new(&root).join("app"),architecture:"arm64".into()};let locks=a_only(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();let selector=selector::read_selector(&roots.app).unwrap();let selected=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selector,"987654",&[7;32]).unwrap();cleanup(&roots,&locks,&mut j,&selected).unwrap();panic!("SIGKILL hook did not fire");
    }
    #[test]
    fn staged_key_real_sigkill_after_intent_and_unlink_resumes(){
        use std::os::unix::process::ExitStatusExt;
        for point in ["staged-key-intent","staged-key-unlinked"]{let mut f=fixture(SourceLocation::Root);let path=staged_key(&f);write(&path,&[7;32]);drop(f.locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::cleanup::tests::staged_key_kill_child","--nocapture"]).env("NWUI_STAGED_KEY_CHILD",f._temp.path()).env("NWUI_CLEANUP_KILL_POINT",point).env("NWUI_CLEANUP_KILL_ENTRY","data_key").status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));
            f.locks=a_only(&f.roots);f.j=Journal::open(&f.roots.app,&f.locks).unwrap();assert_eq!(staged_key_state(&f.j,f.selected.selector.generation.as_deref().unwrap()).unwrap(),Some(StepState::Intent));assert_eq!(path.exists(),point=="staged-key-intent");cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!path.exists());assert_eq!(staged_key_state(&f.j,f.selected.selector.generation.as_deref().unwrap()).unwrap(),Some(StepState::Done));
        }
    }
    #[test]
    fn staged_key_intent_resumes_with_present_or_already_removed_file(){
        for exists in [false,true]{let mut f=fixture(SourceLocation::Root);let path=staged_key(&f);if exists{write(&path,&[7;32]);}
            f.j.record(Step::P6Cleanup,StepState::Intent,Some(serde_json::json!({"stagedKey":f.selected.selector.generation}))).unwrap();
            cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!path.exists());
            assert!(f.j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.state==StepState::Done&&r.data==Some(serde_json::json!({"stagedKey":f.selected.selector.generation}))));
        }
    }
    #[test]
    fn only_verified_manifest_names_are_deleted_and_second_call_is_idempotent(){
        for location in [SourceLocation::Root,SourceLocation::Pgdata]{let mut f=fixture(location);assert!(!f.locks.holds_d());
            for name in ["foreign","lost+found",".nwui-extra",".nwui","NWUI-RESTORE-IN-PROGRESS.txt"]{write(&f.l.join(name),b"preserve");}
            cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!f.l.join("base").exists());assert!(!f.l.join("global").exists());assert!(!f.l.join("PG_VERSION.nwui-pre17").exists());assert_protected(&f);
            for name in ["foreign","lost+found",".nwui-extra",".nwui","NWUI-RESTORE-IN-PROGRESS.txt"]{assert_eq!(fs::read(f.l.join(name)).unwrap(),b"preserve");}
            let records=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),records);
        }
    }
    #[test]
    fn stale_activation_proofs_skip_without_any_journal_or_file_change(){
        for bad in ["generation","identity","no-proof","outcome","handoff","selector15"]{let mut f=fixture_with_proof(SourceLocation::Root,bad!="no-proof");
            match bad {"generation"=>f.j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"generation":uuid::Uuid::new_v4().to_string(),"bootIdentity":super::super::boot_identity::current().unwrap(),"observedAt":chrono::Utc::now().to_rfc3339()}))).unwrap(),"identity"=>f.j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"generation":f.selected.selector.generation,"bootIdentity":"a".repeat(64),"observedAt":chrono::Utc::now().to_rfc3339()}))).unwrap(),"no-proof"=>{},"outcome"=>f.j.set_outcome(Outcome::Upgrading).unwrap(),"handoff"=>f.j.handoff(Handoff::Intent,&"b".repeat(64)).unwrap(),_=>{f.selected.selector.active_major=15;f.selected.selector.generation=None;}}
            let before=serde_json::to_vec(f.j.load()).unwrap();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);assert_eq!(fs::read(f.l.join("base/one")).unwrap(),b"obsolete PG15 one");assert_protected(&f);
        }
    }
    #[test]
    fn approved_symlink_is_unlinked_without_following_external_target(){
        let mut f=fixture(SourceLocation::Root);let outside=f._temp.path().join("outside");fs::create_dir(&outside).unwrap();write(&outside.join("keep"),b"external");fs::remove_dir_all(f.l.join("base")).unwrap();symlink(&outside,f.l.join("base")).unwrap();
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(fs::symlink_metadata(f.l.join("base")).is_err());assert_eq!(fs::read(outside.join("keep")).unwrap(),b"external");assert_protected(&f);
    }
    #[test]
    fn reserved_names_are_exact_and_seventeen_depends_on_source_location(){
        let names=["PG_VERSION","base","UPGRADED-TO-PG17.txt","NWUI-RESTORE-IN-PROGRESS.txt","lost+found",".nwui",".nwui-auto-work","17","lock","journal","postmaster.opts"].into_iter().map(str::to_owned).collect();
        let root=closed_names(&names,SourceLocation::Root).unwrap();let pgdata=closed_names(&names,SourceLocation::Pgdata).unwrap();
        for name in ["lock","journal","postmaster.opts","PG_VERSION.nwui-pre17"]{assert!(root.iter().any(|n|n==name));}
        assert!(!root.iter().any(|n|n=="17"));assert!(pgdata.iter().any(|n|n=="17"));
        for name in ["UPGRADED-TO-PG17.txt","NWUI-RESTORE-IN-PROGRESS.txt","lost+found",".nwui",".nwui-auto-work"]{assert!(!root.iter().any(|n|n==name));}
    }
    #[test]
    fn nofollow_preflight_refuses_real_cross_mount_without_mutation(){
        let parent=File::open("/dev").unwrap();let scope=boundary(&parent).unwrap();let mut seen=0;
        assert_eq!(visit(&parent,c"shm",&scope,0,&mut seen,false,"shm").unwrap_err().code,"pair_mismatch");
        assert!(std::fs::metadata("/dev/shm").unwrap().is_dir());
    }
    #[test]
    fn recursive_fd_removal_and_symlinks_stay_inside_the_approved_entry(){
        let t=tempfile::tempdir().unwrap();let l=t.path().join("l");let outside=t.path().join("outside");fs::create_dir(&l).unwrap();fs::create_dir(&outside).unwrap();write(&outside.join("preserve"),b"outside");fs::create_dir(l.join("base")).unwrap();write(&l.join("base/one"),b"obsolete");symlink(&outside,l.join("base/link")).unwrap();write(&l.join("stray"),b"preserve");
        let parent=File::open(&l).unwrap();let scope=boundary(&parent).unwrap();let mut seen=0;count(&mut seen).unwrap();visit(&parent,c"base",&scope,0,&mut seen,false,"base").unwrap();assert!(l.join("base/one").exists());
        let mut seen=0;count(&mut seen).unwrap();visit(&parent,c"base",&scope,0,&mut seen,true,"base").unwrap();parent.sync_all().unwrap();assert!(!l.join("base").exists());assert_eq!(fs::read(outside.join("preserve")).unwrap(),b"outside");assert_eq!(fs::read(l.join("stray")).unwrap(),b"preserve");
    }
    #[test]
    fn a_mount_identity_mismatch_refuses_even_on_the_same_device(){
        let t=tempfile::tempdir().unwrap();write(&t.path().join("entry"),b"preserve");let parent=File::open(t.path()).unwrap();let mut scope=boundary(&parent).unwrap();scope.mount+=1;let mut seen=0;
        assert_eq!(visit(&parent,c"entry",&scope,0,&mut seen,true,"entry").unwrap_err().code,"pair_mismatch");assert_eq!(fs::read(t.path().join("entry")).unwrap(),b"preserve");
    }
    #[test]
    fn entry_records_are_exact_and_unowned_or_unpaired_records_refuse(){
        for bad in ["unknown","done-first","extra"]{let mut f=fixture(SourceLocation::Root);let names=closed_names(&backup::open_verified_manifest(&f.roots,&f.j).unwrap().top_names,SourceLocation::Root).unwrap();
            let data=match bad{"unknown"=>serde_json::json!({"entry":"foreign"}),"extra"=>serde_json::json!({"entry":"base","other":true}),_=>serde_json::json!({"entry":"base"})};f.j.record(Step::P6Cleanup,if bad=="done-first"{StepState::Done}else{StepState::Intent},Some(data)).unwrap();
            assert_eq!(entry_states(&f.j,&names).unwrap_err().code,"pair_mismatch");assert!(f.l.join("base/one").exists());assert_protected(&f);
        }
        let mut f=fixture(SourceLocation::Root);let names=closed_names(&backup::open_verified_manifest(&f.roots,&f.j).unwrap().top_names,SourceLocation::Root).unwrap();
        for name in &names{for state in [StepState::Intent,StepState::Done]{f.j.record(Step::P6Cleanup,state,Some(serde_json::json!({"entry":name}))).unwrap();}}
        assert!(entry_states(&f.j,&names).unwrap().0.values().all(|s|s==&StepState::Done));f.j.record(Step::P6Cleanup,StepState::Done,None).unwrap();assert!(entry_states(&f.j,&names).unwrap().1);assert!(f.l.join("base/one").exists());
    }
    fn shared_d(roots:&Roots)->File {
        let f=File::open(&roots.db).unwrap();assert_eq!(unsafe{libc::flock(f.as_raw_fd(),libc::LOCK_SH|libc::LOCK_NB)},0);f
    }
    fn a_only(roots:&Roots)->StorageLocks {
        let f=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&roots.app).unwrap();
        assert_eq!(unsafe{libc::flock(f.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)},0);
        StorageLocks::from_custody(&roots.db,&roots.app,f.into(),None).unwrap()
    }
    #[test]
    fn operator_core_ignores_activation_and_never_drains_then_cleanup_drains_only_reserves(){
        let mut f=fixture_with_proof(SourceLocation::Root,false);f.j.fail_attempt(Step::Phase2Monitor,&StepError{code:"activation_failed",message:"synthetic observer failure",detail:None},Outcome::ActivationFailed,chrono::Utc::now()).unwrap();let _live_pg=shared_d(&f.roots);
        let staged=staged_key(&f);write(&staged,&[7;32]);
        let before=super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap();assert_eq!((before.app_files,before.db_files),(8,8));
        let slots=fs::metadata(f.roots.app.join(".nwui-auto/slots")).unwrap().ino();
        delete_closed_set(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!f.l.join("base").exists());assert_protected(&f);assert_eq!(fs::read(&staged).unwrap(),[7;32],"operator entry cleanup must preserve the staged copy");
        assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),before);
        assert!(!f.j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.data.is_none()));
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),before);
        f.j.set_outcome(Outcome::Pg17).unwrap();f.j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"bootIdentity":super::super::boot_identity::current().unwrap(),"generation":f.selected.selector.generation,"observedAt":chrono::Utc::now().to_rfc3339()}))).unwrap();
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),super::super::reserves::ReserveReport::default());
        // Journal commits can refill the pool; P6 keeps its directory and never treats slots as reserves.
        assert_eq!(fs::metadata(f.roots.app.join(".nwui-auto/slots")).unwrap().ino(),slots);assert!((8..=16).contains(&fs::read_dir(f.roots.app.join(".nwui-auto/slots")).unwrap().count()));Journal::open(&f.roots.app,&f.locks).unwrap();assert_eq!(f.j.load().records.last().unwrap().step,Step::P6Cleanup);assert!(f.j.load().records.last().unwrap().data.is_none());
    }
    #[test]
    fn unsafe_marker_refuses_before_any_intent_or_deletion(){
        for link in [false,true]{let mut f=fixture(SourceLocation::Root);let before=serde_json::to_vec(f.j.load()).unwrap();let marker=f.l.join("UPGRADED-TO-PG17.txt");
            if link {let outside=f._temp.path().join("marker");fs::rename(&marker,&outside).unwrap();symlink(&outside,&marker).unwrap();}else{write(&marker,b"wrong marker");}
            assert!(cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).is_err());assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);assert!(f.l.join("base/one").exists());assert!(f.l.join("global/pg_control").exists());
        }
    }
    #[test]
    fn done_entries_are_never_deleted_again_even_when_the_name_reappears(){
        let mut f=fixture(SourceLocation::Root);delete_closed_set(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();
        write(&f.l.join("base"),b"new owner");fs::remove_file(f.l.join("UPGRADED-TO-PG17.txt")).unwrap();let before=serde_json::to_vec(f.j.load()).unwrap();
        delete_closed_set(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);assert_eq!(fs::read(f.l.join("base")).unwrap(),b"new owner");
    }
    #[test]
    fn kill_child(){
        let Ok(root)=std::env::var("NWUI_CLEANUP_TEST_ROOT")else{return};let root=Path::new(&root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let locks=a_only(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
        let selected=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selector::read_selector(&roots.app).unwrap(),"987654",&[7;32]).unwrap();
        cleanup(&roots,&locks,&mut j,&selected).unwrap();panic!("kill point was not reached");
    }
    #[test]
    fn real_sigkill_at_each_durable_boundary_resumes_with_a_only_and_live_shared_d(){
        use std::os::unix::process::ExitStatusExt;
        for point in ["intent","mid-unlink","after-fsync","before-done"] {
            let mut f=fixture(SourceLocation::Root);let _live_pg=shared_d(&f.roots);write(&f.l.join("foreign"),b"preserve");drop(f.locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::cleanup::tests::kill_child","--nocapture","--test-threads=1"])
                .env("NWUI_CLEANUP_TEST_ROOT",f._temp.path()).env("NWUI_CLEANUP_KILL_POINT",point).env("NWUI_CLEANUP_KILL_ENTRY","base").status().unwrap();
            assert_eq!(status.signal(),Some(libc::SIGKILL),"{point}");
            f.locks=a_only(&f.roots);f.j=Journal::open(&f.roots.app,&f.locks).unwrap();
            let records=f.j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&r.data.as_ref().and_then(|v|v.get("entry")).and_then(|v|v.as_str())==Some("base")).collect::<Vec<_>>();
            assert_eq!(records.len(),1);assert_eq!(records[0].state,StepState::Intent);
            match point {"intent"=>assert!(f.l.join("base/one").exists()),"mid-unlink"=>{assert!(!f.l.join("base/one").exists());assert!(f.l.join("base/two").exists());},_=>assert!(!f.l.join("base").exists())}
            cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!f.l.join("base").exists());assert!(!f.l.join("global").exists());assert_protected(&f);assert_eq!(fs::read(f.l.join("foreign")).unwrap(),b"preserve");
            let records=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),records);
        }
    }

    fn work_path(f:&Fixture)->PathBuf {f.roots.db.join(".nwui-auto-work").join(&f.j.load().attempt_id)}
    #[test]
    fn empty_recovery_work_is_removed_and_journaled_before_reserve_drain(){
        let mut f=fixture(SourceLocation::Root);let w=work_path(&f);fs::create_dir(&w).unwrap();fs::create_dir(w.join("linked")).unwrap();let _live_pg=shared_d(&f.roots);
        let other=w.parent().unwrap().join(uuid::Uuid::new_v4().to_string());fs::create_dir(&other).unwrap();write(&other.join("keep"),b"other attempt");
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!w.exists());assert_protected(&f);assert_eq!(fs::read(other.join("keep")).unwrap(),b"other attempt");
        let states=f.j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&r.data==Some(serde_json::json!({"workDir":f.j.load().attempt_id}))).map(|r|r.state.clone()).collect::<Vec<_>>();assert_eq!(states,vec![StepState::Intent,StepState::Done]);
        let records=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),records);
    }
    #[test]
    fn foreign_work_is_preserved_and_reserves_are_not_drained(){
        for bad in ["file","linked-content","symlink","work-link"]{let mut f=fixture(SourceLocation::Root);let w=work_path(&f);let outside=f._temp.path().join("external");fs::create_dir(&outside).unwrap();write(&outside.join("keep"),b"outside");
            if bad=="work-link"{symlink(&outside,&w).unwrap();}else{fs::create_dir(&w).unwrap();match bad{"file"=>write(&w.join("target"),b"foreign"),"linked-content"=>{fs::create_dir(w.join("linked")).unwrap();write(&w.join("linked/keep"),b"foreign");},_=>symlink(&outside,w.join("linked")).unwrap()}}
            let before=super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap();assert!(cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).is_err(),"{bad}");assert!(fs::symlink_metadata(&w).is_ok());assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),before);assert_eq!(fs::read(outside.join("keep")).unwrap(),b"outside");
            assert!(!f.j.load().records.iter().any(|r|r.step==Step::P6Cleanup&&r.data==Some(serde_json::json!({"workDir":f.j.load().attempt_id}))&&r.state==StepState::Done));
        }
    }
    #[test]
    fn work_intent_resumes_absent_or_partial_tree_and_done_preserves_a_recreated_name(){
        for remaining in ["absent","empty","linked"]{let mut f=fixture(SourceLocation::Root);let w=work_path(&f);delete_closed_set(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();
            if remaining!="absent"{fs::create_dir(&w).unwrap();if remaining=="linked"{fs::create_dir(w.join("linked")).unwrap();}}f.j.record(Step::P6Cleanup,StepState::Intent,Some(serde_json::json!({"workDir":f.j.load().attempt_id}))).unwrap();
            cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!w.exists());write(&w,b"recreated");let records=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),records);assert_eq!(fs::read(&w).unwrap(),b"recreated");assert_protected(&f);
        }
    }

    #[test]
    fn literal_source_work_and_attempt_work_have_independent_resume_keys(){
        let mut f=fixture_with_original_work(SourceLocation::Root,true,true);let w=work_path(&f);fs::create_dir(&w).unwrap();fs::create_dir(w.join("linked")).unwrap();
        cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert!(!f.l.join("work").exists());assert!(!w.exists());assert_protected(&f);
        assert_eq!(fs::read(f.roots.db.join(".nwui-pg15-backup").join(&f.j.load().attempt_id).join("work")).unwrap(),b"original L/work");
        for data in [serde_json::json!({"entry":"work"}),serde_json::json!({"workDir":f.j.load().attempt_id})]{
            let states=f.j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&r.data.as_ref()==Some(&data)).map(|r|r.state.clone()).collect::<Vec<_>>();assert_eq!(states,vec![StepState::Intent,StepState::Done]);
        }
        let records=f.j.load().records.len();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();assert_eq!(f.j.load().records.len(),records);
    }
    #[test]
    fn public_work_primitive_has_no_activation_gate_and_refuses_wrong_attempt_or_shape(){
        let mut f=fixture_with_proof(SourceLocation::Root,false);let w=work_path(&f);fs::create_dir(&w).unwrap();fs::create_dir(w.join("linked")).unwrap();let before=super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap();
        remove_work_dir(&f.roots,&f.locks,&mut f.j).unwrap();assert!(!w.exists());assert!(f.l.join("base/one").exists());assert_eq!(super::super::reserves::report(&f.roots,&f.locks,&f.j).unwrap(),before);
        for bad in ["attempt","extra-entry","done-first"]{let mut f=fixture(SourceLocation::Root);let w=work_path(&f);fs::create_dir(&w).unwrap();
            let data=match bad{"attempt"=>serde_json::json!({"workDir":uuid::Uuid::new_v4().to_string()}),"extra-entry"=>serde_json::json!({"workDir":f.j.load().attempt_id,"entry":"work"}),_=>serde_json::json!({"workDir":f.j.load().attempt_id})};
            let records=f.j.load().records.len();assert!(f.j.record(Step::P6Cleanup,if bad=="done-first"{StepState::Done}else{StepState::Intent},Some(data)).is_err(),"{bad}");assert!(w.exists());assert_eq!(f.j.load().records.len(),records);
        }
    }

    #[test]
    fn public_delete_entry_reuses_nofollow_walker_without_a_p6_allowlist(){
        let t=tempfile::tempdir().unwrap();let parent=File::open(t.path()).unwrap();let outside=tempfile::tempdir().unwrap();write(&outside.path().join("keep"),b"outside");
        // Reserved P6 names are ordinary leaves here: the operator, not this walker, owns admission.
        fs::create_dir(t.path().join(".nwui-test")).unwrap();write(&t.path().join(".nwui-test/rows"),b"owned");symlink(outside.path(),t.path().join(".nwui-test/link")).unwrap();write(&t.path().join("stray"),b"preserve");
        for bad in ["",".","..","../outside","a/b","bad\0name"]{assert_eq!(delete_entry(&parent,bad).unwrap_err().code,"pair_mismatch");assert!(t.path().join(".nwui-test/rows").exists());}
        delete_entry(&parent,".nwui-test").unwrap();delete_entry(&parent,".nwui-test").unwrap();assert!(!t.path().join(".nwui-test").exists());assert_eq!(fs::read(outside.path().join("keep")).unwrap(),b"outside");assert_eq!(fs::read(t.path().join("stray")).unwrap(),b"preserve");
        let dev=File::open("/dev").unwrap();assert_eq!(delete_entry(&dev,"shm").unwrap_err().code,"pair_mismatch");assert!(fs::metadata("/dev/shm").unwrap().is_dir());
    }

    fn new_restore_cycle(f:&mut Fixture){
        let generation=f.selected.selector.generation.as_ref().unwrap();f.j.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::json!({"backupId":f.j.load().attempt_id,"generation":generation,"discardId":generation}))).unwrap();
    }
    #[test]
    fn completed_p6_then_restore_intent_removes_recreated_work_in_its_new_cycle(){
        let mut f=fixture(SourceLocation::Root);let w=work_path(&f);fs::create_dir(&w).unwrap();fs::create_dir(w.join("linked")).unwrap();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();
        new_restore_cycle(&mut f);fs::create_dir(&w).unwrap();fs::create_dir(w.join("linked")).unwrap();remove_work_dir(&f.roots,&f.locks,&mut f.j).unwrap();assert!(!w.exists());assert_eq!(f.j.work_dir_state(&f.j.load().attempt_id),super::super::journal::WorkDirState::Done);assert_protected(&f);
    }
    #[test]
    fn no_work_and_no_current_cycle_intent_is_a_journal_noop(){
        let mut f=fixture(SourceLocation::Root);let before=serde_json::to_vec(f.j.load()).unwrap();remove_work_dir(&f.roots,&f.locks,&mut f.j).unwrap();assert_eq!(serde_json::to_vec(f.j.load()).unwrap(),before);
    }
    #[test]
    fn work_cycle_kill_child(){
        let Ok(root)=std::env::var("NWUI_CLEANUP_CYCLE_ROOT")else{return};let root=Path::new(&root);let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let locks=a_only(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
        remove_work_dir(&roots,&locks,&mut j).unwrap();panic!("work cycle kill point not reached");
    }
    #[test]
    fn real_kill_after_new_cycle_work_removal_resumes_the_current_intent(){
        use std::os::unix::process::ExitStatusExt;
        let mut f=fixture(SourceLocation::Root);let w=work_path(&f);fs::create_dir(&w).unwrap();cleanup(&f.roots,&f.locks,&mut f.j,&f.selected).unwrap();new_restore_cycle(&mut f);fs::create_dir(&w).unwrap();drop(f.locks);
        let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::cleanup::tests::work_cycle_kill_child","--nocapture","--test-threads=1"]).env("NWUI_CLEANUP_CYCLE_ROOT",f._temp.path()).env("NWUI_CLEANUP_KILL_POINT","work-removed").env("NWUI_CLEANUP_KILL_ENTRY","work").status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));assert!(!w.exists());
        f.locks=a_only(&f.roots);f.j=Journal::open(&f.roots.app,&f.locks).unwrap();assert_eq!(f.j.work_dir_state(&f.j.load().attempt_id),super::super::journal::WorkDirState::Intent);remove_work_dir(&f.roots,&f.locks,&mut f.j).unwrap();assert_eq!(f.j.work_dir_state(&f.j.load().attempt_id),super::super::journal::WorkDirState::Done);assert_protected(&f);
    }

}
