//! F1 finishes through the existing fresh-pair intent before the runner journal exists.
use super::{Branch,Journal,Step,StepError,StepState,StorageLocks,journal::Candidate};
use crate::db_upgrade::{fresh,layout::{self,Decision,Roots},pair::Pair,postgres};
use std::{path::Path,sync::Arc};
fn map(e:crate::db_upgrade::error::Error)->StepError {StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)}}
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"새 저장소 준비 결과와 실제 데이터가 일치하지 않습니다. 기존 내용을 보존했습니다.",detail:None}}
fn identifier(path:&Path)->crate::db_upgrade::error::Result<String> {
    let control=postgres::read_control(17,path)?;control.require_cold()?;Ok(control.system_identifier)
}
/// Caller keeps custody.during alive until this blocking task and its private server finish.
/// envdir is the fixed root-owned s6 environment directory in the image.
pub(crate) async fn provision(roots:Roots,locks:Arc<StorageLocks>,candidate:Candidate,inputs:fresh::Inputs,envdir:std::path::PathBuf)->Result<(Journal,Pair),StepError> {
    let runtime=tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move|| {
        if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app){return Err(mismatch());}
        // No coordinator allocation before F1. A pre-existing journal belongs to resume instead.
        match std::fs::symlink_metadata(roots.app.join(".nwui-auto")) {Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(mismatch())}
        match layout::classify_locked(&locks,&roots,&identifier).map_err(map)? {Decision::Empty|Decision::Fresh(_)=>{},Decision::Ready(_)=>return Err(mismatch())}
        fresh::secrets_locked(&locks,&roots,&inputs,&envdir).map_err(map)?;
        runtime.block_on(fresh::provision_locked(&locks,&roots)).map_err(map)?;
        let Decision::Ready(pair)=layout::classify_locked(&locks,&roots,&identifier).map_err(map)? else{return Err(mismatch())};
        if pair.mode!="fresh" || !pair.ready || pair.fresh_step.is_some(){return Err(mismatch());}
        let mut journal=Journal::begin(&roots.app,&locks,Branch::Fresh,candidate)?;
        journal.record(Step::F1,StepState::Done,None)?;
        Ok((journal,pair))
    }).await.map_err(|_|StepError{code:"fresh_failed",message:"새 저장소 준비를 완료하지 못했습니다. 준비 중인 데이터는 보존했습니다.",detail:None})?
}
#[cfg(test)]
mod tests {
    use super::*;use std::{fs,os::unix::fs::{PermissionsExt,DirBuilderExt}};
    fn fixture()->(tempfile::TempDir,Roots,Arc<StorageLocks>,Candidate) {
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o711)).unwrap();
        let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::paths::architecture().unwrap().into()};
        for p in [&roots.db,&roots.app]{fs::DirBuilder::new().mode(0o700).create(p).unwrap();}
        let locks=Arc::new(StorageLocks::acquire(&roots.db,&roots.app).unwrap());let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};(t,roots,locks,candidate)
    }
    #[tokio::test]
    async fn w1_f1_refuses_existing_coordinator_before_material_or_env_publication() {
        let (t,roots,locks,candidate)=fixture();fs::create_dir(roots.app.join(".nwui-auto")).unwrap();fs::write(roots.app.join(".nwui-auto/foreign"),b"keep").unwrap();
        let env=t.path().join("env");fs::create_dir(&env).unwrap();let before=crate::db_upgrade::files::snapshot(&roots.app).unwrap();
        assert_eq!(provision(roots.clone(),locks,candidate,fresh::Inputs::default(),env.clone()).await.err().unwrap().code,"pair_mismatch");assert_eq!(crate::db_upgrade::files::snapshot(&roots.app).unwrap(),before);assert!(fs::read_dir(env).unwrap().next().is_none());assert!(fs::read_dir(roots.db).unwrap().next().is_none());
    }
    #[tokio::test]
    #[ignore="native PG17: disposable roots and local initdb/private server"]
    async fn w1_f1_native_ready_pair_precedes_runner_journal_and_keeps_both_locks() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        let (t,roots,locks,candidate)=fixture();let env=t.path().join("env");
        // An F1 interruption uses only its own pair intent; no runner coordinator exists yet.
        assert!(provision(roots.clone(),locks.clone(),candidate.clone(),fresh::Inputs{auth_secret:Some("fixture-auth-only".into()),admin_password:None},env.clone()).await.is_err());
        assert!(!roots.app.join(".nwui-auto").exists());assert!(!roots.app.join("secrets/data_key").exists());
        let Decision::Fresh(partial)=layout::classify_locked(&locks,&roots,&identifier).unwrap() else{panic!("fresh intent")};assert!(!partial.ready);
        fs::DirBuilder::new().mode(0o700).create(&env).unwrap();
        let (j,pair)=provision(roots.clone(),locks.clone(),candidate,fresh::Inputs{auth_secret:Some("fixture-auth-only".into()),admin_password:None},env.clone()).await.unwrap();
        assert!(pair.ready && pair.mode=="fresh" && pair.fresh_step.is_none());assert!(!pair.encryption_key_required);assert!(!roots.app.join("secrets/data_key").exists());
        assert_eq!(j.load().branch,Branch::Fresh);assert_eq!(j.load().records.len(),1);assert_eq!(j.load().records[0].step,Step::F1);assert_eq!(j.load().records[0].state,StepState::Done);
        assert_eq!(fs::read(env.join("AUTH_SECRET")).unwrap(),b"fixture-auth-only");assert!(fs::metadata(env.join("DATABASE_URL")).unwrap().len()>0);assert!(!env.join("ADMIN_PASSWORD").exists());
        let Decision::Ready(saved)=layout::classify_locked(&locks,&roots,&identifier).unwrap() else{panic!("ready pair")};assert_eq!(saved,pair);
        assert!(fs::File::open(&roots.db).unwrap().try_lock().is_err());assert!(fs::File::open(&roots.app).unwrap().try_lock().is_err());assert!(!roots.db.join("17/data/postmaster.pid").exists());
        let before=crate::db_upgrade::files::snapshot(&roots.app).unwrap();assert_eq!(provision(roots.clone(),locks,candidate_copy(&j),fresh::Inputs::default(),env).await.err().unwrap().code,"pair_mismatch");assert_eq!(crate::db_upgrade::files::snapshot(&roots.app).unwrap(),before);
    }
    fn candidate_copy(j:&Journal)->Candidate {j.load().candidate.clone()}
}
