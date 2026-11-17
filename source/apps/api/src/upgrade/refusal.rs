//! Same-boot final refusal replay; only journal first, descriptor/status next, readiness last.
use super::{Journal,Step,StepState,StepError,StorageLocks,StorageSelection,Outcome,KeyBinding,journal::JournalV1};
use crate::db_upgrade::layout::Roots;
use serde::{Serialize,Deserialize};
use std::path::Path;
#[derive(Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Receipt {authority_event:String,boot_identity:String,outcome:Outcome}
fn failed()->StepError {StepError{code:"verification_failed",message:"이 실행에서 확정한 보류 결과를 확인하지 못했습니다. 데이터를 보존했습니다.",detail:None}}
pub(super) fn final_outcome(outcome:Outcome)->bool {matches!(outcome,Outcome::RefusedPreflight|Outcome::RefusedFresh|Outcome::RefusedPrivilege|Outcome::Postponed|Outcome::Restored|Outcome::RestoreIncomplete|Outcome::ActivationFailed)}
pub(super) fn receipt(outcome:Outcome,identity:&str)->Result<serde_json::Value,StepError>{
    if !final_outcome(outcome) || identity.len()!=64 || !identity.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)){return Err(failed());}
    serde_json::to_value(Receipt{authority_event:"refusal-readiness".into(),boot_identity:identity.into(),outcome}).map_err(|_|failed())
}
pub(crate) fn outcome(j:&JournalV1,identity:&str)->Option<Outcome> {
    let last=j.records.last()?;
    if last.step!=Step::P5Publish || last.state!=StepState::Done{return None;}
    let receipt:Receipt=serde_json::from_value(last.data.as_ref()?.clone()).ok()?;
    (receipt.authority_event=="refusal-readiness" && receipt.boot_identity==identity && final_outcome(receipt.outcome) && j.outcome==receipt.outcome).then_some(receipt.outcome)
}
fn checked(roots:&Roots,locks:&StorageLocks,j:&Journal,identity:&str)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || identity.len()!=64 || !identity.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) || !final_outcome(j.load().outcome){return Err(failed());}
    if let Some(value)=j.load().records.iter().rev().find(|r|r.step==Step::P5Publish && r.state==StepState::Done && r.data.as_ref().and_then(|d|d.get("authorityEvent")).and_then(|v|v.as_str())==Some("refusal-readiness")).and_then(|r|r.data.as_ref()){
        let saved:Receipt=serde_json::from_value(value.clone()).map_err(|_|failed())?;if saved.outcome!=j.load().outcome{return Err(failed());}
    }
    if matches!(j.load().outcome,Outcome::Restored|Outcome::RestoreIncomplete) && !operator_restored(j.load()) && super::journal::last_attempt_failure(&j.load().records).is_none(){return Err(failed());}
    if serde_json::to_value(Journal::open(&roots.app,locks)?.load()).map_err(|_|failed())?!=serde_json::to_value(j.load()).map_err(|_|failed())?{return Err(failed());}Ok(())
}
/// Await on a blocking thread under custody. No runtime readiness before this durable receipt.
pub(crate) fn record(roots:&Roots,locks:&StorageLocks,j:&mut Journal,identity:&str)->Result<(),StepError> {
    checked(roots,locks,j,identity)?;
    if outcome(j.load(),identity).is_none(){j.record(Step::P5Publish,StepState::Done,Some(receipt(j.load().outcome,identity)?))?;}Ok(())
}
/// Replays only an existing receipt and never mutates the journal or retries admission.
pub(crate) fn publish(roots:&Roots,locks:&StorageLocks,j:&Journal,identity:&str,current:&super::journal::Candidate,label:Option<&str>)->Result<(),StepError> {publish_at(roots,locks,j,identity,current,label,Path::new("/run/nwui-storage.json"),Path::new(super::status::STATUS_PATH))}
fn publish_at(roots:&Roots,locks:&StorageLocks,j:&Journal,identity:&str,current:&super::journal::Candidate,label:Option<&str>,descriptor:&Path,status:&Path)->Result<(),StepError> {
    checked(roots,locks,j,identity)?;let outcome=outcome(j.load(),identity).ok_or_else(failed)?;let last=j.load().records.last().ok_or_else(failed)?;
    let value=status_value(roots,j.load(),j.load().outcome,current,label)?;
    let profile=super::profile::proof(j).ok();
    let selection=StorageSelection{version:1,epoch:last.seq,outcome,branch:j.load().branch,pgdata:roots.db.join("17/data"),backup_id:value.backup.as_ref().map(|p|p.id.clone()),schema_profile:profile.map(|p|p.profile_id).unwrap_or_else(||"none".into()),key_binding:KeyBinding::Unverified};
    super::types::write_selection_at(descriptor,&selection)?;
    if let Err(error)=super::status::publish_at(status,&value){eprintln!("[upgrade] status publish failed: {}",error.code);}Ok(())
}
/// Read-only classification selects existing headlines without touching source data or keys.
pub(super) fn unrecorded_branch(db:&Path)->(super::Branch,Outcome){
    for (path,branch,outcome) in [(db.join("PG_VERSION"),super::Branch::Upgrade,Outcome::RefusedPreflight),(db.join("pgdata/PG_VERSION"),super::Branch::Upgrade,Outcome::RefusedPreflight),(db.join("17"),super::Branch::Ready,Outcome::RefusedPrivilege)] {
        match std::fs::symlink_metadata(path){Ok(_)=>return (branch,outcome),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return (super::Branch::Upgrade,Outcome::RefusedPreflight)}
    }
    (super::Branch::Fresh,Outcome::RefusedFresh)
}
/// 2.0.1 F10: a storage-permission refusal names the path, condition and host command itself.
fn next_action(error:&StepError)->String {
    match (error.code,error.detail.as_deref()) {
        ("requires_2_0_first",_)=>"같은 볼륨을 그대로 연결해 2.0.x 최신(2.0.3 이상)에서 이전을 마친 뒤 2.1로 올리세요.".into(),
        ("storage_permissions",Some(detail)) if !detail.is_empty()=>detail.chars().filter(|c|!c.is_control()).take(2048).collect(),
        _=>"원인을 해결한 뒤 컨테이너를 다시 시작하면 다시 확인합니다.".into(),
    }
}
pub(super) fn publish_unrecorded_at(error:&StepError,db:&Path,descriptor:&Path,status:&Path,notify:impl FnOnce()->Result<(),StepError>)->Result<(),StepError>{
    let (branch,outcome)=if error.code=="requires_2_0_first"{(super::Branch::Upgrade,Outcome::RefusedPreflight)}else{unrecorded_branch(db)};
    publish_runtime_refusal(error,branch,outcome,descriptor,status,notify)
}
pub(super) fn publish_readonly_refusal_at(error:&StepError,descriptor:&Path,status:&Path,notify:impl FnOnce()->Result<(),StepError>)->Result<(),StepError>{
    publish_runtime_refusal(error,super::Branch::Upgrade,Outcome::RefusedPreflight,descriptor,status,notify)
}
fn publish_runtime_refusal(error:&StepError,branch:super::Branch,outcome:Outcome,descriptor:&Path,status:&Path,notify:impl FnOnce()->Result<(),StepError>)->Result<(),StepError>{
    use super::types::*;
    let now=chrono::Utc::now().to_rfc3339();
    let selection=StorageSelection{version:1,epoch:1,outcome,branch,pgdata:crate::db_upgrade::paths::PGDATA.into(),backup_id:None,schema_profile:"none".into(),key_binding:KeyBinding::Unverified};
    super::types::write_selection_at(descriptor,&selection)?;
    let value=UpgradeStatus{version:1,updated_at:now.clone(),outcome,branch,step:StatusStep{id:Step::P0,index:0,total:0,started_at:now,bytes_done:0,bytes_total:0},failure:Some(StatusFailure{code:error.code.into(),class:super::class_of(error.code),message:error.message.into(),next_action:next_action(error)}),space:None,locations:vec![],backup:None,discarded:vec![],cleanup_pending:false,retry:StatusRetry{next_eligible_at:None,reason:None},warnings:super::status::storage_warnings()};
    if let Err(error)=super::status::publish_at(status,&value){eprintln!("[upgrade] status publish failed: {}",error.code);}
    let detail=error.detail.as_deref().unwrap_or("").chars().filter(|c|!c.is_control()).take(2048).collect::<String>();
    eprintln!("[upgrade] preparation failed before the recovery record: {} {} {detail}",error.code,error.message);
    notify()
}
pub(crate) fn publish_status_only(roots:&Roots,j:&Journal,would_be:Outcome,current:&super::journal::Candidate,label:Option<&str>){
    match status_value(roots,j.load(),would_be,current,label){Ok(value)=>super::commands::publish_status(&value),Err(error)=>eprintln!("[upgrade] status projection failed: {}",error.code)}
}
fn operator_restored(j:&JournalV1)->bool{
    j.records.iter().rev().find(|r|r.step==Step::CmdRestorePg15).is_some_and(|r|r.state==StepState::Done)
}
pub(crate) fn bytes(path:&Path,device:u64,depth:u8,count:&mut u64)->Result<u64,StepError>{
    use std::os::unix::fs::MetadataExt;
    *count=count.checked_add(1).ok_or_else(failed)?;if *count>super::backup_format::MAX_ENTRIES || depth>64{return Err(failed());}
    let m=std::fs::symlink_metadata(path).map_err(|_|failed())?;if m.dev()!=device{return Err(failed());}
    if m.is_file(){return Ok(m.len());}if !m.is_dir(){return Err(failed());}
    let mut total=0u64;for child in std::fs::read_dir(path).map_err(|_|failed())?{total=total.checked_add(bytes(&child.map_err(|_|failed())?.path(),device,depth+1,count)?).ok_or_else(failed)?;}Ok(total)
}
/// Project only frames validated by Journal::load or journal::read_only.
pub(super) fn status_value(roots:&Roots,j:&JournalV1,outcome:Outcome,current:&super::journal::Candidate,label:Option<&str>)->Result<super::types::UpgradeStatus,StepError>{
    use super::types::*;
    let last=j.records.last().ok_or_else(failed)?;
    let saved=j.records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done);
    let mut backup=saved.map(|r|RetainedCopy{id:j.attempt_id.clone(),bytes:r.data.as_ref().and_then(|v|v.get("sourceBytes")).and_then(|v|v.as_u64()).unwrap_or(0),remove_command:format!("nwui-db-upgrade remove-pg15-backup --confirm {}",j.attempt_id)});
    if j.records.iter().any(|r|r.step==Step::CmdRemoveBackup && r.state==StepState::Done && r.data.as_ref().and_then(|d|d.get("backupId")).and_then(|v|v.as_str())==Some(j.attempt_id.as_str())) {backup=None;}
    let mut locations=backup.as_ref().map(|p|vec![roots.db.join(".nwui-pg15-backup").join(&p.id).display().to_string()]).unwrap_or_default();
    let discarded=super::commands::retained_discards(roots,j)?;let mut retry=StatusRetry{next_eligible_at:j.retry.backoff_until.clone(),reason:None};
    let mut failure=super::journal::last_attempt_failure(&j.records).map(|f|StatusFailure{class:super::class_of(&f.code),code:f.code,message:"시작 준비를 보류했습니다. 원본과 복구 기록은 보존했습니다.".into(),next_action:"기록된 원인을 해결한 뒤 컨테이너를 다시 시작하세요.".into()});
    match outcome{
        Outcome::Postponed=>{
            retry=StatusRetry{next_eligible_at:None,reason:Some(super::legacy::GUIDANCE.into())};
            if let Some(f)=failure.as_mut(){f.next_action=super::legacy::GUIDANCE.into();}
        },
        Outcome::Restored=>{
            backup=None;locations.clear();
            let admission=super::retry::admit(j,current,label,true,chrono::Utc::now())?;
            retry.next_eligible_at=None;
            if operator_restored(j){
                failure=None;retry.reason=Some(super::retry::restored_next_action(&admission).into());
            }else{
                let f=failure.as_mut().ok_or_else(failed)?;f.message="업그레이드를 끝내지 못해 원본 PG15 데이터를 검증 후 복원했습니다.".into();
                f.next_action=super::legacy::GUIDANCE.into();
            }
        },
        Outcome::RefusedPrivilege=>{
            retry=StatusRetry{next_eligible_at:None,reason:None};let f=failure.as_mut().ok_or_else(failed)?;f.message="PG17 데이터는 변경되지 않았습니다 — 원인을 해결하고 재시작하세요".into();f.next_action="원인을 해결한 뒤 컨테이너를 다시 시작하세요.".into();
        },
        Outcome::ActivationFailed=>{
            if let Some(retained)=&backup {
                let path=roots.db.join(".nwui-pg15-backup").join(&retained.id);
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_dir()=>crate::db_upgrade::files::real_path(&path).map_err(|_|failed())?,
                    Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{backup=None;locations.clear();},
                    _=>return Err(failed()),
                }
            }
            retry=StatusRetry{next_eligible_at:None,reason:None};
            let f=failure.as_mut().ok_or_else(failed)?;f.message="업그레이드는 완료됐지만 서비스 시작에 실패했습니다".into();
            f.next_action=match &backup{Some(backup)=>format!("컨테이너를 중지하고 2.0.x 최신(2.0.3 이상) 이미지의 offline entrypoint로 nwui-db-upgrade restore-pg15 --confirm-loss {} 명령을 실행하세요. 복원하면 업그레이드 이후의 변경은 사라집니다. 2.0 이미지를 정상 시작해 2.1 데이터를 열지 마세요.",backup.id),None=>"컨테이너를 다시 시작하면 이어서 진행합니다.".into()};
        },
        Outcome::RestoreIncomplete=>{
            backup=None;locations.clear();retry=StatusRetry{next_eligible_at:None,reason:None};
            let f=failure.as_mut().ok_or_else(failed)?;f.message="원본 복원을 끝내지 못했습니다. 데이터는 아래 위치에 보존돼 있습니다.".into();f.next_action="컨테이너를 중지하고 2.0.x 최신(2.0.3 이상) 이미지의 offline entrypoint에서 미완료 복구를 마치세요. 2.0 이미지를 정상 시작해 2.1 데이터를 열지 마세요. 기존 데이터는 보존했습니다.".into();
            let location=saved.and_then(|r|r.data.as_ref()).and_then(|v|v.get("header")).and_then(|v|v.get("sourceLocation")).and_then(|v|v.as_str()).ok_or_else(failed)?;
            let source=match location{"root"=>roots.db.clone(),"pgdata"=>roots.db.join("pgdata"),_=>return Err(failed())};let work=roots.db.join(".nwui-auto-work").join(&j.attempt_id);
            for path in [source,roots.db.join(".nwui-pg15-backup").join(&j.attempt_id),work.join("linked"),work.join("target"),roots.db.join("17")]{match std::fs::symlink_metadata(&path){Ok(_)=>locations.push(path.display().to_string()),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err(failed())}}
        },_=>{},
    }
    if outcome!=Outcome::RestoreIncomplete{if let Some(f)=failure.as_mut(){if let Some(guidance)=super::status::source_guidance(f.code=="source_too_old"){f.message.push(' ');f.message.push_str(guidance);f.next_action=super::legacy::GUIDANCE.into();}}}
    if let Some(f)=failure.as_mut(){if f.code=="database_newer" && !matches!(outcome,Outcome::Restored|Outcome::RestoreIncomplete) {
        f.message="이 DB는 더 새 버전의 이미지가 사용했습니다. 데이터는 변경하지 않았습니다.".into();
        f.next_action="더 새 이미지로 다시 시작하세요. 이 버전으로 되돌리려면 업데이트 전에 만든 DB·앱 백업 쌍을 복원한 뒤 시작하세요.".into();
    }}
    let space=j.records.iter().rev().find_map(|r|r.data.as_ref()?.get("space")).map(|value|serde_json::from_value(value.clone())).transpose().map_err(|_|failed())?;
    if let Some(f)=failure.as_mut() {if let Some(detail)=super::storage_modes::refusal_detail(&f.code) {f.next_action=detail;}}
    Ok(UpgradeStatus{warnings:super::status::storage_warnings(),version:1,updated_at:last.at.clone(),outcome,branch:j.branch,step:StatusStep{id:Step::P5Publish,index:0,total:0,started_at:last.at.clone(),bytes_done:0,bytes_total:0},failure,space,locations,backup,discarded,cleanup_pending:false,retry})

}
#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::PermissionsExt};
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks,Journal){
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,super::super::Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();(t,roots,locks,j)
    }
    #[test]
    fn database_newer_status_explains_recovery_without_changing_the_branch() {
        for code in ["database_newer","migration_failed"] {
            let(_t,r,_l,mut j)=fixture();
            j.fail_attempt(Step::Y2,&StepError{code,message:"fixture",detail:None},Outcome::RefusedPrivilege,chrono::Utc::now()).unwrap();
            let value=status_value(&r,j.load(),Outcome::RefusedPrivilege,&j.load().candidate,None).unwrap();
            let failure=value.failure.unwrap();assert_eq!(failure.code,code);
            if code=="database_newer" {
                assert_eq!(failure.message,"이 DB는 더 새 버전의 이미지가 사용했습니다. 데이터는 변경하지 않았습니다.");
                assert_eq!(failure.next_action,"더 새 이미지로 다시 시작하세요. 이 버전으로 되돌리려면 업데이트 전에 만든 DB·앱 백업 쌍을 복원한 뒤 시작하세요.");
            } else {
                assert_eq!(failure.message,"PG17 데이터는 변경되지 않았습니다 — 원인을 해결하고 재시작하세요");
                assert_eq!(failure.next_action,"원인을 해결한 뒤 컨테이너를 다시 시작하세요.");
            }
            assert!(!failure.next_action.contains("restore-pg15"));
        }
    }
    #[test]
    fn activation_failure_offers_restore_only_while_the_backup_directory_exists(){
        let(_t,r,_l,mut j)=fixture();j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":12}))).unwrap();j.fail_attempt(Step::S4Activate,&StepError{code:"activation_timeout",message:"fixture",detail:None},Outcome::ActivationFailed,chrono::Utc::now()).unwrap();
        let value=status_value(&r,j.load(),Outcome::ActivationFailed,&j.load().candidate,None).unwrap();assert!(value.backup.is_none());assert!(!value.failure.unwrap().next_action.contains("restore-pg15"));
        let parent=r.db.join(".nwui-pg15-backup");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();let backup=parent.join(&j.load().attempt_id);fs::create_dir(&backup).unwrap();fs::set_permissions(&backup,fs::Permissions::from_mode(0o700)).unwrap();assert!(status_value(&r,j.load(),Outcome::ActivationFailed,&j.load().candidate,None).unwrap().failure.unwrap().next_action.contains("restore-pg15"));
        fs::remove_dir(&backup).unwrap();assert!(status_value(&r,j.load(),Outcome::ActivationFailed,&j.load().candidate,None).unwrap().backup.is_none());
        fs::create_dir(&backup).unwrap();fs::set_permissions(&backup,fs::Permissions::from_mode(0o700)).unwrap();let data=Some(serde_json::json!({"backupId":j.load().attempt_id}));j.record(Step::CmdRemoveBackup,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRemoveBackup,StepState::Done,data).unwrap();assert!(!status_value(&r,j.load(),Outcome::ActivationFailed,&j.load().candidate,None).unwrap().failure.unwrap().next_action.contains("restore-pg15"));
    }
    #[test]
    fn earlier_release_guidance_keeps_the_verified_restore_text_and_offline_parity(){
        let(_t,r,_l,mut j)=fixture();j.record_retry_failure(Step::P2Preflight,"source_too_old",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::Restored).unwrap();let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap();let failure=value.failure.as_ref().unwrap();assert!(failure.message.contains("원본 PG15 데이터를 검증 후 복원했습니다"));assert!(failure.message.contains(super::super::status::source_guidance(true).unwrap()));assert_eq!(failure.code,"source_too_old");let offline=super::super::commands::offline_status(&r,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();assert_eq!(serde_json::to_value(value.failure).unwrap(),serde_json::to_value(offline.failure).unwrap());
    }
    #[test]
    fn capacity_details_survive_restore_records_and_invalid_details_refuse(){
        let(_t,r,_l,mut j)=fixture();let space=super::super::types::StatusSpace{required_bytes:123,available_bytes:100,inodes_required:9,inodes_available:8};
        j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"space":space}))).unwrap();j.record_retry_failure(Step::P2Preflight,"insufficient_space",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::Restored).unwrap();
        let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap();assert_eq!(serde_json::to_value(value.space.unwrap()).unwrap(),serde_json::to_value(space).unwrap());assert_eq!(value.failure.unwrap().code,"insufficient_space");
        j.record(Step::P2Preflight,StepState::Intent,Some(serde_json::json!({"space":{"requiredBytes":"invalid"}}))).unwrap();assert!(status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).is_err());
    }
    #[test]
    fn automatic_restore_without_failure_or_matching_outcome_cannot_publish_readiness(){
        let(t,r,l,mut j)=fixture();let id="a".repeat(64);j.set_outcome(Outcome::Restored).unwrap();j.record(Step::P5Publish,StepState::Done,Some(receipt(Outcome::Restored,&id).unwrap())).unwrap();let descriptor=t.path().join("selection");let status=t.path().join("status");assert!(publish_at(&r,&l,&j,&id,&j.load().candidate,None,&descriptor,&status).is_err());assert!(!descriptor.exists());
        j.record_retry_failure(Step::RRestore,"io_error",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::RestoreIncomplete).unwrap();assert!(record(&r,&l,&mut j,&id).is_err());assert!(!descriptor.exists());
    }
    #[test]
    fn restored_status_requires_the_intermediate_image_without_retry(){
        let(_t,r,_l,mut j)=fixture();let now=chrono::Utc::now();for _ in 0..2{j.record_retry_failure(Step::P3Convert,"interrupted",now).unwrap();}j.set_outcome(Outcome::Restored).unwrap();let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap();assert!(value.failure.unwrap().next_action.contains("2.0.x 최신(2.0.3 이상)"));assert!(value.retry.next_eligible_at.is_none());
        let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,Some("fixed")).unwrap();assert!(value.failure.unwrap().next_action.contains("2.0.x 최신(2.0.3 이상)"));assert!(value.retry.next_eligible_at.is_none());
        j.record_retry_failure(Step::P2Preflight,"insufficient_space",now).unwrap();let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap();assert!(value.failure.unwrap().next_action.contains("2.0.x 최신(2.0.3 이상)"));
    }
    #[test]
    fn operator_status_and_shared_cli_text_have_the_same_admission_and_no_failure(){
        for code in [None,Some("privilege_activate_failed")]{
            let(_t,r,_l,mut j)=fixture();if let Some(code)=code{j.record_retry_failure(Step::S4Activate,code,chrono::Utc::now()).unwrap();}
            let g=uuid::Uuid::new_v4().to_string();j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();let data=Some(serde_json::json!({"backupId":j.load().attempt_id,"generation":g,"discardId":g}));j.record(Step::CmdRestorePg15,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRestorePg15,StepState::Done,data).unwrap();j.set_outcome(Outcome::Restored).unwrap();let discard=r.db.join(".nwui-discard").join(&g);fs::create_dir_all(&discard).unwrap();fs::write(discard.join("retained"),b"1234").unwrap();
            let admission=super::super::retry::admit(j.load(),&j.load().candidate,None,true,chrono::Utc::now()).unwrap();let value=status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap();assert!(value.failure.is_none());assert!(value.backup.is_none());assert_eq!(value.retry.reason.as_deref(),Some(super::super::retry::restored_next_action(&admission)));assert_eq!(value.discarded.len(),1);assert_eq!(value.discarded[0].bytes,4);assert_eq!(value.discarded[0].id,g);if code.is_some(){assert!(value.retry.reason.unwrap().contains("2.0.x 최신(2.0.3 이상)"));}
            fs::remove_dir_all(&discard).unwrap();j.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(serde_json::json!({"discardId":g}))).unwrap();j.finish_remove_discarded(&g).unwrap();assert!(status_value(&r,j.load(),Outcome::Restored,&j.load().candidate,None).unwrap().discarded.is_empty());
        }
    }
    #[test]
    fn automatic_restore_outcomes_park_with_fixed_status_and_no_removal_commands(){
        for (outcome,code) in [(Outcome::Restored,"insufficient_space"),(Outcome::Restored,"ledger_unknown"),(Outcome::RestoreIncomplete,"io_error")]{
            let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,super::super::Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"header":{"backupId":j.load().attempt_id,"sourceLocation":"root"},"sourceBytes":12}))).unwrap();j.record_retry_failure(Step::RRestore,code,chrono::Utc::now()).unwrap();j.set_outcome(outcome).unwrap();
            // Projection fixture only; the production outcome commit must include its receipt.
            let identity="a".repeat(64);j.record(Step::P5Publish,StepState::Done,Some(receipt(outcome,&identity).unwrap())).unwrap();record(&roots,&locks,&mut j,&identity).unwrap();let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();let descriptor=t.path().join("selection");let status=t.path().join("status");publish_at(&roots,&locks,&j,&identity,&j.load().candidate,None,&descriptor,&status).unwrap();let value=super::super::status::read_at(&status).unwrap();assert_eq!(value.outcome,outcome);assert!(value.backup.is_none());assert!(value.discarded.is_empty());assert!(!serde_json::to_string(&value).unwrap().contains("removeCommand"));let failure=value.failure.unwrap();assert_eq!(failure.code,code);
            if outcome==Outcome::Restored{assert_eq!(failure.message,"업그레이드를 끝내지 못해 원본 PG15 데이터를 검증 후 복원했습니다.");assert!(value.locations.is_empty());assert!(failure.next_action.contains("2.0.x 최신(2.0.3 이상)"));}else{assert_eq!(failure.message,"원본 복원을 끝내지 못했습니다. 데이터는 아래 위치에 보존돼 있습니다.");assert_eq!(value.locations,vec![roots.db.display().to_string()]);assert!(failure.next_action.contains("offline entrypoint에서 미완료 복구"));}
            assert_eq!(super::super::runner::boot_plan(j.load(),Some(&identity)),super::super::runner::BootPlan::ParkedRefusal);assert_eq!(super::super::runner::boot_plan(j.load(),Some(&"b".repeat(64))),super::super::runner::BootPlan::Phase1);publish_at(&roots,&locks,&j,&identity,&j.load().candidate,None,&descriptor,&status).unwrap();assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert!(fs::File::open(&roots.db).unwrap().try_lock().is_err());assert!(fs::File::open(&roots.app).unwrap().try_lock().is_err());
        }
    }
    #[test]
    fn w1_final_refusal_republishes_same_boot_without_journal_mutation_and_new_boot_readmits() {
        let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,super::super::Branch::Ready,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.fail_attempt(Step::Y2,&StepError{code:"pair_mismatch",message:"fixture",detail:None},Outcome::RefusedPrivilege,chrono::Utc::now()).unwrap();let id="a".repeat(64);let descriptor=t.path().join("selection");let status=t.path().join("status");
        assert!(publish_at(&roots,&locks,&j,&id,&j.load().candidate,None,&descriptor,&status).is_err());assert!(!descriptor.exists());record(&roots,&locks,&mut j,&id).unwrap();let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        for _ in 0..2{publish_at(&roots,&locks,&j,&id,&j.load().candidate,None,&descriptor,&status).unwrap();assert_eq!(super::super::types::read_selection_at(&descriptor).unwrap().outcome,Outcome::RefusedPrivilege);assert_eq!(super::super::status::read_at(&status).unwrap().outcome,Outcome::RefusedPrivilege);assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);}
        assert_eq!(super::super::runner::boot_plan(j.load(),Some(&id)),super::super::runner::BootPlan::ParkedRefusal);j.record(Step::P0b,StepState::Intent,None).unwrap();assert_eq!(super::super::runner::boot_plan(j.load(),Some(&id)),super::super::runner::BootPlan::Phase1);assert!(publish_at(&roots,&locks,&j,&id,&j.load().candidate,None,&descriptor,&status).is_err());for identity in [Some("b".repeat(64)),None]{assert_eq!(super::super::runner::boot_plan(j.load(),identity.as_deref()),super::super::runner::BootPlan::Phase1);}
    }
    fn offline_parity_case(outcome:Outcome,code:&str,operator:bool,backoff:bool) {
        let(t,roots,locks,mut j)=fixture();let now=chrono::Utc::now();
        j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":12,"header":{"sourceLocation":"root"}}))).unwrap();
        fs::create_dir_all(roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id)).unwrap();
        if backoff {j.record_retry_failure(Step::P3Convert,"interrupted",now).unwrap();}
        j.record_retry_failure(Step::P3Convert,code,now).unwrap();
        if backoff && code=="insufficient_space" {
            let mut frame=j.load().clone();frame.retry.backoff_until=Some((now+chrono::Duration::hours(24)).to_rfc3339());
            crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/journal-v1.json"),&frame).unwrap();j=Journal::open(&roots.app,&locks).unwrap();
        }
        if operator {
            let g=uuid::Uuid::new_v4().to_string();j.record(Step::P5Commit,StepState::Intent,Some(serde_json::json!({"generation":g}))).unwrap();j.record(Step::P5Commit,StepState::Done,None).unwrap();
            let data=Some(serde_json::json!({"backupId":j.load().attempt_id,"generation":g,"discardId":g}));j.record(Step::CmdRestorePg15,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRestorePg15,StepState::Done,data).unwrap();
            let discard=roots.db.join(".nwui-discard").join(g);fs::create_dir_all(&discard).unwrap();fs::write(discard.join("retained"),b"1234").unwrap();
        }
        j.set_outcome(outcome).unwrap();let identity="a".repeat(64);record(&roots,&locks,&mut j,&identity).unwrap();
        let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        let path=t.path().join("runner-status");publish_at(&roots,&locks,&j,&identity,&j.load().candidate,None,&t.path().join("selection"),&path).unwrap();
        let online=super::super::status::read_at(&path).unwrap();let offline=super::super::commands::offline_status(&roots,&j.load().candidate,now).unwrap().unwrap();
        assert_eq!(serde_json::to_value((&offline.failure,&offline.retry)).unwrap(),serde_json::to_value((&online.failure,&online.retry)).unwrap(),"{outcome:?}");
        if outcome==Outcome::Restored {if operator {assert!(offline.failure.is_none());assert!(offline.retry.reason.is_some());}else{assert_eq!(offline.failure.as_ref().unwrap().message,"업그레이드를 끝내지 못해 원본 PG15 데이터를 검증 후 복원했습니다.");}}
        if outcome==Outcome::ActivationFailed {
            assert_eq!(offline.failure.as_ref().unwrap().message,"업그레이드는 완료됐지만 서비스 시작에 실패했습니다");
            assert_eq!(offline.failure.as_ref().unwrap().next_action,format!("컨테이너를 중지하고 2.0.x 최신(2.0.3 이상) 이미지의 offline entrypoint로 nwui-db-upgrade restore-pg15 --confirm-loss {} 명령을 실행하세요. 복원하면 업그레이드 이후의 변경은 사라집니다. 2.0 이미지를 정상 시작해 2.1 데이터를 열지 마세요.",j.load().attempt_id));
            assert!(offline.retry.next_eligible_at.is_none());assert!(offline.retry.reason.is_none());
        }
        assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
    }
    #[test] fn offline_automatic_restored_matches_published_attempt(){offline_parity_case(Outcome::Restored,"insufficient_space",false,false);}
    #[test] fn offline_capacity_fix_ignores_stale_backoff_like_runner(){offline_parity_case(Outcome::Restored,"insufficient_space",false,true);}
    #[test] fn offline_automatic_restored_matches_published_backoff(){offline_parity_case(Outcome::Restored,"interrupted",false,true);}
    #[test] fn offline_automatic_restored_matches_published_deterministic(){offline_parity_case(Outcome::Restored,"ledger_unknown",false,false);}
    #[test] fn offline_operator_restored_matches_published_reason_without_failure(){offline_parity_case(Outcome::Restored,"privilege_activate_failed",true,false);}
    #[test] fn offline_restore_incomplete_matches_published_recovery(){offline_parity_case(Outcome::RestoreIncomplete,"io_error",false,false);}
    #[test] fn offline_activation_failed_has_fixed_restart_action_without_retry(){offline_parity_case(Outcome::ActivationFailed,"interrupted",false,true);}
    #[test] fn offline_refused_preflight_matches_published_failure(){offline_parity_case(Outcome::RefusedPreflight,"ledger_unknown",false,false);}
    #[test] fn offline_other_finals_match_published_failure(){for outcome in [Outcome::RefusedFresh,Outcome::RefusedPrivilege,Outcome::Postponed] {offline_parity_case(outcome,"pair_mismatch",false,false);}}
    #[test] fn postponed_history_requires_the_intermediate_image_without_retry(){
        let (_t,roots,_locks,mut j)=fixture();j.record(Step::P0,StepState::Failed{code:"pair_mismatch".into()},None).unwrap();j.set_outcome(Outcome::Postponed).unwrap();
        let value=status_value(&roots,j.load(),Outcome::Postponed,&j.load().candidate,None).unwrap();
        assert!(value.retry.next_eligible_at.is_none());assert_eq!(value.retry.reason.as_deref(),Some(super::super::legacy::GUIDANCE));assert_eq!(value.failure.unwrap().next_action,super::super::legacy::GUIDANCE);
    }
    #[test] fn offline_and_runner_do_not_resurrect_removed_backup_on_activation_failure() {
        let(t,roots,locks,mut j)=fixture();j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":12}))).unwrap();
        let data=Some(serde_json::json!({"backupId":j.load().attempt_id}));j.record(Step::CmdRemoveBackup,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRemoveBackup,StepState::Done,data).unwrap();
        j.record_retry_failure(Step::S4Activate,"privilege_activate_failed",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::ActivationFailed).unwrap();let identity="a".repeat(64);record(&roots,&locks,&mut j,&identity).unwrap();
        let path=t.path().join("runner-status");publish_at(&roots,&locks,&j,&identity,&j.load().candidate,None,&t.path().join("selection"),&path).unwrap();
        let online=super::super::status::read_at(&path).unwrap();let offline=super::super::commands::offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();
        for value in [&online,&offline] {assert!(value.backup.is_none());assert_eq!(value.failure.as_ref().unwrap().next_action,"컨테이너를 다시 시작하면 이어서 진행합니다.");}
        assert_eq!(serde_json::to_value((&offline.failure,&offline.retry)).unwrap(),serde_json::to_value((&online.failure,&online.retry)).unwrap());
    }

    #[test]
    fn activation_restore_guidance_requires_p1_unremoved_and_present_backup() {
        for case in ["no-p1","missing-bk","retained","removed"] {
            let(t,roots,locks,mut j)=fixture();
            if case!="no-p1" {j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":12}))).unwrap();}
            let bk=roots.db.join(".nwui-pg15-backup").join(&j.load().attempt_id);
            if case!="missing-bk" {fs::create_dir_all(&bk).unwrap();}
            if case=="removed" {let data=Some(serde_json::json!({"backupId":j.load().attempt_id}));j.record(Step::CmdRemoveBackup,StepState::Intent,data.clone()).unwrap();j.record(Step::CmdRemoveBackup,StepState::Done,data).unwrap();}
            j.record_retry_failure(Step::S4Activate,"privilege_activate_failed",chrono::Utc::now()).unwrap();j.set_outcome(Outcome::ActivationFailed).unwrap();let identity="a".repeat(64);record(&roots,&locks,&mut j,&identity).unwrap();
            let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();let path=t.path().join("runner-status");
            publish_at(&roots,&locks,&j,&identity,&j.load().candidate,None,&t.path().join("selection"),&path).unwrap();
            let online=super::super::status::read_at(&path).unwrap();let offline=super::super::commands::offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();
            assert_eq!(serde_json::to_value((&online.failure,&online.retry)).unwrap(),serde_json::to_value((&offline.failure,&offline.retry)).unwrap(),"{case}");
            for value in [&online,&offline] {
                assert!(value.retry.next_eligible_at.is_none());assert!(value.retry.reason.is_none());
                if case=="retained" {assert!(value.backup.is_some());assert!(value.failure.as_ref().unwrap().next_action.contains(&format!("restore-pg15 --confirm-loss {}",j.load().attempt_id)));}
                else {assert!(value.backup.is_none(),"{case}");assert_eq!(value.failure.as_ref().unwrap().next_action,"컨테이너를 다시 시작하면 이어서 진행합니다.","{case}");}
            }
            assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        }
    }

}

#[cfg(test)]mod unrecorded_tests{
    use super::*;
    use std::{fs,os::unix::fs::PermissionsExt};
    #[test]fn f1_unrecorded_publication_is_runtime_only_and_notifies_once_after_status(){
        for (layout,outcome) in [("legacy",Outcome::RefusedPreflight),("nested",Outcome::RefusedPreflight),("ready",Outcome::RefusedPrivilege),("fresh",Outcome::RefusedFresh)]{
            for code in ["unsupported_layout","storage_busy","io_error","insufficient_space"]{
                let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();}
                match layout{"legacy"=>fs::write(db.join("PG_VERSION"),"15").unwrap(),"nested"=>{fs::create_dir(db.join("pgdata")).unwrap();fs::write(db.join("pgdata/PG_VERSION"),"15").unwrap();},"ready"=>fs::create_dir(db.join("17")).unwrap(),_=>{}}
                let before_db=crate::db_upgrade::files::snapshot(&db).unwrap();let before_app=crate::db_upgrade::files::snapshot(&app).unwrap();
                let descriptor=t.path().join("storage.json");let status=t.path().join("status.json");let calls=std::cell::Cell::new(0);
                publish_unrecorded_at(&StepError{code,message:"original startup cause",detail:None},&db,&descriptor,&status,||{
                    let selection=super::super::types::read_selection_at(&descriptor).unwrap();let value=super::super::status::read_at(&status).unwrap();
                    assert_eq!(selection.outcome,outcome);assert_eq!(selection.epoch,1);assert_eq!(selection.pgdata,Path::new(crate::db_upgrade::paths::PGDATA));assert!(selection.backup_id.is_none());assert_eq!(selection.schema_profile,"none");assert!(matches!(selection.key_binding,KeyBinding::Unverified));
                    assert_eq!(value.outcome,outcome);let failure=value.failure.unwrap();assert_eq!(failure.code,code);assert_eq!(failure.class,super::super::class_of(code));assert_eq!(failure.message,"original startup cause");assert_eq!(failure.next_action,"원인을 해결한 뒤 컨테이너를 다시 시작하면 다시 확인합니다.");calls.set(calls.get()+1);Ok(())
                }).unwrap();
                assert_eq!(calls.get(),1);assert_eq!(crate::db_upgrade::files::snapshot(&db).unwrap(),before_db);assert_eq!(crate::db_upgrade::files::snapshot(&app).unwrap(),before_app);
            }
        }
    }
    #[test]fn f1_descriptor_failure_never_publishes_status_or_readiness(){
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o777)).unwrap();let status=t.path().join("status.json");
        assert!(publish_unrecorded_at(&failed(),&t.path().join("db"),&t.path().join("storage.json"),&status,||panic!("unsafe descriptor must not notify")).is_err());assert!(!status.exists());
    }
}
