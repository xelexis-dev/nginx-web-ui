//! Read historical retained generations and clean consumed manifests after commit.
use super::{journal::{JournalV1,Record},Journal,Step,StepState,StepError,StorageLocks};
use crate::db_upgrade::{files,layout::Roots};
use std::{fs::{self,File},path::Path,os::unix::fs::MetadataExt};
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"이전 시도의 복구가 끝나지 않아 새 시도를 시작하지 않았습니다.",detail:None}}
fn io_error()->StepError {StepError{code:"io_error",message:"소비한 보관 목록을 정리하지 못했습니다. 복구 기록과 기존 데이터를 보존했습니다.",detail:None}}
fn authority<'a>(record:&'a Record,name:&str)->Result<&'a str,StepError>{let id=record.data.as_ref().and_then(|d|d.get(name)).and_then(|v|v.as_str()).ok_or_else(mismatch)?;if !super::types::canonical_generation(id){return Err(mismatch());}Ok(id)}
/// Only the frozen command records own a retained discard; directory names are not evidence.
pub(crate) fn retention(old:&JournalV1)->Result<(Option<String>,Option<String>),StepError>{
    let Some(restored)=old.records.iter().filter(|r|r.step==Step::CmdRestorePg15&&r.state==StepState::Done).max_by_key(|r|r.seq)else{return Ok((None,None))};
    let generation=authority(restored,"generation")?;let discard=authority(restored,"discardId")?;
    if authority(restored,"backupId")?!=old.attempt_id || discard!=generation{return Err(mismatch());}
    let commit=old.records.iter().filter(|r|r.step==Step::P5Commit&&r.state==StepState::Intent&&r.seq<restored.seq).max_by_key(|r|r.seq).ok_or_else(mismatch)?;
    if authority(commit,"generation")?!=generation || !old.records.iter().any(|r|r.step==Step::P5Commit&&r.state==StepState::Done&&r.seq>commit.seq&&r.seq<restored.seq){return Err(mismatch());}
    if old.records.iter().any(|r|r.step==Step::CmdRemoveDiscarded&&r.state==StepState::Done&&r.seq>restored.seq&&r.data.as_ref().and_then(|d|d.get("discardId")).and_then(|v|v.as_str())==Some(discard)){return Ok((None,None));}
    Ok((Some(generation.into()),Some(discard.into())))
}
fn check_locks(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError>{if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app)||!j.belongs_to(&roots.app){return Err(mismatch());}Ok(())}
/// Also called on resume: an interrupted post-commit unlink is not a new admission.
pub(crate) fn cleanup_consumed_manifests(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError>{
    check_locks(roots,locks,j)?;let root=roots.app.join(".nwui-auto");files::real_path(&root).map_err(|_|mismatch())?;
    for old in &j.load().previous{
        if !super::types::canonical_generation(&old.attempt_id)||old.attempt_id==j.load().attempt_id{return Err(mismatch());}
        let p=root.join(format!("backup-{}.jsonl",old.attempt_id));
        match fs::symlink_metadata(&p){Ok(m)=>{if !m.is_file()||m.uid()!=0||m.mode()&0o7777!=0o600||m.nlink()!=1{return Err(mismatch());}fs::remove_file(p).map_err(|_|io_error())?;},Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err(io_error())}
    }
    File::open(root).and_then(|f|f.sync_all()).map_err(|_|io_error())
}

#[cfg(test)] mod restored_tests {use super::*;use super::super::{Branch,Outcome,journal::{Candidate,PreviousAttempt}};use chrono::{DateTime,Utc};use std::{fs,path::Path,os::unix::fs::{PermissionsExt,symlink}};use serde_json::json;
    fn now()->DateTime<Utc>{"2026-10-28T00:00:00Z".parse().unwrap()}
    fn candidate()->Candidate{Candidate{revision:"1".repeat(40),pins_sha256:"2".repeat(64),migrations_sha256:"3".repeat(64)}}
    fn roots(p:&Path)->Roots{Roots{db:p.join("db"),app:p.join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()}}
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks,Journal){
        let t=tempfile::tempdir().unwrap();let r=roots(t.path());
        for p in [&r.db,&r.app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j={let fixture_journal=Journal::begin(&r.app,&locks,Branch::Upgrade,candidate()).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
        j.record_retry_failure(Step::P2Preflight,"ledger_unknown",now()).unwrap();j.set_outcome(Outcome::Restored).unwrap();(t,r,locks,j)
    }
    fn save(r:&Roots,l:&StorageLocks,state:&JournalV1)->Journal{
        fs::write(r.app.join(".nwui-auto/journal-v1.json"),serde_json::to_vec(state).unwrap()).unwrap();Journal::open(&r.app,l).unwrap()
    }
    fn append(s:&mut JournalV1,step:Step,state:StepState,data:Option<serde_json::Value>){
        let seq=s.records.iter().map(|r|r.seq).chain(s.moves.iter().map(|m|m.seq)).max().unwrap_or(0)+1;
        s.records.push(Record{seq,step,state,at:(now()+chrono::Duration::seconds(seq as i64)).to_rfc3339(),data});
    }
    fn restore(s:&mut JournalV1)->String{
        let g=uuid::Uuid::new_v4().to_string();append(s,Step::P5Commit,StepState::Intent,Some(json!({"generation":g})));
        append(s,Step::P5Commit,StepState::Done,None);
        let backup=s.attempt_id.clone();append(s,Step::CmdRestorePg15,StepState::Done,Some(json!({"backupId":backup,"generation":g,"discardId":g})));g
    }
    fn manifest(r:&Roots,id:&str){let p=r.app.join(format!(".nwui-auto/backup-{id}.jsonl"));fs::write(&p,b"consumed manifest").unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o600)).unwrap();}
    #[test]fn resume_unlinks_only_consumed_manifest_names(){let (_t,r,l,j)=fixture();let mut s=j.load().clone();let p=PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:Outcome::Restored,candidate:candidate(),last_failure:None,generation:None,discard_id:None,ended_at:now().to_rfc3339()};manifest(&r,&p.attempt_id);manifest(&r,&s.attempt_id);let consumed=r.app.join(format!(".nwui-auto/backup-{}.jsonl",p.attempt_id));s.previous.push(p);let j=save(&r,&l,&s);cleanup_consumed_manifests(&r,&l,&j).unwrap();assert!(!consumed.exists());assert!(r.app.join(format!(".nwui-auto/backup-{}.jsonl",s.attempt_id)).exists());cleanup_consumed_manifests(&r,&l,&j).unwrap();}
    #[test]fn manifest_cleanup_refuses_symlink_and_wrong_locks(){let (_t,r,l,j)=fixture();let mut s=j.load().clone();let mut p=PreviousAttempt{attempt_id:j.load().attempt_id.clone(),outcome:j.load().outcome,candidate:candidate(),last_failure:None,generation:None,discard_id:None,ended_at:now().to_rfc3339()};p.attempt_id=uuid::Uuid::new_v4().to_string();let path=r.app.join(format!(".nwui-auto/backup-{}.jsonl",p.attempt_id));s.previous.push(p);let j=save(&r,&l,&s);let outside=r.app.join("sentinel");fs::write(&outside,b"keep").unwrap();symlink(&outside,&path).unwrap();assert!(cleanup_consumed_manifests(&r,&l,&j).is_err());assert_eq!(fs::read(&outside).unwrap(),b"keep");let (_t2,_r2,l2,_j2)=fixture();assert!(cleanup_consumed_manifests(&r,&l2,&j).is_err());}
#[test] fn historical_retention_requires_exact_completed_restore_authority(){
 let (_t,_r,_l,j)=fixture();assert_eq!(retention(j.load()).unwrap(),(None,None));let mut saved=j.load().clone();let g=restore(&mut saved);assert_eq!(retention(&saved).unwrap(),(Some(g.clone()),Some(g.clone())));
 for field in ["backupId","generation","discardId"]{let mut bad=saved.clone();bad.records.last_mut().unwrap().data.as_mut().unwrap()[field]=uuid::Uuid::new_v4().to_string().into();assert!(retention(&bad).is_err(),"{field}");}
 append(&mut saved,Step::CmdRemoveDiscarded,StepState::Done,Some(json!({"discardId":g})));assert_eq!(retention(&saved).unwrap(),(None,None));
}
#[test] fn historical_retention_uses_the_latest_done_restore_and_ignores_unrelated_removal(){
 let (_t,_r,_l,j)=fixture();let mut saved=j.load().clone();let first=restore(&mut saved);let second=restore(&mut saved);append(&mut saved,Step::CmdRemoveDiscarded,StepState::Done,Some(json!({"discardId":first})));assert_eq!(retention(&saved).unwrap(),(Some(second.clone()),Some(second.clone())));append(&mut saved,Step::CmdRestorePg15,StepState::Failed{code:"io_error".into()},None);assert_eq!(retention(&saved).unwrap(),(Some(second.clone()),Some(second)));
}

}
