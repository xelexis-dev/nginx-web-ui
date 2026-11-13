//! Offline operator commands; no AppState or canonical-key creation.
use super::{Journal, Outcome, Step, StepError, StepState, StorageLocks, journal::JournalV1, selector::Selector};
use crate::db_upgrade::layout::Roots;
use std::{fs::File, io, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind { Restore, RemoveBackup, RemoveDiscarded }
#[derive(Debug)]
struct Command { kind: Kind, id: String }
#[derive(Debug, PartialEq, Eq)]
enum Progress { New, Resume, Done }

fn refuse(code: &'static str, message: &'static str) -> StepError {
    StepError { code, message, detail: None }
}

fn parse(command: &str, args: &[String]) -> Result<Command, StepError> {
    let invalid = || refuse("invalid_arguments", "명령과 확인 인자를 확인하세요: restore-pg15 --confirm-loss <보관 ID>, remove-pg15-backup --confirm <보관 ID>, remove-discarded --confirm <보관 ID>.");
    let (kind, flag) = match command {
        "restore-pg15" => (Kind::Restore, "--confirm-loss"),
        "remove-pg15-backup" => (Kind::RemoveBackup, "--confirm"),
        "remove-discarded" => (Kind::RemoveDiscarded, "--confirm"),
        _ => return Err(invalid()),
    };
    let [given, id] = args else { return Err(invalid()); };
    if given != flag || !uuid::Uuid::parse_str(id).is_ok_and(|value| value.get_version() == Some(uuid::Version::Random) && value.hyphenated().to_string() == *id) {
        return Err(invalid());
    }
    Ok(Command { kind, id: id.clone() })
}

fn open(roots: &Roots) -> Result<(StorageLocks, Journal), StepError> {
    let locks = StorageLocks::acquire(&roots.db, &roots.app)?;
    let journal = Journal::open(&roots.app, &locks)?;
    Ok((locks, journal))
}

/// Runs before RuntimeConfig/AppState. All filesystem, key-probe and PG tool work is blocking.
pub(crate) async fn run(command:&str,args:&[String])->Result<String,StepError> {
    let command=parse(command,args)?;
    // A completed restore must still drain reserves even when image metadata is unavailable.
    let candidate=if command.kind==Kind::Restore {Some(super::journal::Candidate::current().await)}else{None};
    tokio::task::spawn_blocking(move||{
        let roots=crate::db_upgrade::storage::roots().map_err(|error|StepError{code:error.code,message:error.message,detail:None})?;
        execute(&roots,command,candidate)
    }).await.map_err(|_|refuse("boot_step_failed","관리 명령을 완료하지 못했습니다. 보관 기록을 확인하고 같은 명령을 다시 실행하세요."))?
}

fn target_identifier(path:&Path)->Result<String,StepError> {
    let control=crate::db_upgrade::postgres::read_control(17,path).map_err(|error|StepError{code:error.code,message:error.message,detail:None})?;
    // A+D excludes the production/private postmasters; an unclean control state is preserved.
    super::source::control(&control)?;
    Ok(control.system_identifier)
}

fn execute(roots:&Roots,command:Command,candidate:Option<Result<super::journal::Candidate,StepError>>)->Result<String,StepError> {
    let (locks,mut journal)=open(roots)?;
    match command.kind {
        Kind::RemoveBackup=>remove_backup(roots,&locks,&mut journal,&command.id),
        Kind::RemoveDiscarded=>remove_discarded_with(roots,&locks,&mut journal,&command.id,&target_identifier),
        Kind::Restore=>{
            let data=restore_data(&journal,&command.id)?;
            let owner=crate::db_upgrade::postgres::postgres_ids().map_err(|error|StepError{code:error.code,message:error.message,detail:None})?;
            if progress(journal.load(),Step::CmdRestorePg15,&data)?==Progress::Done {
                let candidate=journal.load().candidate.clone();
                return restore_command(roots,&locks,&mut journal,&command.id,&[],owner,&target_identifier,&candidate);
            }
            let candidate=candidate.ok_or_else(||refuse("verification_failed","이미지의 빌드 정보를 확인하지 못했습니다. 데이터 변경을 시작하지 않았습니다."))??;
            let raw=crate::nonroot::startup_value("DATA_KEY_FILE");
            if raw.as_deref()==Some("") {return Err(refuse("key_path_invalid","DATA_KEY_FILE을 비우지 말고 실제 키 파일 경로를 지정하세요."));}
            let source=super::KeySource::Canonical;let path=source.path(&roots.app,raw.as_deref().map(std::ffi::OsStr::new));
            if !path.is_absolute() {return Err(refuse("key_path_invalid","DATA_KEY_FILE에는 절대 경로를 지정하세요."));}
            let key=crate::db_upgrade::key_file::read_key_file(&path,crate::db_upgrade::key_file::policy_for(&source,&roots.app,&path)).map_err(|error|match error {
                crate::db_upgrade::key_file::KeyFileError::Missing=>refuse("key_missing","기존 암호화 키가 없습니다. 키를 만들지 않았습니다. 기존 키 파일을 확인하세요."),
                _=>refuse("key_handle_failed","기존 암호화 키를 검증하지 못했습니다. 파일과 권한을 확인하세요. 키를 바꾸지 않았습니다."),
            })?;
            restore_command(roots,&locks,&mut journal,&command.id,&key.bytes,owner,&target_identifier,&candidate)
        },
    }
}

fn restore_command(roots:&Roots,locks:&StorageLocks,journal:&mut Journal,id:&str,key_bytes:&[u8],owner:(u32,u32),read_id:&dyn Fn(&Path)->Result<String,StepError>,candidate:&super::journal::Candidate)->Result<String,StepError> {
    let data=restore_data(journal,id)?;
    if progress(journal.load(),Step::CmdRestorePg15,&data)?==Progress::Done {
        super::reserves::drain(roots,locks,journal)?;
        return Ok(format!("이미 복원했습니다: {id}"));
    }
    let bound=prepare_restore(roots,locks,journal,id,key_bytes,owner,read_id)?;
    journal.finish_restore_pg15(roots,locks,&bound)?;
    checkpoint("restore-done");
    super::reserves::drain(roots,locks,journal)?;
    checkpoint("restore-reserves-drained");
    let admission=super::retry::admit(journal.load(),candidate,None,true,chrono::Utc::now())?;
    Ok(super::retry::restored_next_action(&admission).into())
}

fn started_once(journal: &JournalV1, selected: &Selector) -> Result<(), StepError> {
    selected.validate()?;
    let proved = journal.outcome == Outcome::Pg17 && selected.active_major == 17 && selected.generation.as_ref().is_some_and(|generation| {
        journal.records.iter().any(|record| record.step == Step::Phase2Monitor && record.state == StepState::Done
            && record.data.as_ref().and_then(|data| data.get("generation")).and_then(|value| value.as_str()) == Some(generation.as_str()))
    });
    if !proved { return Err(refuse("verification_failed", "활성 PG17 세대의 정상 시작 기록이 없어 백업 삭제를 시작하지 않았습니다. 먼저 정상 시작 상태를 확인하세요.")); }
    Ok(())
}

fn progress(journal: &JournalV1, step: Step, data: &serde_json::Value) -> Result<Progress,StepError> {
    let others=journal.records.iter().filter(|record| !(record.step==step && record.data.as_ref()==Some(data))).cloned().collect::<Vec<_>>();
    if !super::journal::commands_settled(&others) { return Err(refuse("pair_mismatch","다른 관리 명령이 끝나지 않았습니다. 기록된 명령부터 다시 실행해 완료하세요.")); }
    let latest=journal.records.iter().rev().find(|record| record.step==step && record.data.as_ref()==Some(data) && matches!(record.state,StepState::Intent|StepState::Done));
    Ok(match latest.map(|record|&record.state) {
        Some(StepState::Done) => Progress::Done,
        Some(StepState::Intent) => Progress::Resume,
        _ => Progress::New,
    })
}

fn remove_backup(roots:&Roots, locks:&StorageLocks, journal:&mut Journal, id:&str) -> Result<String,StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !journal.belongs_to(&roots.app) || id!=journal.load().attempt_id {return Err(layout_error());}
    let data=serde_json::json!({"backupId":id});
    let state=progress(journal.load(),Step::CmdRemoveBackup,&data)?;
    if state==Progress::Done {return Ok(format!("이미 삭제했습니다: {id}"));}
    let (selected,selector_bytes):(super::selector::Selector,_)=super::selector::read_record(&roots.app.join(".nwui-auto/active-v1.json"))?;
    started_once(journal.load(),&selected)?;
    let activated=journal.load().records.iter().rev().find(|r|r.step==Step::S4Activate).is_some_and(|r|r.state==StepState::Done);
    let committed=journal.load().records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Intent)
        .and_then(|r|r.data.as_ref()).and_then(|v|v.get("generation")).and_then(|v|v.as_str())==selected.generation.as_deref()
        && journal.load().records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Intent).and_then(|r|r.data.as_ref()).and_then(|v|v.get("selectorSha256")).and_then(|v|v.as_str())==Some(super::selector::digest(&selector_bytes).as_str())
        && journal.load().records.iter().rev().find(|r|r.step==Step::P5Commit).is_some_and(|r|r.state==StepState::Done);
    if !activated || !committed || selected.backup_id.as_deref()!=Some(id) || selected.origin!=super::selector::Origin::Auto {return Err(refuse("pair_mismatch","활성 세대의 완료 기록과 백업 ID가 일치하지 않아 보존했습니다."));}
    let manifest=super::backup::open_verified_manifest(roots,journal)?;
    let bytes=manifest.summary.data_bytes;
    // A durable Intent authorizes the partially deleted tree; rehash only before that Intent.
    if state==Progress::New {
        super::backup::verify_backup(roots,journal)?;
        journal.record(Step::CmdRemoveBackup,StepState::Intent,Some(data.clone()))?;
    }
    checkpoint("remove-backup-intent");
    let parent=super::target::directory(&roots.db.join(".nwui-pg15-backup"))?;
    super::cleanup::delete_entry(&parent,id)?;
    checkpoint("remove-backup-deleted");
    journal.record(Step::CmdRemoveBackup,StepState::Done,Some(data))?;
    checkpoint("remove-backup-done");
    Ok(format!("PG15 백업 {id} 삭제 완료: {bytes} 바이트를 비웠습니다. 이 백업으로 PG15를 복원할 수 없습니다."))
}

fn remove_discarded_with(roots:&Roots,locks:&StorageLocks,journal:&mut Journal,id:&str,read_id:&dyn Fn(&Path)->Result<String,StepError>)->Result<String,StepError> {
    if !super::types::canonical_generation(id) || !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !journal.belongs_to(&roots.app) {return Err(layout_error());}
    let data=serde_json::json!({"discardId":id});let state=progress(journal.load(),Step::CmdRemoveDiscarded,&data)?;
    if state==Progress::Done {return Ok(format!("이미 삭제했습니다: {id}"));}
    let selected=super::selector::read_selector(&roots.app)?;
    retained_attempt(journal.load(),&selected,id)?;
    if selected.active_major==17 {
        started_once(journal.load(),&selected)?;
        if !journal.load().records.iter().rev().find(|r|r.step==Step::S4Activate).is_some_and(|r|r.state==StepState::Done) {return Err(layout_error());}
    }
    let discard_parent=super::target::directory(&roots.db.join(".nwui-discard"))?;
    let gen_parent=super::target::directory(&roots.app.join(".nwui-auto/gen"))?;
    let discard=roots.db.join(".nwui-discard").join(id);let generation=roots.app.join(".nwui-auto/gen").join(id);
    let mut bytes=0u64;
    for path in [&discard,&generation] {
        if exists(path)? {super::target::directory(path)?;bytes=bytes.checked_add(crate::db_upgrade::files::tree_info(path).map_err(super::backup_format::io_error)?.bytes).ok_or_else(layout_error)?;}
    }
    // The exact command Intent survives partial deletion of pg_control and manifest.json.
    if state==Progress::New {
        if exists(&generation)? {
            let (manifest,_):(super::selector::GenerationManifest,_)=super::selector::read_record(&generation.join("manifest.json"))?;
            manifest.validate()?;
            if manifest.generation!=id {return Err(layout_error());}
            if exists(&discard)? && read_id(&discard.join("17/data"))?!=manifest.system_identifier {return Err(layout_error());}
        }else if exists(&discard)? {return Err(layout_error());}
        journal.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(data))?;
    }
    checkpoint("remove-discarded-intent");
    super::cleanup::delete_entry(&discard_parent,id)?;checkpoint("remove-discarded-tree");
    super::cleanup::delete_entry(&gen_parent,id)?;checkpoint("remove-discarded-generation");
    journal.finish_remove_discarded(id)?;checkpoint("remove-discarded-done");
    Ok(format!("보관 PG17 {id}와 실행 파일 세대 삭제 완료: 이 실행에서 {bytes} 바이트를 비웠습니다."))
}

fn restore_data(journal:&Journal,id:&str)->Result<serde_json::Value,StepError> {
    if journal.load().branch!=super::Branch::Upgrade || id!=journal.load().attempt_id {return Err(layout_error());}
    let intent=journal.load().records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Intent).ok_or_else(layout_error)?;
    let generation=intent.data.as_ref().and_then(|v|v.get("generation")).and_then(|v|v.as_str()).filter(|g|super::types::canonical_generation(g)).ok_or_else(layout_error)?;
    if !journal.load().records.iter().any(|r|r.step==Step::P5Commit&&r.state==StepState::Done&&r.seq>intent.seq) {return Err(layout_error());}
    Ok(serde_json::json!({"backupId":id,"generation":generation,"discardId":generation}))
}

fn retry_db<T>(roots:&Roots,locks:&StorageLocks,journal:&mut Journal,mut action:impl FnMut(&mut Journal)->Result<T,StepError>)->Result<T,StepError> {
    match action(journal) {
        Err(error) if error.code=="insufficient_space"=>{
            if !super::reserves::release_one_db(roots,locks,journal)? {return Err(refuse("restore_incomplete","복원용 여유 공간을 모두 사용했습니다. 모든 사본을 보존했습니다. 공간을 확보하고 같은 명령을 다시 실행하세요."));}
            action(journal).map_err(|error|if error.code=="insufficient_space"{refuse("restore_incomplete","복원 공간이 부족합니다. 공간을 확보하고 같은 명령을 다시 실행하세요.")}else{error})
        }, result=>result,
    }
}

fn retry_app<T>(journal:&Journal,mut action:impl FnMut()->Result<T,StepError>)->Result<T,StepError> {
    match action() {
        Err(error) if error.code=="insufficient_space"=>{
            if !journal.release_app_reserve()? {return Err(refuse("restore_incomplete","복원 기록용 여유 공간을 모두 사용했습니다. 공간을 확보하고 같은 명령을 다시 실행하세요."));}
            action().map_err(|error|if error.code=="insufficient_space"{refuse("restore_incomplete","복원 기록 공간이 부족합니다. 공간을 확보하고 같은 명령을 다시 실행하세요.")}else{error})
        },result=>result,
    }
}

/// Stops before the owner's atomic Done/outcome commit; only that helper may finish restore.
fn prepare_restore(roots:&Roots,locks:&StorageLocks,journal:&mut Journal,id:&str,key_bytes:&[u8],owner:(u32,u32),read_id:&dyn Fn(&Path)->Result<String,StepError>)->Result<super::journal::RestorePg15Data,StepError> {
    use std::os::unix::fs::PermissionsExt;
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !journal.belongs_to(&roots.app) {return Err(layout_error());}
    let data=restore_data(journal,id)?;let state=progress(journal.load(),Step::CmdRestorePg15,&data)?;
    if state==Progress::Done || !matches!(journal.load().outcome,Outcome::Pg17|Outcome::ActivationFailed) {return Err(layout_error());}
    let g=data.get("generation").and_then(|v|v.as_str()).ok_or_else(layout_error)?;
    let manifest=super::backup::open_verified_manifest(roots,journal)?;let header=manifest.header.clone();let names=manifest.top_names.clone();
    let generation_root=roots.app.join(".nwui-auto/gen").join(g);
    let (generation,raw):(super::selector::GenerationManifest,_)=super::selector::read_record(&generation_root.join("manifest.json"))?;generation.validate()?;
    let selected=Selector{version:1,active_major:17,generation:Some(g.into()),system_identifier:generation.system_identifier.clone(),origin:generation.origin,backup_id:Some(id.into()),profile_id:generation.profile_id.clone(),profile_sha256:generation.profile_sha256.clone(),manifest_sha256:Some(super::selector::digest(&raw)),operator_reason:None};selected.validate()?;
    if selected.origin!=super::selector::Origin::Auto {return Err(layout_error());}
    let old_bytes=serde_json::to_vec(&selected).map_err(|_|layout_error())?;
    let bound=journal.load().records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Intent).and_then(|r|r.data.as_ref()).and_then(|v|v.get("selectorSha256")).and_then(|v|v.as_str());
    if bound!=Some(super::selector::digest(&old_bytes).as_str()) {return Err(layout_error());}
    let mut restored=selected.clone();restored.active_major=15;restored.generation=None;restored.system_identifier=header.system_identifier.clone();restored.manifest_sha256=None;restored.operator_reason=Some("restore-pg15".into());restored.validate()?;
    let bound_data=super::journal::RestorePg15Data::bind(id,g,g,&restored)?;
    let (current,current_bytes):(Selector,_)=super::selector::read_record(&roots.app.join(".nwui-auto/active-v1.json"))?;
    let restored_bytes=serde_json::to_vec(&restored).map_err(|_|layout_error())?;
    if !((current==selected&&current_bytes==old_bytes)||(state==Progress::Resume&&current==restored&&current_bytes==restored_bytes)) {return Err(layout_error());}
    let source=roots.db.join("17");let discarded=roots.db.join(".nwui-discard").join(g).join("17");
    if exists(&source)?==exists(&discarded)? {return Err(layout_error());}
    let target=if exists(&source)?{source.join("data")}else{discarded.join("data")};super::target::directory(&target)?;
    if read_id(&target)?!=selected.system_identifier {return Err(layout_error());}
    let selected_generation=super::selector::read_prepared(&roots.app,&roots.architecture,selected.clone(),&selected.system_identifier,key_bytes)?;
    if state==Progress::New {journal.record(Step::CmdRestorePg15,StepState::Intent,Some(data.clone()))?;}
    checkpoint("restore-intent");
    let l=match header.source_location {super::backup_format::SourceLocation::Root=>roots.db.clone(),super::backup_format::SourceLocation::Pgdata=>roots.db.join("pgdata")};
    super::source::require_idle(roots,locks,&l)?;
    let moving=journal.load().moves.iter().any(|m|m.record.phase==super::MovePhase::Restore);
    if !moving {
        super::backup::verify_backup(roots,journal)?;
        // A resume after the first live mutation validates the remaining pools instead of refilling them.
        if exists(&discarded)? {super::reserves::report(roots,locks,journal)?;} else {super::reserves::ensure(roots,locks,journal)?;}
        checkpoint("restore-reserves");
        retry_db(roots,locks,journal,|j|super::cleanup::delete_closed_set(roots,locks,j,&selected_generation))?;
        checkpoint("restore-p6");
    }
    retry_db(roots,locks,journal,|_|set_aside_with(roots,locks,g,&selected.system_identifier,owner,read_id))?;
    checkpoint("restore-target");
    let all_done=names.iter().all(|name|journal.load().moves.iter().rev().find(|m|m.record.phase==super::MovePhase::Restore&&m.record.entry==*name).is_some_and(|m|m.record.state==StepState::Done));
    if !all_done {
        let mut reader=super::backup::open_verified_manifest(roots,journal)?;super::backup::verify_restore_union(roots,journal,&mut reader)?;
        let (original,backup,linked)=retry_db(roots,locks,journal,|j|prepare_restore_dirs(roots,locks,j,&l,owner))?;
        let dirs=super::moves::MoveDirs{l:&original,bk:&backup,w_linked:&linked};
        for name in names.iter().filter(|name|name.as_str()!="PG_VERSION").chain(names.get("PG_VERSION")) {
            retry_db(roots,locks,journal,|j|super::restore::restore_backup_entry(locks,j,&dirs,name,&names))?;
            if name=="PG_VERSION" {checkpoint("restore-version-last");}else{checkpoint("restore-entry");}
        }
    }
    let original=super::target::directory(&l)?;
    retry_db(roots,locks,journal,|_|{
        if unsafe{libc::fchown(original.as_raw_fd(),header.root.uid,header.root.gid)}!=0 {return Err(super::backup_format::io_error(io::Error::last_os_error()));}
        original.set_permissions(std::fs::Permissions::from_mode(header.root.mode)).and_then(|_|original.sync_all()).map_err(super::backup_format::io_error)
    })?;
    super::backup::verify_restored(roots,journal)?;checkpoint("restore-verified");
    let backup_path=roots.db.join(".nwui-pg15-backup").join(id);
    if exists(&backup_path)? {
        super::target::directory(&backup_path)?;
        if std::fs::read_dir(&backup_path).map_err(super::backup_format::io_error)?.next().is_some() {return Err(layout_error());}
    }
    // Refuse foreign work before removing either marker or the empty backup directory.
    retry_db(roots,locks,journal,|j|super::cleanup::remove_work_dir(roots,locks,j))?;
    if exists(&roots.db.join(".nwui-auto-work").join(id))? {return Err(layout_error());}
    checkpoint("restore-work-removed");
    let guard=l.join("NWUI-RESTORE-IN-PROGRESS.txt");let upgraded=l.join("UPGRADED-TO-PG17.txt");
    if exists(&guard)? {retry_db(roots,locks,journal,|j|super::restore::restore_marker_for_step(j,Step::CmdRestorePg15,&original,&l,id))?;}
    if exists(&upgraded)? {super::guard::verify_marker(&original,id,g)?;}
    retry_db(roots,locks,journal,|_|{
        for name in [c"NWUI-RESTORE-IN-PROGRESS.txt",c"UPGRADED-TO-PG17.txt"] {
            if unsafe{libc::unlinkat(original.as_raw_fd(),name.as_ptr(),0)}!=0 && io::Error::last_os_error().kind()!=io::ErrorKind::NotFound {return Err(super::backup_format::io_error(io::Error::last_os_error()));}
            if name==c"NWUI-RESTORE-IN-PROGRESS.txt" {checkpoint("restore-guard-unlinked");}
        }
        original.sync_all().map_err(super::backup_format::io_error)
    })?;checkpoint("restore-markers-removed");
    let backup_parent=super::target::directory(&roots.db.join(".nwui-pg15-backup"))?;let name=std::ffi::CString::new(id).map_err(|_|layout_error())?;
    retry_db(roots,locks,journal,|_|{
        if unsafe{libc::unlinkat(backup_parent.as_raw_fd(),name.as_ptr(),libc::AT_REMOVEDIR)}!=0 && io::Error::last_os_error().kind()!=io::ErrorKind::NotFound {return Err(super::backup_format::io_error(io::Error::last_os_error()));}
        backup_parent.sync_all().map_err(super::backup_format::io_error)
    })?;checkpoint("restore-backup-removed");
    if current==selected {retry_app(journal,||crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/active-v1.json"),&restored).map_err(super::backup_format::io_error))?;}
    checkpoint("restore-selector15");
    let pair_id=match &selected_generation.key_record.key_binding {super::KeyBinding::Verified{pair_id}=>pair_id,_=>return Err(layout_error())};
    retry_app(journal,||remove_auto_pair(roots,locks,&restored,&selected_generation.app_pair,pair_id))?;
    checkpoint("restore-companion");
    Ok(bound_data)
}

fn retained_attempt<'a>(journal: &'a JournalV1, selected: &Selector, id: &str) -> Result<&'a str,StepError> {
    let mismatch=||refuse("pair_mismatch","이 ID를 보관한 복원 기록과 선택 기록이 일치하지 않습니다. 보관 사본을 보존했습니다.");
    selected.validate()?;
    if !super::types::canonical_generation(id) || selected.generation.as_deref()==Some(id) { return Err(mismatch()); }
    let (generation,discard)=super::rollover::retention(journal)?;
    let mut owners=Vec::new();
    if discard.as_deref()==Some(id) {
        if generation.as_deref()!=Some(id) { return Err(mismatch()); }
        owners.push(journal.attempt_id.as_str());
    }
    for previous in journal.previous.iter().filter(|previous|previous.discard_id.as_deref()==Some(id)) {
        if previous.generation.as_deref()!=Some(id) { return Err(mismatch()); }
        owners.push(previous.attempt_id.as_str());
    }
    if owners.len()!=1 || (selected.active_major==15 && selected.backup_id.as_deref()!=Some(owners[0])) { return Err(mismatch()); }
    Ok(owners[0])
}

fn layout_error() -> StepError {
    refuse("unsupported_layout", "PG17 원본과 보관 위치를 확인하지 못했습니다. 모든 사본을 보존했습니다. 같은 명령을 다시 실행하기 전에 위치를 확인하세요.")
}
fn exists(path: &Path) -> Result<bool, StepError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(super::backup_format::io_error(error)),
    }
}

fn owned_directory(parent: &File, name: &std::ffi::CStr, owner: (u32,u32), point: &str) -> Result<File, StepError> {
    let created = if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } == 0 { true }
        else if io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) { false }
        else { return Err(super::backup_format::io_error(io::Error::last_os_error())); };
    if created { checkpoint(&format!("{point}-created")); }
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC) };
    if fd < 0 { return Err(super::backup_format::io_error(io::Error::last_os_error())); }
    let dir = unsafe { File::from_raw_fd(fd) };
    if created && (unsafe { libc::fchown(fd, owner.0, owner.1) } != 0 || unsafe { libc::fchmod(fd, 0o700) } != 0) {
        return Err(super::backup_format::io_error(io::Error::last_os_error()));
    }
    let meta = dir.metadata().map_err(super::backup_format::io_error)?;
    // A.18(c) also admits root-owned D artifacts after a crash before chown.
    if !(meta.uid()==0 || (meta.uid()==owner.0 && meta.gid()==owner.1)) || meta.mode()&0o7777!=0o700 || meta.dev()!=parent.metadata().map_err(super::backup_format::io_error)?.dev() { return Err(layout_error()); }
    dir.sync_all().and_then(|_|parent.sync_all()).map_err(super::backup_format::io_error)?;
    checkpoint(&format!("{point}-durable"));
    Ok(dir)
}

// This whole-cluster operator move is deliberately outside A.16's closed per-entry table.
fn set_aside_with(roots: &Roots, locks: &StorageLocks, generation: &str, identifier: &str, owner: (u32,u32), read_id: &dyn Fn(&Path)->Result<String,StepError>) -> Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !super::types::canonical_generation(generation) { return Err(layout_error()); }
    let db = super::target::directory(&roots.db)?;
    let source = roots.db.join("17");
    let parent = roots.db.join(".nwui-discard");
    if exists(&parent)? { super::target::directory(&parent)?; }
    let discarded = parent.join(generation);
    if exists(&discarded)? { super::target::directory(&discarded)?; }
    let target = discarded.join("17");
    let placement = (exists(&source)?,exists(&target)?);
    if placement.0 == placement.1 { return Err(layout_error()); }
    let current = if placement.0 { &source } else { &target };
    let device = db.metadata().map_err(super::backup_format::io_error)?.dev();
    let data = current.join("data");
    for dir in [current.as_path(),data.as_path()] {
        if super::target::directory(&dir)?.metadata().map_err(super::backup_format::io_error)?.dev()!=device { return Err(layout_error()); }
    }
    if read_id(&current.join("data"))? != identifier { return Err(layout_error()); }
    let parent = owned_directory(&db,c".nwui-discard",owner,"discard-parent")?;
    let name = std::ffi::CString::new(generation).map_err(|_|layout_error())?;
    let destination = owned_directory(&parent,&name,owner,"discard-generation")?;
    if placement.0 {
        if exists(&target)? { return Err(layout_error()); }
        if unsafe { libc::renameat(db.as_raw_fd(), c"17".as_ptr(), destination.as_raw_fd(), c"17".as_ptr()) } != 0 {
            return Err(super::backup_format::io_error(io::Error::last_os_error()));
        }
        checkpoint("discard-rename");
    }
    db.sync_all().map_err(super::backup_format::io_error)?; checkpoint("discard-source-fsync");
    destination.sync_all().map_err(super::backup_format::io_error)?; checkpoint("discard-destination-fsync");
    Ok(())
}

fn prepare_restore_dirs(roots: &Roots, locks: &StorageLocks, journal: &mut Journal, original: &Path, owner: (u32,u32)) -> Result<(File,File,File),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !journal.belongs_to(&roots.app) { return Err(layout_error()); }
    let id=journal.load().attempt_id.clone();
    if !super::types::canonical_generation(&id) { return Err(layout_error()); }
    if original!=roots.db.as_path() && original!=roots.db.join("pgdata").as_path() { return Err(layout_error()); }
    let l=super::target::directory(original)?;
    // The command variant reuses its full durable Intent; it adds no narrower Cmd record.
    super::restore::restore_marker_for_step(journal,Step::CmdRestorePg15,&l,original,&id)?;
    checkpoint("restore-guard-durable");
    let db=super::target::directory(&roots.db)?;
    let parent=owned_directory(&db,c".nwui-auto-work",owner,"restore-work-parent")?;
    let name=std::ffi::CString::new(id.as_str()).map_err(|_|layout_error())?;
    let work=owned_directory(&parent,&name,owner,"restore-work")?;
    let linked=owned_directory(&work,c"linked",owner,"restore-linked")?;
    let linked_path=roots.db.join(".nwui-auto-work").join(&id).join("linked");
    if std::fs::read_dir(linked_path).map_err(super::backup_format::io_error)?.next().is_some() { return Err(layout_error()); }
    let backup=super::target::directory(&roots.db.join(".nwui-pg15-backup").join(id))?;
    let device=l.metadata().map_err(super::backup_format::io_error)?.dev();
    if backup.metadata().map_err(super::backup_format::io_error)?.dev()!=device || linked.metadata().map_err(super::backup_format::io_error)?.dev()!=device { return Err(layout_error()); }
    Ok((l,backup,linked))
}

/// expected is the immutable generation snapshot already validated with its manifest/key binding.
fn remove_auto_pair(roots: &Roots, locks: &StorageLocks, restored: &Selector, expected: &crate::db_upgrade::pair::Pair, pair_id: &str) -> Result<(),StepError> {
    let mismatch=||refuse("pair_mismatch","복원된 선택 기록과 라이브 pair가 일치하지 않습니다. 기존 내용을 보존했습니다. 복원 명령을 다시 확인하세요.");
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) { return Err(mismatch()); }
    restored.validate()?;
    expected.validate().map_err(|_|mismatch())?;
    let (actual,bytes):(Selector,_) = super::selector::read_record(&roots.app.join(".nwui-auto/active-v1.json"))?;
    if restored.active_major!=15 || restored.origin!=super::selector::Origin::Auto || !expected.ready || expected.pair_id!=pair_id || actual!=*restored || bytes!=serde_json::to_vec(restored).map_err(|_|mismatch())? { return Err(mismatch()); }
    let app=super::target::directory(&roots.app)?;
    let path=roots.app.join(".nwui");
    if !exists(&path)? { return app.sync_all().map_err(super::backup_format::io_error); }
    let directory=super::target::directory(&path)?;
    let meta=directory.metadata().map_err(super::backup_format::io_error)?;
    if meta.uid()!=0 || meta.mode()&0o7777!=0o700 { return Err(mismatch()); }
    for entry in std::fs::read_dir(&path).map_err(super::backup_format::io_error)? {
        if entry.map_err(super::backup_format::io_error)?.file_name()!=std::ffi::OsStr::new("pair-v1.json") { return Err(mismatch()); }
    }
    if exists(&path.join("pair-v1.json"))? {
        let (live,_):(crate::db_upgrade::pair::Pair,_) = super::selector::read_record(&path.join("pair-v1.json"))?;
        super::selector::same_pair_except_flag(expected,&live)?;
        checkpoint("pair-before-unlink");
        if unsafe { libc::unlinkat(directory.as_raw_fd(),c"pair-v1.json".as_ptr(),0) } != 0 { return Err(super::backup_format::io_error(io::Error::last_os_error())); }
        checkpoint("pair-unlink");
    }
    directory.sync_all().map_err(super::backup_format::io_error)?; checkpoint("pair-directory-fsync");
    if unsafe { libc::unlinkat(app.as_raw_fd(),c".nwui".as_ptr(),libc::AT_REMOVEDIR) } != 0 { return Err(super::backup_format::io_error(io::Error::last_os_error())); }
    checkpoint("pair-rmdir");
    app.sync_all().map_err(super::backup_format::io_error)?; checkpoint("pair-parent-fsync");
    Ok(())
}

#[cfg(test)]
fn checkpoint(point: &str) {
    if std::env::var("NWUI_OPERATOR_KILL_POINT").as_deref()==Ok(point) {
        static MATCHES:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
        let occurrence=std::env::var("NWUI_OPERATOR_KILL_OCCURRENCE").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(1);
        if MATCHES.fetch_add(1,std::sync::atomic::Ordering::Relaxed)+1==occurrence {unsafe { libc::kill(libc::getpid(),libc::SIGKILL); }}
    }
}
#[cfg(not(test))]
fn checkpoint(_point: &str) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::{Branch, Journal, Outcome, Step, StepState, StorageLocks, journal::Candidate, selector::{Origin, Selector}};
    use crate::db_upgrade::layout::Roots;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fixture() -> (tempfile::TempDir, Roots, StorageLocks, Journal, Selector) {
        let temp = tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let roots = Roots { db: temp.path().join("db"), app: temp.path().join("app"), architecture: "arm64".into() };
        for path in [&roots.db, &roots.app] {
            fs::create_dir(path).unwrap(); fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let locks = StorageLocks::acquire(&roots.db, &roots.app).unwrap();
        let journal = Journal::begin(&roots.app, &locks, Branch::Upgrade, Candidate { revision: "0".repeat(40), pins_sha256: "1".repeat(64), migrations_sha256: "2".repeat(64) }).unwrap();
        journal.ensure_app_pool().unwrap();
        let selected = Selector { version: 1, active_major: 17, generation: Some(uuid::Uuid::new_v4().to_string()), system_identifier: "123456".into(), origin: Origin::Auto, backup_id: Some(journal.load().attempt_id.clone()), profile_id: Some("legacy".into()), profile_sha256: Some("3".repeat(64)), manifest_sha256: Some("4".repeat(64)), operator_reason: None };
        (temp, roots, locks, journal, selected)
    }

    fn backup_fixture() -> (tempfile::TempDir, Roots, StorageLocks, Journal, Selector) { backup_fixture_started(true) }
    fn backup_fixture_started(started:bool) -> (tempfile::TempDir, Roots, StorageLocks, Journal, Selector) {
        let (t,roots,locks,mut j,selected)=fixture();
        let original=roots.db.join("pgdata");fs::create_dir(&original).unwrap();fs::set_permissions(&original,fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(original.join("global")).unwrap();fs::write(original.join("global/pg_control"),"cold control").unwrap();fs::write(original.join("PG_VERSION"),"15\n").unwrap();
        fs::write(original.join("zeta"),"last non-version entry").unwrap();
        let fields=crate::db_upgrade::control::COMPATIBILITY_FIELDS.iter().map(|f|format!("{f}: 1\n")).collect::<String>();
        let control=crate::db_upgrade::control::Control::parse(&format!("Database system identifier: 123456\nDatabase cluster state: shut down\n{fields}"),"").unwrap();
        let names=std::collections::BTreeSet::from(["PG_VERSION".into(),"global".into(),"zeta".into()]);
        let prepared=super::super::backup::prepare(&roots,&locks,&j.load().attempt_id,super::super::backup_format::SourceLocation::Pgdata,&names,&control,"2026-10-29T00:00:00Z").unwrap();
        let parent=roots.db.join(".nwui-pg15-backup");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();
        let bk=parent.join(&j.load().attempt_id);fs::create_dir(&bk).unwrap();fs::set_permissions(&bk,fs::Permissions::from_mode(0o700)).unwrap();
        for name in &names {crate::db_upgrade::files::copy_tree(&original.join(name),&bk.join(name)).unwrap();}
        j.record(Step::P1Backup,StepState::Intent,Some(serde_json::json!({"header":prepared.header}))).unwrap();
        super::super::backup::publish_verified(&roots,&locks,&mut j,&prepared).unwrap();
        j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":selected.generation,"selectorSha256":super::super::selector::digest(&serde_json::to_vec(&selected).unwrap())}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
        j.record(Step::S4Activate,StepState::Done,None).unwrap();
        if started {j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"generation":selected.generation,"bootIdentity":"old-boot","observedAt":"2026-10-29T00:00:00Z"}))).unwrap();}j.set_outcome(Outcome::Pg17).unwrap();
        let path=roots.app.join(".nwui-auto/active-v1.json");fs::write(&path,serde_json::to_vec(&selected).unwrap()).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
        (t,roots,locks,j,selected)
    }

    #[test]
    fn backup_removal_uses_manifest_authority_and_preserves_original_and_other_backups() {
        let (_t,roots,locks,mut j,_)=backup_fixture();let id=j.load().attempt_id.clone();let parent=roots.db.join(".nwui-pg15-backup");
        let other=parent.join(uuid::Uuid::new_v4().to_string());fs::create_dir(&other).unwrap();fs::write(other.join("keep"),"foreign").unwrap();
        assert!(remove_backup(&roots,&locks,&mut j,&id).unwrap().contains(&id));assert!(!parent.join(&id).exists());assert_eq!(fs::read(other.join("keep")).unwrap(),b"foreign");assert_eq!(fs::read(roots.db.join("pgdata/PG_VERSION")).unwrap(),b"15\n");
        let sequence=j.load().records.len();assert!(remove_backup(&roots,&locks,&mut j,&id).unwrap().contains("이미 삭제했습니다"));assert_eq!(j.load().records.len(),sequence);
    }

    #[test]
    fn backup_removal_kill_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_BACKUP_FIXTURE") else {return;};let root=std::path::PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let (locks,mut journal)=open(&roots).unwrap();let id=journal.load().attempt_id.clone();
        remove_backup(&roots,&locks,&mut journal,&id).unwrap();
    }

    #[test]
    fn backup_removal_real_kills_resume_intent_deletion_and_done() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["remove-backup-intent","remove-backup-deleted","remove-backup-done"] {
            let (t,roots,locks,j,_)=backup_fixture();let id=j.load().attempt_id.clone();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::operator::tests::backup_removal_kill_child","--nocapture"])
                .env_clear().env("NWUI_OPERATOR_BACKUP_FIXTURE",t.path()).env("NWUI_OPERATOR_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9),"{point}");
            let (locks,mut j)=open(&roots).unwrap();remove_backup(&roots,&locks,&mut j,&id).unwrap();assert!(!roots.db.join(".nwui-pg15-backup").join(&id).exists());
            assert_eq!(fs::read(roots.db.join("pgdata/PG_VERSION")).unwrap(),b"15\n");
            assert_eq!(j.load().records.iter().filter(|r|r.step==Step::CmdRemoveBackup&&r.state==StepState::Done).count(),1);
        }
    }

    #[test]
    fn backup_removal_refuses_wrong_id_missing_activation_and_foreign_content_before_intent() {
        for change in ["id","activation","content","selector-bytes"] {
            let (_t,roots,locks,mut j,selected)=backup_fixture();let mut id=j.load().attempt_id.clone();let bk=roots.db.join(".nwui-pg15-backup").join(&id);
            match change {"id"=>id=uuid::Uuid::new_v4().to_string(),"activation"=>{j.record(Step::S4Activate,StepState::Failed{code:"activation_timeout".into()},None).unwrap();},"selector-bytes"=>{let mut bytes=serde_json::to_vec(&selected).unwrap();bytes.push(b'\n');fs::write(roots.app.join(".nwui-auto/active-v1.json"),bytes).unwrap();},_=>fs::write(bk.join("foreign"),"keep").unwrap()}
            assert!(remove_backup(&roots,&locks,&mut j,&id).is_err());assert!(bk.exists());assert!(!j.load().records.iter().any(|r|r.step==Step::CmdRemoveBackup));
        }
    }

    fn discarded_fixture() -> (tempfile::TempDir,Roots,StorageLocks,Journal,Selector,String) {
        let (t,roots,locks,mut j,mut selected)=fixture();let g=selected.generation.take().unwrap();
        j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
        let data=serde_json::json!({"backupId":j.load().attempt_id,"generation":g,"discardId":g});j.record(Step::CmdRestorePg15,StepState::Intent,Some(data.clone())).unwrap();j.record(Step::CmdRestorePg15,StepState::Done,Some(data)).unwrap();
        selected.active_major=15;selected.manifest_sha256=None;selected.operator_reason=Some("restore-pg15".into());
        let path=roots.app.join(".nwui-auto/active-v1.json");fs::write(&path,serde_json::to_vec(&selected).unwrap()).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
        let generation_root=roots.app.join(".nwui-auto/gen").join(&g);let discard=roots.db.join(".nwui-discard").join(&g);
        for path in [generation_root.parent().unwrap(),&generation_root,discard.parent().unwrap(),&discard,&discard.join("17"),&discard.join("17/data")] {fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        fs::write(discard.join("17/data/canary"),"old target").unwrap();
        let manifest=super::super::selector::GenerationManifest{version:1,generation:g.clone(),system_identifier:"123456".into(),origin:Origin::Auto,profile_id:Some("legacy".into()),profile_sha256:Some("3".repeat(64)),db_pair_sha256:"4".repeat(64),app_pair_sha256:"5".repeat(64),key_binding_sha256:"6".repeat(64),created_at:"2026-10-29T00:00:00Z".into()};
        let path=generation_root.join("manifest.json");fs::write(&path,serde_json::to_vec(&manifest).unwrap()).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
        (t,roots,locks,j,selected,g)
    }

    #[test]
    fn discarded_generation_deletion_accepts_the_post_p6_missing_staged_key() {
        for removed in [false,true] {
            let (_t,roots,locks,mut j,_selected,g)=discarded_fixture();let generation=roots.app.join(".nwui-auto/gen").join(&g);
            let key=generation.join("data_key");fs::write(&key,b"staged key").unwrap();fs::set_permissions(&key,fs::Permissions::from_mode(0o600)).unwrap();if removed {fs::remove_file(&key).unwrap();}
            let manifest=fs::read(generation.join("manifest.json")).unwrap();assert!(!manifest.is_empty());
            let canonical=roots.app.join("secrets/data_key");fs::create_dir(canonical.parent().unwrap()).unwrap();fs::write(&canonical,b"canonical stays").unwrap();
            remove_discarded_with(&roots,&locks,&mut j,&g,&|_|Ok("123456".into())).unwrap();
            assert!(!generation.exists());assert!(!roots.db.join(".nwui-discard").join(&g).exists());assert_eq!(fs::read(canonical).unwrap(),b"canonical stays");
            assert_eq!(super::super::rollover::retention(j.load()).unwrap(),(None,None));
        }
    }

    #[test]
    fn discarded_removal_under_restored15_deletes_only_named_roots_then_settles_atomically() {
        let (_t,roots,locks,mut j,_,g)=discarded_fixture();let other=roots.db.join(".nwui-discard").join(uuid::Uuid::new_v4().to_string());fs::create_dir(&other).unwrap();fs::write(other.join("keep"),"foreign").unwrap();
        let text=remove_discarded_with(&roots,&locks,&mut j,&g,&|_|Ok("123456".into())).unwrap();assert!(text.contains(&g));assert!(text.contains("바이트"));
        assert!(!roots.db.join(".nwui-discard").join(&g).exists());assert!(!roots.app.join(".nwui-auto/gen").join(&g).exists());assert_eq!(fs::read(other.join("keep")).unwrap(),b"foreign");
        assert_eq!(super::super::rollover::retention(j.load()).unwrap(),(None,None));
        assert!(remove_discarded_with(&roots,&locks,&mut j,&g,&|_|panic!("Done never revalidates deleted control")).unwrap().contains("이미 삭제했습니다"));
    }

    #[test]
    fn discarded_removal_refuses_identity_wrong_id_and_symlink_before_intent() {
        use std::os::unix::fs::symlink;
        for bad in ["identity","id","symlink"] {let (t,roots,locks,mut j,_,g)=discarded_fixture();let mut requested=g.clone();
            if bad=="id" {requested=uuid::Uuid::new_v4().to_string();}if bad=="symlink" {let d=roots.db.join(".nwui-discard").join(&g);fs::rename(&d,t.path().join("foreign")).unwrap();symlink(t.path().join("foreign"),&d).unwrap();}
            let id=if bad=="identity"{"999999"}else{"123456"};assert!(remove_discarded_with(&roots,&locks,&mut j,&requested,&|_|Ok(id.into())).is_err());
            assert!(roots.app.join(".nwui-auto/gen").join(&g).exists());assert!(!j.load().records.iter().any(|r|r.step==Step::CmdRemoveDiscarded));
        }
    }

    #[test]
    fn discarded_removal_kill_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_DISCARDED_FIXTURE")else{return;};let root=std::path::PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let (locks,mut j)=open(&roots).unwrap();
        let g=std::env::var("NWUI_OPERATOR_DISCARDED_ID").unwrap();remove_discarded_with(&roots,&locks,&mut j,&g,&|_|Ok("123456".into())).unwrap();
    }

    #[test]
    fn discarded_removal_real_kills_resume_both_deletions_before_atomic_done() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["remove-discarded-intent","remove-discarded-tree","remove-discarded-generation","remove-discarded-done"] {
            let (t,roots,locks,j,_,g)=discarded_fixture();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::operator::tests::discarded_removal_kill_child","--nocapture"])
                .env_clear().env("NWUI_OPERATOR_DISCARDED_FIXTURE",t.path()).env("NWUI_OPERATOR_DISCARDED_ID",&g).env("NWUI_OPERATOR_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9),"{point}");
            let (locks,mut j)=open(&roots).unwrap();remove_discarded_with(&roots,&locks,&mut j,&g,&|_|Ok("123456".into())).unwrap();
            assert!(!roots.db.join(".nwui-discard").join(&g).exists());assert!(!roots.app.join(".nwui-auto/gen").join(&g).exists());assert_eq!(super::super::rollover::retention(j.load()).unwrap(),(None,None));
            assert_eq!(j.load().records.iter().filter(|r|r.step==Step::CmdRemoveDiscarded&&r.state==StepState::Done).count(),1);
        }
    }

    fn restore_fixture() -> (tempfile::TempDir,Roots,StorageLocks,Journal,Selector,Vec<u8>) {
        let (t,roots,locks,mut j,mut selected)=backup_fixture_started(false);selected.system_identifier="654321".into();let g=selected.generation.as_ref().unwrap();
        let root=roots.app.join(".nwui-auto/gen").join(g);fs::create_dir(root.parent().unwrap()).unwrap();fs::create_dir(&root).unwrap();
        for path in [root.parent().unwrap(),&root] {fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        let write=|path:&Path,value:&serde_json::Value| {let bytes=serde_json::to_vec(value).unwrap();fs::write(path,&bytes).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();bytes};
        let mut pair=crate::db_upgrade::pair::Pair::fresh("arm64");pair.ready=true;pair.fresh_step=None;pair.mode="migration".into();pair.system_identifier=Some(selected.system_identifier.clone());
        let db=write(&root.join("db-pair.json"),&serde_json::to_value(&pair).unwrap());let app=write(&root.join("app-pair.json"),&serde_json::to_value(&pair).unwrap());let key=b"01".repeat(32);
        let record=super::super::selector::KeyRecord{version:1,key_binding:super::super::KeyBinding::Verified{pair_id:pair.pair_id.clone()},key_source:super::super::selector::StoredKeySource::Canonical,key_sha256:super::super::selector::digest(&key)};
        let binding=write(&root.join("key-binding.json"),&serde_json::to_value(record).unwrap());
        let manifest=super::super::selector::GenerationManifest{version:1,generation:g.clone(),system_identifier:selected.system_identifier.clone(),origin:Origin::Auto,profile_id:selected.profile_id.clone(),profile_sha256:selected.profile_sha256.clone(),db_pair_sha256:super::super::selector::digest(&db),app_pair_sha256:super::super::selector::digest(&app),key_binding_sha256:super::super::selector::digest(&binding),created_at:"2026-10-29T00:00:00Z".into()};
        let manifest_bytes=write(&root.join("manifest.json"),&serde_json::to_value(manifest).unwrap());selected.manifest_sha256=Some(super::super::selector::digest(&manifest_bytes));
        write(&roots.app.join(".nwui-auto/active-v1.json"),&serde_json::to_value(&selected).unwrap());
        j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g,"selectorSha256":super::super::selector::digest(&serde_json::to_vec(&selected).unwrap())}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
        let live=roots.app.join(".nwui");fs::create_dir(&live).unwrap();fs::set_permissions(&live,fs::Permissions::from_mode(0o700)).unwrap();write(&live.join("pair-v1.json"),&serde_json::to_value(pair).unwrap());
        target(&roots);
        let l=roots.db.join("pgdata");fs::rename(l.join("PG_VERSION"),l.join("PG_VERSION.nwui-pre17")).unwrap();
        let marker=format!("NWUI: PostgreSQL 17 upgrade. Do not start an older image on these volumes.\nBackup: {}\nGeneration: {}\nStop the container and follow the documented PG15 restore procedure. Keep both volumes and the backup.\n",j.load().attempt_id,g);
        let path=l.join("UPGRADED-TO-PG17.txt");fs::write(&path,marker).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o600)).unwrap();
        (t,roots,locks,j,selected,key)
    }

    #[test]
    fn restore_preparation_reinstates_verified_pg15_and_preserves_pg17_without_writing_done() {
        let (_t,roots,locks,mut j,selected,key)=restore_fixture();let id=j.load().attempt_id.clone();let g=selected.generation.unwrap();
        assert!(!j.load().records.iter().any(|r|r.step==Step::Phase2Monitor));let data=prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into())).unwrap();
        assert_eq!(serde_json::to_value(&data).unwrap(),serde_json::json!({"backupId":id,"generation":g,"discardId":g}));
        assert_eq!(super::super::selector::read_selector(&roots.app).unwrap().active_major,15);assert_eq!(super::super::selector::read_selector(&roots.app).unwrap().system_identifier,"123456");assert!(!roots.app.join(".nwui").exists());
        assert!(!roots.db.join("17").exists());assert_eq!(fs::read(roots.db.join(".nwui-discard").join(&g).join("17/data/canary")).unwrap(),b"keep PG17");
        assert!(!roots.db.join(".nwui-pg15-backup").join(&id).exists());assert!(!roots.db.join(".nwui-auto-work").join(&id).exists());
        super::super::backup::verify_restored(&roots,&j).unwrap();assert_eq!(fs::read(roots.db.join("pgdata/PG_VERSION")).unwrap(),b"15\n");
        assert_eq!(j.load().outcome,Outcome::Pg17);assert_eq!(j.load().records.iter().filter(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done).count(),0);
        assert_eq!(serde_json::to_value(prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into())).unwrap()).unwrap(),serde_json::to_value(&data).unwrap());
        let work=j.load().records.iter().filter(|r|r.step==Step::P6Cleanup&&r.data.as_ref().is_some_and(|d|d.get("workDir").is_some())).collect::<Vec<_>>();assert_eq!(work.len(),2);assert!(work.iter().all(|r|r.data.as_ref().unwrap().get("entry").is_none()));
    }

    #[test]
    fn restore_preparation_rejects_selector_control_key_and_backup_mismatch_before_live_mutation() {
        for bad in ["selector","control","key","backup"] {let (_t,roots,locks,mut j,selected,mut key)=restore_fixture();let id=j.load().attempt_id.clone();
            if bad=="selector" {let mut bytes=serde_json::to_vec(&selected).unwrap();bytes.push(b'\n');fs::write(roots.app.join(".nwui-auto/active-v1.json"),bytes).unwrap();}
            if bad=="key" {key[0]=b'2';}if bad=="backup" {fs::write(roots.db.join(".nwui-pg15-backup").join(&id).join("global/pg_control"),"corrupt").unwrap();}
            let original=crate::db_upgrade::files::snapshot(&roots.db.join("pgdata")).unwrap();let id_value=if bad=="control"{"999999"}else{"654321"};
            assert!(prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok(id_value.into())).is_err());
            assert_eq!(crate::db_upgrade::files::snapshot(&roots.db.join("pgdata")).unwrap(),original);assert!(roots.db.join("17/data/canary").exists());
        }
    }

    #[test]
    fn restore_after_normal_p6_work_done_removes_its_own_recreated_linked_directory() {
        let (_t,roots,locks,mut j,selected,key)=restore_fixture();let id=j.load().attempt_id.clone();
        let prepared=super::super::selector::read_prepared(&roots.app,&roots.architecture,selected.clone(),&selected.system_identifier,&key).unwrap();
        super::super::cleanup::delete_closed_set(&roots,&locks,&mut j,&prepared).unwrap();
        let work=roots.db.join(".nwui-auto-work").join(&id);fs::create_dir(work.parent().unwrap()).unwrap();fs::set_permissions(work.parent().unwrap(),fs::Permissions::from_mode(0o700)).unwrap();fs::create_dir(&work).unwrap();fs::set_permissions(&work,fs::Permissions::from_mode(0o700)).unwrap();
        super::super::cleanup::remove_work_dir(&roots,&locks,&mut j).unwrap();assert!(!work.exists());
        let bound=prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into())).unwrap();assert!(!work.exists());
        j.finish_restore_pg15(&roots,&locks,&bound).unwrap();
        assert_eq!(j.load().outcome,Outcome::Restored);assert_eq!(j.load().handoff,super::super::journal::Handoff::None);
    }

    #[test]
    fn restore_activation_failed_without_started_once_proof_is_allowed_and_foreign_work_is_preserved() {
        for foreign in [false,true] {let (_t,roots,locks,mut j,_selected,key)=restore_fixture();let id=j.load().attempt_id.clone();
            j.record(Step::S4Activate,StepState::Failed{code:"activation_timeout".into()},None).unwrap();j.set_outcome(Outcome::ActivationFailed).unwrap();
            let work=roots.db.join(".nwui-auto-work").join(&id);
            if foreign {fs::create_dir_all(&work).unwrap();for p in [work.parent().unwrap(),&work] {fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}fs::write(work.join("foreign"),"keep").unwrap();}
            let result=prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into()));
            if foreign {assert!(result.is_err());assert_eq!(fs::read(work.join("foreign")).unwrap(),b"keep");assert!(roots.db.join("pgdata/NWUI-RESTORE-IN-PROGRESS.txt").exists());assert_eq!(super::super::selector::read_selector(&roots.app).unwrap().active_major,17);}else{result.unwrap();}
            assert_eq!(j.load().outcome,Outcome::ActivationFailed);assert!(!j.load().records.iter().any(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done));
        }
    }

    #[test]
    fn restore_prepare_kill_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_RESTORE_FIXTURE")else{return;};let root=std::path::PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let (locks,mut j)=open(&roots).unwrap();let id=j.load().attempt_id.clone();
        prepare_restore(&roots,&locks,&mut j,&id,&b"01".repeat(32),crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into())).unwrap();
    }

    #[test]
    fn restore_prepare_real_kills_resume_every_durable_step_and_pg_version_is_last() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["restore-intent","restore-reserves","restore-p6","restore-target","restore-guard-durable","restore-entry","restore-version-last","restore-verified","restore-work-removed","restore-guard-unlinked","restore-markers-removed","restore-backup-removed","restore-selector15","restore-companion"] {
            let (t,roots,locks,j,selected,key)=restore_fixture();let id=j.load().attempt_id.clone();let g=selected.generation.unwrap();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::operator::tests::restore_prepare_kill_child","--nocapture"])
                .env_clear().env("NWUI_OPERATOR_RESTORE_FIXTURE",t.path()).env("NWUI_OPERATOR_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(9),"{point}");
            if matches!(point,"restore-guard-durable"|"restore-entry") {assert!(!roots.db.join("pgdata/PG_VERSION").exists());assert!(roots.db.join("pgdata/NWUI-RESTORE-IN-PROGRESS.txt").exists());}
            let (locks,mut j)=open(&roots).unwrap();prepare_restore(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into())).unwrap();
            super::super::backup::verify_restored(&roots,&j).unwrap();assert!(!roots.app.join(".nwui").exists());assert!(!roots.db.join(".nwui-auto-work").join(&id).exists());
            assert!(!roots.db.join(".nwui-pg15-backup").join(&id).exists());assert!(roots.db.join(".nwui-discard").join(g).join("17/data/canary").exists());
            assert_eq!(super::super::selector::read_selector(&roots.app).unwrap().system_identifier,"123456");assert!(!j.load().records.iter().any(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done));
        }
    }

    #[test]
    fn restore_first_and_last_non_version_moves_resume_the_partial_backup_union() {
        use std::os::unix::process::ExitStatusExt;
        for occurrence in [1,2] {
            let (t,roots,locks,j,_selected,key)=restore_fixture();let id=j.load().attempt_id.clone();drop(j);drop(locks);
            let status=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::operator::tests::restore_prepare_kill_child","--nocapture"])
                .env_clear().env("NWUI_OPERATOR_RESTORE_FIXTURE",t.path()).env("NWUI_OPERATOR_KILL_POINT","restore-entry").env("NWUI_OPERATOR_KILL_OCCURRENCE",occurrence.to_string()).status().unwrap();
            assert_eq!(status.signal(),Some(9),"non-version move {occurrence}");
            let (locks,mut j)=open(&roots).unwrap();
            let moved=j.load().moves.iter().filter(|m|m.record.phase==super::super::MovePhase::Restore&&m.record.state==StepState::Done).map(|m|m.record.entry.as_str()).collect::<Vec<_>>();
            assert_eq!(moved,if occurrence==1{vec!["global"]}else{vec!["global","zeta"]});
            assert!(!roots.db.join("pgdata/PG_VERSION").exists());assert!(roots.db.join("pgdata/NWUI-RESTORE-IN-PROGRESS.txt").exists());
            assert!(roots.db.join(".nwui-pg15-backup").join(&id).join("PG_VERSION").exists());
            assert_eq!(roots.db.join(".nwui-pg15-backup").join(&id).join("zeta").exists(),occurrence==1);
            // Full-BK verification must fail here; the restore union is the valid resume authority.
            assert!(super::super::backup::verify_backup(&roots,&j).is_err());
            let mut manifest=super::super::backup::open_verified_manifest(&roots,&j).unwrap();super::super::backup::verify_restore_union(&roots,&j,&mut manifest).unwrap();
            let candidate=j.load().candidate.clone();
            restore_command(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into()),&candidate).unwrap();
            assert_eq!(j.load().outcome,Outcome::Restored);super::super::backup::verify_restored(&roots,&j).unwrap();
        }
    }

    #[test]
    fn restore_command_kill_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_COMPLETE_FIXTURE")else{return;};let root=std::path::PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};let (locks,mut j)=open(&roots).unwrap();let id=j.load().attempt_id.clone();
        let candidate=j.load().candidate.clone();
        let message=restore_command(&roots,&locks,&mut j,&id,&b"01".repeat(32),crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into()),&candidate).unwrap();
        println!("{message}");
    }

    #[test]
    fn restore_done_kill_replay_drains_both_pools_and_prints_already_restored() {
        use std::os::unix::process::ExitStatusExt;
        let (t,roots,locks,j,_selected,_key)=restore_fixture();drop(j);drop(locks);
        let child=|| {let mut c=std::process::Command::new(std::env::current_exe().unwrap());c.args(["--exact","upgrade::operator::tests::restore_command_kill_child","--nocapture"]).env_clear().env("NWUI_OPERATOR_COMPLETE_FIXTURE",t.path());c};
        let killed=child().env("NWUI_OPERATOR_KILL_POINT","restore-done").output().unwrap();assert_eq!(killed.status.signal(),Some(9));
        {
            let (locks,j)=open(&roots).unwrap();assert_eq!(j.load().outcome,Outcome::Restored);assert_eq!(j.load().handoff,super::super::journal::Handoff::None);
            let remaining=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!((remaining.app_files,remaining.db_files),(8,8));
        }
        let resumed=child().output().unwrap();assert!(resumed.status.success(),"{}",String::from_utf8_lossy(&resumed.stderr));
        let stdout=String::from_utf8(resumed.stdout).unwrap();assert!(stdout.contains("이미 복원했습니다:"),"{stdout}");
        let (locks,j)=open(&roots).unwrap();let remaining=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!((remaining.app_files,remaining.db_files),(0,0));
        assert_eq!(j.load().records.iter().filter(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done).count(),1);
        super::super::backup::verify_restored(&roots,&j).unwrap();
    }

    #[test]
    fn restore_capacity_retry_releases_only_the_affected_filesystem_and_retries_once() {
        let (_t,roots,locks,mut j,_)=fixture();super::super::reserves::ensure(&roots,&locks,&j).unwrap();let before=super::super::reserves::report(&roots,&locks,&j).unwrap();
        let mut calls=0;retry_app(&j,||{calls+=1;if calls==1 {Err(refuse("insufficient_space","injected app ENOSPC"))}else{Ok(())}}).unwrap();assert_eq!(calls,2);
        let after=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!(after.app_files,before.app_files-1);assert_eq!(after.db_files,before.db_files);
        let mut calls=0;retry_db(&roots,&locks,&mut j,|_|{calls+=1;if calls==1 {Err(refuse("insufficient_space","injected db ENOSPC"))}else{Ok(())}}).unwrap();assert_eq!(calls,2);
        let db_after=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!(db_after.app_files,after.app_files);assert_eq!(db_after.db_files,after.db_files-1);
        let mut calls=0;assert_eq!(retry_db::<()>(&roots,&locks,&mut j,|_|{calls+=1;Err(refuse("journal_write_failed","injected A failure inside a move"))}).unwrap_err().code,"journal_write_failed");assert_eq!(calls,1);
        assert_eq!(super::super::reserves::report(&roots,&locks,&j).unwrap().db_files,db_after.db_files);
        let mut calls=0;assert_eq!(retry_app::<()>(&j,||{calls+=1;Err(refuse("insufficient_space","still full"))}).unwrap_err().code,"restore_incomplete");assert_eq!(calls,2);
    }

    #[test]
    fn restored_message_matches_same_candidate_admission_after_complete_restore() {
        for failure in [None,Some("privilege_activate_failed"),Some("interrupted")] {
            let (_t,roots,locks,mut j,_selected,key)=restore_fixture();let id=j.load().attempt_id.clone();let candidate=j.load().candidate.clone();
            if let Some(code)=failure {
                let now=chrono::Utc::now();
                if code=="interrupted" {super::super::retry::failed(&mut j,Step::S4Activate,code,now).unwrap();}
                super::super::retry::failed(&mut j,Step::S4Activate,code,now).unwrap();j.set_outcome(Outcome::ActivationFailed).unwrap();
            }
            let expected=super::super::retry::admit(j.load(),&candidate,None,true,chrono::Utc::now()).unwrap();
            let text=restore_command(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into()),&candidate).unwrap();
            assert_eq!(j.load().outcome,Outcome::Restored);super::super::backup::verify_restored(&roots,&j).unwrap();
            assert_eq!(text,super::super::retry::restored_next_action(&expected),"failure={failure:?}");
        }
    }

    #[test]
    fn restore_command_finishes_atomically_then_removes_discard_and_replays_without_generation() {
        let (_t,roots,locks,mut j,_selected,key)=restore_fixture();let id=j.load().attempt_id.clone();
        let candidate=j.load().candidate.clone();
        let text=restore_command(&roots,&locks,&mut j,&id,&key,crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|Ok("654321".into()),&candidate).unwrap();
        assert!(text.contains("PG15"));assert_eq!(j.load().outcome,Outcome::Restored);assert_eq!(j.load().handoff,super::super::journal::Handoff::None);
        let reserves=super::super::reserves::report(&roots,&locks,&j).unwrap();assert_eq!((reserves.app_files,reserves.db_files),(0,0));
        assert_eq!(j.load().records.iter().filter(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done).count(),1);
        let (_,g)=super::super::rollover::retention(j.load()).unwrap();let g=g.unwrap();
        remove_discarded_with(&roots,&locks,&mut j,&g,&|_|Ok("654321".into())).unwrap();
        let candidate=j.load().candidate.clone();
        assert!(restore_command(&roots,&locks,&mut j,&id,&[],crate::db_upgrade::postgres::postgres_ids().unwrap(),&|_|panic!("already restored must not read discarded PG17"),&candidate).unwrap().contains("이미 복원했습니다"));
        drop(j);drop(locks);
        assert!(execute(&roots,Command{kind:Kind::Restore,id},Some(Err(refuse("verification_failed","image metadata unavailable")))).unwrap().contains("이미 복원했습니다"));
    }

    #[tokio::test]
    async fn dispatch_rejects_invalid_confirmation_before_fixed_roots_or_secrets() {
        assert_eq!(run("restore-pg15",&["--confirm".into(),uuid::Uuid::new_v4().to_string()]).await.unwrap_err().code,"invalid_arguments");
    }

    #[test]
    fn execute_refusal_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_EXECUTE_FIXTURE")else{return;};let root=std::path::PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};
        let id=super::super::journal::read_only(&roots.app).unwrap().attempt_id;
        let candidate=super::super::journal::read_only(&roots.app).unwrap().candidate;
        let error=execute(&roots,Command{kind:Kind::Restore,id},Some(Ok(candidate))).unwrap_err();assert_eq!(error.code,std::env::var("NWUI_OPERATOR_EXPECTED_REFUSAL").unwrap());
    }

    #[test]
    fn restore_dispatch_never_creates_a_canonical_key_or_intent_on_key_refusal() {
        for mode in ["missing","empty","relative"] {
            let (t,roots,locks,j,_selected,_key)=restore_fixture();let before=crate::db_upgrade::files::snapshot(&roots.db).unwrap();drop(j);drop(locks);
            let mut child=std::process::Command::new(std::env::current_exe().unwrap());
            child.args(["--exact","upgrade::operator::tests::execute_refusal_child","--nocapture"]).env_clear().env("NWUI_OPERATOR_EXECUTE_FIXTURE",t.path())
                .env("NWUI_OPERATOR_EXPECTED_REFUSAL",if mode=="missing"{"key_missing"}else{"key_path_invalid"});
            if mode=="empty" {child.env("DATA_KEY_FILE","");}if mode=="relative" {child.env("DATA_KEY_FILE","relative/key");}
            assert!(child.status().unwrap().success(),"{mode}");assert_eq!(crate::db_upgrade::files::snapshot(&roots.db).unwrap(),before);
            assert!(!roots.app.join("secrets/data_key").exists());assert!(!super::super::journal::read_only(&roots.app).unwrap().records.iter().any(|r|r.step==Step::CmdRestorePg15));
        }
    }

    #[test]
    fn parsing_requires_the_exact_confirmation_and_one_canonical_id() {
        let id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_string();
        for (name, flag) in [("restore-pg15", "--confirm-loss"), ("remove-pg15-backup", "--confirm"), ("remove-discarded", "--confirm")] {
            let parsed = parse(name, &[flag.into(), id.clone()]).unwrap();
            assert_eq!(parsed.id, id);
            assert_eq!(parsed.kind, match name { "restore-pg15" => Kind::Restore, "remove-pg15-backup" => Kind::RemoveBackup, _ => Kind::RemoveDiscarded });
            for args in [vec![], vec![id.clone()], vec!["--wrong".into(), id.clone()], vec![flag.into(), "../other".into()], vec![flag.into(), id.clone(), "extra".into()], vec![flag.into(), id.to_uppercase()]] {
                assert_eq!(parse(name, &args).unwrap_err().code, "invalid_arguments");
            }
        }
        assert_eq!(parse("unknown", &["--confirm".into(), id]).unwrap_err().code, "invalid_arguments");
    }

    #[test]
    fn an_active_service_lock_refuses_before_opening_or_mutating_the_journal() {
        let (_temp, roots, _locks, journal, _selected) = fixture();
        let before = fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        assert_eq!(open(&roots).err().unwrap().code, "storage_busy");
        assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(), before);
        assert!(!roots.app.join("secrets/data_key").exists());
        assert_eq!(journal.load().outcome, Outcome::Upgrading);
    }

    #[test]
    fn backup_removal_needs_success_for_the_active_generation_but_not_the_current_boot() {
        let (_temp, _roots, _locks, mut journal, selected) = fixture();
        assert_eq!(started_once(journal.load(), &selected).unwrap_err().code, "verification_failed");
        journal.set_outcome(Outcome::Pg17).unwrap();
        journal.record(Step::Phase2Monitor, StepState::Intent, Some(serde_json::json!({"generation":selected.generation}))).unwrap();
        journal.record(Step::Phase2Monitor, StepState::Done, Some(serde_json::json!({"generation":uuid::Uuid::new_v4().to_string(),"bootIdentity":"old"}))).unwrap();
        assert!(started_once(journal.load(), &selected).is_err());
        journal.record(Step::Phase2Monitor, StepState::Done, Some(serde_json::json!({"generation":selected.generation,"bootIdentity":"previous-boot","observedAt":"2026-10-28T00:00:00Z"}))).unwrap();
        started_once(journal.load(), &selected).unwrap();
        let mut restored = selected.clone(); restored.active_major = 15; restored.generation = None;
        assert!(started_once(journal.load(), &restored).is_err());
        journal.record(Step::S4Activate, StepState::Failed { code: "activation_timeout".into() }, None).unwrap();
        journal.set_outcome(Outcome::ActivationFailed).unwrap();
        assert!(started_once(journal.load(), &selected).is_err());
    }

    #[test]
    fn only_the_same_intent_can_resume_and_unrelated_done_or_failure_cannot_settle_it() {
        let (_temp,_roots,_locks,mut journal,_selected)=fixture();
        let own=serde_json::json!({"discardId":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"});
        let foreign=serde_json::json!({"discardId":"ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee"});
        assert_eq!(progress(journal.load(),Step::CmdRemoveDiscarded,&own).unwrap(),Progress::New);
        journal.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(own.clone())).unwrap();
        journal.record(Step::CmdRemoveDiscarded,StepState::Failed{code:"io_error".into()},Some(own.clone())).unwrap();
        assert_eq!(progress(journal.load(),Step::CmdRemoveDiscarded,&own).unwrap(),Progress::Resume);
        assert!(progress(journal.load(),Step::CmdRemoveDiscarded,&foreign).is_err());
        journal.record(Step::CmdRemoveDiscarded,StepState::Done,Some(foreign.clone())).unwrap();
        assert!(progress(journal.load(),Step::CmdRemoveDiscarded,&foreign).is_err());
        journal.record(Step::CmdRemoveDiscarded,StepState::Done,Some(own.clone())).unwrap();
        assert_eq!(progress(journal.load(),Step::CmdRemoveDiscarded,&own).unwrap(),Progress::Done);
        assert_eq!(progress(journal.load(),Step::CmdRemoveDiscarded,&foreign).unwrap(),Progress::Done);
    }

    #[test]
    fn restored15_can_remove_only_its_uncleared_recorded_discard() {
        let (_temp,_roots,_locks,mut journal,mut selected)=fixture();
        let g=selected.generation.take().unwrap();selected.active_major=15;selected.manifest_sha256=None;selected.operator_reason=Some("restore-pg15".into());
        journal.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();
        journal.record(Step::P5Commit,StepState::Done,None).unwrap();
        let data=serde_json::json!({"backupId":journal.load().attempt_id,"generation":g,"discardId":g});
        journal.record(Step::CmdRestorePg15,StepState::Intent,Some(data.clone())).unwrap();
        journal.record(Step::CmdRestorePg15,StepState::Done,Some(data)).unwrap();
        assert_eq!(retained_attempt(journal.load(),&selected,&g).unwrap(),journal.load().attempt_id);
        assert!(retained_attempt(journal.load(),&selected,&uuid::Uuid::new_v4().to_string()).is_err());
        let mut wrong=selected.clone();wrong.backup_id=Some(uuid::Uuid::new_v4().to_string());
        assert!(retained_attempt(journal.load(),&wrong,&g).is_err());
        journal.record(Step::CmdRemoveDiscarded,StepState::Done,Some(serde_json::json!({"discardId":g}))).unwrap();
        assert!(retained_attempt(journal.load(),&selected,&g).is_err());
    }

    #[test]
    fn restored15_retention_survives_rollover_but_never_authorizes_an_active_generation() {
        let (_temp,_roots,_locks,journal,mut selected)=fixture();let mut saved=journal.load().clone();
        let g=selected.generation.take().unwrap();selected.active_major=15;selected.manifest_sha256=None;selected.operator_reason=Some("restore-pg15".into());
        let previous=uuid::Uuid::new_v4().to_string();selected.backup_id=Some(previous.clone());
        saved.previous.push(crate::upgrade::journal::PreviousAttempt {attempt_id:previous.clone(),outcome:Outcome::Restored,candidate:saved.candidate.clone(),last_failure:None,generation:Some(g.clone()),discard_id:Some(g.clone()),ended_at:"2026-10-29T00:00:00Z".into()});
        assert_eq!(retained_attempt(&saved,&selected,&g).unwrap(),previous);
        let mut active=selected.clone();active.active_major=17;active.operator_reason=None;active.manifest_sha256=Some("4".repeat(64));active.generation=Some(g.clone());
        assert!(retained_attempt(&saved,&active,&g).is_err());
        saved.previous[0].generation=Some(uuid::Uuid::new_v4().to_string());
        assert!(retained_attempt(&saved,&selected,&g).is_err());
    }

    fn restore_intent(journal: &mut Journal, selected: &Selector) {
        journal.record(Step::CmdRestorePg15,StepState::Intent,Some(serde_json::json!({"backupId":journal.load().attempt_id,"generation":selected.generation,"discardId":selected.generation}))).unwrap();
    }

    #[test]
    fn restore_guard_reuses_full_intent_and_supplies_the_real_linked_capability() {
        for nested in [false,true] {
            let (_temp,roots,locks,mut journal,selected)=fixture();restore_intent(&mut journal,&selected);
            let original=if nested {let path=roots.db.join("pgdata");fs::create_dir(&path).unwrap();path} else {roots.db.clone()};
            fs::create_dir_all(roots.db.join(".nwui-pg15-backup").join(&journal.load().attempt_id)).unwrap();
            let before=serde_json::to_vec(journal.load()).unwrap();
            let (l,bk,linked)=prepare_restore_dirs(&roots,&locks,&mut journal,&original,(0,0)).unwrap();
            assert!(original.join("NWUI-RESTORE-IN-PROGRESS.txt").is_file());
            let expected=roots.db.join(".nwui-auto-work").join(&journal.load().attempt_id).join("linked");
            assert_eq!(fs::read_link(format!("/proc/self/fd/{}",linked.as_raw_fd())).unwrap(),expected);
            assert_ne!(l.metadata().unwrap().ino(),bk.metadata().unwrap().ino());
            assert_ne!(l.metadata().unwrap().ino(),linked.metadata().unwrap().ino());
            assert_eq!(serde_json::to_vec(journal.load()).unwrap(),before,"no narrower Cmd record may replace the full Intent");
            prepare_restore_dirs(&roots,&locks,&mut journal,&original,(0,0)).unwrap();
        }
    }

    #[test]
    fn restore_dirs_require_the_command_intent_and_preserve_foreign_work() {
        let (_temp,roots,locks,mut journal,selected)=fixture();
        assert!(prepare_restore_dirs(&roots,&locks,&mut journal,&roots.db,(0,0)).is_err());
        assert!(!roots.db.join("NWUI-RESTORE-IN-PROGRESS.txt").exists());
        restore_intent(&mut journal,&selected);
        let linked=roots.db.join(".nwui-auto-work").join(&journal.load().attempt_id).join("linked");
        fs::create_dir_all(&linked).unwrap();
        for path in [linked.parent().unwrap().parent().unwrap(),linked.parent().unwrap(),&linked] {fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        fs::write(linked.join("foreign"),b"preserve").unwrap();
        let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        assert!(prepare_restore_dirs(&roots,&locks,&mut journal,&roots.db,(0,0)).is_err());
        assert_eq!(fs::read(linked.join("foreign")).unwrap(),b"preserve");
        assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
    }

    fn target(roots: &Roots) {
        fs::create_dir_all(roots.db.join("17/data")).unwrap();
        fs::write(roots.db.join("17/data/canary"), b"keep PG17").unwrap();
    }

    fn move_fixture(roots: &Roots, locks: &StorageLocks, generation: &str) -> Result<(), StepError> {
        set_aside_with(roots, locks, generation, "123456", (0, 0), &|_| Ok("123456".into()))
    }

    #[test]
    fn discarded_target_move_preserves_bytes_and_resumes_without_deleting_the_target() {
        let (_temp, roots, locks, _journal, selected) = fixture(); target(&roots);
        let g = selected.generation.as_deref().unwrap();
        move_fixture(&roots, &locks, g).unwrap(); move_fixture(&roots, &locks, g).unwrap();
        assert!(!roots.db.join("17").exists());
        assert_eq!(fs::read(roots.db.join(".nwui-discard").join(g).join("17/data/canary")).unwrap(), b"keep PG17");
    }

    #[test]
    fn discarded_target_move_refuses_foreign_identity_both_neither_and_symlink() {
        use std::os::unix::fs::symlink;
        for bad in ["identifier", "both", "neither", "symlink"] {
            let (temp, roots, locks, _journal, selected) = fixture(); let g = selected.generation.as_deref().unwrap();
            if bad != "neither" { target(&roots); }
            if bad == "both" { fs::create_dir_all(roots.db.join(".nwui-discard").join(g).join("17/data")).unwrap(); }
            if bad == "symlink" { symlink(temp.path(), roots.db.join(".nwui-discard")).unwrap(); }
            let before = crate::db_upgrade::files::snapshot(&roots.app).unwrap();
            let result = if bad == "identifier" { set_aside_with(&roots, &locks, g, "other", (0,0), &|_| Ok("123456".into())) } else { move_fixture(&roots, &locks, g) };
            assert!(result.is_err(), "{bad}");
            assert_eq!(crate::db_upgrade::files::snapshot(&roots.app).unwrap(), before);
            if bad != "neither" { assert_eq!(fs::read(roots.db.join("17/data/canary")).unwrap(), b"keep PG17"); }
        }
    }

    #[test]
    fn discard_move_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_FIXTURE") else { return; };
        let root=std::path::PathBuf::from(root);
        let roots=Roots {db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        move_fixture(&roots,&locks,"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap();
    }

    #[test]
    fn real_kills_at_discard_durable_boundaries_resume_without_losing_pg17() {
        use std::os::unix::process::ExitStatusExt;
        let g="aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        for point in ["discard-parent-created","discard-generation-created","discard-parent-durable","discard-generation-durable","discard-rename","discard-source-fsync","discard-destination-fsync"] {
            let (temp,roots,locks,journal,_)=fixture();target(&roots);drop(journal);drop(locks);
            let child=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","upgrade::operator::tests::discard_move_child"])
                .env_clear().env("NWUI_OPERATOR_FIXTURE",temp.path()).env("NWUI_OPERATOR_KILL_POINT",point).output().unwrap();
            assert_eq!(child.status.signal(),Some(libc::SIGKILL),"{point}: {}",String::from_utf8_lossy(&child.stderr));
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
            move_fixture(&roots,&locks,g).unwrap();
            assert_eq!(fs::read(roots.db.join(".nwui-discard").join(g).join("17/data/canary")).unwrap(),b"keep PG17");
            assert!(!roots.db.join("17").exists());
        }
    }

    fn pair_fixture(roots: &Roots, selected: &Selector) -> (Selector, crate::db_upgrade::pair::Pair) {
        let mut restored = selected.clone(); restored.active_major=15; restored.generation=None; restored.manifest_sha256=None; restored.operator_reason=Some("restore-pg15".into());
        crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/active-v1.json"),&restored).unwrap();
        let mut pair=crate::db_upgrade::pair::Pair::fresh("arm64");pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some("123456".into());
        let directory=roots.app.join(".nwui");fs::create_dir(&directory).unwrap();fs::set_permissions(&directory,fs::Permissions::from_mode(0o700)).unwrap();
        crate::db_upgrade::files::atomic_json(&directory.join("pair-v1.json"),&pair).unwrap();
        (restored,pair)
    }

    #[test]
    fn restored_pair_removal_allows_only_the_monotonic_flag_difference_and_is_idempotent() {
        let (_temp,roots,locks,_journal,selected)=fixture();let (restored,pair)=pair_fixture(&roots,&selected);
        let mut live=pair.clone();live.encryption_key_required=true;
        crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui/pair-v1.json"),&live).unwrap();
        remove_auto_pair(&roots,&locks,&restored,&pair,&pair.pair_id).unwrap();
        remove_auto_pair(&roots,&locks,&restored,&pair,&pair.pair_id).unwrap();
        assert!(!roots.app.join(".nwui").exists());
        assert_eq!(super::super::selector::read_selector(&roots.app).unwrap(),restored);
    }

    #[test]
    fn foreign_or_mismatching_live_pair_is_preserved_before_any_unlink() {
        for bad in ["foreign", "pair", "binding", "selector", "selector-bytes"] {
            let (_temp,roots,locks,_journal,selected)=fixture();let (restored,pair)=pair_fixture(&roots,&selected);
            if bad=="foreign" {fs::write(roots.app.join(".nwui/foreign"),b"keep").unwrap();}
            if bad=="pair" {let mut other=pair.clone();other.pair_id=uuid::Uuid::new_v4().to_string();crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui/pair-v1.json"),&other).unwrap();}
            if bad=="selector" {crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/active-v1.json"),&selected).unwrap();}
            if bad=="selector-bytes" {let path=roots.app.join(".nwui-auto/active-v1.json");let mut bytes=fs::read(&path).unwrap();bytes.push(b'\n');fs::write(path,bytes).unwrap();}
            let before=crate::db_upgrade::files::snapshot(&roots.app).unwrap();
            let binding=if bad=="binding" {"wrong"} else {&pair.pair_id};
            assert!(remove_auto_pair(&roots,&locks,&restored,&pair,binding).is_err(),"{bad}");
            assert_eq!(crate::db_upgrade::files::snapshot(&roots.app).unwrap(),before);
        }
    }

    #[test]
    fn pair_remove_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_PAIR_FIXTURE") else { return; };
        let root=std::path::PathBuf::from(root);
        let roots=Roots {db:root.join("db"),app:root.join("app"),architecture:"arm64".into()};
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let restored=super::super::selector::read_selector(&roots.app).unwrap();
        let expected:crate::db_upgrade::pair::Pair=crate::db_upgrade::files::read_json(&root.join("expected.json")).unwrap();
        remove_auto_pair(&roots,&locks,&restored,&expected,&expected.pair_id).unwrap();
    }

    #[test]
    fn real_kills_during_pair_removal_resume_and_preserve_the_canonical_key() {
        use std::os::unix::process::ExitStatusExt;
        for point in ["pair-before-unlink","pair-unlink","pair-directory-fsync","pair-rmdir","pair-parent-fsync"] {
            let (temp,roots,locks,journal,selected)=fixture();let (restored,pair)=pair_fixture(&roots,&selected);
            crate::db_upgrade::files::atomic_json(&temp.path().join("expected.json"),&pair).unwrap();
            fs::create_dir(roots.app.join("secrets")).unwrap();fs::write(roots.app.join("secrets/data_key"),b"unchanged canonical key").unwrap();
            drop(journal);drop(locks);
            let child=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","upgrade::operator::tests::pair_remove_child"])
                .env_clear().env("NWUI_OPERATOR_PAIR_FIXTURE",temp.path()).env("NWUI_OPERATOR_KILL_POINT",point).output().unwrap();
            assert_eq!(child.status.signal(),Some(libc::SIGKILL),"{point}: {}",String::from_utf8_lossy(&child.stderr));
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
            remove_auto_pair(&roots,&locks,&restored,&pair,&pair.pair_id).unwrap();
            assert!(!roots.app.join(".nwui").exists());
            assert_eq!(fs::read(roots.app.join("secrets/data_key")).unwrap(),b"unchanged canonical key");
        }
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use crate::db_upgrade::{cluster,files,inspect,migration::UpgradeMode,postgres};
    use std::{fs,path::PathBuf,os::unix::fs::PermissionsExt,time::Duration};

    struct Native { root:PathBuf, roots:Roots, logs:PathBuf, baseline:String, generation:String, backup:String }
    fn row(name:&str,status:&str,detail:serde_json::Value) {println!("OPERATOR_NATIVE_ROW {}",serde_json::json!({"row":name,"status":status,"detail":detail}));}
    async fn snapshot(socket:&Path)->String {
        let value=cluster::inspect_source(socket,"native-fixture-only",None).await.unwrap();
        super::super::selector::digest(&serde_json::to_vec(&value).unwrap())
    }
    // Same physical P1/P3/P5 path as conversion's owner native fixture; P2/P4 are attestations.
    async fn converted(mode:UpgradeMode)->Native {converted_in(mode,false).await}
    async fn converted_in(mode:UpgradeMode,fixed_roots:bool)->Native {
        assert_eq!(std::env::var("NWUI_OPERATOR_NATIVE").as_deref(),Ok("1"));
        assert!(!crate::nonroot::PRIVILEGE_SPLIT,"N2 split assembly is outside this W1 fixture");
        let evidence=PathBuf::from(std::env::var_os("NWUI_OPERATOR_NATIVE_DIR").unwrap());
        let t=tempfile::Builder::new().prefix("operator-").tempdir_in(&evidence).unwrap();let root=t.keep();fs::set_permissions(&root,fs::Permissions::from_mode(0o711)).unwrap();
        let (db,app)=if fixed_roots {
            let db=PathBuf::from(crate::db_upgrade::paths::PGROOT);let app=PathBuf::from(crate::db_upgrade::paths::APPROOT);
            for path in [&db,&app] {assert_eq!(fs::read_dir(path).unwrap().count(),0,"native CLI requires task-owned EMPTY fixed-root mounts");}
            (db,app)
        }else{(root.join("db"),root.join("app"))};
        let roots=Roots{db,app,architecture:crate::db_upgrade::engine::architecture().unwrap().into()};
        if fixed_roots {
            // Docker already created these asserted-empty task-owned mount points.
            fs::set_permissions(&roots.db,fs::Permissions::from_mode(0o700)).unwrap();postgres::own(&roots.db).unwrap();
            fs::set_permissions(&roots.app,fs::Permissions::from_mode(0o700)).unwrap();
        }else{postgres::directory(&roots.db).unwrap();}
        files::private_dir(&roots.app).unwrap();let logs=root.join("logs");postgres::directory(&logs).unwrap();
        let mut init=postgres::command(15,"initdb").unwrap();init.arg("-D").arg(&roots.db).args(["--no-locale","--encoding=UTF8","--auth=trust"]);postgres::capture(init,&logs,Duration::from_secs(300)).unwrap();
        let source_log=logs.join("source");postgres::directory(&source_log).unwrap();let source=postgres::Private::start(15,&roots.db,&source_log).unwrap();
        let admin=inspect::connect(&source.socket,"postgres","postgres",None).await.unwrap();admin.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();
        admin.client.batch_execute("CREATE ROLE nginx LOGIN PASSWORD 'native-fixture-only'").await.unwrap();admin.client.batch_execute("CREATE DATABASE nginx_control OWNER nginx").await.unwrap();drop(admin);
        let app=inspect::connect(&source.socket,"nginx_control","postgres",None).await.unwrap();app.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();app.client.batch_execute("SET ROLE nginx; SET search_path=public").await.unwrap();
        let migrations=crate::migrations::read_migrations(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/db/drizzle")).await.unwrap();
        for migration in migrations.iter().take(28) {for sql in &migration.statements {app.client.batch_execute(sql).await.unwrap();}}
        app.client.batch_execute("CREATE SCHEMA drizzle; CREATE TABLE drizzle.__drizzle_migrations(id serial primary key,hash text NOT NULL,created_at bigint)").await.unwrap();
        for migration in migrations.iter().take(28) {app.client.execute("INSERT INTO drizzle.__drizzle_migrations(hash,created_at) VALUES($1,$2)",&[&migration.hash,&migration.timestamp]).await.unwrap();}
        app.client.batch_execute("INSERT INTO public.users(id,email) VALUES('native-operator','native-operator@example.test')").await.unwrap();drop(app);
        let before=cluster::inspect_source(&source.socket,"native-fixture-only",None).await.unwrap();let baseline=super::super::selector::digest(&serde_json::to_vec(&before).unwrap());source.stop().unwrap();
        let control=postgres::read_control(15,&roots.db).unwrap();let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();
        let mut j=Journal::begin(&roots.app,&locks,super::super::Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:super::super::journal::migrations_sha256(&migrations)}).unwrap();j.ensure_app_pool().unwrap();
        let names=fs::read_dir(&roots.db).unwrap().map(|e|e.unwrap().file_name().into_string().unwrap()).collect();let prepared=super::super::backup::prepare(&roots,&locks,&j.load().attempt_id,super::super::backup_format::SourceLocation::Root,&names,&control,"2026-10-29T00:00:00Z").unwrap();
        super::super::backup::create_verified(&roots,&locks,&mut j,&prepared).unwrap();
        let proof=super::super::profile::packaged_identity().unwrap();j.record(Step::P2Preflight,StepState::Done,Some(super::super::profile::fixture_done(mode))).unwrap();
        let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id);postgres::directory(work.parent().unwrap()).unwrap();postgres::directory(&work).unwrap();
        super::super::conversion::run_locked(&roots,&locks,&mut j,&control,&before).unwrap();let data=work.join("data");
        let target_log=logs.join("target");postgres::directory(&target_log).unwrap();let target=postgres::Private::start(17,&data,&target_log).unwrap();
        let target_baseline=snapshot(&target.socket).await;
        files::private_dir(&roots.app.join("secrets")).unwrap();let generation=uuid::Uuid::new_v4().to_string();
        super::super::key_write::write_locked(&locks,&roots,&mut j,super::super::key_write::Purpose::Staged{generation:generation.clone()},super::super::key_write::Mode::Publish).unwrap();
        super::super::key_write::write_locked(&locks,&roots,&mut j,super::super::key_write::Purpose::Publication{generation:generation.clone()},super::super::key_write::Mode::Prepare).unwrap();
        let source_key=super::super::KeySource::Staged{generation:generation.clone()};let key=crate::db_upgrade::key_file::read_key_file(&source_key.path(&roots.app,None),crate::db_upgrade::key_file::KeyPolicy::Strict).unwrap();
        let identifier=postgres::read_control(17,&data).unwrap().system_identifier;
        let mut pair=crate::db_upgrade::pair::Pair::fresh(&roots.architecture);pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(identifier.clone());
        let selection=super::super::StorageSelection{version:1,epoch:7,outcome:Outcome::Upgrading,branch:super::super::Branch::Upgrade,pgdata:data.clone(),backup_id:Some(j.load().attempt_id.clone()),schema_profile:proof.profile_id.clone(),key_binding:super::super::KeyBinding::Verified{pair_id:pair.pair_id.clone()}};
        j.record(Step::P4Target,StepState::Done,None).unwrap();let pg=super::super::PrivatePg{socket_dir:target.socket.clone(),port:50432,dbname:"nginx_control".into(),role:"nginx".into(),major:17};
        super::super::runner::prepare_privileges(&selection,&locks,&pg,&mut j,crate::publisher::boot::BootMode::Conversion).await.unwrap();target.stop().unwrap();
        let draft=super::super::generation::GenerationDraft{generation:&generation,origin:super::super::selector::Origin::Auto,source_profile:Some((&proof.profile_id,&proof.profile_sha256)),actual_id:&identifier,db_pair:&pair,app_pair:&pair,agreement:&selection.key_binding,key_source:&source_key,key:&key};
        super::super::publication::publish_upgrade_locked(&roots,&locks,&mut j,&selection,draft,&super::super::runner::Phase1Step::new(Step::P5Publish)).unwrap();
        let mut j=super::super::committed::resume(roots.clone(),std::sync::Arc::new(locks),j).await.unwrap();j.set_outcome(Outcome::Pg17).unwrap();
        let backup=j.load().attempt_id.clone();let fixture=Native{root,roots,logs,baseline,generation,backup};drop(j);
        row("conversion","PREFIX",serde_json::json!({"mode":match mode{UpgradeMode::Link=>"link",UpgradeMode::Copy=>"copy"},"fixture":fixture.root,"baseline":fixture.baseline,"targetSnapshot":target_baseline,"scope":"real P1/P3/P5; P2/P4 owner-native fixture attestations"}));
        fixture
    }

    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17, real CLI binary, isolated empty fixed-root mounts"]
    async fn operator_native_real_cli_restore_starts_pg15_and_replays_already_restored() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let binary=PathBuf::from(std::env::var_os("NWUI_OPERATOR_CLI_BIN").unwrap());assert!(binary.is_absolute());
        let fixture=converted_in(UpgradeMode::Link,true).await;
        let command=||{let mut c=std::process::Command::new(&binary);c.args(["db-upgrade","restore-pg15","--confirm-loss",&fixture.backup]).env_clear()
            .env("PATH","/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            .env("APP_GIT_SHA",std::env::var("NWUI_OPERATOR_CLI_REVISION").unwrap())
            .env("MIGRATIONS_DIR",std::env::var_os("MIGRATIONS_DIR").unwrap());c};
        let restored=command().output().unwrap();assert!(restored.status.success(),"{}",String::from_utf8_lossy(&restored.stderr));
        let message=String::from_utf8(restored.stdout).unwrap();assert!(message.contains("PG15로 복원했습니다."),"{message}");
        let replay=command().output().unwrap();assert!(replay.status.success(),"{}",String::from_utf8_lossy(&replay.stderr));
        let replay=String::from_utf8(replay.stdout).unwrap();assert!(replay.contains("이미 복원했습니다:"),"{replay}");
        let log=fixture.logs.join("cli-restored");postgres::directory(&log).unwrap();let server=postgres::Private::start(15,&fixture.roots.db,&log).unwrap();
        let observed=snapshot(&server.socket).await;server.stop().unwrap();assert_eq!(observed,fixture.baseline);
        row("real-cli-restore-start15-already-restored","PASS",serde_json::json!({"fixture":fixture.root,"baseline":fixture.baseline,"restoredSnapshot":observed,"message":message.trim(),"replay":replay.trim(),"scope":"real binary dispatch/P1/P3/P5, fixture-attested P2/P4; read-only build-inputs identity fixture"}));
    }

    #[test]
    fn operator_native_command_child() {
        let Some(root)=std::env::var_os("NWUI_OPERATOR_NATIVE_ROOT")else{return;};let root=PathBuf::from(root);
        let roots=Roots{db:root.join("db"),app:root.join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};
        let id=super::super::journal::read_only(&roots.app).unwrap().attempt_id;
        let candidate=super::super::journal::read_only(&roots.app).unwrap().candidate;
        let result=execute(&roots,parse("restore-pg15",&["--confirm-loss".into(),id]).unwrap(),Some(Ok(candidate)));
        let value=match result{Ok(message)=>serde_json::json!({"ok":true,"message":message}),Err(error)=>serde_json::json!({"ok":false,"code":error.code,"message":error.message})};
        fs::write(root.join("command-result.json"),serde_json::to_vec(&value).unwrap()).unwrap();println!("{value}");
    }

    async fn restore_row(fixture:Native,name:&str,kill:bool) {
        use std::os::unix::process::ExitStatusExt;
        let child=||{let mut command=std::process::Command::new(std::env::current_exe().unwrap());command.args(["--exact","upgrade::operator::native_tests::operator_native_command_child","--nocapture"]).env_clear().env("NWUI_OPERATOR_NATIVE_ROOT",&fixture.root);command};
        if kill {
            let status=child().env("NWUI_OPERATOR_KILL_POINT","restore-entry").status().unwrap();assert_eq!(status.signal(),Some(9));
            let (locks,j)=open(&fixture.roots).unwrap();let mut manifest=super::super::backup::open_verified_manifest(&fixture.roots,&j).unwrap();super::super::backup::verify_restore_union(&fixture.roots,&j,&mut manifest).unwrap();drop(j);drop(locks);
        }
        let diagnostics=fixture.roots.db.join(".nwui-auto-work/logs").join(&fixture.backup);
        let diagnostics_before=if diagnostics.is_dir(){Some(files::snapshot(&diagnostics).unwrap())}else{None};
        let result=child().output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));fs::write(fixture.root.join("command-output.log"),&result.stdout).unwrap();
        let command:serde_json::Value=serde_json::from_slice(&fs::read(fixture.root.join("command-result.json")).unwrap()).unwrap();
        if command["ok"]==false {
            let work=fixture.roots.db.join(".nwui-auto-work").join(&fixture.backup);let remaining=fs::read_dir(&work).unwrap().map(|e|e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
            assert_eq!(command["code"],"pair_mismatch");assert!(remaining.iter().any(|name|name.starts_with("convert-")),"unexpected failure {command}: {remaining:?}");
            row(name,"BLOCKED",serde_json::json!({"reason":"A25i diagnostic relocation is absent; real conversion leaves nonempty convert-* in W; no deletion workaround","command":command,"fixture":fixture.root,"workEntries":remaining,"nativeKillObserved":kill}));return;
        }
        assert!(diagnostics_before.is_some(),"successful A25i restore must retain real conversion diagnostics in K");
        assert_eq!(Some(files::snapshot(&diagnostics).unwrap()),diagnostics_before);
        let restored_log=fixture.logs.join("restored");postgres::directory(&restored_log).unwrap();let restored=postgres::Private::start(15,&fixture.roots.db,&restored_log).unwrap();let observed=snapshot(&restored.socket).await;restored.stop().unwrap();assert_eq!(observed,fixture.baseline);
        let key=fs::read(fixture.roots.app.join("secrets/data_key")).unwrap();
        execute(&fixture.roots,parse("remove-discarded",&["--confirm".into(),fixture.generation.clone()]).unwrap(),None).unwrap();
        assert!(!fixture.roots.db.join(".nwui-discard").join(&fixture.generation).exists());assert!(!fixture.roots.app.join(".nwui-auto/gen").join(&fixture.generation).exists());assert_eq!(fs::read(fixture.roots.app.join("secrets/data_key")).unwrap(),key);
        row(name,"PASS",serde_json::json!({"baseline":fixture.baseline,"restoredSnapshot":observed,"fixture":fixture.root,"nativeKillObserved":kill,"discardRemoved":true,"diagnosticsRetained":true}));
    }

    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="requires explicit native PG15+PG17 image and task-owned fixture volume"]
    async fn operator_native_link_copy_restore_and_kill_matrix() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        assert_eq!(std::env::var("NWUI_OPERATOR_NATIVE").as_deref(),Ok("1"));
        let link=converted(UpgradeMode::Link).await;
        let error=execute(&link.roots,parse("remove-pg15-backup",&["--confirm".into(),link.backup.clone()]).unwrap(),None).unwrap_err();assert_eq!(error.code,"verification_failed");
        assert!(link.roots.db.join(".nwui-pg15-backup").join(&link.backup).is_dir());
        row("remove-backup-without-phase2-proof","PREFIX",serde_json::json!({"reason":"negative admission control; the separate fixture-attested phase-2 proof row supplies the approved positive prerequisite","refusalWithoutProof":error.code,"fixture":link.root}));
        restore_row(link,"link-restore-start15-remove-discarded",false).await;
        restore_row(converted(UpgradeMode::Copy).await,"copy-restore-start15-remove-discarded",false).await;
        restore_row(converted(UpgradeMode::Link).await,"native-restore-kill-resume-start15",true).await;
    }

    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    #[ignore="native PG15+PG17; fixture-attested phase-2 proof, not monitor::check qualification"]
    async fn operator_native_remove_backup_fixture_attested_phase2_proof_pg17_restarts() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let fixture=converted(UpgradeMode::Link).await;
        let before_log=fixture.logs.join("before-backup-removal");postgres::directory(&before_log).unwrap();
        let (locks,mut journal)=open(&fixture.roots).unwrap();
        let server=postgres::Private::start(17,&fixture.roots.db.join("17/data"),&before_log).unwrap();
        let before=snapshot(&server.socket).await;server.stop().unwrap();
        let identity=super::super::boot_identity::current().unwrap();
        journal.handoff(super::super::journal::Handoff::Intent,&identity).unwrap();journal.handoff(super::super::journal::Handoff::Done,&identity).unwrap();
        journal.record(Step::S4Activate,StepState::Done,None).unwrap();
        // Use the same Journal::record path and fields as the operation owner.
        journal.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::json!({"bootIdentity":identity,"generation":fixture.generation,"observedAt":chrono::Utc::now().to_rfc3339()}))).unwrap();
        drop(journal);drop(locks);
        let canonical=fs::read(fixture.roots.app.join("secrets/data_key")).unwrap();
        let message=execute(&fixture.roots,parse("remove-pg15-backup",&["--confirm".into(),fixture.backup.clone()]).unwrap(),None).unwrap();
        assert!(!fixture.roots.db.join(".nwui-pg15-backup").join(&fixture.backup).exists());
        assert_eq!(fs::read(fixture.roots.app.join("secrets/data_key")).unwrap(),canonical);
        let after_log=fixture.logs.join("after-backup-removal");postgres::directory(&after_log).unwrap();
        let server=postgres::Private::start(17,&fixture.roots.db.join("17/data"),&after_log).unwrap();let after=snapshot(&server.socket).await;server.stop().unwrap();assert_eq!(before,after);
        let journal=super::super::journal::read_only(&fixture.roots.app).unwrap();
        assert_eq!(journal.records.iter().filter(|record|record.step==Step::CmdRemoveBackup&&record.state==StepState::Done).count(),1);
        row("remove-backup-fixture-attested-phase-2-proof","PASS",serde_json::json!({"proof":"fixture-attested phase-2 proof","realMonitorCheckQualified":false,"backupRemoved":true,"pg17Restarted":true,"beforeSnapshot":before,"afterSnapshot":after,"message":message,"fixture":fixture.root}));
    }

}
