//! Source preflight and merge-joined verification of frozen A.16 JSONL backup authority.
use super::{Journal,Step,StepError,StepState,StorageLocks,restore::set_aside_allowed,backup_format::{self as format,Entry,Header,Kind,Reader,Root,SourceLocation,StreamWriter,Summary}};
use crate::db_upgrade::{control::Control,files,layout::Roots};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet,fs::{self,OpenOptions},io::{self,Read,Write},os::unix::fs::{MetadataExt,OpenOptionsExt},path::{Path,PathBuf}};
fn failed()->StepError {StepError {code:"verification_failed",message:"원본·보관 사본·보관 기록의 항목이 일치하지 않습니다. 기존 데이터를 보존했습니다.",detail:None}}
fn interrupted()->Result<(),StepError> {
    if files::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {return Err(StepError {code:"interrupted",message:"데이터 확인을 중단했습니다. 기존 데이터를 보존했습니다.",detail:None});}Ok(())
}
pub(super) fn invalid_name(name:&std::ffi::OsStr)->StepError {
    StepError {code:"unsupported_layout",message:"UTF-8로 표현할 수 없는 파일 이름이 있습니다. 원본을 변경하지 않았습니다. 해당 이름을 확인한 뒤 다시 준비하세요.",detail:Some(format!("{name:?}").chars().take(2048).collect())}
}
fn names(path:&Path)->Result<Vec<String>,StepError> {
    let mut names=fs::read_dir(path).map_err(format::io_error)?.map(|item| {
        let name=item.map_err(format::io_error)?.file_name();name.into_string().map_err(|name|invalid_name(&name))
    }).collect::<Result<Vec<_>,_>>()?;
    names.sort_by(|a,b|a.as_bytes().cmp(b.as_bytes()));Ok(names)
}
fn root_info(path:&Path)->Result<Root,StepError> {
    files::real_path(path).map_err(|_|failed())?;let m=fs::symlink_metadata(path).map_err(format::io_error)?;
    if !m.is_dir(){return Err(failed());}Ok(Root {mode:m.mode()&0o7777,uid:m.uid(),gid:m.gid()})
}
fn entry(root:&Path,relative:String,hash_contents:bool)->Result<Entry,StepError> {
    interrupted()?;if relative.len()>format::MAX_LINE {return Err(format::too_large());}if !format::canonical_path(&relative){return Err(failed());}
    let path=root.join(&relative);files::real_path(&path).map_err(|_|failed())?;
    let before=fs::symlink_metadata(&path).map_err(format::io_error)?;
    if !before.is_file() && !before.is_dir(){return Err(failed());}
    let mut value=Entry {kind:if before.is_file(){Kind::File}else{Kind::Dir},path:relative,bytes:if before.is_file(){before.len()}else{0},mode:before.mode()&0o7777,uid:before.uid(),gid:before.gid(),sha256:None};
    if before.is_file() {
        value.sha256=Some("0".repeat(64));
        if hash_contents {
            let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&path).map_err(format::io_error)?;
            let m=file.metadata().map_err(format::io_error)?;
            if !m.is_file() || m.dev()!=before.dev() || m.ino()!=before.ino(){return Err(failed());}
            let mut file=file.take(before.len().checked_add(1).ok_or_else(failed)?);let mut count=0u64;let mut digest=Sha256::new();let mut buffer=[0u8;65536];
            loop {interrupted()?;let n=file.read(&mut buffer).map_err(format::io_error)?;if n==0{break;}count=count.checked_add(n as u64).ok_or_else(failed)?;digest.update(&buffer[..n]);}
            let after=file.get_ref().metadata().map_err(format::io_error)?;
            if count!=before.len() || before.len()!=after.len() || before.mtime()!=after.mtime() || before.mtime_nsec()!=after.mtime_nsec()
                || before.mode()!=after.mode() || before.uid()!=after.uid() || before.gid()!=after.gid(){return Err(failed());}
            value.sha256=Some(hex::encode(digest.finalize()));
        }
    }
    Ok(value)
}
struct Frame {prefix:String,names:std::vec::IntoIter<String>}
struct TreeWalk {root:PathBuf,stack:Vec<Frame>,hash_contents:bool,failed:bool}
impl TreeWalk {
    fn new(root:&Path,selected:Option<&BTreeSet<String>>,hash_contents:bool)->Result<Self,StepError> {
        root_info(root)?;let all=names(root)?;
        let names=if let Some(selected)=selected {
            if selected.iter().any(|n|n.contains('/') || !all.contains(n)){return Err(failed());}
            all.into_iter().filter(|n|selected.contains(n)).collect()
        }else{all};
        Ok(Self {root:root.into(),stack:vec![Frame {prefix:String::new(),names:names.into_iter()}],hash_contents,failed:false})
    }
    fn next_entry(&mut self)->Result<Option<Entry>,StepError> {
        while let Some(frame)=self.stack.last_mut() {
            let Some(name)=frame.names.next() else {self.stack.pop();continue;};
            let relative=if frame.prefix.is_empty(){name}else{format!("{}/{name}",frame.prefix)};
            let value=entry(&self.root,relative.clone(),self.hash_contents)?;
            if value.kind==Kind::Dir {
                if self.stack.len()>=65 {return Err(failed());}
                self.stack.push(Frame {prefix:relative.clone(),names:names(&self.root.join(relative))?.into_iter()});
            }
            return Ok(Some(value));
        }
        Ok(None)
    }
}
impl Iterator for TreeWalk {
    type Item=Result<Entry,StepError>;
    fn next(&mut self)->Option<Self::Item> {
        if self.failed{return None;}match self.next_entry(){Ok(Some(v))=>Some(Ok(v)),Ok(None)=>None,Err(e)=>{self.failed=true;Some(Err(e))}}
    }
}
fn source(roots:&Roots,location:SourceLocation)->PathBuf {match location {SourceLocation::Root=>roots.db.clone(),SourceLocation::Pgdata=>roots.db.join("pgdata")}}
/// Only scalar preflight results and the small top-level original inventory are retained.
pub(crate) struct PreparedBackup {pub header:Header,pub entries:u64,pub manifest_bytes:u64,pub source_bytes:u64,names:BTreeSet<String>}
/// Run before journal/coordinator allocation; original_names is the validated original inventory.
/// The small header is associated with the attempt ID before output is created.
pub(crate) fn prepare(roots:&Roots,locks:&StorageLocks,attempt_id:&str,location:SourceLocation,original_names:&BTreeSet<String>,control:&Control,created_at:&str)->Result<PreparedBackup,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(StepError {code:"storage_busy",message:"저장소 잠금이 유지되지 않았습니다.",detail:None});}
    super::source::control(control)?;super::source::require_idle(roots,locks,&source(roots,location))?;
    if original_names.iter().any(|name|!set_aside_allowed(name,location==SourceLocation::Root)) || !original_names.contains("PG_VERSION") || !original_names.contains("global"){return Err(failed());}
    let root=source(roots,location);let checkpoint=entry(&root,"global/pg_control".into(),true)?;
    let header=Header {version:1,backup_id:attempt_id.into(),source:"L".into(),created_at:created_at.into(),source_location:location,control_state:control.cluster_state.clone(),system_identifier:control.system_identifier.clone(),checkpoint_sha256:checkpoint.sha256.ok_or_else(failed)?,root:root_info(&root)?};
    let mut entries=0u64;let mut source_bytes=0u64;
    let measured=TreeWalk::new(&root,Some(original_names),false)?.map(|value|{
        let value=value?;entries=entries.checked_add(1).ok_or_else(format::too_large)?;source_bytes=source_bytes.checked_add(value.bytes).ok_or_else(format::too_large)?;Ok(value)
    });
    let manifest_bytes=super::space::manifest_bytes(&header,measured)?;format::authority_space(entries)?;
    Ok(PreparedBackup {header,entries,manifest_bytes,source_bytes,names:original_names.clone()})
}
/// Apparent app bytes/inodes, excluding our coordinator pool; metadata only, no secret reads.
pub(super) fn app_usage(roots:&Roots,locks:&StorageLocks)->Result<(u64,u64),StepError>{
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app){return Err(failed());}
    let selected=names(&roots.app)?.into_iter().filter(|name|name!=".nwui-auto").collect();let mut bytes=0u64;let mut count=1u64;
    for value in TreeWalk::new(&roots.app,Some(&selected),false)?{
        let value=value?;count=count.checked_add(1).ok_or_else(format::too_large)?;if count>format::MAX_ENTRIES{return Err(format::too_large());}bytes=bytes.checked_add(value.bytes).ok_or_else(format::too_large)?;
    }Ok((bytes,count))
}
fn write_source<W:Write>(output:W,prepared:&PreparedBackup,root:&Path)->Result<(W,Summary),StepError> {
    if root_info(root)?!=prepared.header.root{return Err(failed());}
    let mut writer=StreamWriter::new(output,&prepared.header)?;
    for value in TreeWalk::new(root,Some(&prepared.names),true)? {writer.entry(&value?)?;}
    let (output,summary)=writer.finish()?;
    if summary.entries!=prepared.entries || summary.bytes!=prepared.manifest_bytes || summary.data_bytes!=prepared.source_bytes {return Err(failed());}Ok((output,summary))
}
/// Pass 2 is a merge-join with independent, identically ordered tree walks, not path lookup.
fn verify_pair(reader:&mut Reader,prepared:&PreparedBackup,original:&Path,backup:&Path)->Result<(),StepError> {
    if reader.header!=prepared.header || root_info(original)?!=prepared.header.root || root_info(backup)?!=prepared.header.root {return Err(failed());}
    let mut original=TreeWalk::new(original,Some(&prepared.names),true)?;let mut backup=TreeWalk::new(backup,None,true)?;
    let mut has_control=false;let mut has_version=false;
    let names=reader.for_each(|expected| {
        for walk in [&mut original,&mut backup] {let actual=walk.next().ok_or_else(failed)??;if actual!=*expected{return Err(failed());}}
        if expected.path=="global/pg_control" {has_control=expected.sha256.as_deref()==Some(prepared.header.checkpoint_sha256.as_str());}
        if expected.path=="PG_VERSION" {has_version=expected.kind==Kind::File;}
        Ok(())
    })?;
    if original.next().is_some() || backup.next().is_some() || names!=prepared.names || !has_control || !has_version {return Err(failed());}Ok(())
}
/// The fixed authority pathname is derived from the validated journal, never JSON path metadata.
pub(crate) fn publish_verified(roots:&Roots,locks:&StorageLocks,j:&mut Journal,prepared:&PreparedBackup)->Result<Summary,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || prepared.header.backup_id!=j.load().attempt_id {return Err(failed());}
    let record=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup).ok_or_else(failed)?;
    let expected=serde_json::to_value(&prepared.header).map_err(|_|failed())?;
    if !matches!(record.state,StepState::Intent|StepState::Done) || record.data.as_ref().and_then(|v|v.get("header"))!=Some(&expected) {return Err(failed());}
    let done=record.state==StepState::Done;
    let original=source(roots,prepared.header.source_location);let backup=roots.db.join(".nwui-pg15-backup").join(&prepared.header.backup_id);
    let path=roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",prepared.header.backup_id));
    let summary=match fs::symlink_metadata(&path) {
        Ok(_)=>{let mut reader=Reader::open(&path)?;verify_pair(&mut reader,prepared,&original,&backup)?;j.finish_backup_publication()?;reader.summary},
        Err(e)if e.kind()==io::ErrorKind::NotFound=>j.publish_backup_stream(locks,&prepared.header,|file| {
            let (_,summary)=write_source(&mut *file,prepared,&original)?;file.sync_all().map_err(format::io_error)?;
            let mut reader=Reader::from_file(file.try_clone().map_err(format::io_error)?)?;verify_pair(&mut reader,prepared,&original,&backup)?;Ok(summary)
        })?,
        Err(e)=>return Err(format::io_error(e)),
    };
    if !done {j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":prepared.header,"entries":summary.entries,"manifestBytes":summary.bytes,"manifestSha256":summary.sha256,"sourceBytes":summary.data_bytes})))?;}
    Ok(summary)
}

/// Authority is bound to the journal before any R-restore entry is allowed to move.
pub(crate) fn open_verified_manifest(roots:&Roots,j:&Journal)->Result<Reader,StepError> {
    if !j.belongs_to(&roots.app) {return Err(failed());}
    let path=roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",j.load().attempt_id));let reader=Reader::open(&path)?;
    let saved=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done).and_then(|r|r.data.as_ref()).ok_or_else(failed)?;
    if reader.header.backup_id!=j.load().attempt_id || saved.get("header")!=Some(&serde_json::to_value(&reader.header).map_err(|_|failed())?)
        || saved.get("manifestSha256").and_then(|v|v.as_str())!=Some(reader.summary.sha256.as_str())
        || saved.get("sourceBytes").is_some_and(|v|v.as_u64()!=Some(reader.summary.data_bytes))
        || saved.get("entries").and_then(|v|v.as_u64())!=Some(reader.summary.entries) || saved.get("manifestBytes").and_then(|v|v.as_u64())!=Some(reader.summary.bytes) {return Err(failed());}
    Ok(reader)
}
/// P2 may legitimately change the original; revalidate the cold backup alone before P3.
pub(crate) fn verify_backup(roots:&Roots,j:&Journal)->Result<Header,StepError> {
    let mut reader=open_verified_manifest(roots,j)?;let path=roots.db.join(".nwui-pg15-backup").join(&reader.header.backup_id);
    if root_info(&path)?!=reader.header.root{return Err(failed());}let mut walk=TreeWalk::new(&path,None,true)?;
    reader.for_each(|expected|{if walk.next().ok_or_else(failed)??!=*expected{return Err(failed());}Ok(())})?;
    if walk.next().is_some(){return Err(failed());}Ok(reader.header)
}

/// Before a resumed restore, verify the union of remaining BK and journal-restored L entries.
/// Only the small top-level placement sets are retained; both trees are independently merge-joined.
pub(crate) fn verify_restore_union(roots:&Roots,j:&Journal,reader:&mut Reader)->Result<(),StepError> {
    let original=source(roots,reader.header.source_location);let backup=roots.db.join(".nwui-pg15-backup").join(&reader.header.backup_id);
    if root_info(&backup)?!=reader.header.root {return Err(failed());}
    let mut bk=BTreeSet::new();let mut restored=BTreeSet::new();
    if j.load().moves.iter().any(|r|r.record.phase==super::MovePhase::Restore && !reader.top_names.contains(&r.record.entry)){return Err(failed());}
    for name in &reader.top_names {
        let record=j.load().moves.iter().rev().find(|r|r.record.phase==super::MovePhase::Restore && &r.record.entry==name);
        let exists=|root:&Path|->Result<bool,StepError>{match fs::symlink_metadata(root.join(name)){Ok(_)=>Ok(true),Err(e)if e.kind()==io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(format::io_error(e))}};
        let in_bk=exists(&backup)?;
        let destination=if let Some(record)=record {
            if record.record.from!="BK" || record.record.to!="L" || !matches!(record.record.state,StepState::Intent|StepState::Done) {return Err(failed());}
            let in_l=exists(&original)?;if in_bk==in_l || record.record.state==StepState::Done && in_bk {return Err(failed());}!in_bk
        }else{if !in_bk{return Err(failed());}false};
        if destination {restored.insert(name.clone());}else{bk.insert(name.clone());}
    }
    if names(&backup)?.into_iter().collect::<BTreeSet<_>>()!=bk {return Err(failed());}
    let mut old=TreeWalk::new(&backup,Some(&bk),true)?.peekable();let mut new=TreeWalk::new(&original,Some(&restored),true)?.peekable();
    let expected_names=reader.top_names.clone();let top=reader.for_each(|expected| {
        let choose_old=match(old.peek(),new.peek()) {
            (Some(Ok(a)),Some(Ok(b)))=>match format::compare_paths(&a.path,&b.path){std::cmp::Ordering::Less=>true,std::cmp::Ordering::Greater=>false,std::cmp::Ordering::Equal=>return Err(failed())},
            (Some(Err(_)),_)|(_,Some(Err(_)))=>return Err(failed()),(Some(_),None)=>true,(None,Some(_))=>false,(None,None)=>return Err(failed()),
        };
        let actual=if choose_old{old.next()}else{new.next()}.ok_or_else(failed)??;if actual!=*expected{return Err(failed());}Ok(())
    })?;
    if old.next().is_some() || new.next().is_some() || top!=expected_names {return Err(failed());}Ok(())
}
/// Independent full original-entry walk catches missing AND extra entries after R-restore.
/// Guard marker/coordinator paths are deliberately outside the old-cluster entry inventory.
pub(crate) fn verify_restored(roots:&Roots,j:&Journal)->Result<BTreeSet<String>,StepError> {
    let mut reader=open_verified_manifest(roots,j)?;let root=source(roots,reader.header.source_location);
    if root_info(&root)?!=reader.header.root {return Err(failed());}
    let l_is_d=reader.header.source_location==SourceLocation::Root;let mut selected=BTreeSet::new();
    for name in names(&root)? {
        if set_aside_allowed(&name,l_is_d) {selected.insert(name);continue;}
        let allowed=matches!(name.as_str(),"UPGRADED-TO-PG17.txt"|"NWUI-RESTORE-IN-PROGRESS.txt"|"lost+found")
            || l_is_d && matches!(name.as_str(),".nwui-auto-work"|".nwui-pg15-backup"|".nwui-discard"|"17");
        if !allowed {return Err(failed());}
        let m=fs::symlink_metadata(root.join(&name)).map_err(format::io_error)?;
        if !storage_owner(m.uid()) || m.file_type().is_symlink() || (!m.is_dir() && !m.is_file()) || m.mode()&0o022!=0 || m.is_file() && m.nlink()!=1 {return Err(failed());}
        if name=="lost+found" && (!m.is_dir() || m.uid()!=0 || fs::read_dir(root.join(&name)).map_err(format::io_error)?.next().is_some()) {return Err(failed());}
    }
    let mut actual=TreeWalk::new(&root,Some(&selected),true)?;
    let checkpoint_sha=reader.header.checkpoint_sha256.clone();let mut control=false;let mut version=false;
    let top=reader.for_each(|expected| {
        if actual.next().ok_or_else(failed)??!=*expected {return Err(failed());}
        if expected.path=="global/pg_control" {control=expected.sha256.as_deref()==Some(checkpoint_sha.as_str());}
        if expected.path=="PG_VERSION" {version=expected.kind==Kind::File;}
        Ok(())
    })?;
    if actual.next().is_some() || top!=selected || !control || !version {return Err(failed());}Ok(top)
}

fn storage_owner(uid:u32)->bool {uid==0 || crate::db_upgrade::postgres::postgres_ids().is_ok_and(|(postgres,_)|uid==postgres)}
fn backup_parent(roots:&Roots)->Result<PathBuf,StepError> {
    let path=roots.db.join(".nwui-pg15-backup");
    // Accepted legacy root-owned parents are not migrated.
    match fs::symlink_metadata(&path) {
        Err(e)if e.kind()==io::ErrorKind::NotFound=>crate::db_upgrade::postgres::directory_io(&path).map_err(format::io_error)?,
        Ok(_)=>{},Err(e)=>return Err(format::io_error(e)),
    }
    files::real_path(&path).map_err(|_|failed())?;let m=fs::symlink_metadata(&path).map_err(format::io_error)?;
    if !m.is_dir() || m.mode()&0o7777!=0o700 {return Err(failed());}
    if !storage_owner(m.uid()) {return Err(failed());}
    Ok(path)
}
fn remove_partial(parent:&Path,id:&str)->Result<(),StepError> {
    use std::os::fd::AsRawFd;
    let parent=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_DIRECTORY).open(parent).map_err(format::io_error)?;
    // The journal owns this exact attempt; Linux removal never follows children or a replaced leaf.
    let path=PathBuf::from(format!("/proc/self/fd/{}",parent.as_raw_fd())).join(id);
    match fs::symlink_metadata(&path) {
        Ok(m)if m.is_dir()&&!m.file_type().is_symlink()=>fs::remove_dir_all(&path).map_err(format::io_error)?,
        Err(e)if e.kind()==io::ErrorKind::NotFound=>{},_=>return Err(failed()),
    }
    parent.sync_all().map_err(format::io_error)
}
#[cfg(test)]fn copy_checkpoint(point:&str) {if std::env::var("NWUI_BACKUP_COPY_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn copy_checkpoint(_point:&str) {}
/// P1 copies and independently re-reads the inactive source, before recovery/link mutations.
/// An interrupted pre-manifest copy is discarded only under its existing exact journal intent.
pub(crate) fn create_verified(roots:&Roots,locks:&StorageLocks,j:&mut Journal,prepared:&PreparedBackup)->Result<Summary,StepError> {
    use std::os::unix::fs::{DirBuilderExt,PermissionsExt};
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || prepared.header.backup_id!=j.load().attempt_id
        || j.load().records.iter().any(|r|r.step==Step::P3Convert) {return Err(failed());}
    let parent=roots.db.join(".nwui-pg15-backup");let backup=parent.join(&prepared.header.backup_id);
    let authority=roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",prepared.header.backup_id));
    let expected=serde_json::to_value(&prepared.header).map_err(|_|failed())?;
    let previous=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup);
    if let Some(record)=previous {
        if record.data.as_ref().and_then(|v|v.get("header"))!=Some(&expected) || !matches!(record.state,StepState::Intent|StepState::Done) {return Err(failed());}
        if record.state==StepState::Done || fs::symlink_metadata(&authority).is_ok() {return publish_verified(roots,locks,j,prepared);}
    }else{
        match fs::symlink_metadata(&backup) {Err(e)if e.kind()==io::ErrorKind::NotFound=>{},_=>return Err(failed())}
        j.record(Step::P1Backup,StepState::Intent,Some(serde_json::json!({"header":prepared.header})))?;copy_checkpoint("intent");
    }
    let original=source(roots,prepared.header.source_location);
    // Hash/canonical-name checks precede the first backup allocation, including resumed attempts.
    super::source::require_idle(roots,locks,&original)?;
    write_source(io::sink(),prepared,&original)?;
    let parent=backup_parent(roots)?;
    j.record(Step::P1Backup,StepState::Intent,Some(serde_json::json!({"header":prepared.header,"backupCopy":"reset-intent"})))?;
    remove_partial(&parent,&prepared.header.backup_id)?;copy_checkpoint("reset");
    let copy=(|| {
        fs::DirBuilder::new().mode(0o700).create(&backup).map_err(format::io_error)?;
        for name in &prepared.names {files::copy_tree(&original.join(name),&backup.join(name)).map_err(format::io_error)?;copy_checkpoint("entry");}
        std::os::unix::fs::chown(&backup,Some(prepared.header.root.uid),Some(prepared.header.root.gid)).map_err(format::io_error)?;
        fs::set_permissions(&backup,fs::Permissions::from_mode(prepared.header.root.mode)).map_err(format::io_error)?;
        std::fs::File::open(&backup).and_then(|f|f.sync_all()).map_err(format::io_error)?;
        std::fs::File::open(&parent).and_then(|f|f.sync_all()).map_err(format::io_error)?;copy_checkpoint("copy-synced");
        publish_verified(roots,locks,j,prepared)
    })();
    if copy.is_err() && matches!(fs::symlink_metadata(&authority),Err(e)if e.kind()==io::ErrorKind::NotFound) {remove_partial(&parent,&prepared.header.backup_id)?;}
    copy
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,journal::Candidate};
    use crate::db_upgrade::control::COMPATIBILITY_FIELDS;
    use std::os::unix::{fs::{PermissionsExt,symlink},ffi::OsStringExt};
    fn fixture(location:SourceLocation)->(tempfile::TempDir,Roots,StorageLocks,Journal,PreparedBackup) { fixture_owned(location,false) }
    fn fixture_owned(location:SourceLocation,postgres_owned:bool)->(tempfile::TempDir,Roots,StorageLocks,Journal,PreparedBackup) {
        let t=tempfile::tempdir().unwrap();let roots=Roots {db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let root=source(&roots,location);if root!=roots.db {fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();}
        for name in ["global","a","a-c"] {fs::create_dir(root.join(name)).unwrap();}
        fs::write(root.join("PG_VERSION"),"15\n").unwrap();fs::write(root.join("global/pg_control"),"control-fixture").unwrap();
        fs::write(root.join("a/b"),"rows").unwrap();fs::write(root.join("a-c/123"),"other").unwrap();fs::set_permissions(root.join("a/b"),fs::Permissions::from_mode(0o640)).unwrap();
        if postgres_owned {fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();crate::db_upgrade::postgres::own_tree(&roots.db).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let attempt=uuid::Uuid::new_v4().to_string();
        let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());
        let control=Control::parse(&text,"").unwrap();let names=BTreeSet::from(["PG_VERSION".into(),"global".into(),"a".into(),"a-c".into()]);
        let prepared=prepare(&roots,&locks,&attempt,location,&names,&control,"2026-10-28T00:00:00Z").unwrap();
        let j={let fixture_journal=Journal::begin_with_attempt(&roots.app,&locks,Branch::Upgrade,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)},attempt).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        (t,roots,locks,j,prepared)
    }
    #[test]
    #[ignore="native postgres UID: NWUI_TEST_PROFILE_NATIVE=1"]
    fn a32_native_p1_new_parents_are_postgres_and_existing_root_parents_are_accepted() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        assert_eq!(unsafe{libc::geteuid()},0);
        let (uid,gid)=crate::db_upgrade::postgres::postgres_ids().unwrap();assert_ne!(uid,0);
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            for legacy_parent in [false,true] {
                let (_t,roots,locks,mut j,prepared)=fixture_owned(location,true);
                let parent=roots.db.join(".nwui-pg15-backup");
                if legacy_parent {fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();}
                create_verified(&roots,&locks,&mut j,&prepared).unwrap();
                let m=fs::symlink_metadata(&parent).unwrap();
                assert_eq!((m.uid(),m.gid(),m.mode()&0o7777),if legacy_parent{(0,0,0o700)}else{(uid,gid,0o700)});
                if !legacy_parent {
                    let entry=parent.join(&prepared.header.backup_id).join("a/b");
                    let result=std::process::Command::new("/usr/sbin/runuser").args(["-u","postgres","--","/usr/bin/stat","-c","%u:%g:%a"]).arg(entry).output().unwrap();
                    assert!(result.status.success(),"postgres could not lstat its backup entry: {}",String::from_utf8_lossy(&result.stderr));
                }
                publish_verified(&roots,&locks,&mut j,&prepared).unwrap();
            }
        }
    }
    fn make_backup(roots:&Roots,prepared:&PreparedBackup)->PathBuf {
        let parent=roots.db.join(".nwui-pg15-backup");fs::create_dir(&parent).unwrap();let path=parent.join(&prepared.header.backup_id);fs::create_dir(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(prepared.header.root.mode)).unwrap();
        for name in &prepared.names {files::copy_tree(&source(roots,prepared.header.source_location).join(name),&path.join(name)).unwrap();}path
    }
    fn intent(j:&mut Journal,prepared:&PreparedBackup) {j.record(Step::P1Backup,StepState::Intent,Some(serde_json::json!({"header":prepared.header}))).unwrap();}
    fn retry_locks(roots:&Roots)->StorageLocks {
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {match StorageLocks::acquire(&roots.db,&roots.app) {Ok(v)=>return v,Err(e)if e.code=="storage_busy" && std::time::Instant::now()<deadline=>std::thread::sleep(std::time::Duration::from_millis(5)),Err(e)=>panic!("{e}")}}
    }
    fn replay_copy(roots:&Roots,location:SourceLocation)->Summary {
        let locks=retry_locks(roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
        let text=format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{}",COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>());
        let control=Control::parse(&text,"").unwrap();let names=BTreeSet::from(["PG_VERSION".into(),"global".into(),"a".into(),"a-c".into()]);
        let prepared=prepare(roots,&locks,&j.load().attempt_id,location,&names,&control,"2026-10-28T00:00:00Z").unwrap();create_verified(roots,&locks,&mut j,&prepared).unwrap()
    }
    #[test]
    fn w1_backup_preflight_source_bytes_and_published_manifest_agree_without_an_extra_tree_walk() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {let (_t,roots,locks,mut j,prepared)=fixture(location);assert_eq!(prepared.source_bytes,27);let summary=create_verified(&roots,&locks,&mut j,&prepared).unwrap();assert_eq!(summary.data_bytes,prepared.source_bytes);assert_eq!(open_verified_manifest(&roots,&j).unwrap().summary.data_bytes,27);let r=j.load().records.last().unwrap();assert_eq!(r.data.as_ref().unwrap().get("sourceBytes").and_then(|v|v.as_u64()),Some(27));}
    }
    #[test]
    fn w1_backup_reverification_does_not_compare_the_mutated_original_but_rejects_backup_drift() {
        let (_t,roots,locks,mut j,prepared)=fixture(SourceLocation::Root);create_verified(&roots,&locks,&mut j,&prepared).unwrap();fs::write(roots.db.join("a/b"),b"legitimate original recovery writes").unwrap();verify_backup(&roots,&j).unwrap();
        let backup=roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id);fs::write(backup.join("a/b"),b"damaged backup").unwrap();assert!(verify_backup(&roots,&j).is_err());assert_eq!(fs::read(roots.db.join("a/b")).unwrap(),b"legitimate original recovery writes");
    }
    #[test]
    fn w1_backup_copy_kill_child() {
        let Some(base)=std::env::var_os("NWUI_BACKUP_COPY_TEST_ROOT")else{return;};let base=PathBuf::from(base);let roots=Roots {db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};
        let location=if std::env::var("NWUI_BACKUP_COPY_TEST_LAYOUT").as_deref()==Ok("root"){SourceLocation::Root}else{SourceLocation::Pgdata};replay_copy(&roots,location);
    }
    #[test]
    fn w1_backup_copy_real_kills_discard_only_the_partial_attempt_then_verify_both_layouts() {
        use std::os::unix::process::ExitStatusExt;
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {for point in ["intent","reset","entry","copy-synced"] {
            let (t,roots,locks,j,prepared)=fixture(location);let original=source(&roots,location);let before=TreeWalk::new(&original,Some(&prepared.names),true).unwrap().collect::<Result<Vec<_>,_>>().unwrap();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::backup::tests::w1_backup_copy_kill_child","--nocapture"])
                .env("NWUI_BACKUP_COPY_TEST_ROOT",t.path()).env("NWUI_BACKUP_COPY_TEST_LAYOUT",if location==SourceLocation::Root{"root"}else{"pgdata"}).env("NWUI_BACKUP_COPY_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9),"{point}");
            replay_copy(&roots,location);assert_eq!(TreeWalk::new(&original,Some(&prepared.names),true).unwrap().collect::<Result<Vec<_>,_>>().unwrap(),before);
            let path=roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",prepared.header.backup_id));let mut reader=Reader::open(&path).unwrap();verify_pair(&mut reader,&prepared,&original,&roots.db.join(".nwui-pg15-backup").join(&prepared.header.backup_id)).unwrap();
        }}
    }
    #[test]
    fn w1_backup_create_copies_both_layouts_verifies_independently_and_never_touches_source() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            let (_t,roots,locks,mut j,prepared)=fixture(location);let original=source(&roots,location);let original_records=||TreeWalk::new(&original,Some(&prepared.names),true).unwrap().collect::<Result<Vec<_>,_>>().unwrap();let before=original_records();
            create_verified(&roots,&locks,&mut j,&prepared).unwrap();let path=roots.db.join(".nwui-pg15-backup").join(&prepared.header.backup_id);
            let file=path.join("a/b");assert_ne!(fs::metadata(&file).unwrap().ino(),fs::metadata(original.join("a/b")).unwrap().ino());
            assert_eq!(TreeWalk::new(&path,None,true).unwrap().collect::<Result<Vec<_>,_>>().unwrap(),before);assert_eq!(original_records(),before);
            let inode=fs::metadata(&file).unwrap().ino();create_verified(&roots,&locks,&mut j,&prepared).unwrap();assert_eq!(inode,fs::metadata(file).unwrap().ino());
        }
    }
    #[test]
    fn w1_backup_create_refuses_unclaimed_copy_or_after_conversion_without_deleting_anything() {
        for phase in [false,true] {let (_t,roots,locks,mut j,prepared)=fixture(SourceLocation::Pgdata);let path=make_backup(&roots,&prepared);let before=files::snapshot(&path).unwrap();
            if phase {j.record(Step::P3Convert,StepState::Intent,None).unwrap();}
            assert!(create_verified(&roots,&locks,&mut j,&prepared).is_err());assert_eq!(files::snapshot(&path).unwrap(),before);
        }
    }
    #[test]
    fn w1_backup_restored_verification_binds_journal_and_rejects_extra_missing_or_changed_files() {
        for change in ["none","extra","reserved-extra","missing","content","journal"] {
            let (_t,roots,locks,mut j,prepared)=fixture(SourceLocation::Pgdata);make_backup(&roots,&prepared);intent(&mut j,&prepared);publish_verified(&roots,&locks,&mut j,&prepared).unwrap();
            let original=source(&roots,SourceLocation::Pgdata);
            match change {"extra"=>fs::write(original.join("foreign"),"keep").unwrap(),"reserved-extra"=>fs::write(original.join(".nwui-foreign"),"keep").unwrap(),"missing"=>fs::remove_file(original.join("a/b")).unwrap(),"content"=>fs::write(original.join("a/b"),"changed").unwrap(),"journal"=>{j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":prepared.header,"manifestSha256":"0".repeat(64)}))).unwrap();},_=>{}}
            let result=verify_restored(&roots,&j);assert_eq!(result.is_ok(),change=="none","{change}");
            if let Ok(top)=result {assert_eq!(top,prepared.names);}
        }
    }
    #[test]
    fn w1_backup_jsonl_publishes_only_after_two_pass_merge_join_and_replays_without_rewrite() {
        for location in [SourceLocation::Root,SourceLocation::Pgdata] {
            let (_t,roots,locks,mut j,prepared)=fixture(location);let backup=make_backup(&roots,&prepared);intent(&mut j,&prepared);
            let summary=publish_verified(&roots,&locks,&mut j,&prepared).unwrap();assert!(summary.entries>4);assert_eq!(prepared.manifest_bytes,summary.bytes);
            let path=roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",prepared.header.backup_id));let before=fs::metadata(&path).unwrap();
            let mut reader=Reader::open(&path).unwrap();verify_pair(&mut reader,&prepared,&source(&roots,location),&backup).unwrap();
            let count=j.load().records.len();assert_eq!(publish_verified(&roots,&locks,&mut j,&prepared).unwrap(),summary);assert_eq!(j.load().records.len(),count);assert_eq!(before.ino(),fs::metadata(path).unwrap().ino());
        }
    }
    #[test]
    fn w1_backup_merge_join_refuses_extra_missing_content_metadata_and_changed_original() {
        for change in ["extra","missing","content","mode","source"] {
            let (_t,roots,locks,mut j,prepared)=fixture(SourceLocation::Root);let backup=make_backup(&roots,&prepared);intent(&mut j,&prepared);
            match change {"extra"=>fs::write(backup.join("a/extra"),"foreign").unwrap(),"missing"=>fs::remove_file(backup.join("a/b")).unwrap(),"content"=>fs::write(backup.join("a/b"),"evil").unwrap(),"mode"=>fs::set_permissions(backup.join("a/b"),fs::Permissions::from_mode(0o600)).unwrap(),_=>fs::write(roots.db.join("a/b"),"changed").unwrap()};
            assert!(publish_verified(&roots,&locks,&mut j,&prepared).is_err());assert!(!j.load().records.iter().any(|r|r.step==Step::P1Backup && r.state==StepState::Done));
            assert!(!roots.app.join(".nwui-auto").join(format!("backup-{}.jsonl",prepared.header.backup_id)).exists());
        }
    }
    #[test]
    fn w1_backup_non_utf8_name_is_refused_with_an_escaped_detail_before_output() {
        let (_t,roots,_locks,_j,_prepared)=fixture(SourceLocation::Root);let name=std::ffi::OsString::from_vec(vec![0xff,b'x']);fs::write(roots.db.join(&name),"unchanged").unwrap();
        let error=TreeWalk::new(&roots.db,None,false).err().unwrap();assert_eq!(error.code,"unsupported_layout");assert!(error.detail.unwrap().contains("\\x"));
    }
    #[test]
    fn w1_backup_source_walk_refuses_symlink_and_fifo_without_following_or_blocking() {
        for special in ["link","fifo"] {
            let (_t,roots,_locks,_j,prepared)=fixture(SourceLocation::Root);fs::remove_file(roots.db.join("a/b")).unwrap();
            if special=="link" {symlink(roots.app.join("own-test-secret"),roots.db.join("a/b")).unwrap();}else{let name=std::ffi::CString::new(roots.db.join("a/b").as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);}
            let result=write_source(io::sink(),&prepared,&roots.db);assert!(result.is_err());
        }
    }
}
