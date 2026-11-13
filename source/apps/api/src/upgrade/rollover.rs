//! A new attempt replaces the settled journal in one durable slot commit.
use super::{journal::{Candidate,JournalV1,PreviousAttempt,Record,Handoff,last_attempt_failure,moves_done},retry::{self,Admission},Branch,Outcome,Journal,Step,StepState,StepError,StorageLocks};
use crate::db_upgrade::{files,layout::Roots};
use chrono::{DateTime,Utc};
use std::{fs::{self,File},path::Path,os::unix::fs::MetadataExt};
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"이전 시도의 복구가 끝나지 않아 새 시도를 시작하지 않았습니다.",detail:None}}
fn io_error()->StepError {StepError{code:"io_error",message:"소비한 보관 목록을 정리하지 못했습니다. 복구 기록과 기존 데이터를 보존했습니다.",detail:None}}
fn command(step:Step)->bool {matches!(step,Step::CmdRestorePg15|Step::CmdRemoveBackup|Step::CmdRemoveDiscarded)}
fn present(path:&Path)->Result<bool,StepError>{match fs::symlink_metadata(path){Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(io_error())}}
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
fn fold(old:&JournalV1)->Result<PreviousAttempt,StepError>{
    let (generation,discard_id)=retention(old)?;
    let last_failure=last_attempt_failure(&old.records);
    Ok(PreviousAttempt{attempt_id:old.attempt_id.clone(),outcome:old.outcome,candidate:old.candidate.clone(),last_failure,generation,discard_id,ended_at:old.records.iter().max_by_key(|r|r.seq).ok_or_else(mismatch)?.at.clone()})
}
fn check_locks(roots:&Roots,locks:&StorageLocks,j:&Journal)->Result<(),StepError>{if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app)||!j.belongs_to(&roots.app){return Err(mismatch());}Ok(())}
fn settled(roots:&Roots,old:&JournalV1)->Result<(),StepError>{
    if old.branch!=Branch::Upgrade||matches!(old.outcome,Outcome::Upgrading|Outcome::Pg17|Outcome::ActivationFailed|Outcome::RestoreIncomplete){return Err(mismatch());}
    if !moves_done(&old.moves){return Err(mismatch());}
    for r in &old.records{if command(r.step)&&r.state==StepState::Intent&&!old.records.iter().any(|n|n.seq>r.seq&&n.step==r.step&&matches!(n.state,StepState::Done|StepState::Failed{..})){return Err(mismatch());}}
    if !super::types::canonical_generation(&old.attempt_id){return Err(mismatch());}
    for root in [".nwui-auto-work",".nwui-pg15-backup"]{let parent=roots.db.join(root);if present(&parent)?{files::real_path(&parent).map_err(|_|mismatch())?;if present(&parent.join(&old.attempt_id))?{return Err(mismatch());}}}
    let retained=retention(old)?.0;
    // Inspect only journal-owned generation paths, including a staged key before publish.
    for record in &old.records{
        let Some(data)=&record.data else{continue};
        let generation=match record.step{Step::P5Publish|Step::P5Commit=>data.get("generation"),Step::P4Target=>data.pointer("/keyWrite/intent/purpose/generation"),_=>None};
        let Some(value)=generation else{continue};let g=value.as_str().ok_or_else(mismatch)?;
        if !super::types::canonical_generation(g){return Err(mismatch());}
        let parent=roots.app.join(".nwui-auto/gen");if present(&parent)?{files::real_path(&parent).map_err(|_|mismatch())?;let p=parent.join(g);if present(&p)?{files::real_path(&p).map_err(|_|mismatch())?;if retained.as_deref()!=Some(g){return Err(mismatch());}}}
    }
    Ok(())
}
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum RolloverOutcome {RolledOver,NotAdmitted(Admission)}
#[cfg(test)]fn before_discard_checkpoint(){if std::env::var("NWUI_ROLLOVER_CLEANUP_KILL_POINT").as_deref()==Ok("before-discard"){unsafe{libc::kill(libc::getpid(),libc::SIGKILL);}}}
#[cfg(not(test))]fn before_discard_checkpoint(){}
pub(crate) async fn rollover(roots:&Roots,locks:&StorageLocks,j:&mut Journal,candidate:Candidate,label:Option<&str>,space_sufficient:bool,now:DateTime<Utc>)->Result<RolloverOutcome,StepError>{
    check_locks(roots,locks,j)?;
    // Command failures are operator diagnostics, not the attempt's retry classification.
    let mut admission=j.load().clone();admission.records.retain(|r|!command(r.step));
    match retry::admit(&admission,&candidate,label,space_sufficient,now)?{
        Admission::Attempt=>{},
        not_admitted=>return Ok(RolloverOutcome::NotAdmitted(not_admitted)),
    }
    settled(roots,j.load())?;
    cleanup_consumed_manifests(roots,locks,j)?;
    before_discard_checkpoint();
    let proof=j.discard_for_rollover(roots,locks).await?;
    let old=j.load();let previous_attempt=fold(old)?;
    let mut previous=vec![previous_attempt];previous.extend(old.previous.clone());let mut unretained=0;
    previous.retain(|p|{if p.discard_id.is_some(){true}else{unretained+=1;unretained<=8}});
    let mut retry=old.retry.clone();if let Some(label)=label{if !retry.consumed_labels.iter().any(|s|s==label){retry.consumed_labels.push(label.into());}}
    let at=now.to_rfc3339();
    let next=JournalV1{version:1,attempt_id:uuid::Uuid::new_v4().to_string(),branch:Branch::Upgrade,candidate,
        records:vec![Record{seq:1,step:Step::P0,state:StepState::Done,at:at.clone(),data:None},Record{seq:2,step:Step::P0b,state:StepState::Intent,at,data:None}],
        moves:Vec::new(),handoff:Handoff::None,outcome:Outcome::Upgrading,retry,previous};
    j.replace_attempt_locked(roots,locks,next,proof)?;
    cleanup_consumed_manifests(roots,locks,j)?;
    Ok(RolloverOutcome::RolledOver)
}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::{Branch,Outcome,Step,StepState,journal::{Handoff,JournalMove,Record,PreviousFailure},MovePhase,MoveRecord};
    use std::{fs,os::unix::fs::{PermissionsExt,symlink},os::unix::process::ExitStatusExt,path::Path,process::Command};
    use serde_json::json;
    fn now()->DateTime<Utc>{"2026-10-28T00:00:00Z".parse().unwrap()}
    fn candidate()->Candidate{Candidate{revision:"1".repeat(40),pins_sha256:"2".repeat(64),migrations_sha256:"3".repeat(64)}}
    fn roots(p:&Path)->Roots{Roots{db:p.join("db"),app:p.join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()}}
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
    #[test]fn fold_without_commands_has_no_retention(){let (_t,_r,_l,j)=fixture();let p=fold(j.load()).unwrap();assert!(p.generation.is_none()&&p.discard_id.is_none());assert_eq!(p.attempt_id,j.load().attempt_id);assert_eq!(p.outcome,j.load().outcome);assert_eq!(p.candidate,j.load().candidate);}
    #[test]fn fold_restored_generation_is_retained(){let (_t,_r,_l,j)=fixture();let mut s=j.load().clone();let g=restore(&mut s);let p=fold(&s).unwrap();assert_eq!(p.generation,Some(g.clone()));assert_eq!(p.discard_id,Some(g));}
    #[test]fn fold_later_removal_clears_only_the_named_generation(){let (_t,_r,_l,j)=fixture();let mut s=j.load().clone();let g=restore(&mut s);append(&mut s,Step::CmdRemoveDiscarded,StepState::Done,Some(json!({"discardId":uuid::Uuid::new_v4().to_string()})));assert_eq!(fold(&s).unwrap().discard_id,Some(g.clone()));append(&mut s,Step::CmdRemoveDiscarded,StepState::Done,Some(json!({"discardId":g})));let p=fold(&s).unwrap();assert!(p.discard_id.is_none()&&p.generation.is_none());}
    #[test]fn fold_uses_highest_restore_and_has_no_failure_when_only_commands_failed(){let (_t,_r,_l,j)=fixture();let mut s=j.load().clone();s.records.retain(|r|!matches!(r.state,StepState::Failed{..}));let first=restore(&mut s);let last=restore(&mut s);assert_ne!(first,last);append(&mut s,Step::CmdRemoveBackup,StepState::Failed{code:"io_error".into()},None);let p=fold(&s).unwrap();assert_eq!(p.discard_id,Some(last));assert!(p.last_failure.is_none());assert_eq!(p.ended_at,s.records.last().unwrap().at);}
    #[test]fn fold_refuses_mismatched_authority(){for bad in ["discard","commit","backup","no-done","alias"]{let (_t,_r,_l,j)=fixture();let mut s=j.load().clone();restore(&mut s);match bad{"discard"=>s.records.last_mut().unwrap().data.as_mut().unwrap()["discardId"]=json!(uuid::Uuid::new_v4().to_string()),"commit"=>s.records.iter_mut().find(|r|r.step==Step::P5Commit).unwrap().data.as_mut().unwrap()["generation"]=json!(uuid::Uuid::new_v4().to_string()),"backup"=>s.records.last_mut().unwrap().data.as_mut().unwrap()["backupId"]=json!(uuid::Uuid::new_v4().to_string()),"no-done"=>s.records.retain(|r|!(r.step==Step::P5Commit&&r.state==StepState::Done)),_=>s.records.last_mut().unwrap().data.as_mut().unwrap()["generation"]=json!("../escape")};assert_eq!(fold(&s).unwrap_err().code,"pair_mismatch","{bad}");}}
    #[test]fn fold_ignores_later_command_failure_and_uses_last_record_time(){let (_t,_r,_l,j)=fixture();let mut s=j.load().clone();append(&mut s,Step::CmdRemoveBackup,StepState::Failed{code:"io_error".into()},None);let p=fold(&s).unwrap();assert_eq!(p.last_failure,Some(PreviousFailure{step:Step::P2Preflight,code:"ledger_unknown".into()}));assert_eq!(p.ended_at,s.records.last().unwrap().at);assert_eq!(p,fold(&s).unwrap());}
    #[tokio::test]async fn deterministic_same_candidate_is_refused_without_mutation(){let (_t,r,l,mut j)=fixture();let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(matches!(rollover(&r,&l,&mut j,candidate(),None,true,now()).await.unwrap(),RolloverOutcome::NotAdmitted(Admission::SameCandidateRefused)));assert_eq!(before,fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap());assert_eq!(j.load().records.last().unwrap().state,StepState::Failed{code:"ledger_unknown".into()});}
    #[tokio::test]async fn changed_candidate_admits_but_used_label_does_not(){let (_t,r,l,mut j)=fixture();let mut s=j.load().clone();s.retry.consumed_labels.push("used".into());j=save(&r,&l,&s);assert_eq!(rollover(&r,&l,&mut j,candidate(),Some("used"),true,now()).await.unwrap(),RolloverOutcome::NotAdmitted(Admission::SameCandidateRefused));let mut c=candidate();c.revision="f".repeat(40);assert_eq!(rollover(&r,&l,&mut j,c.clone(),None,true,now()).await.unwrap(),RolloverOutcome::RolledOver);assert_eq!(j.load().candidate,c);assert_eq!(j.load().retry.consumed_labels,["used"]);}
    #[tokio::test]async fn later_command_failure_does_not_bypass_deterministic_admission(){let (_t,r,l,mut j)=fixture();j.record(Step::CmdRemoveBackup,StepState::Failed{code:"io_error".into()},None).unwrap();let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(matches!(rollover(&r,&l,&mut j,candidate(),None,true,now()).await.unwrap(),RolloverOutcome::NotAdmitted(Admission::SameCandidateRefused)));assert_eq!(before,fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap());}
    #[tokio::test]async fn fresh_frame_drops_old_steps_and_consumes_label_once(){let (_t,r,l,mut j)=fixture();let mut s=j.load().clone();for step in [Step::P1Backup,Step::Handoff]{append(&mut s,step,StepState::Done,None);}
        let selected=crate::upgrade::StorageSelection{version:1,epoch:1,outcome:Outcome::Upgrading,branch:Branch::Upgrade,pgdata:r.db.join(".nwui-auto-work").join(&s.attempt_id).join("data"),backup_id:Some(s.attempt_id.clone()),schema_profile:"legacy-28".into(),key_binding:crate::upgrade::KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()}};
        let first=s.records.last().unwrap().seq+1;s.records.extend(crate::nonroot::fixture_prepare_records(&selected,&s.attempt_id,first,true));s.handoff=Handoff::Done;j=save(&r,&l,&s);let old=j.load().attempt_id.clone();manifest(&r,&old);
        rollover(&r,&l,&mut j,candidate(),Some("operator"),true,now()).await.unwrap();let n=j.load();assert_ne!(n.attempt_id,old);assert_eq!(uuid::Uuid::parse_str(&n.attempt_id).unwrap().get_version(),Some(uuid::Version::Random));assert_eq!(n.records.len(),2);assert_eq!((n.records[0].step,n.records[1].step),(Step::P0,Step::P0b));assert_eq!(n.records[0].state,StepState::Done);assert_eq!(n.records[1].state,StepState::Intent);assert_eq!(n.handoff,Handoff::None);assert_eq!(n.branch,Branch::Upgrade);assert_eq!(n.outcome,Outcome::Upgrading);assert!(n.moves.is_empty());assert_eq!(n.previous[0].attempt_id,old);assert_eq!(n.retry.consumed_labels,["operator"]);assert!(!r.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());assert!(rollover(&r,&l,&mut j,candidate(),Some("operator"),true,now()).await.is_err());}
    #[tokio::test]async fn interruptions_and_backoff_are_carried_verbatim(){let (_t,r,l,mut j)=fixture();j.record_retry_failure(Step::P1Backup,"interrupted",now()).unwrap();j.record_retry_failure(Step::P1Backup,"interrupted",now()).unwrap();let retry=j.load().retry.clone();let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(matches!(rollover(&r,&l,&mut j,candidate(),None,true,now()).await.unwrap(),RolloverOutcome::NotAdmitted(Admission::Backoff(_))));assert_eq!(before,fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap());rollover(&r,&l,&mut j,candidate(),Some("override"),true,now()).await.unwrap();assert_eq!(j.load().retry.interruptions,retry.interruptions);assert_eq!(j.load().retry.backoff_until,retry.backoff_until);}
    #[tokio::test]async fn retained_discard_and_generation_remain_named_after_rollover(){let (_t,r,l,mut j)=fixture();let mut s=j.load().clone();let g=restore(&mut s);fs::create_dir_all(r.app.join(".nwui-auto/gen").join(&g)).unwrap();fs::create_dir_all(r.db.join(".nwui-discard").join(&g)).unwrap();j=save(&r,&l,&s);rollover(&r,&l,&mut j,candidate(),Some("keep"),true,now()).await.unwrap();assert_eq!(j.load().previous[0].generation.as_deref(),Some(g.as_str()));assert_eq!(j.load().previous[0].discard_id.as_deref(),Some(g.as_str()));assert!(r.app.join(".nwui-auto/gen").join(&g).is_dir());assert!(r.db.join(".nwui-discard").join(&g).is_dir());
        let online=super::super::refusal::status_value(&r,j.load(),j.load().outcome,&candidate(),None).unwrap();let offline=super::super::commands::offline_status(&r,&candidate(),now()).unwrap().unwrap();assert_eq!(online.discarded.len(),1);assert_eq!(online.discarded[0].id,g);assert_eq!(online.discarded[0].remove_command,format!("nwui-db-upgrade remove-discarded --confirm {g}"));assert_eq!(serde_json::to_value(&online.discarded).unwrap(),serde_json::to_value(&offline.discarded).unwrap());
        let mut s=j.load().clone();s.previous[0].generation=None;s.previous[0].discard_id=None;append(&mut s,Step::CmdRemoveDiscarded,StepState::Done,Some(json!({"discardId":g})));assert!(save(&r,&l,&s).load().previous[0].discard_id.is_none());}
    #[tokio::test]async fn unsettled_moves_paths_outcomes_and_commands_do_not_roll(){for bad in ["move","backup","work","gen","gen-link","command","upgrading","pg17","activation","restore"]{let (_t,r,l,mut j)=fixture();let mut s=j.load().clone();match bad{"move"=>s.moves.push(JournalMove{seq:4,record:MoveRecord{phase:MovePhase::Restore,entry:"base".into(),from:"BK".into(),to:"L".into(),state:StepState::Intent}}),"backup"=>fs::create_dir_all(r.db.join(".nwui-pg15-backup").join(&s.attempt_id)).unwrap(),"work"=>fs::create_dir_all(r.db.join(".nwui-auto-work").join(&s.attempt_id)).unwrap(),"gen"|"gen-link"=>{let g=uuid::Uuid::new_v4().to_string();append(&mut s,Step::P5Publish,StepState::Intent,Some(json!({"authorityEvent":"generation-intent","generation":g})));let p=r.app.join(".nwui-auto/gen");fs::create_dir(&p).unwrap();if bad=="gen"{fs::create_dir(p.join(g)).unwrap()}else{symlink("/missing",p.join(g)).unwrap()}},"command"=>append(&mut s,Step::CmdRemoveBackup,StepState::Intent,None),"upgrading"=>s.outcome=Outcome::Upgrading,"pg17"=>s.outcome=Outcome::Pg17,"activation"=>s.outcome=Outcome::ActivationFailed,_=>s.outcome=Outcome::RestoreIncomplete};j=save(&r,&l,&s);manifest(&r,&s.attempt_id);let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(rollover(&r,&l,&mut j,candidate(),Some("must-not-consume"),true,now()).await.is_err(),"{bad}");assert_eq!(before,fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),"{bad}");}}
    #[tokio::test]async fn completed_move_intent_done_pair_is_settled(){let (_t,r,l,mut j)=fixture();let m=MoveRecord{phase:MovePhase::Restore,entry:"base".into(),from:"BK".into(),to:"L".into(),state:StepState::Intent};j.record_move(m.clone()).unwrap();let mut done=m;done.state=StepState::Done;j.record_move(done).unwrap();rollover(&r,&l,&mut j,candidate(),Some("settled"),true,now()).await.unwrap();assert!(j.load().moves.is_empty());}
    #[tokio::test]async fn finished_command_is_foldable_and_later_cmd_failure_is_not_attempt_failure(){let (_t,r,l,mut j)=fixture();j.record(Step::CmdRemoveBackup,StepState::Intent,None).unwrap();j.record(Step::CmdRemoveBackup,StepState::Failed{code:"io_error".into()},None).unwrap();let before=fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap();assert!(rollover(&r,&l,&mut j,candidate(),Some("retry"),true,now()).await.is_err());assert_eq!(fs::read(r.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);j.record(Step::CmdRemoveBackup,StepState::Done,None).unwrap();j.record(Step::CmdRemoveBackup,StepState::Failed{code:"io_error".into()},None).unwrap();rollover(&r,&l,&mut j,candidate(),Some("retry"),true,now()).await.unwrap();assert_eq!(j.load().previous[0].last_failure.as_ref().unwrap().code,"ledger_unknown");}
    #[tokio::test]async fn previous_pruning_keeps_all_retained_and_newest_eight_others(){let (_t,r,l,mut j)=fixture();let mut s=j.load().clone();let p=PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:Outcome::Restored,candidate:candidate(),last_failure:None,generation:None,discard_id:None,ended_at:now().to_rfc3339()};for i in 0..12{let mut p=p.clone();p.attempt_id=uuid::Uuid::new_v4().to_string();if i%3==0{let g=uuid::Uuid::new_v4().to_string();p.generation=Some(g.clone());p.discard_id=Some(g);}s.previous.push(p);}let retained=s.previous.iter().filter(|p|p.discard_id.is_some()).cloned().collect::<Vec<_>>();j=save(&r,&l,&s);rollover(&r,&l,&mut j,candidate(),Some("prune"),true,now()).await.unwrap();assert_eq!(j.load().previous.iter().filter(|p|p.discard_id.is_none()).count(),8);for p in retained{assert!(j.load().previous.contains(&p));}assert_eq!(j.load().previous[1],s.previous[0]);assert!(!j.load().previous.iter().any(|p|p.attempt_id==s.previous.last().unwrap().attempt_id));}
    #[test]fn resume_unlinks_only_consumed_manifest_names(){let (_t,r,l,j)=fixture();let mut s=j.load().clone();let p=PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:Outcome::Restored,candidate:candidate(),last_failure:None,generation:None,discard_id:None,ended_at:now().to_rfc3339()};manifest(&r,&p.attempt_id);manifest(&r,&s.attempt_id);let consumed=r.app.join(format!(".nwui-auto/backup-{}.jsonl",p.attempt_id));s.previous.push(p);let j=save(&r,&l,&s);cleanup_consumed_manifests(&r,&l,&j).unwrap();assert!(!consumed.exists());assert!(r.app.join(format!(".nwui-auto/backup-{}.jsonl",s.attempt_id)).exists());cleanup_consumed_manifests(&r,&l,&j).unwrap();}
    #[test]fn manifest_cleanup_refuses_symlink_and_wrong_locks(){let (_t,r,l,j)=fixture();let mut s=j.load().clone();let mut p=fold(j.load()).unwrap();p.attempt_id=uuid::Uuid::new_v4().to_string();let path=r.app.join(format!(".nwui-auto/backup-{}.jsonl",p.attempt_id));s.previous.push(p);let j=save(&r,&l,&s);let outside=r.app.join("sentinel");fs::write(&outside,b"keep").unwrap();symlink(&outside,&path).unwrap();assert!(cleanup_consumed_manifests(&r,&l,&j).is_err());assert_eq!(fs::read(&outside).unwrap(),b"keep");let (_t2,_r2,l2,_j2)=fixture();assert!(cleanup_consumed_manifests(&r,&l2,&j).is_err());}
    #[test]
    fn precommit_cleanup_precedes_pruning_and_never_unlinks_the_current_manifest() {
        let (t,r,l,j)=fixture();let mut state=j.load().clone();let current=state.attempt_id.clone();
        for _ in 0..8 {state.previous.push(PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:Outcome::Restored,candidate:candidate(),last_failure:None,generation:None,discard_id:None,ended_at:now().to_rfc3339()});}
        let oldest=state.previous.last().unwrap().attempt_id.clone();manifest(&r,&oldest);manifest(&r,&current);
        let current_path=r.app.join(format!(".nwui-auto/backup-{current}.jsonl"));let current_bytes=fs::read(&current_path).unwrap();
        let j=save(&r,&l,&state);let journal_path=r.app.join(".nwui-auto/journal-v1.json");let before=fs::read(&journal_path).unwrap();drop(j);drop(l);
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::rollover::tests::kill_child","--nocapture"])
            .env("NWUI_ROLLOVER_CHILD_ROOT",t.path()).env("NWUI_ROLLOVER_CLEANUP_KILL_POINT","before-discard").status().unwrap();
        assert_eq!(status.signal(),Some(libc::SIGKILL));assert_eq!(fs::read(&journal_path).unwrap(),before);assert_eq!(fs::read(&current_path).unwrap(),current_bytes);
        assert!(!r.app.join(format!(".nwui-auto/backup-{oldest}.jsonl")).exists());
        let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&l).unwrap();
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(rollover(&r,&l,&mut j,candidate(),Some("kill-label"),true,now())).unwrap();
        assert_eq!(j.load().previous.len(),8);assert!(!j.load().previous.iter().any(|p|p.attempt_id==oldest));assert_eq!(j.load().previous[0].attempt_id,current);
    }
    #[test]fn real_sigkill_at_all_slot_windows_preserves_one_authority_and_label(){for point in ["slot-written","slot-fsynced","journal-renamed","journal-durable"]{let (t,r,l,j)=fixture();let old=j.load().attempt_id.clone();manifest(&r,&old);drop(j);drop(l);let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::rollover::tests::kill_child","--nocapture"]).env("NWUI_ROLLOVER_CHILD_ROOT",t.path()).env("NWUI_JOURNAL_KILL_POINT",point).status().unwrap();assert_eq!(status.signal(),Some(libc::SIGKILL),"{point}");let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&l).unwrap();assert!(r.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());let before_rename=matches!(point,"slot-written"|"slot-fsynced");if before_rename{assert_eq!(j.load().attempt_id,old);assert!(j.load().retry.consumed_labels.is_empty());tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(rollover(&r,&l,&mut j,candidate(),Some("kill-label"),true,now())).unwrap();}else{assert_ne!(j.load().attempt_id,old);assert_eq!(j.load().records.len(),2);}assert_eq!(j.load().retry.consumed_labels,["kill-label"]);cleanup_consumed_manifests(&r,&l,&j).unwrap();assert!(!r.app.join(format!(".nwui-auto/backup-{old}.jsonl")).exists());}}
    #[tokio::test]async fn kill_child(){let Some(p)=std::env::var_os("NWUI_ROLLOVER_CHILD_ROOT")else{return};let r=roots(Path::new(&p));let l=StorageLocks::acquire(&r.db,&r.app).unwrap();let mut j=Journal::open(&r.app,&l).unwrap();rollover(&r,&l,&mut j,candidate(),Some("kill-label"),true,now()).await.unwrap();panic!("kill checkpoint did not fire");}
}
