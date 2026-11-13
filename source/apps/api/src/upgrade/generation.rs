//! A.18 write-once prepared records and the durable selector commit boundary.
use super::{Branch,Journal,KeyBinding,KeySource,Step,StepError,StepState,StorageLocks,selector::{self,GenerationManifest,KeyRecord,Origin,Selector,StoredKeySource}};
use crate::db_upgrade::{files,key_file::{self,KeyPolicy,ValidatedKeyFile},layout::Roots,pair::Pair};
use std::{fs::{self,File,OpenOptions},io::Write,os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt},path::Path};
// No Debug/Serialize: the draft borrows validated key FILE bytes.
pub(crate) struct GenerationDraft<'a> {
    pub generation:&'a str,pub origin:Origin,pub source_profile:Option<(&'a str,&'a str)>,pub actual_id:&'a str,
    pub db_pair:&'a Pair,pub app_pair:&'a Pair,pub agreement:&'a KeyBinding,pub key_source:&'a KeySource,pub key:&'a ValidatedKeyFile,
}
fn mismatch()->StepError {StepError {code:"pair_mismatch",message:"준비한 세대와 복구 기록이 일치하지 않습니다. 기존 내용을 보존했습니다.",detail:None}}
fn io_error(e:std::io::Error)->StepError {
    let code=match e.raw_os_error() {Some(libc::EROFS)=>"storage_read_only",Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EINTR)=>"interrupted",_=>"io_error"};
    StepError {code,message:"세대 기록을 준비하지 못했습니다. 기존 내용을 보존했습니다.",detail:None}
}
fn exists(path:&Path)->Result<bool,StepError> {
    match fs::symlink_metadata(path) {Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(io_error(e))}
}
fn data<'a>(j:&'a Journal,generation:&str,event:&str)->Option<&'a serde_json::Value> {
    j.load().records.iter().rev().filter_map(|r|r.data.as_ref()).find(|v|v.get("authorityEvent").and_then(|v|v.as_str())==Some(event) && v.get("generation").and_then(|v|v.as_str())==Some(generation))
}
fn record(j:&mut Journal,event:&str,generation:&str,created_at:&str,sha:&str)->Result<(),StepError> {
    j.record(Step::P5Publish,if event=="generation-intent"{StepState::Intent}else{StepState::Done},Some(serde_json::json!({"authorityEvent":event,"generation":generation,"createdAt":created_at,"manifestSha256":sha})))
}
fn write_once(root:&Path,name:&str,bytes:&[u8])->Result<(),StepError> {write_record(root,name,bytes,None)}
fn write_record(root:&Path,name:&str,bytes:&[u8],replace:Option<&str>)->Result<(),StepError> {
    if bytes.len()>65536{return Err(mismatch());}
    let path=root.join(name);let temp=root.join(format!(".{name}.init"));
    if exists(&path)? {
        let (_,saved):(serde_json::Value,_)=selector::read_record(&path)?;
        if saved==bytes {if exists(&temp)?{return Err(mismatch());}return Ok(());}
        if replace!=Some(selector::digest(&saved).as_str()){return Err(mismatch());}
    }
    let mut file=if exists(&temp)? {
        files::real_path(&temp).map_err(|_|mismatch())?;
        let file=OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(&temp).map_err(io_error)?;
        let m=file.metadata().map_err(io_error)?;
        if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()!=1 || m.len()>65536{return Err(mismatch());}file
    }else{
        let file=OpenOptions::new().read(true).write(true).create_new(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).mode(0o600).open(&temp).map_err(io_error)?;
        file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(io_error)?;file
    };
    file.set_len(0).map_err(io_error)?;file.write_all(bytes).map_err(io_error)?;file.sync_all().map_err(io_error)?;
    checkpoint("record-fsync");checkpoint(&format!("record-fsync:{name}"));
    if let Some(expected)=replace {let(_,raw):(serde_json::Value,_)=selector::read_record(&path)?;if selector::digest(&raw)!=expected{return Err(mismatch());}}else if exists(&path)? {return Err(mismatch());}
    fs::rename(&temp,&path).map_err(io_error)?;checkpoint("record-rename");checkpoint(&format!("record-rename:{name}"));File::open(root).and_then(|f|f.sync_all()).map_err(io_error)?;checkpoint("record-dir-fsync");checkpoint(&format!("record-dir-fsync:{name}"));Ok(())
}
#[cfg(test)] fn checkpoint(point:&str) {if std::env::var("NWUI_GENERATION_KILL_POINT").as_deref()==Ok(point){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))] fn checkpoint(_point:&str) {}
/// Run within the awaited blocking runner operation, after K/agreement and real-data V2.
pub(crate) fn prepare(roots:&Roots,locks:&StorageLocks,j:&mut Journal,draft:GenerationDraft<'_>)->Result<Selector,StepError> {
    prepare_with_split(roots,locks,j,draft,crate::nonroot::PRIVILEGE_SPLIT)
}
fn prepare_with_split(roots:&Roots,locks:&StorageLocks,j:&mut Journal,draft:GenerationDraft<'_>,split:bool)->Result<Selector,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(mismatch());}
    let safe=completed_preparation_for(j,split).is_some();
    if !safe {return Err(StepError {code:"privilege_prepare_failed",message:"서비스 실행 준비가 완료되지 않아 세대 기록을 만들지 않았습니다.",detail:None});}
    let key_source=j.key_source()?;
    if key_source!=*draft.key_source {return Err(mismatch());}
    let key_kind=match &key_source {KeySource::Canonical=>StoredKeySource::Canonical,KeySource::Staged {generation} if generation==draft.generation=>StoredKeySource::Staged,_=>return Err(mismatch())};
    draft.db_pair.validate().and_then(|_|draft.app_pair.validate()).map_err(|_|mismatch())?;
    let mut other=draft.app_pair.clone();other.encryption_key_required=draft.db_pair.encryption_key_required;
    if other!=*draft.db_pair || !draft.db_pair.ready || draft.db_pair.architecture!=roots.architecture || draft.db_pair.system_identifier.as_deref()!=Some(draft.actual_id)
        || !matches!(draft.agreement,KeyBinding::Verified {pair_id} if pair_id==&draft.db_pair.pair_id) {return Err(mismatch());}
    let backup_id=if draft.origin==Origin::Auto {
        let proof=super::profile::proof(j)?;
        if draft.source_profile!=Some((proof.profile_id.as_str(),proof.profile_sha256.as_str())){return Err(mismatch());}
        if j.load().branch!=Branch::Upgrade || !j.load().records.iter().any(|r|r.step==Step::P4Target && r.state==StepState::Done && r.data.is_none()) {return Err(mismatch());}
        let backed=j.load().records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done).ok_or_else(mismatch)?;
        if backed.data.as_ref().and_then(|v|v.get("header")).and_then(|v|v.get("backupId")).and_then(|v|v.as_str())!=Some(j.load().attempt_id.as_str()) {return Err(mismatch());}
        Some(j.load().attempt_id.clone())
    }else{if !matches!(j.load().branch,Branch::Fresh|Branch::Ready){return Err(mismatch());}None};
    let created_at=if let Some(intent)=data(j,draft.generation,"generation-intent") {intent.get("createdAt").and_then(|v|v.as_str()).ok_or_else(mismatch)?.to_owned()}else{chrono::Utc::now().to_rfc3339()};
    let key=KeyRecord {version:1,key_binding:draft.agreement.clone(),key_source:key_kind,key_sha256:selector::digest(&draft.key.bytes)};
    let db_bytes=serde_json::to_vec(draft.db_pair).map_err(|_|mismatch())?;let app_bytes=serde_json::to_vec(draft.app_pair).map_err(|_|mismatch())?;let key_bytes=serde_json::to_vec(&key).map_err(|_|mismatch())?;
    let manifest=GenerationManifest {version:1,generation:draft.generation.into(),system_identifier:draft.actual_id.into(),origin:draft.origin,
        profile_id:draft.source_profile.map(|(id,_)|id.into()),profile_sha256:draft.source_profile.map(|(_,sha)|sha.into()),db_pair_sha256:selector::digest(&db_bytes),app_pair_sha256:selector::digest(&app_bytes),key_binding_sha256:selector::digest(&key_bytes),created_at:created_at.clone()};
    manifest.validate()?;let manifest_bytes=serde_json::to_vec(&manifest).map_err(|_|mismatch())?;let manifest_sha=selector::digest(&manifest_bytes);
    let selected=Selector {version:1,active_major:17,generation:Some(draft.generation.into()),system_identifier:draft.actual_id.into(),origin:draft.origin,backup_id,
        profile_id:manifest.profile_id.clone(),profile_sha256:manifest.profile_sha256.clone(),manifest_sha256:Some(manifest_sha.clone()),operator_reason:None};selected.validate()?;
    let root=roots.app.join(".nwui-auto/gen").join(draft.generation);
    let planned=serde_json::json!({"authorityEvent":"generation-intent","generation":draft.generation,"createdAt":created_at,"manifestSha256":manifest_sha});
    let previous=data(j,draft.generation,"generation-intent");if previous.is_some_and(|v|v!=&planned){return Err(mismatch());}
    if exists(&root)? {
        files::private_dir(&root).map_err(|_|mismatch())?;
        if previous.is_none() && key_kind!=StoredKeySource::Staged {return Err(mismatch());}
        for item in fs::read_dir(&root).map_err(io_error)? {
            let name=item.map_err(io_error)?.file_name();let name=name.to_str().ok_or_else(mismatch)?;
            if !(matches!(name,"db-pair.json"|"app-pair.json"|"key-binding.json"|"manifest.json"|".db-pair.json.init"|".app-pair.json.init"|".key-binding.json.init"|".manifest.json.init") || name=="data_key" && key_kind==StoredKeySource::Staged) {return Err(mismatch());}
            if previous.is_none() && name!="data_key" {return Err(mismatch());}
        }
    }
    if key_kind==StoredKeySource::Staged {
        let staged=key_file::read_key_file(&root.join("data_key"),KeyPolicy::Strict).map_err(|_|mismatch())?;
        if staged.bytes!=draft.key.bytes {return Err(mismatch());}
    }
    if previous.is_none(){record(j,"generation-intent",draft.generation,&created_at,&manifest_sha)?;checkpoint("intent");}
    let parent=roots.app.join(".nwui-auto/gen");files::private_dir(&parent).map_err(|_|mismatch())?;files::private_dir(&root).map_err(|_|mismatch())?;checkpoint("directory");
    for (name,bytes) in [("db-pair.json",&db_bytes),("app-pair.json",&app_bytes),("key-binding.json",&key_bytes),("manifest.json",&manifest_bytes)] {write_once(&root,name,bytes)?;}
    File::open(&root).and_then(|f|f.sync_all()).map_err(io_error)?;File::open(&parent).and_then(|f|f.sync_all()).map_err(io_error)?;
    if data(j,draft.generation,"generation-done").is_none() {record(j,"generation-done",draft.generation,&created_at,&manifest_sha)?;checkpoint("done");}
    Ok(selected)
}

// S4Prepare belongs exclusively to N2; its latest completed record owns both handles.
pub(super) fn completed_preparation(j:&Journal)->Option<crate::nonroot::Prepared> {
    completed_preparation_for(j,crate::nonroot::PRIVILEGE_SPLIT)
}
pub(super) fn completed_preparation_for(j:&Journal,split:bool)->Option<crate::nonroot::Prepared> {
    let split_off=j.load().records.iter().any(|r|r.step==Step::P5Publish && r.state==StepState::Done && r.data.as_ref().and_then(|v|v.get("authorityEvent")).and_then(|v|v.as_str())==Some("privilege-split-off"));
    if !split {
        if j.load().records.iter().any(|r|r.step==Step::S4Prepare) {return None;}
        let latest=j.load().records.iter().rev().find(|r|r.step==Step::P5Publish && r.state==StepState::Done && matches!(r.data.as_ref().and_then(|v|v.get("authorityEvent")).and_then(|v|v.as_str()),Some("privilege-split-off"|"privilege-discarded")))?;
        return (latest.data.as_ref()?.get("authorityEvent")?.as_str()==Some("privilege-split-off")).then_some(crate::nonroot::Prepared{payload:None,key_receipt:None});
    }
    if split_off {return None;}
    let owner=j.load().records.iter().rev().find(|r|r.step==Step::S4Prepare && matches!(r.state,StepState::Intent|StepState::Done))?;
    if owner.state!=StepState::Done || j.load().records.iter().any(|r|r.seq>owner.seq && r.step==Step::P5Publish && r.data.as_ref().and_then(|v|v.get("authorityEvent")).and_then(|v|v.as_str())==Some("privilege-discarded")) {return None;}
    crate::nonroot::prepared_from_journal(j).filter(|p|p.payload.is_some() && p.key_receipt.is_some())
}
fn prepare_completed(j:&Journal)->bool {completed_preparation(j).is_some()}
fn done(j:&Journal,step:Step)->bool {
    j.load().records.iter().rev().find(|r|r.step==step).is_some_and(|r|r.state==StepState::Done && r.data.is_none())
}
fn reject_d_mirror(roots:&Roots)->Result<(),StepError> {
    if exists(&roots.db.join(".nwui"))? {return Err(StepError {code:"unsupported_layout",message:"자동 이전 데이터에 외래 D/.nwui 항목이 있습니다. 기존 내용을 보존했습니다.",detail:None});}Ok(())
}
/// The caller supplies the fixed target's actual control ID and this boot's validated key.
/// An on-disk selector, even before the journal Done, is committed: never R-restore it.
pub(crate) fn commit(roots:&Roots,locks:&StorageLocks,j:&mut Journal,selected:&Selector,actual_id:&str,key:&ValidatedKeyFile)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) {return Err(mismatch());}
    let prepared=selector::read_prepared(&roots.app,&roots.architecture,selected.clone(),actual_id,&key.bytes)?;
    let generation=selected.generation.as_deref().ok_or_else(mismatch)?;
    let evidence=data(j,generation,"generation-done").ok_or_else(mismatch)?;
    if evidence.get("manifestSha256").and_then(|v|v.as_str())!=selected.manifest_sha256.as_deref() {return Err(mismatch());}
    if selected.origin==Origin::Auto {
        let proof=super::profile::proof(j)?;
        if selected.profile_id.as_deref()!=Some(proof.profile_id.as_str()) || selected.profile_sha256.as_deref()!=Some(proof.profile_sha256.as_str()){return Err(mismatch());}
        reject_d_mirror(roots)?;
        if j.load().branch!=Branch::Upgrade || selected.backup_id.as_deref()!=Some(j.load().attempt_id.as_str()) || !done(j,Step::P5Guard) {return Err(mismatch());}
    }
    if selected.origin==Origin::Auto {
        let pair_path=roots.app.join(".nwui/pair-v1.json");
        if exists(&pair_path)? {let (live,_):(Pair,_)=selector::read_record(&pair_path)?;selector::same_pair_except_flag(&prepared.app_pair,&live)?;}
    }else{
        selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected.clone(),actual_id,&key.bytes)?;
    }
    let root=roots.app.join(".nwui-auto");let path=root.join("active-v1.json");
    let bytes=serde_json::to_vec(selected).map_err(|_|mismatch())?;
    let mut intent=serde_json::json!({"authorityEvent":"selector-intent","generation":generation,"selectorSha256":selector::digest(&bytes)});
    let previous=j.load().records.iter().rev().find(|r|r.step==Step::P5Commit && r.state==StepState::Intent).map(|r|r.data.clone().ok_or_else(mismatch)).transpose()?;
    let saved=if exists(&path)?{Some(selector::read_record::<Selector>(&path)?)}else{None};
    let replacing=if let Some((old,raw))=&saved {
        if old.active_major==15 {
            if selected.origin!=Origin::Auto || done(j,Step::P5Commit){return Err(mismatch());}
            super::replacement::restored(j,old,super::replacement::source_id(j)?)?;Some(selector::digest(raw))
        }else{
            if old!=selected || raw!=&bytes || previous.is_none(){return Err(mismatch());}None
        }
    }else{None};
    if let Some(sha)=&replacing{intent["replaces"]=serde_json::json!({"selectorSha256":sha});}
    else if let Some(replaces)=previous.as_ref().and_then(|v|v.get("replaces")) {
        let sha=replaces.get("selectorSha256").and_then(|v|v.as_str()).filter(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).ok_or_else(mismatch)?;
        if replaces!=&serde_json::json!({"selectorSha256":sha}) || saved.is_none(){return Err(mismatch());}intent["replaces"]=replaces.clone();
    }
    if previous.as_ref().is_some_and(|p|p!=&intent){return Err(mismatch());}
    if saved.is_none() || replacing.is_some() {
        if done(j,Step::P5Commit) {return Err(mismatch());}
        if !prepare_completed(j) {return Err(StepError {code:"privilege_prepare_failed",message:"서비스 실행 준비가 완료되지 않아 데이터 선택을 확정하지 않았습니다.",detail:None});}
        if previous.is_none() {j.record(Step::P5Commit,StepState::Intent,Some(intent))?;checkpoint("selector-intent");}
        write_record(&root,"active-v1.json",&bytes,replacing.as_deref())?;
    }
    // Resume also syncs a rename completed before a kill; Done must follow that sync.
    File::open(&root).and_then(|f|f.sync_all()).map_err(io_error)?;checkpoint("selector-durable");
    if selected.origin==Origin::Auto {
        let pair_root=roots.app.join(".nwui");let pair_path=pair_root.join("pair-v1.json");
        if exists(&pair_path)? {
            let (live,_):(Pair,_)=selector::read_record(&pair_path)?;selector::same_pair_except_flag(&prepared.app_pair,&live)?;
        }else{
            if done(j,Step::P5Commit) {return Err(mismatch());}
            files::private_dir(&pair_root).map_err(io_error)?;
            write_once(&pair_root,"pair-v1.json",&serde_json::to_vec(&prepared.app_pair).map_err(|_|mismatch())?)?;
            File::open(&pair_root).and_then(|f|f.sync_all()).map_err(io_error)?;
        }
    }
    selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected.clone(),actual_id,&key.bytes)?;
    if !done(j,Step::P5Commit) {j.record(Step::P5Commit,StepState::Done,None)?;checkpoint("selector-done");}Ok(())
}
/// Call only after canonical key publication/agreement. Immutable generation snapshots stay unchanged.
pub(crate) fn mark_key_required(roots:&Roots,locks:&StorageLocks,actual_id:&str,key:&ValidatedKeyFile)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) {return Err(mismatch());}
    let canonical_path=crate::db_upgrade::paths::startup_data_key_file(&roots.app).map_err(|e|StepError {code:e.code,message:e.message,detail:None})?;
    let canonical=key_file::read_key_file(&canonical_path,key_file::canonical_policy(&roots.app,&canonical_path)).map_err(|_|mismatch())?;
    if canonical.bytes!=key.bytes {return Err(mismatch());}
    let selected=selector::read_selector(&roots.app)?;
    let prepared=selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected,actual_id,&canonical.bytes)?;
    let targets=if prepared.selector.origin==Origin::Auto {vec![&roots.app]}else{vec![&roots.db,&roots.app]};
    for root in targets {
        let path=root.join(".nwui/pair-v1.json");let (mut live,_):(Pair,_)=selector::read_record(&path)?;
        if !live.encryption_key_required {live.encryption_key_required=true;files::atomic_json(&path,&live).map_err(io_error)?;}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::journal::Candidate;
    use crate::crypto::SecretBox;
    use std::os::unix::fs::symlink;
    struct Fixture { _temp:tempfile::TempDir,roots:Roots,locks:StorageLocks,j:Journal,pair:Pair,key:ValidatedKeyFile,generation:String }
    impl Fixture {
        fn new()->Self {Self::new_branch(Branch::Ready)}
        fn new_branch(branch:Branch)->Self {let mut f=Self::unprepared(branch);f.prepared(true);f}
        fn unprepared(branch:Branch)->Self {
            let t=tempfile::tempdir().unwrap();let roots=Roots {db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
            for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,branch,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            let mut pair=Pair::fresh("arm64");pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some("18446744073709551614".into());
            let key=ValidatedKeyFile {secrets:SecretBox::from_key([1;32]),bytes:hex::encode([1u8;32]).into_bytes()};
            Self {_temp:t,roots,locks,j,pair,key,generation:uuid::Uuid::new_v4().to_string()}
        }
        fn prepared(&mut self,completed:bool) {
            if !crate::nonroot::PRIVILEGE_SPLIT && completed {self.j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-split-off"}))).unwrap();return;}
            self.owner_prepared(completed);
        }
        fn owner_prepared(&mut self,completed:bool) {
            let selected=super::super::StorageSelection {version:1,epoch:1,outcome:super::super::Outcome::Pg17,branch:self.j.load().branch,pgdata:self.roots.db.join("17/data"),backup_id:None,schema_profile:"none".into(),key_binding:KeyBinding::Verified {pair_id:self.pair.pair_id.clone()}};
            let first=self.j.load().records.iter().map(|r|r.seq).chain(self.j.load().moves.iter().map(|r|r.seq)).max().unwrap_or(0)+1;
            for r in crate::nonroot::fixture_prepare_records(&selected,&self.j.load().attempt_id,first,completed) {self.j.record(r.step,r.state,r.data).unwrap();}
        }
        fn prepare(&mut self)->Result<Selector,StepError> {self.prepare_for(crate::nonroot::PRIVILEGE_SPLIT)}
        fn prepare_for(&mut self,split:bool)->Result<Selector,StepError> {
            let agreement=KeyBinding::Verified {pair_id:self.pair.pair_id.clone()};
            prepare_with_split(&self.roots,&self.locks,&mut self.j,GenerationDraft {generation:&self.generation,origin:Origin::Fresh,source_profile:None,actual_id:self.pair.system_identifier.as_deref().unwrap(),db_pair:&self.pair,app_pair:&self.pair,agreement:&agreement,key_source:&KeySource::Canonical,key:&self.key},split)
        }
        fn auto(&mut self)->Selector {
            assert_eq!(self.j.load().branch,Branch::Upgrade);
            self.j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":{"backupId":self.j.load().attempt_id,"systemIdentifier":"123456"}}))).unwrap();
            self.j.record(Step::P4Target,StepState::Done,None).unwrap();self.prepared(true);self.j.record(Step::P5Guard,StepState::Done,None).unwrap();
            let proof=super::super::profile::packaged_identity().unwrap();self.j.record(Step::P2Preflight,StepState::Done,Some(super::super::profile::fixture_done(crate::db_upgrade::migration::UpgradeMode::Copy))).unwrap();
            let agreement=KeyBinding::Verified {pair_id:self.pair.pair_id.clone()};
            prepare(&self.roots,&self.locks,&mut self.j,GenerationDraft {generation:&self.generation,origin:Origin::Auto,source_profile:Some((&proof.profile_id,&proof.profile_sha256)),actual_id:self.pair.system_identifier.as_deref().unwrap(),db_pair:&self.pair,app_pair:&self.pair,agreement:&agreement,key_source:&KeySource::Canonical,key:&self.key}).unwrap()
        }
        fn commit(&mut self,selected:&Selector)->Result<(),StepError> {commit(&self.roots,&self.locks,&mut self.j,selected,self.pair.system_identifier.as_deref().unwrap(),&self.key)}
        fn root(&self)->std::path::PathBuf {self.roots.app.join(".nwui-auto/gen").join(&self.generation)}
    }
    #[test]
    fn w1_generation_resume_refuses_replaced_p2_proof_before_selector_or_live_pair_writes() {
        let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();let before=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();
        f.j.record(Step::P2Preflight,StepState::Done,Some(serde_json::json!({"profileId":"legacy-28","profileSha256":"1".repeat(64)}))).unwrap();let current=crate::db_upgrade::files::snapshot(&f.roots.app).unwrap();
        assert!(commit(&f.roots,&f.locks,&mut f.j,&selected,f.pair.system_identifier.as_deref().unwrap(),&f.key).is_err());assert_eq!(crate::db_upgrade::files::snapshot(&f.roots.app).unwrap(),current);assert!(!f.roots.app.join(".nwui-auto/active-v1.json").exists());assert_ne!(before,current);
    }
    #[test]
    fn w1_staged_selector_commit_kill_child() {
        let Some(base)=std::env::var_os("NWUI_STAGED_COMMIT_ROOT")else{return};let base=std::path::PathBuf::from(base);let roots=Roots{db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};let locks=wait_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();let selected:Selector=serde_json::from_str(&std::env::var("NWUI_STAGED_COMMIT_SELECTOR").unwrap()).unwrap();let source=j.key_source().unwrap();let key=key_file::read_key_file(&source.path(&roots.app,None),KeyPolicy::Strict).unwrap();commit(&roots,&locks,&mut j,&selected,&selected.system_identifier,&key).unwrap();
    }
    #[tokio::test]
    async fn w1_committed_resume_finishes_selector_then_staged_key_without_replacing_bad_material() {
        use std::{sync::Arc,os::unix::process::ExitStatusExt};use super::super::{key_write::{self,Purpose,Mode},committed};use crate::db_upgrade::control::{Control,COMPATIBILITY_FIELDS};
        for change in ["none","stage","canonical"] {
            let mut f=Fixture::unprepared(Branch::Upgrade);files::private_dir(&f.roots.app.join("secrets")).unwrap();key_write::write_locked(&f.locks,&f.roots,&mut f.j,Purpose::Staged{generation:f.generation.clone()},Mode::Publish).unwrap();key_write::write_locked(&f.locks,&f.roots,&mut f.j,Purpose::Publication{generation:f.generation.clone()},Mode::Prepare).unwrap();let stage=KeySource::Staged{generation:f.generation.clone()}.path(&f.roots.app,None);f.key=key_file::read_key_file(&stage,KeyPolicy::Strict).unwrap();let expected=f.key.bytes.clone();
            f.j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":{"backupId":f.j.load().attempt_id}}))).unwrap();f.j.record(Step::P4Target,StepState::Done,None).unwrap();f.prepared(true);f.j.record(Step::P5Guard,StepState::Done,None).unwrap();let agreement=KeyBinding::Verified{pair_id:f.pair.pair_id.clone()};let proof=super::super::profile::packaged_identity().unwrap();f.j.record(Step::P2Preflight,StepState::Done,Some(super::super::profile::fixture_done(crate::db_upgrade::migration::UpgradeMode::Copy))).unwrap();let selected=prepare(&f.roots,&f.locks,&mut f.j,GenerationDraft{generation:&f.generation,origin:Origin::Auto,source_profile:Some((&proof.profile_id,&proof.profile_sha256)),actual_id:f.pair.system_identifier.as_deref().unwrap(),db_pair:&f.pair,app_pair:&f.pair,agreement:&agreement,key_source:&KeySource::Staged{generation:f.generation.clone()},key:&f.key}).unwrap();
            for p in [f.roots.db.join("17"),f.roots.db.join("17/data"),f.roots.db.join("17/data/global")] {files::private_dir(&p).unwrap();}fs::write(f.roots.db.join("17/data/PG_VERSION"),b"17\n").unwrap();fs::write(f.roots.db.join("17/data/global/pg_control"),b"fixture-control").unwrap();let manifest=fs::read(f.root().join("manifest.json")).unwrap();let Fixture{_temp,roots,locks,j,pair,key:_,generation:_}=f;drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::generation::tests::w1_staged_selector_commit_kill_child","--nocapture"]).env("NWUI_STAGED_COMMIT_ROOT",_temp.path()).env("NWUI_STAGED_COMMIT_SELECTOR",serde_json::to_string(&selected).unwrap()).env("NWUI_GENERATION_KILL_POINT","selector-durable").status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL));
            let locks=Arc::new(wait_locks(&roots));let j=Journal::open(&roots.app,&locks).unwrap();assert!(!done(&j,Step::P5Commit));let canonical=roots.app.join("secrets/data_key");let bad=b"02".repeat(32);if change=="stage"{fs::write(&stage,&bad).unwrap();}if change=="canonical"{fs::write(&canonical,&bad).unwrap();fs::set_permissions(&canonical,fs::Permissions::from_mode(0o600)).unwrap();}
            let text=format!("Database system identifier: {}\nDatabase cluster state: shut down\n{}",pair.system_identifier.as_deref().unwrap(),COMPATIBILITY_FIELDS.iter().map(|field|format!("{field}: 1\n")).collect::<String>());let control=Control::parse(&text,"").unwrap();let result=committed::resume_with_control(roots.clone(),locks.clone(),j,control).await;
            if change=="none"{let j=result.unwrap();assert!(done(&j,Step::P5Commit));assert!(fs::read(&canonical).unwrap()==expected);assert!(fs::read(&stage).unwrap()==expected);let (live,_):(Pair,_)=selector::read_record(&roots.app.join(".nwui/pair-v1.json")).unwrap();assert!(live.encryption_key_required);}
            else{assert!(result.is_err());if change=="stage"{assert!(!canonical.exists());assert!(fs::read(&stage).unwrap()==bad);}else{assert!(fs::read(&canonical).unwrap()==bad);assert!(fs::read(&stage).unwrap()==expected);}}
            assert!(!roots.db.join(".nwui").exists());assert_eq!(fs::read(roots.app.join(".nwui-auto/gen").join(selected.generation.as_deref().unwrap()).join("manifest.json")).unwrap(),manifest);
        }
    }
    #[test]
    fn w1_generation_commit_kill_child() {
        let Some(base)=std::env::var_os("NWUI_COMMIT_TEST_ROOT")else{return;};let base=std::path::PathBuf::from(base);
        let roots=Roots {db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};let locks=wait_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();
        let selected:Selector=serde_json::from_str(&std::env::var("NWUI_COMMIT_TEST_SELECTOR").unwrap()).unwrap();let key=ValidatedKeyFile {secrets:SecretBox::from_key([1;32]),bytes:hex::encode([1u8;32]).into_bytes()};
        commit(&roots,&locks,&mut j,&selected,&selected.system_identifier,&key).unwrap();
    }
    #[test]
    fn w1_generation_commit_real_kills_roll_forward_a_only_including_selector_before_journal_done() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["selector-intent","record-fsync:active-v1.json","record-rename:active-v1.json","selector-durable","record-rename:pair-v1.json","selector-done"] {
            let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();let Fixture {_temp,roots,locks,j,pair:_,key,generation:_}=f;drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::generation::tests::w1_generation_commit_kill_child","--nocapture"])
                .env("NWUI_COMMIT_TEST_ROOT",_temp.path()).env("NWUI_COMMIT_TEST_SELECTOR",serde_json::to_string(&selected).unwrap()).env("NWUI_GENERATION_KILL_POINT",point).status().unwrap();
            assert_eq!(status.signal(),Some(9),"kill point not reached: {point}");
            let locks=wait_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();commit(&roots,&locks,&mut j,&selected,&selected.system_identifier,&key).unwrap();
            assert_eq!(selector::read_selector(&roots.app).unwrap(),selected);assert!(done(&j,Step::P5Commit));assert!(!roots.db.join(".nwui").exists());
            selector::load_generation(&roots.db,&roots.app,&roots.architecture,selected.clone(),&selected.system_identifier,&key.bytes).unwrap();
        }
    }
    #[test]
    fn w1_generation_foreign_live_pair_or_existing_different_selector_is_preserved() {
        for change in ["pair","selector"] {let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();
            let path=if change=="pair" {let root=f.roots.app.join(".nwui");files::private_dir(&root).unwrap();let mut other=f.pair.clone();other.pair_id=uuid::Uuid::new_v4().to_string();let path=root.join("pair-v1.json");files::atomic_json(&path,&other).unwrap();path}
            else {let mut other=selected.clone();other.system_identifier="999".into();let path=f.roots.app.join(".nwui-auto/active-v1.json");files::atomic_json(&path,&other).unwrap();path};
            let before=fs::read(&path).unwrap();assert!(f.commit(&selected).is_err());assert_eq!(fs::read(&path).unwrap(),before);
            if change=="pair" {assert!(!f.roots.app.join(".nwui-auto/active-v1.json").exists());}
        }
    }
    #[test]
    fn w1_generation_auto_commit_is_idempotent_a_only_and_flag_keeps_snapshots_immutable() {
        let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();f.commit(&selected).unwrap();
        let manifest=fs::read(f.root().join("manifest.json")).unwrap();let snapshot=fs::read(f.root().join("app-pair.json")).unwrap();
        assert!(!f.roots.db.join(".nwui").exists());assert_eq!(selector::read_selector(&f.roots.app).unwrap(),selected);
        let count=f.j.load().records.len();f.commit(&selected).unwrap();assert_eq!(count,f.j.load().records.len());
        assert!(mark_key_required(&f.roots,&f.locks,f.pair.system_identifier.as_deref().unwrap(),&f.key).is_err());
        let secrets=f.roots.app.join("secrets");files::private_dir(&secrets).unwrap();let canonical=secrets.join("data_key");fs::write(&canonical,&f.key.bytes).unwrap();fs::set_permissions(&canonical,fs::Permissions::from_mode(0o600)).unwrap();
        mark_key_required(&f.roots,&f.locks,f.pair.system_identifier.as_deref().unwrap(),&f.key).unwrap();
        let (live,_):(Pair,_)=selector::read_record(&f.roots.app.join(".nwui/pair-v1.json")).unwrap();assert!(live.encryption_key_required);
        assert!(!f.roots.db.join(".nwui").exists());assert_eq!(fs::read(f.root().join("app-pair.json")).unwrap(),snapshot);assert_eq!(fs::read(f.root().join("manifest.json")).unwrap(),manifest);
    }
    #[test]
    fn w1_generation_commit_rejects_stray_d_authority_and_changed_target_before_any_selector() {
        for stray in [false,true] {let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();
            if stray {fs::create_dir(f.roots.db.join(".nwui")).unwrap();}else{f.pair.system_identifier=Some("999".into());}
            assert!(f.commit(&selected).is_err());assert!(!f.roots.app.join(".nwui-auto/active-v1.json").exists());assert!(!f.roots.app.join(".nwui/pair-v1.json").exists());
        }
    }
    #[test]
    fn w1_generation_records_are_compact_hashed_once_and_reused_without_rewrites() {
        let mut f=Fixture::new();let selected=f.prepare().unwrap();assert!(!f.roots.app.join(".nwui-auto/active-v1.json").exists());
        let first=fs::metadata(f.root().join("manifest.json")).unwrap();let count=f.j.load().records.len();assert_eq!(f.prepare().unwrap(),selected);
        assert_eq!(first.ino(),fs::metadata(f.root().join("manifest.json")).unwrap().ino());assert_eq!(f.j.load().records.len(),count);
        let (manifest,bytes):(GenerationManifest,_)=selector::read_record(&f.root().join("manifest.json")).unwrap();assert_eq!(selected.manifest_sha256.as_deref(),Some(selector::digest(&bytes).as_str()));assert_ne!(bytes.last(),Some(&b'\n'));
        for (name,sha) in [("db-pair.json",manifest.db_pair_sha256),("app-pair.json",manifest.app_pair_sha256),("key-binding.json",manifest.key_binding_sha256)] {let (_,bytes):(serde_json::Value,_)=selector::read_record(&f.root().join(name)).unwrap();assert_eq!(selector::digest(&bytes),sha);}
    }
    const CHILD_ROOT:&str="NWUI_GENERATION_TEST_ROOT";
    fn wait_locks(roots:&Roots)->StorageLocks {
        let until=std::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {match StorageLocks::acquire(&roots.db,&roots.app) {Ok(v)=>return v,Err(e)if e.code=="storage_busy" && std::time::Instant::now()<until=>std::thread::sleep(std::time::Duration::from_millis(5)),Err(e)=>panic!("{e}")}}
    }
    fn replay(roots:&Roots,pair:&Pair,generation:&str)->Selector {
        let locks=wait_locks(roots);let mut j=Journal::open(&roots.app,&locks).unwrap();let agreement=KeyBinding::Verified {pair_id:pair.pair_id.clone()};
        let key=ValidatedKeyFile {secrets:SecretBox::from_key([1;32]),bytes:hex::encode([1u8;32]).into_bytes()};
        prepare(roots,&locks,&mut j,GenerationDraft {generation,origin:Origin::Fresh,source_profile:None,actual_id:pair.system_identifier.as_deref().unwrap(),db_pair:pair,app_pair:pair,agreement:&agreement,key_source:&KeySource::Canonical,key:&key}).unwrap()
    }
    #[test]
    fn w1_generation_kill_child() {
        let Some(base)=std::env::var_os(CHILD_ROOT)else{return;};let base=std::path::PathBuf::from(base);let roots=Roots {db:base.join("db"),app:base.join("app"),architecture:"arm64".into()};
        let pair:Pair=serde_json::from_str(&std::env::var("NWUI_GENERATION_TEST_PAIR").unwrap()).unwrap();let generation=std::env::var("NWUI_GENERATION_TEST_ID").unwrap();replay(&roots,&pair,&generation);
    }
    #[test]
    fn w1_generation_real_kills_resume_through_manifest_rename_and_parent_sync() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["intent","directory","record-fsync:db-pair.json","record-rename:manifest.json","record-dir-fsync:manifest.json","done"] {
            let Fixture {_temp,roots,locks,j,pair,key:_,generation}=Fixture::new();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::generation::tests::w1_generation_kill_child","--nocapture"])
                .env(CHILD_ROOT,_temp.path()).env("NWUI_GENERATION_TEST_PAIR",serde_json::to_string(&pair).unwrap()).env("NWUI_GENERATION_TEST_ID",&generation).env("NWUI_GENERATION_KILL_POINT",point).status().unwrap();
            assert_eq!(status.signal(),Some(9),"kill point not reached: {point}");let selected=replay(&roots,&pair,&generation);
            let root=roots.app.join(".nwui-auto/gen").join(&generation);let (_,bytes):(GenerationManifest,_)=selector::read_record(&root.join("manifest.json")).unwrap();assert_eq!(selected.manifest_sha256.as_deref(),Some(selector::digest(&bytes).as_str()));
            assert!(!roots.app.join(".nwui-auto/active-v1.json").exists());
        }
    }
    #[test]
    fn w1_generation_split_modes_are_exclusive_without_changing_the_build_constant() {
        let mut f=Fixture::unprepared(Branch::Ready);
        for split in [false,true] {assert!(completed_preparation_for(&f.j,split).is_none());assert_eq!(f.prepare_for(split).unwrap_err().code,"privilege_prepare_failed");}
        f.j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-split-off"}))).unwrap();
        assert!(completed_preparation_for(&f.j,false).is_some());assert!(completed_preparation_for(&f.j,true).is_none());
        assert_eq!(f.prepare_for(true).unwrap_err().code,"privilege_prepare_failed");f.prepare_for(false).unwrap();
        f.owner_prepared(true);
        for split in [false,true] {assert!(completed_preparation_for(&f.j,split).is_none());assert_eq!(f.prepare_for(split).unwrap_err().code,"privilege_prepare_failed");}
        let mut owner=Fixture::unprepared(Branch::Ready);owner.owner_prepared(true);
        assert!(completed_preparation_for(&owner.j,true).is_some());assert!(completed_preparation_for(&owner.j,false).is_none());
        owner.j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-discarded"}))).unwrap();assert!(completed_preparation_for(&owner.j,true).is_none());
    }
    #[test]
    fn w1_generation_preserves_the_real_n2_record_and_both_handles_for_activate_and_discard() {
        let mut f=Fixture::unprepared(Branch::Ready);f.owner_prepared(true);let before=serde_json::to_value(crate::nonroot::prepared_from_journal(&f.j).unwrap()).unwrap();assert!(completed_preparation_for(&f.j,true).is_some());
        f.prepare_for(true).unwrap();assert_eq!(serde_json::to_value(crate::nonroot::prepared_from_journal(&f.j).unwrap()).unwrap(),before);
        f.owner_prepared(false);assert!(completed_preparation_for(&f.j,true).is_none());let partial=crate::nonroot::prepared_from_journal(&f.j).unwrap();assert!(partial.payload.is_none() && partial.key_receipt.is_none());
        assert_eq!(f.prepare_for(true).unwrap_err().code,"privilege_prepare_failed");
    }
    #[test]
    fn w1_generation_discarded_owner_done_cannot_be_reused_without_a_new_prepare() {
        let mut f=Fixture::new();assert!(prepare_completed(&f.j));f.j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-discarded"}))).unwrap();
        assert_eq!(f.prepare().unwrap_err().code,"privilege_prepare_failed");assert!(!f.root().exists());f.prepared(true);assert!(f.prepare().is_ok());
    }
    #[test]
    fn w1_generation_cannot_advance_past_a_new_partial_s4_intent() {
        let mut f=Fixture::new();f.prepared(false);
        let count=f.j.load().records.len();assert_eq!(f.prepare().unwrap_err().code,"privilege_prepare_failed");assert_eq!(f.j.load().records.len(),count);assert!(!f.root().exists());
    }
    #[test]
    fn w1_generation_never_adopts_a_foreign_directory_or_unknown_child() {
        let mut f=Fixture::new();files::private_dir(&f.roots.app.join(".nwui-auto/gen")).unwrap();files::private_dir(&f.root()).unwrap();assert!(f.prepare().is_err());
        fs::remove_dir(f.root()).unwrap();f.prepare().unwrap();fs::write(f.root().join("foreign"),"keep").unwrap();assert!(f.prepare().is_err());assert_eq!(fs::read(f.root().join("foreign")).unwrap(),b"keep");
    }
    #[test]
    fn w1_generation_partial_fixed_temp_resumes_only_under_the_matching_intent() {
        let mut f=Fixture::new();f.prepare().unwrap();let root=f.root();let final_path=root.join("db-pair.json");fs::remove_file(&final_path).unwrap();fs::write(root.join(".db-pair.json.init"),b"partial").unwrap();fs::set_permissions(root.join(".db-pair.json.init"),fs::Permissions::from_mode(0o600)).unwrap();f.prepare().unwrap();assert!(!root.join(".db-pair.json.init").exists());
        fs::remove_file(&final_path).unwrap();symlink(root.join("app-pair.json"),root.join(".db-pair.json.init")).unwrap();assert!(f.prepare().is_err());assert!(fs::symlink_metadata(root.join(".db-pair.json.init")).unwrap().file_type().is_symlink());
    }
    #[test]
    fn w1_generation_different_inputs_or_written_bytes_refuse_and_preserve() {
        let mut f=Fixture::new();f.prepare().unwrap();let root=f.root();let before=fs::read(root.join("manifest.json")).unwrap();f.pair.system_identifier=Some("123456".into());assert!(f.prepare().is_err());assert_eq!(fs::read(root.join("manifest.json")).unwrap(),before);
        f.pair.system_identifier=Some("18446744073709551614".into());let path=root.join("db-pair.json");let mut altered=fs::read(&path).unwrap();altered.push(b' ');fs::write(&path,&altered).unwrap();assert!(f.prepare().is_err());assert_eq!(fs::read(path).unwrap(),altered);
    }
    fn restored_selector(f:&mut Fixture,selected:&Selector,retained:bool)->Selector {
        let old_attempt=uuid::Uuid::new_v4().to_string();let old_generation=uuid::Uuid::new_v4().to_string();let mut state=f.j.load().clone();state.previous.push(super::super::journal::PreviousAttempt{attempt_id:old_attempt.clone(),outcome:super::super::Outcome::Restored,candidate:state.candidate.clone(),last_failure:None,generation:Some(old_generation.clone()),discard_id:retained.then_some(old_generation),ended_at:"2026-10-29T00:00:00Z".into()});files::atomic_json(&f.roots.app.join(".nwui-auto/journal-v1.json"),&state).unwrap();f.j=Journal::open(&f.roots.app,&f.locks).unwrap();
        let mut old=selected.clone();old.active_major=15;old.generation=None;old.manifest_sha256=None;old.backup_id=Some(old_attempt);old.system_identifier="123456".into();old.operator_reason=Some("restore-pg15".into());files::atomic_json(&f.roots.app.join(".nwui-auto/active-v1.json"),&old).unwrap();old
    }
    #[test]
    fn w1_selector_cas_accepts_restored15_after_discard_removal_or_history_pruning_and_binds_exact_bytes() {
        for case in ["valid","not-retained","wrong-source","unknown-attempt","pruned-after-nine"] {
            let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();let mut old=restored_selector(&mut f,&selected,case!="not-retained");if case=="wrong-source"{old.system_identifier="999".into();}if case=="unknown-attempt"{old.backup_id=Some(uuid::Uuid::new_v4().to_string());}if case=="pruned-after-nine"{let mut state=f.j.load().clone();state.previous.clear();for _ in 0..9{state.previous.insert(0,super::super::journal::PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:super::super::Outcome::RefusedPreflight,candidate:state.candidate.clone(),last_failure:Some(super::super::journal::PreviousFailure{step:Step::P2Preflight,code:"ledger_unknown".into()}),generation:None,discard_id:None,ended_at:"2026-10-29T00:00:00Z".into()});state.previous.truncate(8);}files::atomic_json(&f.roots.app.join(".nwui-auto/journal-v1.json"),&state).unwrap();f.j=Journal::open(&f.roots.app,&f.locks).unwrap();}let path=f.roots.app.join(".nwui-auto/active-v1.json");files::atomic_json(&path,&old).unwrap();let before=fs::read(&path).unwrap();
            if case!="wrong-source"{f.commit(&selected).unwrap();assert_eq!(selector::read_selector(&f.roots.app).unwrap(),selected);let intent=f.j.load().records.iter().rev().find(|r|r.step==Step::P5Commit && r.state==StepState::Intent).unwrap().data.as_ref().unwrap();assert_eq!(intent["replaces"]["selectorSha256"],selector::digest(&before));}
            else{assert!(f.commit(&selected).is_err(),"{case}");assert_eq!(fs::read(&path).unwrap(),before);assert!(!f.roots.app.join(".nwui/pair-v1.json").exists());}
        }
    }
    #[test]
    fn w1_selector_cas_real_kills_resume_or_refuse_tampered_intent_bytes() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["selector-intent","record-fsync:active-v1.json","record-rename:active-v1.json","selector-durable"] {for tamper in [false,true] {if tamper && point!="selector-intent"{continue;}
            let mut f=Fixture::new_branch(Branch::Upgrade);let selected=f.auto();restored_selector(&mut f,&selected,true);let Fixture{_temp,roots,locks,j,pair:_,key,generation:_}=f;drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::generation::tests::w1_generation_commit_kill_child","--nocapture"]).env("NWUI_COMMIT_TEST_ROOT",_temp.path()).env("NWUI_COMMIT_TEST_SELECTOR",serde_json::to_string(&selected).unwrap()).env("NWUI_GENERATION_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9));
            let locks=wait_locks(&roots);let mut j=Journal::open(&roots.app,&locks).unwrap();let path=roots.app.join(".nwui-auto/active-v1.json");if tamper{let mut raw=fs::read(&path).unwrap();raw.push(b' ');fs::write(&path,&raw).unwrap();assert!(commit(&roots,&locks,&mut j,&selected,&selected.system_identifier,&key).is_err());assert_eq!(fs::read(&path).unwrap(),raw);assert!(!roots.app.join(".nwui/pair-v1.json").exists());}else{commit(&roots,&locks,&mut j,&selected,&selected.system_identifier,&key).unwrap();assert_eq!(selector::read_selector(&roots.app).unwrap(),selected);assert!(done(&j,Step::P5Commit));}
        }}
    }

}
