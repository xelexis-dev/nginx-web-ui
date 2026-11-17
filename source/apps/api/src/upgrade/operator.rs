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
    if command=="restore-pg15"{return Err(super::legacy::required());}
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
        Kind::Restore=>Err(super::legacy::required()),
    }
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

// This whole-cluster operator move is deliberately outside A.16's closed per-entry table.
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
    #[tokio::test]
    async fn dispatch_rejects_invalid_confirmation_before_fixed_roots_or_secrets() {
        assert_eq!(run("restore-pg15",&["--confirm".into(),uuid::Uuid::new_v4().to_string()]).await.unwrap_err().code,"requires_2_0_first");
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

    #[tokio::test]
    #[ignore="native PG17; synthetic committed proof, no PG15 conversion"]
    async fn operator_native_remove_backup_fixture_attested_phase2_proof_pg17_restarts(){
      if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
      use crate::db_upgrade::{postgres,inspect};use std::time::Duration;
      let (t,roots,locks,mut j,mut selected)=backup_fixture();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();fs::set_permissions(&roots.db,fs::Permissions::from_mode(0o755)).unwrap();
      postgres::directory(&roots.db.join("17")).unwrap();postgres::directory(&roots.db.join("17/data")).unwrap();let log=t.path().join("before-removal");postgres::directory(&log).unwrap();
      let mut init=postgres::command(17,"initdb").unwrap();init.arg("-D").arg(roots.db.join("17/data")).args(["--locale=C","--encoding=UTF8","--auth-local=trust","--auth-host=reject"]);postgres::capture(init,&log,Duration::from_secs(30)).unwrap();
      selected.system_identifier=postgres::read_control(17,&roots.db.join("17/data")).unwrap().system_identifier;let selected_bytes=serde_json::to_vec(&selected).unwrap();crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/active-v1.json"),&selected).unwrap();
      let mut saved=serde_json::to_value(j.load()).unwrap();for record in saved["records"].as_array_mut().unwrap(){if record["step"]=="p5-commit"&&record["state"]["kind"]=="intent"{record["data"]["selectorSha256"]=super::super::selector::digest(&selected_bytes).into();}}
      crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/journal-v1.json"),&saved).unwrap();j=Journal::open(&roots.app,&locks).unwrap();
      let pg=postgres::Private::start(17,&roots.db.join("17/data"),&log).unwrap();let session=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();session.client.batch_execute("SET default_transaction_read_only=off").await.unwrap();session.client.batch_execute("CREATE TABLE public.kept(value text);INSERT INTO public.kept VALUES('before backup removal')").await.unwrap();let snapshot="SELECT jsonb_build_object('databases',(SELECT jsonb_agg(to_jsonb(t)) FROM (SELECT oid,datname,datdba,datallowconn FROM pg_database ORDER BY oid)t),'roles',(SELECT jsonb_agg(to_jsonb(t)) FROM (SELECT rolname,rolsuper,rolcanlogin FROM pg_roles ORDER BY rolname)t),'columns',(SELECT jsonb_agg(to_jsonb(t)) FROM (SELECT table_name,column_name,data_type FROM information_schema.columns WHERE table_schema='public' ORDER BY table_name,ordinal_position)t),'rows',(SELECT jsonb_agg(value ORDER BY value) FROM public.kept))";let before:serde_json::Value=session.client.query_one(snapshot,&[]).await.unwrap().get(0);drop(session);pg.stop().unwrap();
      crate::db_upgrade::files::private_dir(&roots.app.join("secrets")).unwrap();let key=roots.app.join("secrets/data_key");fs::write(&key,[7u8;32]).unwrap();fs::set_permissions(&key,fs::Permissions::from_mode(0o600)).unwrap();let canonical=fs::read(&key).unwrap();let id=j.load().attempt_id.clone();drop(j);drop(locks);execute(&roots,parse("remove-pg15-backup",&["--confirm".into(),id.clone()]).unwrap(),None).unwrap();assert!(!roots.db.join(".nwui-pg15-backup").join(&id).exists());assert_eq!(fs::read(&key).unwrap(),canonical);
      let after=t.path().join("after-removal");postgres::directory(&after).unwrap();let pg=postgres::Private::start(17,&roots.db.join("17/data"),&after).unwrap();let session=inspect::connect(&pg.socket,"postgres","postgres",None).await.unwrap();assert_eq!(session.client.query_one(snapshot,&[]).await.unwrap().get::<_,serde_json::Value>(0),before);drop(session);pg.stop().unwrap();assert_eq!(super::super::journal::read_only(&roots.app).unwrap().records.iter().filter(|r|r.step==Step::CmdRemoveBackup&&r.state==StepState::Done).count(),1);
    }

}
