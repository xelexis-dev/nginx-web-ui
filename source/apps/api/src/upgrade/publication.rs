//! Finite pre-handoff P5: prepared authority, cold target rename, legacy guard, selector commit.
use super::{Branch,Journal,Step,StepError,StorageLocks,StorageSelection,KeyBinding,generation::{self,GenerationDraft},selector::Origin};
use crate::db_upgrade::{layout::Roots,postgres};
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"전환할 대상과 실행 준비 기록이 일치하지 않습니다. 모든 위치를 보존했습니다.",detail:None}}
/// Await on a blocking thread under custody. Caller has finished V2/S4 and stopped private PG17.
/// After this returns, committed::resume publishes the key/flag before S4 activation/readiness.
/// On error, a physical selector still means committed: never choose R-restore from the error alone.
pub(crate) fn publish_upgrade_locked(roots:&Roots,locks:&StorageLocks,j:&mut Journal,candidate:&StorageSelection,draft:GenerationDraft<'_>,phase:&super::runner::Phase1Step)->Result<StorageSelection,StepError> {
    phase.set(super::Step::P5Publish);let work=roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("data");
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app) || j.load().branch!=Branch::Upgrade || draft.origin!=Origin::Auto
        || candidate.version!=1 || candidate.epoch==0 || candidate.branch!=Branch::Upgrade || candidate.pgdata!=work || candidate.backup_id.as_deref()!=Some(j.load().attempt_id.as_str())
        || draft.source_profile.map(|(id,_)|id)!=Some(candidate.schema_profile.as_str()) || !matches!(&candidate.key_binding,KeyBinding::Verified{pair_id}if pair_id==&draft.db_pair.pair_id) {return Err(mismatch());}
    let current=Journal::open(&roots.app,locks)?;if serde_json::to_value(current.load()).map_err(|_|mismatch())?!=serde_json::to_value(j.load()).map_err(|_|mismatch())?{return Err(mismatch());}
    let prepared=generation::completed_preparation(j).ok_or_else(mismatch)?;
    if prepared.key_receipt.as_ref().is_some_and(|receipt|receipt.selection_epoch!=candidate.epoch){return Err(mismatch());}
    let control=postgres::read_control(17,&work).map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;control.require_cold().map_err(|_|mismatch())?;
    if control.system_identifier!=draft.actual_id{return Err(mismatch());}
    match std::fs::symlink_metadata(work.join("postmaster.pid")){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}
    let key=draft.key;let actual_id=draft.actual_id;let mut failing=Step::P5Publish;
    let result:Result<StorageSelection,StepError>=(|| {
        super::runner::publish_step(roots,j,Step::P5Publish,0,0,None,false);let selected=generation::prepare(roots,locks,j,draft)?;
        super::target::publish(roots,locks,j,actual_id)?;super::runner::publish_step(roots,j,Step::P5Publish,0,0,None,false);
        failing=Step::P5Guard;phase.set(failing);super::runner::publish_step(roots,j,failing,0,0,None,false);super::guard::install(roots,locks,j,&selected)?;super::runner::publish_step(roots,j,failing,0,0,None,false);
        failing=Step::P5Commit;phase.set(failing);super::runner::publish_step(roots,j,failing,0,0,None,false);generation::commit(roots,locks,j,&selected,actual_id,key)?;super::runner::publish_step(roots,j,failing,0,0,None,false);
        let mut final_selection=candidate.clone();final_selection.outcome=super::Outcome::Pg17;final_selection.pgdata=roots.db.join("17/data");final_selection.backup_id=selected.backup_id;final_selection.schema_profile=selected.profile_id.ok_or_else(mismatch)?;Ok(final_selection)
    })();
    if let Err(error)=&result{j.record_retry_failure(failing,error.code,chrono::Utc::now())?;}result
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::StepState;
    use crate::db_upgrade::{files,key_file::ValidatedKeyFile,pair::Pair};
    use std::{fs,os::unix::fs::PermissionsExt};
    #[test]
    fn w1_p5_refuses_bad_selection_missing_preparation_and_stale_journal_before_tools_or_mutation() {
        for case in ["epoch","path","pair","profile","backup","preparation","stale"] {
            let t=tempfile::tempdir().unwrap();let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for path in [&roots.db,&roots.app]{fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
            let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j={let fixture_journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,super::super::journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();if fixture_journal.load().branch==crate::upgrade::Branch::Upgrade{fixture_journal.ensure_app_pool().unwrap();}fixture_journal};
            let mut pair=Pair::fresh("arm64");pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some("123".into());
            let mut candidate=StorageSelection{version:1,epoch:1,outcome:super::super::Outcome::Upgrading,branch:Branch::Upgrade,pgdata:roots.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("data"),backup_id:Some(j.load().attempt_id.clone()),schema_profile:"source".into(),key_binding:KeyBinding::Verified{pair_id:pair.pair_id.clone()}};
            match case {"epoch"=>candidate.epoch=0,"path"=>candidate.pgdata=roots.db.join("foreign"),"pair"=>candidate.key_binding=KeyBinding::Unverified,"profile"=>candidate.schema_profile="other".into(),"backup"=>candidate.backup_id=None,"stale"=>{let mut current=Journal::open(&roots.app,&locks).unwrap();current.record(Step::P0b,StepState::Intent,None).unwrap();},_=>{}}
            let generation=uuid::Uuid::new_v4().to_string();let source=super::super::KeySource::Canonical;let key=ValidatedKeyFile{secrets:crate::crypto::SecretBox::from_key([1;32]),bytes:hex::encode([1u8;32]).into_bytes()};
            let draft=GenerationDraft{generation:&generation,origin:Origin::Auto,source_profile:Some(("source",&"1".repeat(64))),actual_id:"123",db_pair:&pair,app_pair:&pair,agreement:&KeyBinding::Verified{pair_id:pair.pair_id.clone()},key_source:&source,key:&key};
            let before=(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap());let records=j.load().records.len();assert!(publish_upgrade_locked(&roots,&locks,&mut j,&candidate,draft,&super::super::runner::Phase1Step::new(Step::P5Publish)).is_err(),"{case}");assert_eq!(j.load().records.len(),records);assert_eq!(before,(files::snapshot(&roots.db).unwrap(),files::snapshot(&roots.app).unwrap()));
        }
    }
}
