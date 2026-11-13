//! Caller policy for the frozen A.16 SetAside operation.
/// Move helper separately validates a normal single entry name on every path.
/// l_is_d comes from the validated Legacy15 root layout, never a directory listing.
pub(crate) fn set_aside_allowed(entry:&str,l_is_d:bool)->bool {
    !matches!(entry,"UPGRADED-TO-PG17.txt"|"NWUI-RESTORE-IN-PROGRESS.txt"|"lost+found"|".nwui")
        && !entry.starts_with(".nwui-") && !(l_is_d && entry=="17")
}

use super::{Journal,MovePhase,MoveRecord,StepError,StepState,StorageLocks,moves::{self,MoveDirs}};
use std::collections::BTreeSet;

/// Synchronous runner step; call only inside the runner's awaited blocking operation.
pub(crate) fn set_aside_entry(locks:&StorageLocks,j:&mut Journal,dirs:&MoveDirs<'_>,entry:&str,l_is_d:bool)->Result<(),StepError> {
    move_one(locks,j,dirs,MovePhase::SetAside,entry,&|name|set_aside_allowed(name,l_is_d))
}
/// manifest_entries must come from the verified backup manifest, never the live directory.
pub(crate) fn restore_backup_entry(locks:&StorageLocks,j:&mut Journal,dirs:&MoveDirs<'_>,entry:&str,manifest_entries:&BTreeSet<String>)->Result<(),StepError> {
    move_one(locks,j,dirs,MovePhase::Restore,entry,&|name|manifest_entries.contains(name))
}
fn move_one(locks:&StorageLocks,j:&mut Journal,dirs:&MoveDirs<'_>,phase:MovePhase,entry:&str,allowed:&dyn Fn(&str)->bool)->Result<(),StepError> {
    let (from,to)=match phase {MovePhase::SetAside=>("L","W"),MovePhase::Restore=>("BK","L"),_=>unreachable!("private runner step uses the closed A.16 table")};
    let state=j.load().moves.iter().rev().find(|m|m.record.phase==phase && m.record.entry==entry)
        .map(|m|m.record.state.clone()).unwrap_or(StepState::Intent);
    moves::move_entry(locks,j,dirs,allowed,&MoveRecord {phase,entry:entry.into(),from:from.into(),to:to.into(),state})
}

fn failed()->StepError {StepError {code:"verification_failed",message:"원본 복원 준비를 확인하지 못했습니다. 원본·보관 사본·작업 사본을 모두 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {super::backup_format::io_error(e)}
fn absent(path:&std::path::Path)->Result<bool,StepError> {match std::fs::symlink_metadata(path){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(true),Ok(_)=>Ok(false),Err(e)=>Err(io_error(e))}}
fn ensure_dir(path:&std::path::Path)->Result<std::fs::File,StepError> {
    match std::fs::symlink_metadata(path) {
        Err(e)if e.kind()==std::io::ErrorKind::NotFound=>crate::db_upgrade::postgres::directory_io(path).map_err(io_error)?,
        Ok(_)=>{},Err(e)=>return Err(io_error(e)),
    }
    super::target::directory(path)
}
fn no_live_postmaster(root:&std::path::Path)->Result<(),StepError> {super::source::require_no_postmaster(root)}
const RESTORE_CONTENT:&[u8]=b"NWUI: original PostgreSQL data restoration is in progress. Keep all copies until verification completes.\n";
fn restore_marker(j:&mut Journal,original:&std::fs::File,l:&std::path::Path,backup_id:&str)->Result<(),StepError> {
    restore_marker_for_step(j,super::Step::RRestore,original,l,backup_id)
}
/// Caller holds A+D and supplies the validated L capability. Command intent keeps its frozen full payload.
pub(crate) fn restore_marker_for_step(j:&mut Journal,owner:super::Step,original:&std::fs::File,l:&std::path::Path,backup_id:&str)->Result<(),StepError> {
    use std::{fs::File,io::{Read,Seek,SeekFrom,Write},os::{fd::{AsRawFd,FromRawFd},unix::fs::MetadataExt}};
    const NAME:&std::ffi::CStr=c"NWUI-RESTORE-IN-PROGRESS.txt";
    const CONTENT:&[u8]=RESTORE_CONTENT;
    if owner==super::Step::CmdRestorePg15 {
        let record=j.load().records.iter().rev().find(|r|r.step==owner).ok_or_else(failed)?;
        let data=record.data.as_ref().ok_or_else(failed)?;
        let generation=data.get("generation").and_then(|v|v.as_str()).filter(|g|super::types::canonical_generation(g)).ok_or_else(failed)?;
        if record.state!=StepState::Intent || backup_id!=j.load().attempt_id || data.get("backupId").and_then(|v|v.as_str())!=Some(backup_id) || data.get("discardId").and_then(|v|v.as_str())!=Some(generation){return Err(failed());}
    }else if owner==super::Step::RRestore {
    let owned=j.load().records.iter().rev().filter(|r|r.step==owner).filter_map(|r|r.data.as_ref()).find(|v|v.get("recoveryEvent").and_then(|s|s.as_str())==Some("restore-marker-intent"));
    if let Some(record)=owned {if record.get("backupId").and_then(|s|s.as_str())!=Some(backup_id){return Err(failed());}}
    else {
        if !absent(&l.join(NAME.to_str().unwrap()))? {return Err(failed());}
        j.record(owner,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"restore-marker-intent","backupId":backup_id})))?;
    }
    }else{return Err(failed());}
    let mut fd=unsafe{libc::openat(original.as_raw_fd(),NAME.as_ptr(),libc::O_RDWR|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC,0o600)};
    if fd<0 && std::io::Error::last_os_error().raw_os_error()==Some(libc::EEXIST) {fd=unsafe{libc::openat(original.as_raw_fd(),NAME.as_ptr(),libc::O_RDWR|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};}
    if fd<0{return Err(io_error(std::io::Error::last_os_error()));}let mut file=unsafe{File::from_raw_fd(fd)};let m=file.metadata().map_err(io_error)?;
    if !m.is_file() || m.nlink()!=1 || m.mode()&0o7777!=0o600 || !(m.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|m.uid()==uid)) || m.len()>CONTENT.len()as u64 {return Err(failed());}
    let mut before=Vec::new();(&mut file).take(CONTENT.len()as u64+1).read_to_end(&mut before).map_err(io_error)?;
    if !CONTENT.starts_with(&before) {return Err(failed());}
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;file.write_all(CONTENT).and_then(|_|file.sync_all()).map_err(io_error)?;original.sync_all().map_err(io_error)
}
#[cfg(test)]fn checkpoint(point:&str){if std::env::var("NWUI_RESTORE_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn checkpoint(_point:&str){}
/// Restore only original entries. The coordinator quarantines the target and discards N2 first.
/// It must finish owned-key/work cleanup before recording Outcome::Restored.
pub(crate) fn restore_entries(roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,j:&mut Journal)->Result<BTreeSet<String>,StepError> {
    use std::{fs,os::{fd::AsRawFd,unix::fs::PermissionsExt}};
    if j.load().branch!=super::Branch::Upgrade || !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app)
        || !absent(&roots.db.join("17/data"))? {return Err(failed());}
    super::replacement::precommit(&roots.app,j)?;
    if let Some(prepare)=j.load().records.iter().rev().find(|r|r.step==super::Step::S4Prepare) {
        if !j.load().records.iter().any(|r|r.seq>prepare.seq && r.step==super::Step::P5Publish && r.state==StepState::Done && r.data.as_ref().and_then(|v|v.get("authorityEvent")).and_then(|v|v.as_str())==Some("privilege-discarded")) {return Err(StepError {code:"privilege_prepare_failed",message:"실행 준비의 폐기를 완료한 뒤 원본을 복원해야 합니다.",detail:None});}
    }
    let mut manifest=super::backup::open_verified_manifest(roots,j)?;
    let l_is_d=manifest.header.source_location==super::backup_format::SourceLocation::Root;
    let l=if l_is_d{roots.db.clone()}else{roots.db.join("pgdata")};no_live_postmaster(&l)?;
    let id=&j.load().attempt_id;if !super::types::canonical_generation(id){return Err(failed());}
    let w=roots.db.join(".nwui-auto-work").join(id);if !absent(&w.join("target"))? {no_live_postmaster(&w.join("target"))?;}
    super::backup::verify_restore_union(roots,j,&mut manifest)?;
    super::reserves::release_for_restore(roots,locks,j)?;
    let names=manifest.top_names.clone();let header=manifest.header.clone();
    j.record(super::Step::RRestore,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"entries-intent","backupId":header.backup_id})))?;
    let original=super::target::directory(&l)?;restore_marker(j,&original,&l,&header.backup_id)?;
    ensure_dir(&roots.db.join(".nwui-auto-work"))?;ensure_dir(&w)?;let linked=ensure_dir(&w.join("linked"))?;
    let backup=super::target::directory(&roots.db.join(".nwui-pg15-backup").join(&header.backup_id))?;
    let dirs=MoveDirs{l:&original,bk:&backup,w_linked:&linked};
    let mut current=fs::read_dir(&l).map_err(io_error)?.map(|r|r.map_err(io_error)?.file_name().into_string().map_err(|_|failed())).collect::<Result<BTreeSet<_>,_>>()?;
    current.extend(j.load().moves.iter().filter(|r|r.record.phase==MovePhase::SetAside).map(|r|r.record.entry.clone()));
    // Disable a 1.19 boot before setting aside any other original entry, regardless of sort order.
    let version_names=["PG_VERSION","PG_VERSION.nwui-pre17"];
    for name in version_names.iter().filter_map(|name|current.get(*name)).chain(current.iter().filter(|name|!version_names.contains(&name.as_str()))) {
        if !set_aside_allowed(&name,l_is_d) || j.load().moves.iter().any(|r|r.record.phase==MovePhase::Restore && r.record.entry==*name) {continue;}
        set_aside_entry(locks,j,&dirs,&name,l_is_d)?;checkpoint("after-setaside");
    }
    checkpoint("before-restore");
    // PG_VERSION is restored last, so the old image cannot boot a half-restored cluster.
    for name in names.iter().filter(|name|name.as_str()!="PG_VERSION").chain(names.get("PG_VERSION")) {restore_backup_entry(locks,j,&dirs,name,&names)?;checkpoint("after-restore");}
    if unsafe{libc::fchown(original.as_raw_fd(),header.root.uid,header.root.gid)}!=0 {return Err(io_error(std::io::Error::last_os_error()));}
    original.set_permissions(fs::Permissions::from_mode(header.root.mode)).map_err(io_error)?;original.sync_all().map_err(io_error)?;
    let restored=super::backup::verify_restored(roots,j)?;checkpoint("verified");
    j.record(super::Step::RRestore,StepState::Done,Some(serde_json::json!({"recoveryEvent":"entries-verified","backupId":header.backup_id})))?;Ok(restored)
}

fn quarantine_needed(roots:&crate::db_upgrade::layout::Roots,j:&Journal)->Result<bool,StepError> {
    if !absent(&roots.db.join("17/data"))? {return Ok(true);}
    let movement=j.load().moves.iter().rev().find(|m|m.record.phase==MovePhase::Target && m.record.entry=="target" && m.record.from=="D17" && m.record.to=="W");
    if movement.is_some_and(|m|m.record.state==StepState::Intent){return Ok(true);}
    let discarding=j.load().records.iter().any(|r|r.step==super::Step::RRestore && r.state==StepState::Intent && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("work-discard-intent") && d.get("attemptId").and_then(|v|v.as_str())==Some(j.load().attempt_id.as_str())));
    if discarding && movement.is_some_and(|m|m.record.state==StepState::Done){return Ok(false);}
    Ok(movement.is_some() || !absent(&roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("target"))?)
}

fn capacity_error(error:&StepError)->bool {matches!(error.code,"insufficient_space"|"insufficient_inodes")}
fn incomplete()->StepError {StepError{code:"restore_incomplete",message:"복원 공간이 부족합니다. 원본·백업·복원 중인 파일을 보존했습니다. 공간을 확보한 뒤 같은 복원을 다시 실행하세요.",detail:None}}
fn with_restore_reserves<T>(roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,j:&mut Journal,mut operation:impl FnMut(&mut Journal)->Result<T,StepError>)->Result<T,StepError> {
    super::reserves::release_for_restore(roots,locks,j)?;
    match operation(j) {
        Err(error) if capacity_error(&error)=>{
            if !super::reserves::release_one_db(roots,locks,j)? {return Err(incomplete());}
            operation(j).map_err(|error|if capacity_error(&error){incomplete()}else{error})
        },
        result=>result,
    }
}

/// Whole pre-commit recovery, awaited under the caller's custody. Private PG operations
/// must already have returned/reaped; an unexplained PID or committed selector is preserved.
pub(crate) async fn recover_with_context(roots:crate::db_upgrade::layout::Roots,locks:std::sync::Arc<StorageLocks>,mut j:Journal,failed_step:super::Step,error_code:&str,identity:Option<String>,current:super::journal::Candidate,label:Option<String>)->Result<Journal,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=super::Branch::Upgrade {return Err(failed());}
    super::replacement::precommit(&roots.app,&j)?;
    if j.load().outcome==super::Outcome::Restored {
        return tokio::task::spawn_blocking(move||{let result=complete(&roots,&locks,&mut j,identity.as_deref());if identity.is_none(){super::refusal::publish_status_only(&roots,&j,super::Outcome::Restored,&current,label.as_deref());}result?;Ok(j)}).await.map_err(|_|failed())?;
    }
    j.record_retry_failure(failed_step,error_code,chrono::Utc::now())?;
    let result=super::runner::discard_privileges(&mut j).await;
    if let Err(error)=result {
        let recorded=j.fail_attempt_with_identity(super::Step::RRestore,&error,super::Outcome::RestoreIncomplete,chrono::Utc::now(),identity.as_deref());
        if identity.is_none(){super::refusal::publish_status_only(&roots,&j,super::Outcome::RestoreIncomplete,&current,label.as_deref());}recorded?;return Err(error);
    }
    tokio::task::spawn_blocking(move|| {
        let result=with_restore_reserves(&roots,&locks,&mut j,|j| {
            // A post-rename crash may precede its Done: the journaled publication ID is authority.
            if quarantine_needed(&roots,&j)? {
                let id=j.load().records.iter().rev().filter(|r|r.step==super::Step::P5Publish).filter_map(|r|r.data.as_ref()).find(|d|d.get("targetEvent").and_then(|v|v.as_str())==Some("publishing")).and_then(|d|d.get("systemIdentifier")).and_then(|v|v.as_str()).filter(|id|crate::db_upgrade::state::identifier(id)).ok_or_else(failed)?.to_owned();
                super::target::quarantine(&roots,&locks,j,&id)?;
            }
            let entries_done=j.load().records.iter().any(|r|r.step==super::Step::RRestore && r.state==StepState::Done && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("entries-verified") && d.get("backupId").and_then(|v|v.as_str())==Some(j.load().attempt_id.as_str())));
            if entries_done{super::backup::verify_restored(&roots,&j)?;}else{restore_entries(&roots,&locks,j)?;}
            super::key_write::discard_upgrade_keys(&roots,&locks,j)?;
            discard_work(&roots,&locks,j)?;
            complete(&roots,&locks,j,identity.as_deref())
        });
        if let Err(error)=result {
        if identity.is_none() && error.detail.as_deref()==Some("boot_identity"){super::refusal::publish_status_only(&roots,&j,super::Outcome::Restored,&current,label.as_deref());return Err(error);}
        let recorded=j.fail_attempt_with_identity(super::Step::RRestore,&error,super::Outcome::RestoreIncomplete,chrono::Utc::now(),identity.as_deref());
        if identity.is_none(){super::refusal::publish_status_only(&roots,&j,super::Outcome::RestoreIncomplete,&current,label.as_deref());}recorded?;return Err(error);
    }
        Ok(j)
    }).await.map_err(|_|failed())?
}

#[cfg(test)]
pub(crate) async fn recover(roots:crate::db_upgrade::layout::Roots,locks:std::sync::Arc<StorageLocks>,j:Journal,step:super::Step,code:&str,identity:Option<String>)->Result<Journal,StepError>{
    let current=j.load().candidate.clone();recover_with_context(roots,locks,j,step,code,identity,current,None).await
}

fn no_owned_child(parent:&std::path::Path,name:&str,authority:bool)->Result<(),StepError> {
    use std::os::{fd::AsRawFd,unix::fs::MetadataExt};if absent(parent)?{return Ok(());}
    let directory=super::target::directory(parent)?;let m=directory.metadata().map_err(io_error)?;if authority && (m.uid()!=0 || m.mode()&0o7777!=0o700){return Err(failed());}
    let name=std::ffi::CString::new(name).map_err(|_|failed())?;let mut m=std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe{libc::fstatat(directory.as_raw_fd(),name.as_ptr(),m.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}==0 || std::io::Error::last_os_error().kind()!=std::io::ErrorKind::NotFound{return Err(failed());}Ok(())
}
fn owned_generations(j:&Journal)->Result<BTreeSet<String>,StepError> {
    let mut generations=BTreeSet::new();for record in &j.load().records {
        let Some(data)=&record.data else{continue};let id=if record.step==super::Step::P4Target{data.pointer("/keyWrite/intent/purpose/generation")}else if matches!(record.step,super::Step::P5Publish|super::Step::P5Commit){data.get("generation")}else{None};
        if let Some(id)=id {let id=id.as_str().filter(|g|super::types::canonical_generation(g)).ok_or_else(failed)?;generations.insert(id.to_owned());}
    }Ok(generations)
}
/// Validate every retained child before deleting any owned working tree. Hard-linked regular
/// files are allowed: removing our names cannot remove the restored original's names.
fn disposable_tree(path:&std::path::Path,device:u64,postgres:Option<u32>,depth:u32,count:&mut u64)->Result<(),StepError> {
    use std::os::unix::fs::MetadataExt;
    *count=count.checked_add(1).ok_or_else(failed)?;if depth>64 || *count>4_004_096{return Err(failed());}
    let m=std::fs::symlink_metadata(path).map_err(io_error)?;
    if m.dev()!=device || !(m.uid()==0 || Some(m.uid())==postgres) || m.mode()&0o022!=0 || !(m.is_file() || m.is_dir()) {return Err(failed());}
    if path.file_name().is_some_and(|n|n=="postmaster.pid"){no_live_postmaster(path.parent().ok_or_else(failed)?)?;}
    if m.is_dir(){super::target::directory(path)?;for child in std::fs::read_dir(path).map_err(io_error)? {disposable_tree(&child.map_err(io_error)?.path(),device,postgres,depth+1,count)?;}}
    Ok(())
}
/// Await on a blocking thread under A+D, after entry restoration and key/N2 discard.
/// Only this attempt's W, empty BK, empty D/17 and intent-named generations are removable.
pub(crate) fn discard_work(roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,j:&mut Journal)->Result<(),StepError> {
    use std::{fs,os::unix::fs::MetadataExt};
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(failed());}j.restore_cleanup_ready()?;
    if serde_json::to_value(Journal::open(&roots.app,locks)?.load()).map_err(|_|failed())?!=serde_json::to_value(j.load()).map_err(|_|failed())?{return Err(failed());}
    super::replacement::precommit(&roots.app,j)?;
    if !absent(&roots.db.join("17/data"))?{return Err(failed());}
    super::backup::verify_restored(roots,j)?;
    let id=j.load().attempt_id.clone();let pg=crate::db_upgrade::postgres::postgres_ids().ok().map(|(uid,_)|uid);
    let mut targets=Vec::new();let mut parents=BTreeSet::new();
    for (parent,name,authority,empty) in [(roots.db.join(".nwui-auto-work"),id.clone(),false,false),(roots.db.join(".nwui-pg15-backup"),id.clone(),false,true),(roots.db.clone(),"17".into(),false,true)] {
        if absent(&parent)?{continue;}let directory=super::target::directory(&parent)?;parents.insert(parent.clone());let path=parent.join(name);
        if !absent(&path)?{let opened=super::target::directory(&path)?;if empty && fs::read_dir(&path).map_err(io_error)?.next().is_some(){return Err(failed());}targets.push((path,directory,opened,authority));}
    }
    let gen_parent=roots.app.join(".nwui-auto/gen");
    for generation in owned_generations(j)? {
        if absent(&gen_parent)?{continue;}let directory=super::target::directory(&gen_parent)?;let m=directory.metadata().map_err(io_error)?;if m.uid()!=0 || m.mode()&0o7777!=0o700{return Err(failed());}parents.insert(gen_parent.clone());let path=gen_parent.join(generation);
        if absent(&path)?{continue;}let opened=super::target::directory(&path)?;
        for entry in fs::read_dir(&path).map_err(io_error)? {let entry=entry.map_err(io_error)?;let name=entry.file_name();let name=name.to_str().ok_or_else(failed)?;
            if !["manifest.json","db-pair.json","app-pair.json","key-binding.json",".manifest.json.init",".db-pair.json.init",".app-pair.json.init",".key-binding.json.init"].contains(&name){return Err(failed());}
            let m=fs::symlink_metadata(entry.path()).map_err(io_error)?;if !m.is_file() || m.uid()!=0 || m.nlink()!=1 || m.mode()&0o7777!=0o600{return Err(failed());}
        }
        targets.push((path,directory,opened,true));
    }
    // Full validation precedes the intent and any unlink, so an unsafe later target preserves all.
    for (path,parent,opened,authority) in &targets {
        let m=opened.metadata().map_err(io_error)?;if m.dev()!=parent.metadata().map_err(io_error)?.dev() || *authority && (m.uid()!=0 || m.mode()&0o7777!=0o700){return Err(failed());}
        disposable_tree(path,m.dev(),pg,0,&mut 0)?;
    }
    let done=j.load().records.iter().any(|r|r.step==super::Step::RRestore && r.state==StepState::Done && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("work-discarded")));
    if done{return if targets.is_empty(){Ok(())}else{Err(failed())};}
    j.record(super::Step::RRestore,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"work-discard-intent","attemptId":id})))?;checkpoint("work-intent");
    for (path,parent,opened,_) in targets {
        let before=opened.metadata().map_err(io_error)?;let current=fs::symlink_metadata(&path).map_err(io_error)?;
        if current.dev()!=before.dev() || current.ino()!=before.ino() || !current.is_dir(){return Err(failed());}
        fs::remove_dir_all(&path).map_err(io_error)?;checkpoint("work-unlink");parent.sync_all().map_err(io_error)?;checkpoint("work-fsync");
    }
    // A prior unlink may have survived a crash before its fsync and is now already absent.
    for parent in parents{super::target::directory(&parent)?.sync_all().map_err(io_error)?;}
    j.record(super::Step::RRestore,StepState::Done,Some(serde_json::json!({"recoveryEvent":"work-discarded","attemptId":id})))?;checkpoint("work-done");Ok(())
}
/// Final pre-commit restore only; operator restore has a separate selector-15 protocol.
pub(crate) fn complete(roots:&crate::db_upgrade::layout::Roots,locks:&StorageLocks,j:&mut Journal,identity:Option<&str>)->Result<(),StepError> {
    use std::{fs::File,io::Read,os::{fd::{AsRawFd,FromRawFd},unix::fs::MetadataExt}};
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(failed());}j.restore_completion_ready()?;
    super::replacement::precommit(&roots.app,j)?;
    for path in [roots.db.join("17")] {if !absent(&path)?{return Err(failed());}}
    super::backup::verify_restored(roots,j)?;
    for name in [".nwui-auto-work",".nwui-pg15-backup"] {no_owned_child(&roots.db.join(name),&j.load().attempt_id,false)?;}
    for generation in owned_generations(j)? {no_owned_child(&roots.app.join(".nwui-auto/gen"),&generation,true)?;}
    let manifest=super::backup::open_verified_manifest(roots,j)?;let l=if manifest.header.source_location==super::backup_format::SourceLocation::Root{roots.db.clone()}else{roots.db.join("pgdata")};let original=super::target::directory(&l)?;
    let owned=j.load().records.iter().any(|r|r.step==super::Step::RRestore && r.state==StepState::Intent && r.data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("restore-finish-intent") && d.get("backupId").and_then(|v|v.as_str())==Some(j.load().attempt_id.as_str())));
    let marker=if !absent(&l.join("NWUI-RESTORE-IN-PROGRESS.txt"))? {
        if j.load().outcome==super::Outcome::Restored {return Err(failed());}
        let fd=unsafe{libc::openat(original.as_raw_fd(),c"NWUI-RESTORE-IN-PROGRESS.txt".as_ptr(),libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};if fd<0{return Err(io_error(std::io::Error::last_os_error()));}let mut file=unsafe{File::from_raw_fd(fd)};let m=file.metadata().map_err(io_error)?;
        if !m.is_file() || m.nlink()!=1 || m.mode()&0o7777!=0o600 || !(m.uid()==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(uid,_)|uid==m.uid())) || m.len()!=RESTORE_CONTENT.len()as u64{return Err(failed());}
        let mut bytes=Vec::new();(&mut file).take(RESTORE_CONTENT.len()as u64+1).read_to_end(&mut bytes).map_err(io_error)?;if bytes!=RESTORE_CONTENT{return Err(failed());}Some(file)
    }else{if !owned{return Err(failed());}None};
    super::guard::revert(roots,locks,j)?;
    if let Some(file)=marker {
        if !owned {j.record(super::Step::RRestore,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"restore-finish-intent","backupId":j.load().attempt_id})))?;}
        let m=file.metadata().map_err(io_error)?;let mut current=std::mem::MaybeUninit::<libc::stat>::uninit();if unsafe{libc::fstatat(original.as_raw_fd(),c"NWUI-RESTORE-IN-PROGRESS.txt".as_ptr(),current.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}!=0{return Err(io_error(std::io::Error::last_os_error()));}let current=unsafe{current.assume_init()};if current.st_dev!=m.dev() || current.st_ino!=m.ino() || current.st_nlink!=1{return Err(failed());}
        if unsafe{libc::unlinkat(original.as_raw_fd(),c"NWUI-RESTORE-IN-PROGRESS.txt".as_ptr(),0)}!=0{return Err(io_error(std::io::Error::last_os_error()));}
    }
    original.sync_all().map_err(io_error)?;checkpoint("finish-marker");j.complete_precommit_restore(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn g49_restore_pidfile_predicate_accepts_reuse_and_refuses_unsafe() {
        use std::{fs,os::unix::fs::PermissionsExt};let t=tempfile::tempdir().unwrap();let path=t.path().join("postmaster.pid");
        for pid in [i32::MAX,std::process::id() as i32] {fs::write(&path,format!("{pid}\n")).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();let before=fs::read(&path).unwrap();no_live_postmaster(t.path()).unwrap();assert_eq!(fs::read(&path).unwrap(),before);}
        fs::write(&path,format!("{}\n",i32::MAX)).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o666)).unwrap();assert!(no_live_postmaster(t.path()).is_err());
    }
    fn reserve_fixture(roots:&crate::db_upgrade::layout::Roots) {
        use std::{fs,os::unix::fs::PermissionsExt};
        let pool=roots.db.join(".nwui-auto-work/reserve");fs::create_dir_all(&pool).unwrap();
        for path in [pool.parent().unwrap(),&pool] {fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        for n in 0..8 {let path=pool.join(n.to_string());fs::write(&path,b"fake allocated reserve").unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();}
    }
    #[test]
    fn r_restore_releases_the_fixed_half_once_and_preserves_the_resume_half() {
        let (_t,roots,locks,mut j,_l,_)=recovery_fixture(super::super::backup_format::SourceLocation::Root);reserve_fixture(&roots);
        restore_entries(&roots,&locks,&mut j).unwrap();assert_eq!(super::super::reserves::report(&roots,&locks,&j).unwrap().db_files,4);
        restore_entries(&roots,&locks,&mut j).unwrap();assert_eq!(super::super::reserves::report(&roots,&locks,&j).unwrap().db_files,4);
    }

    #[test]
    fn r_restore_capacity_failure_releases_one_db_file_retries_once_and_resumes_without_refill() {
        for errno in [libc::ENOSPC,libc::EDQUOT] {
            let (_t,roots,locks,mut j,_l,_)=recovery_fixture(super::super::backup_format::SourceLocation::Root);reserve_fixture(&roots);
            let app=super::super::reserves::report(&roots,&locks,&j).unwrap().app_files;let mut calls=0;
            with_restore_reserves(&roots,&locks,&mut j,|j|{calls+=1;let count=super::super::reserves::report(&roots,&locks,j).unwrap().db_files;
                if calls==1 {assert_eq!(count,4);Err(io_error(std::io::Error::from_raw_os_error(errno)))}else{assert_eq!(count,3);Ok(())}}).unwrap();
            assert_eq!(calls,2);assert_eq!(super::super::reserves::report(&roots,&locks,&j).unwrap().app_files,app);
            with_restore_reserves(&roots,&locks,&mut j,|j|{assert_eq!(super::super::reserves::report(&roots,&locks,j).unwrap().db_files,3);Ok(())}).unwrap();
            restore_entries(&roots,&locks,&mut j).unwrap();super::super::backup::verify_restored(&roots,&j).unwrap();
        }
    }
    #[test]
    fn r_restore_exhausted_or_still_full_stops_after_at_most_one_retry() {
        for (pool,code,expected_calls) in [(true,"insufficient_space",2),(true,"insufficient_inodes",2),(false,"insufficient_space",1)] {
            let (_t,roots,locks,mut j,_l,_)=recovery_fixture(super::super::backup_format::SourceLocation::Root);if pool {reserve_fixture(&roots);}
            let mut calls=0;let error=with_restore_reserves::<()>(&roots,&locks,&mut j,|_|{calls+=1;Err(StepError{code,message:"fake full filesystem",detail:None})}).unwrap_err();
            assert_eq!(error.code,"restore_incomplete");assert_eq!(calls,expected_calls);
        }
        let (_t,roots,locks,mut j,_l,_)=recovery_fixture(super::super::backup_format::SourceLocation::Root);reserve_fixture(&roots);
        let mut calls=0;let error=with_restore_reserves::<()>(&roots,&locks,&mut j,|_|{calls+=1;Err(failed())}).unwrap_err();assert_eq!(error.code,"verification_failed");assert_eq!(calls,1);assert_eq!(super::super::reserves::report(&roots,&locks,&j).unwrap().db_files,4);
    }

    #[test]
    fn w1_restore_marker_shared_owner_core_keeps_recovery_and_command_record_shapes() {
        use std::{fs,os::unix::fs::PermissionsExt};use super::super::{Branch,journal::Candidate};use crate::db_upgrade::files;
        for owner in [super::super::Step::RRestore,super::super::Step::CmdRestorePg15]{
            let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();let id=j.load().attempt_id.clone();let generation=uuid::Uuid::new_v4().to_string();
            let data=serde_json::json!({"backupId":id,"generation":generation,"discardId":generation});let original=super::super::target::directory(&db).unwrap();
            if owner==super::super::Step::CmdRestorePg15{assert!(restore_marker_for_step(&mut j,owner,&original,&db,&id).is_err());j.record(owner,StepState::Intent,Some(data.clone())).unwrap();}
            let count=j.load().records.len();restore_marker_for_step(&mut j,owner,&original,&db,&id).unwrap();let records=serde_json::to_vec(j.load()).unwrap();let before=files::snapshot(&db).unwrap();
            restore_marker_for_step(&mut j,owner,&original,&db,&id).unwrap();assert_eq!(files::snapshot(&db).unwrap(),before);assert_eq!(serde_json::to_vec(j.load()).unwrap(),records);assert_eq!(fs::read(db.join("NWUI-RESTORE-IN-PROGRESS.txt")).unwrap(),RESTORE_CONTENT);
            if owner==super::super::Step::CmdRestorePg15{assert_eq!(j.load().records.len(),count);assert_eq!(j.load().records.last().unwrap().data,Some(data.clone()));j.record(owner,StepState::Done,Some(data)).unwrap();assert!(super::super::journal::commands_settled(&j.load().records));}
            else{assert_eq!(j.load().records.len(),count+1);assert_eq!(j.load().records.last().unwrap().data,Some(serde_json::json!({"recoveryEvent":"restore-marker-intent","backupId":id})));}
            assert!(restore_marker_for_step(&mut j,super::super::Step::P0,&original,&db,&id).is_err());assert_eq!(files::snapshot(&db).unwrap(),before);
        }
    }
    fn recovery_fixture(location:super::super::backup_format::SourceLocation)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal,std::path::PathBuf,BTreeSet<String>) { recovery_fixture_owned(location,false) }
    fn recovery_fixture_owned(location:super::super::backup_format::SourceLocation,postgres_owned:bool)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal,std::path::PathBuf,BTreeSet<String>) { recovery_fixture_at(location,postgres_owned,None) }
    fn recovery_fixture_at(location:super::super::backup_format::SourceLocation,postgres_owned:bool,base:Option<&std::path::Path>)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal,std::path::PathBuf,BTreeSet<String>) {
        use std::{fs,os::unix::fs::PermissionsExt};use super::super::{backup,Branch,journal::Candidate};use crate::db_upgrade::control::{Control,COMPATIBILITY_FIELDS};
        let t=match base {Some(path)=>tempfile::tempdir_in(path),None=>tempfile::tempdir()}.unwrap();let roots=crate::db_upgrade::layout::Roots {db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let l=if location==super::super::backup_format::SourceLocation::Root {roots.db.clone()}else{let p=roots.db.join("pgdata");fs::create_dir(&p).unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o700)).unwrap();p};
        for name in ["global","base"] {fs::create_dir(l.join(name)).unwrap();}fs::write(l.join("global/pg_control"),"original-control").unwrap();fs::write(l.join("base/rows"),"original rows").unwrap();fs::write(l.join("PG_VERSION"),"15\n").unwrap();
        if postgres_owned {fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();crate::db_upgrade::postgres::own_tree(&roots.db).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};let names=BTreeSet::from(["PG_VERSION".into(),"global".into(),"base".into()]);
        let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());let control=Control::parse(&text,"").unwrap();
        let prepared=backup::prepare(&roots,&locks,&j.load().attempt_id,location,&names,&control,"2026-10-28T00:00:00Z").unwrap();backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();j.record(super::super::Step::P3Convert,StepState::Intent,None).unwrap();
        fs::write(l.join("base/rows"),"changed by target").unwrap();fs::write(l.join("new-file"),"generated while converting").unwrap();(t,roots,locks,j,l,names)
    }
    #[test]
    #[ignore="native postgres UID: NWUI_TEST_PROFILE_NATIVE=1"]
    fn a32_native_r_restore_creates_postgres_coordinator_dirs_for_both_layouts() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use std::{fs,os::unix::fs::MetadataExt};
        assert_eq!(unsafe{libc::geteuid()},0);
        let (uid,gid)=crate::db_upgrade::postgres::postgres_ids().unwrap();assert_ne!(uid,0);
        for location in [super::super::backup_format::SourceLocation::Root,super::super::backup_format::SourceLocation::Pgdata] {
            let (_t,roots,locks,mut j,l,names)=recovery_fixture_owned(location,true);
            let work=roots.db.join(".nwui-auto-work");let attempt=work.join(&j.load().attempt_id);let linked=attempt.join("linked");
            assert_eq!(restore_entries(&roots,&locks,&mut j).unwrap(),names);
            for path in [&work,&attempt,&linked] {let m=fs::symlink_metadata(path).unwrap();assert_eq!((m.uid(),m.gid(),m.mode()&0o7777),(uid,gid,0o700),"{}",path.display());}
            let result=std::process::Command::new("/usr/sbin/runuser").args(["-u","postgres","--","/usr/bin/stat","-c","%u:%g:%a"]).arg(linked.join("base/rows")).output().unwrap();
            assert!(result.status.success(),"postgres could not lstat its set-aside entry: {}",String::from_utf8_lossy(&result.stderr));
            assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");
            assert_eq!(fs::read(linked.join("base/rows")).unwrap(),b"changed by target");
            restore_entries(&roots,&locks,&mut j).unwrap();
        }
    }
    #[test]
    #[ignore="native bounded inode-full tmpfs: NWUI_TEST_D_DIR_ENOSPC_ROOT"]
    fn a32_native_linked_enospc_releases_one_reserve_and_retries() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use std::{fs,os::unix::fs::MetadataExt};
        let capacity=std::path::PathBuf::from(std::env::var_os("NWUI_TEST_D_DIR_ENOSPC_ROOT").expect("bounded inode-full native mount"));
        let (_t,roots,locks,mut j,l,_)=recovery_fixture_at(super::super::backup_format::SourceLocation::Root,true,Some(&capacity));
        reserve_fixture(&roots);
        let attempt=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);ensure_dir(&attempt).unwrap();let linked=attempt.join("linked");
        let fillers=_t.path().join("inode-fill");fs::create_dir(&fillers).unwrap();let mut next=0;
        let mut fill=||loop {
            assert!(next<4096,"fixture must be a bounded inode-limited mount");
            let path=fillers.join(next.to_string());
            match fs::OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(_)=>next+=1,
                Err(error)=>{assert_eq!(error.raw_os_error(),Some(libc::ENOSPC));break;}
            }
        };
        fill();let app=super::super::reserves::report(&roots,&locks,&j).unwrap().app_files;let mut calls=0;
        with_restore_reserves(&roots,&locks,&mut j,|j| {
            calls+=1;let count=super::super::reserves::report(&roots,&locks,j).unwrap().db_files;
            if calls==1 {
                assert_eq!(count,4);fill();
                assert_eq!(crate::db_upgrade::postgres::directory(&attempt.join("legacy-wrapper")).unwrap_err().code,"postgres_failed");
            }else{assert_eq!(count,3);}
            let result=ensure_dir(&linked);
            if calls==1 {assert_eq!(result.as_ref().unwrap_err().code,"insufficient_space");}
            result.map(|_|())
        }).unwrap();
        assert_eq!(calls,2);let report=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!(report.db_files,3);assert_eq!(report.app_files,app);
        let m=fs::symlink_metadata(&linked).unwrap();let(uid,gid)=crate::db_upgrade::postgres::postgres_ids().unwrap();assert_eq!((m.uid(),m.gid(),m.mode()&0o7777),(uid,gid,0o700));
        assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"changed by target");
    }
    fn completion_fixture(location:super::super::backup_format::SourceLocation,failed_attempt:bool)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal,std::path::PathBuf) {
        let (t,roots,locks,mut j,l,_)=recovery_fixture(location);
        if failed_attempt{j.record_retry_failure(super::super::Step::P3Convert,"migration_failed",chrono::Utc::now()).unwrap();}
        restore_entries(&roots,&locks,&mut j).unwrap();super::super::key_write::discard_upgrade_keys(&roots,&locks,&mut j).unwrap();
        if failed_attempt{discard_work(&roots,&locks,&mut j).unwrap();}
        else{std::fs::remove_dir_all(roots.db.join(".nwui-auto-work").join(&j.load().attempt_id)).unwrap();std::fs::remove_dir(roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id)).unwrap();j.record(super::super::Step::RRestore,StepState::Done,Some(serde_json::json!({"recoveryEvent":"work-discarded","attemptId":j.load().attempt_id}))).unwrap();}
        (t,roots,locks,j,l)
    }
    #[test]
    fn w1_restore_quarantine_resume_reconciles_destination_only_before_work_disposal() {
        use std::fs;use super::super::backup_format::SourceLocation;
        let (_t,roots,_locks,mut j,_l,_)=recovery_fixture(SourceLocation::Root);assert!(!quarantine_needed(&roots,&j).unwrap());let w=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);fs::create_dir_all(w.join("target")).unwrap();assert!(quarantine_needed(&roots,&j).unwrap());
        let record=MoveRecord{phase:MovePhase::Target,entry:"target".into(),from:"D17".into(),to:"W".into(),state:StepState::Intent};j.record_move(record.clone()).unwrap();assert!(quarantine_needed(&roots,&j).unwrap());
        let mut done=record;done.state=StepState::Done;j.record_move(done).unwrap();assert!(quarantine_needed(&roots,&j).unwrap());
        j.record(super::super::Step::RRestore,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"work-discard-intent","attemptId":j.load().attempt_id}))).unwrap();assert!(!quarantine_needed(&roots,&j).unwrap());
        fs::create_dir_all(roots.db.join("17/data")).unwrap();assert!(quarantine_needed(&roots,&j).unwrap());
    }
    #[tokio::test]
    async fn w1_runner_resumes_restore_then_cleans_consumed_manifests_before_attempt_rollover() {
        use std::{fs,sync::Arc};use super::super::{backup_format::SourceLocation,runner};
        let (_t,roots,locks,j,l,_)=recovery_fixture(SourceLocation::Root);let old=j.load().attempt_id.clone();let mut next=j.load().candidate.clone();next.revision="f".repeat(40);
        let locks=Arc::new(locks);let identity=super::super::boot_identity::current().unwrap();let (j,park)=runner::resume_before_admission(roots.clone(),locks.clone(),j,next.clone(),None,false,chrono::Utc::now(),Some(identity.clone())).await.unwrap();assert!(park.is_none());assert_eq!(j.load().attempt_id,old);assert_eq!(runner::boot_plan(j.load(),Some(&identity)),runner::BootPlan::ParkedRefusal);assert!(roots.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());
        let (j,park)=runner::resume_before_admission(roots.clone(),locks,j,next.clone(),None,false,chrono::Utc::now(),Some("b".repeat(64))).await.unwrap();
        assert!(park.is_none());assert_ne!(j.load().attempt_id,old);assert_eq!(j.load().candidate,next);assert_eq!(j.load().previous[0].attempt_id,old);assert_eq!(j.load().previous[0].outcome,super::super::Outcome::Restored);assert!(j.load().moves.is_empty());assert!(!j.load().records.iter().any(|r|r.step==super::super::Step::P1Backup));assert!(!roots.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");
    }
    #[tokio::test]
    async fn w1_runner_retry_refusal_keeps_the_fully_restored_attempt_and_has_no_new_backup() {
        use std::{fs,sync::Arc};use super::super::{backup_format::SourceLocation,runner,retry::Admission};
        let (_t,roots,locks,mut j,l,_)=recovery_fixture(SourceLocation::Pgdata);let old=j.load().attempt_id.clone();let candidate=j.load().candidate.clone();j.record_retry_failure(super::super::Step::P3Convert,"object_unsupported",chrono::Utc::now()).unwrap();
        let locks=Arc::new(locks);let identity=super::super::boot_identity::current().unwrap();let (j,park)=runner::resume_before_admission(roots.clone(),locks.clone(),j,candidate.clone(),None,false,chrono::Utc::now(),Some(identity.clone())).await.unwrap();assert!(park.is_none());assert_eq!(runner::boot_plan(j.load(),Some(&identity)),runner::BootPlan::ParkedRefusal);
        let (j,park)=runner::resume_before_admission(roots.clone(),locks,j,candidate,None,false,chrono::Utc::now(),Some("b".repeat(64))).await.unwrap();assert_eq!(park,Some(Admission::SameCandidateRefused));assert_eq!(j.load().attempt_id,old);assert!(j.load().previous.is_empty());assert_eq!(j.load().outcome,super::super::Outcome::Restored);assert!(roots.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");
    }
    #[tokio::test]
    async fn w1_restore_coordinator_runs_all_precommit_steps_and_records_failure_before_restored() {
        use std::{fs,sync::Arc};use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            let (_t,roots,locks,j,l,_)=recovery_fixture(location);let locks=Arc::new(locks);let error=StepError{code:"migration_failed",message:"fixture",detail:None};
            let j=recover(roots.clone(),locks.clone(),j,super::super::Step::P3Convert,error.code,super::super::boot_identity::current().ok()).await.unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert!(j.load().records.iter().any(|r|r.step==super::super::Step::P3Convert && matches!(r.state,StepState::Failed{..})));super::super::backup::verify_restored(&roots,&j).unwrap();assert!(fs::File::open(&roots.db).unwrap().try_lock().is_err());assert!(fs::File::open(&roots.app).unwrap().try_lock().is_err());
            let count=j.load().records.len();let repeated=recover(roots.clone(),locks,j,super::super::Step::P3Convert,"migration_failed",super::super::boot_identity::current().ok()).await.unwrap();assert_eq!(repeated.load().records.len(),count);
        }
    }
    #[tokio::test]
    async fn w1_restore_coordinator_preserves_damaged_n2_record_and_reports_restore_incomplete() {
        use std::{fs,sync::Arc};use super::super::backup_format::SourceLocation;
        let (_t,roots,locks,mut j,l,_)=recovery_fixture(SourceLocation::Root);j.record(super::super::Step::S4Prepare,StepState::Done,None).unwrap();let count=j.load().records.len();
        assert_eq!(recover(roots.clone(),Arc::new(locks),j,super::super::Step::P3Convert,"migration_failed",super::super::boot_identity::current().ok()).await.err().unwrap().code,"verification_failed");
        let locks=retry_locks(&roots);let j=Journal::open(&roots.app,&locks).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::RestoreIncomplete);assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"changed by target");assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert!(j.load().records[count..j.load().records.len()-1].iter().all(|r|matches!(r.state,StepState::Failed{..})));assert_eq!(super::super::refusal::outcome(j.load(),&super::super::boot_identity::current().unwrap()),Some(super::super::Outcome::RestoreIncomplete));
    }
    #[tokio::test]
    async fn w1_restore_coordinator_kill_child() {
        let Some(base)=std::env::var_os("NWUI_RESTORE_COORDINATOR_ROOT")else{return};let roots=crate::db_upgrade::layout::Roots{db:std::path::PathBuf::from(&base).join("db"),app:std::path::PathBuf::from(base).join("app"),architecture:"arm64".into()};let locks=std::sync::Arc::new(retry_locks(&roots));let j=Journal::open(&roots.app,&locks).unwrap();recover(roots,locks,j,super::super::Step::P3Convert,"migration_failed",super::super::boot_identity::current().ok()).await.unwrap();
    }
    #[tokio::test]
    async fn w1_restore_coordinator_real_kills_resume_past_consumed_backup_and_work() {
        use std::{fs,sync::Arc,os::unix::process::ExitStatusExt};use super::super::backup_format::SourceLocation;
        for point in ["after-restore","work-unlink","finish-marker"] {let (t,roots,locks,j,l,_)=recovery_fixture(SourceLocation::Root);drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::restore::tests::w1_restore_coordinator_kill_child","--nocapture"]).env("NWUI_RESTORE_COORDINATOR_ROOT",t.path()).env("NWUI_RESTORE_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));
            let locks=Arc::new(retry_locks(&roots));let j=Journal::open(&roots.app,&locks).unwrap();let j=recover(roots.clone(),locks,j,super::super::Step::P3Convert,"migration_failed",super::super::boot_identity::current().ok()).await.unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());super::super::backup::verify_restored(&roots,&j).unwrap();
        }
    }
    fn disposal_fixture(location:super::super::backup_format::SourceLocation)->(tempfile::TempDir,crate::db_upgrade::layout::Roots,StorageLocks,Journal,std::path::PathBuf) {
        let (t,roots,locks,mut j,l,_)=recovery_fixture(location);j.record_retry_failure(super::super::Step::P3Convert,"migration_failed",chrono::Utc::now()).unwrap();restore_entries(&roots,&locks,&mut j).unwrap();super::super::key_write::discard_upgrade_keys(&roots,&locks,&mut j).unwrap();(t,roots,locks,j,l)
    }
    #[test]
    fn w1_restore_work_disposal_preserves_original_and_unrelated_attempts_and_is_idempotent() {
        use std::fs;use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {let (_t,roots,locks,mut j,l)=disposal_fixture(location);let other=roots.db.join(".nwui-auto-work").join(uuid::Uuid::new_v4().to_string());fs::create_dir(&other).unwrap();fs::write(other.join("keep"),b"other attempt").unwrap();
            discard_work(&roots,&locks,&mut j).unwrap();let count=j.load().records.len();discard_work(&roots,&locks,&mut j).unwrap();assert_eq!(count,j.load().records.len());assert_eq!(j.load().outcome,super::super::Outcome::Upgrading);assert!(l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert_eq!(fs::read(other.join("keep")).unwrap(),b"other attempt");super::super::backup::verify_restored(&roots,&j).unwrap();complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);
        }
    }
    #[test]
    fn w1_restore_work_disposal_validates_every_target_before_the_first_unlink() {
        use std::{fs,os::unix::fs::PermissionsExt};use super::super::backup_format::SourceLocation;
        for case in ["symlink","fifo","pid","backup","generation","selector"] {let (_t,roots,locks,mut j,_)=disposal_fixture(SourceLocation::Root);let w=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);
            match case {
                "symlink"=>std::os::unix::fs::symlink("/not-followed",w.join("foreign")).unwrap(),
                "fifo"=>{use std::os::unix::ffi::OsStrExt;let path=std::ffi::CString::new(w.join("fifo").as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(path.as_ptr(),0o600)},0);},
                "pid"=>fs::write(w.join("postmaster.pid"),b"not-a-pid\n").unwrap(),
                "backup"=>fs::write(roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).join("foreign"),b"keep").unwrap(),
                "generation"=>{let g=uuid::Uuid::new_v4().to_string();j.record(super::super::Step::P5Publish,StepState::Intent,Some(serde_json::json!({"authorityEvent":"generation-intent","generation":g}))).unwrap();let parent=roots.app.join(".nwui-auto/gen");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();let root=parent.join(g);fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();fs::write(root.join("foreign"),b"keep").unwrap();},
                _=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),b"committed").unwrap(),
            }
            let count=j.load().records.len();assert!(discard_work(&roots,&locks,&mut j).is_err(),"{case}");assert_eq!(count,j.load().records.len());assert_eq!(fs::read(w.join("linked/base/rows")).unwrap(),b"changed by target");super::super::backup::verify_restored(&roots,&j).unwrap();
        }
    }
    #[test]
    fn w1_restore_dispose_kill_child() {
        let Some(base)=std::env::var_os("NWUI_RESTORE_DISPOSE_ROOT")else{return};let roots=crate::db_upgrade::layout::Roots{db:std::path::PathBuf::from(&base).join("db"),app:std::path::PathBuf::from(base).join("app"),architecture:"arm64".into()};let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();discard_work(&roots,&locks,&mut j).unwrap();
    }
    #[test]
    fn w1_restore_work_disposal_real_kills_resume_without_deleting_original_or_claiming_restored() {
        use std::{fs,os::unix::{fs::PermissionsExt,process::ExitStatusExt}};use super::super::backup_format::SourceLocation;
        for point in ["work-intent","work-unlink","work-fsync","work-done"] {let (t,roots,locks,mut j,l)=disposal_fixture(SourceLocation::Root);
            let g=uuid::Uuid::new_v4().to_string();j.record(super::super::Step::P5Publish,StepState::Intent,Some(serde_json::json!({"authorityEvent":"generation-intent","generation":g}))).unwrap();let parent=roots.app.join(".nwui-auto/gen");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();let generation=parent.join(g);fs::create_dir(&generation).unwrap();fs::set_permissions(&generation,fs::Permissions::from_mode(0o700)).unwrap();let record=generation.join(".manifest.json.init");fs::write(&record,b"partial owned record").unwrap();fs::set_permissions(&record,fs::Permissions::from_mode(0o600)).unwrap();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::restore::tests::w1_restore_dispose_kill_child","--nocapture"]).env("NWUI_RESTORE_DISPOSE_ROOT",t.path()).env("NWUI_RESTORE_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));
            let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();discard_work(&roots,&locks,&mut j).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Upgrading);assert!(!generation.exists());assert!(l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());super::super::backup::verify_restored(&roots,&j).unwrap();complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);
        }
    }
    #[test]
    fn w1_restore_complete_removes_the_last_marker_only_after_all_proofs_and_is_idempotent() {
        use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {let (_t,roots,locks,mut j,l)=completion_fixture(location,true);complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert_eq!(j.load().outcome,super::super::Outcome::Restored);assert_eq!(super::super::refusal::outcome(j.load(),&super::super::boot_identity::current().unwrap()),Some(super::super::Outcome::Restored));let before=j.load().records.len();complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();assert_eq!(j.load().records.len(),before);assert_eq!(std::fs::read(l.join("base/rows")).unwrap(),b"original rows");}
    }
    #[test]
    fn w1_restore_complete_preserves_markers_when_cleanup_bytes_or_failure_authority_are_missing() {
        use std::fs;use super::super::backup_format::SourceLocation;
        for case in ["work","row","marker","failure","selector"] {let (_t,roots,locks,mut j,l)=completion_fixture(SourceLocation::Root,case!="failure");
            match case{"work"=>{fs::create_dir(roots.db.join(".nwui-auto-work").join(&j.load().attempt_id)).unwrap();},"row"=>fs::write(l.join("base/rows"),b"changed").unwrap(),"marker"=>fs::write(l.join("NWUI-RESTORE-IN-PROGRESS.txt"),b"foreign").unwrap(),"selector"=>fs::write(roots.app.join(".nwui-auto/active-v1.json"),b"committed").unwrap(),_=>{}}
            let marker=fs::read(l.join("NWUI-RESTORE-IN-PROGRESS.txt")).unwrap();let count=j.load().records.len();assert!(complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).is_err(),"{case}");assert_eq!(fs::read(l.join("NWUI-RESTORE-IN-PROGRESS.txt")).unwrap(),marker);assert_eq!(j.load().records.len(),count);assert_ne!(j.load().outcome,super::super::Outcome::Restored);
        }
    }
    #[test]
    fn w1_restore_complete_kill_child() {
        let Some(base)=std::env::var_os("NWUI_RESTORE_COMPLETE_ROOT")else{return};let roots=crate::db_upgrade::layout::Roots{db:std::path::PathBuf::from(&base).join("db"),app:std::path::PathBuf::from(base).join("app"),architecture:"arm64".into()};let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();
    }
    #[test]
    fn w1_restore_missing_identity_keeps_finish_intent_then_commits_outcome_and_receipt_together(){
        use super::super::{backup_format::SourceLocation,Outcome,runner,refusal};
        let (_t,r,l,mut j,_source)=completion_fixture(SourceLocation::Root,true);let original=super::super::journal::last_attempt_failure(&j.load().records).unwrap();let error=complete(&r,&l,&mut j,None).unwrap_err();assert_eq!(error.detail.as_deref(),Some("boot_identity"));assert_ne!(j.load().outcome,Outcome::Restored);assert!(refusal::outcome(j.load(),&"a".repeat(64)).is_none());assert_eq!(super::super::journal::last_attempt_failure(&j.load().records).unwrap().code,original.code);assert!(j.load().records.iter().rev().find(|r|r.step==super::super::Step::RRestore).unwrap().data.as_ref().is_some_and(|d|d.get("recoveryEvent").and_then(|v|v.as_str())==Some("restore-finish-intent")));assert_eq!(runner::boot_plan(j.load(),None),runner::BootPlan::Phase1);
        let id="a".repeat(64);complete(&r,&l,&mut j,Some(&id)).unwrap();assert_eq!(j.load().outcome,Outcome::Restored);assert_eq!(refusal::outcome(j.load(),&id),Some(Outcome::Restored));let before=std::fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();complete(&r,&l,&mut j,Some(&id)).unwrap();assert_eq!(std::fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert_eq!(runner::boot_plan(j.load(),Some(&id)),runner::BootPlan::ParkedRefusal);
    }
    #[test]
    fn w1_restore_complete_real_kills_never_separate_done_from_restored_or_reconsume_backup() {
        use std::{fs,os::unix::process::ExitStatusExt};use super::super::backup_format::SourceLocation;
        for point in ["finish-marker","slot-written","slot-fsynced","journal-renamed","journal-durable"] {
            let (t,roots,locks,mut j,l)=completion_fixture(SourceLocation::Root,true);
            j.record(super::super::Step::RRestore,StepState::Intent,Some(serde_json::json!({"recoveryEvent":"restore-finish-intent","backupId":j.load().attempt_id}))).unwrap();drop(j);drop(locks);
            let mut child=std::process::Command::new(std::env::current_exe().unwrap());child.args(["--exact","upgrade::restore::tests::w1_restore_complete_kill_child","--nocapture"]).env("NWUI_RESTORE_COMPLETE_ROOT",t.path());if point=="finish-marker"{child.env("NWUI_RESTORE_KILL_POINT",point);}else{child.env("NWUI_JOURNAL_KILL_POINT",point);}
            assert_eq!(child.status().unwrap().signal(),Some(libc::SIGKILL));let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
            assert_eq!(j.load().outcome==super::super::Outcome::Restored,j.load().records.iter().rev().find(|r|r.step==super::super::Step::RRestore).is_some_and(|r|r.state==StepState::Done && r.data.is_none()) && super::super::refusal::outcome(j.load(),&super::super::boot_identity::current().unwrap())==Some(super::super::Outcome::Restored));
            complete(&roots,&locks,&mut j,Some(&super::super::boot_identity::current().unwrap())).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Restored);assert!(!l.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());assert!(!roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id).exists());super::super::backup::verify_restored(&roots,&j).unwrap();assert_eq!(fs::read(l.join("PG_VERSION")).unwrap(),b"15\n");
        }
    }
    #[test]
    fn w1_restore_first_setaside_disables_old_boot_even_when_an_entry_sorts_before_pg_version() {
        use std::{fs,os::unix::process::ExitStatusExt};use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            let (t,roots,locks,j,l,_)=recovery_fixture(location);fs::write(l.join(".before-version"),b"target addition").unwrap();let id=j.load().attempt_id.clone();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::restore::tests::w1_restore_entries_kill_child","--nocapture"]).env("NWUI_RESTORE_TEST_ROOT",t.path()).env("NWUI_RESTORE_KILL_POINT","after-setaside").status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));
            assert!(!l.join("PG_VERSION").exists());assert_eq!(fs::read(l.join(".before-version")).unwrap(),b"target addition");assert_eq!(fs::read(roots.db.join(".nwui-auto-work").join(&id).join("linked/PG_VERSION")).unwrap(),b"15\n");
            let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();restore_entries(&roots,&locks,&mut j).unwrap();super::super::backup::verify_restored(&roots,&j).unwrap();assert!(!l.join(".before-version").exists());
        }
    }
    #[test]
    fn w1_restore_entries_consumes_only_the_verified_backup_and_replays_after_partial_restore() {
        use std::fs;use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {let (_t,roots,locks,mut j,l,names)=recovery_fixture(location);let id=j.load().attempt_id.clone();
            assert_eq!(restore_entries(&roots,&locks,&mut j).unwrap(),names);assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");assert_eq!(fs::read(l.join("PG_VERSION")).unwrap(),b"15\n");assert!(!l.join("new-file").exists());
            let linked=roots.db.join(".nwui-auto-work").join(&id).join("linked");assert_eq!(fs::read(linked.join("base/rows")).unwrap(),b"changed by target");assert_eq!(fs::read(linked.join("new-file")).unwrap(),b"generated while converting");
            assert_eq!(fs::read_dir(roots.db.join(".nwui-pg15-backup").join(id)).unwrap().count(),0);restore_entries(&roots,&locks,&mut j).unwrap();assert_eq!(j.load().outcome,super::super::Outcome::Upgrading);
        }
    }
    #[test]
    fn w1_restore_entries_never_runs_past_an_unsafe_pidfile_or_unfinished_privilege_discard() {
        use std::fs;use super::super::{backup_format::SourceLocation,StorageSelection,KeyBinding,Outcome};
        for reason in ["pid","prepare"] {let (_t,roots,locks,mut j,l,_names)=recovery_fixture(SourceLocation::Pgdata);
            if reason=="pid" {fs::write(l.join("postmaster.pid"),b"not-a-pid\n").unwrap();}else{
                let selected=StorageSelection{version:1,epoch:1,outcome:Outcome::Upgrading,branch:super::super::Branch::Upgrade,pgdata:roots.db.join("work/data"),backup_id:None,schema_profile:"legacy-28".into(),key_binding:KeyBinding::Verified{pair_id:"00000000-0000-4000-8000-000000000001".into()}};
                for r in crate::nonroot::fixture_prepare_records(&selected,&j.load().attempt_id,j.load().records.last().unwrap().seq+1,false){j.record(r.step,r.state,r.data).unwrap();}
            }
            let before=crate::db_upgrade::files::snapshot(&roots.db).unwrap();let count=j.load().records.len();let error=restore_entries(&roots,&locks,&mut j).unwrap_err();assert_eq!(error.code,if reason=="pid"{"unsupported_layout"}else{"privilege_prepare_failed"});assert_eq!(count,j.load().records.len());assert_eq!(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),before);
        }
    }
    fn retry_locks(roots:&crate::db_upgrade::layout::Roots)->StorageLocks {let end=std::time::Instant::now()+std::time::Duration::from_secs(3);loop{match StorageLocks::acquire(&roots.db,&roots.app){Ok(v)=>return v,Err(e)if e.code=="storage_busy"&&std::time::Instant::now()<end=>std::thread::sleep(std::time::Duration::from_millis(5)),Err(e)=>panic!("{e}")}}}
    #[test]
    fn w1_restore_entries_kill_child() {
        let Some(base)=std::env::var_os("NWUI_RESTORE_TEST_ROOT")else{return;};let base=std::path::PathBuf::from(base);let roots=crate::db_upgrade::layout::Roots {db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();restore_entries(&roots,&locks,&mut j).unwrap();
    }
    #[test]
    fn w1_restore_entries_real_kills_resume_without_setting_aside_already_restored_entries() {
        use std::{fs,os::unix::process::ExitStatusExt};use super::super::backup_format::SourceLocation;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {for point in ["after-setaside","before-restore","after-restore","verified"] {
            let (t,roots,locks,j,l,names)=recovery_fixture(location);drop(j);drop(locks);
            let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::restore::tests::w1_restore_entries_kill_child","--nocapture"]).env("NWUI_RESTORE_TEST_ROOT",t.path()).env("NWUI_RESTORE_KILL_POINT",point).status().unwrap();assert_eq!(result.signal(),Some(9),"{point}");
            if point=="before-restore" {assert!(!l.join("PG_VERSION").exists());assert!(l.join("NWUI-RESTORE-IN-PROGRESS.txt").is_file());}
            let locks=retry_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();assert_eq!(restore_entries(&roots,&locks,&mut j).unwrap(),names);assert_eq!(fs::read(l.join("base/rows")).unwrap(),b"original rows");assert_eq!(fs::read(l.join("PG_VERSION")).unwrap(),b"15\n");
        }}
    }
    #[test]
    fn w1_restore_caller_wires_frozen_policy_and_manifest_membership_on_replay() {
        use std::{fs,os::unix::fs::{OpenOptionsExt,PermissionsExt}};
        use super::super::{Branch,journal::Candidate};
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        for name in ["l","bk","linked"] {fs::create_dir(db.join(name)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();
        let mut j={let fixture_journal=Journal::begin(&app,&locks,Branch::Upgrade,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        let [l,bk,w]=["l","bk","linked"].map(|n|fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW).open(db.join(n)).unwrap());
        let dirs=MoveDirs {l:&l,bk:&bk,w_linked:&w};
        fs::write(db.join("l/lock"),"old").unwrap();
        set_aside_entry(&locks,&mut j,&dirs,"lock",true).unwrap();
        set_aside_entry(&locks,&mut j,&dirs,"lock",true).unwrap();
        assert_eq!(fs::read(db.join("linked/lock")).unwrap(),b"old");
        fs::write(db.join("l/17"),"current").unwrap();let count=j.load().moves.len();
        assert!(set_aside_entry(&locks,&mut j,&dirs,"17",true).is_err());assert_eq!(j.load().moves.len(),count);
        assert!(fs::exists(db.join("l/17")).unwrap());
        fs::write(db.join("bk/base"),"backup").unwrap();fs::write(db.join("bk/foreign"),"keep").unwrap();
        let manifest=BTreeSet::from(["base".to_owned()]);
        assert!(restore_backup_entry(&locks,&mut j,&dirs,"foreign",&manifest).is_err());
        restore_backup_entry(&locks,&mut j,&dirs,"base",&manifest).unwrap();
        restore_backup_entry(&locks,&mut j,&dirs,"base",&manifest).unwrap();
        assert!(restore_backup_entry(&locks,&mut j,&dirs,"base",&BTreeSet::new()).is_err());
        assert_eq!(fs::read(db.join("l/base")).unwrap(),b"backup");assert_eq!(fs::read(db.join("bk/foreign")).unwrap(),b"keep");
    }
    #[test]
    fn w1_setaside_reserved_names_match_the_byte_exact_frozen_list() {
        for entry in ["UPGRADED-TO-PG17.txt","NWUI-RESTORE-IN-PROGRESS.txt","lost+found",".nwui",".nwui-",".nwui-auto",".nwui-nginx-files.lock"] {
            for l_is_d in [true,false] {assert!(!set_aside_allowed(entry,l_is_d),"{entry}");}
        }
        for entry in ["PG_VERSION","PG_VERSION.nwui-pre17","postmaster.opts","base","global","pg_wal","journal-v1.json",".journal-v1.json.init","arbitrary.lock",".NWUI-auto","upgraded-to-pg17.txt","Lost+found",".nwui_extra","017"] {
            for l_is_d in [true,false] {assert!(set_aside_allowed(entry,l_is_d),"{entry}");}
        }
    }
    #[test]
    fn w1_setaside_17_is_reserved_only_when_the_original_cluster_is_the_d_root() {
        assert!(!set_aside_allowed("17",true));assert!(set_aside_allowed("17",false));
    }
}
