//! Runner restart decisions and the pre-readiness cleanup of its own /run records.
use super::{Outcome,Step,StepError,StepState,journal::{Handoff,JournalV1}};
use std::{fs::OpenOptions,os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}},path::Path};
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum BootPlan {Phase1,Phase2,ParkedRefusal}
#[derive(Clone)]
pub(crate) struct Phase1Step(std::sync::Arc<std::sync::Mutex<Step>>);
impl Phase1Step {
    pub(super) fn new(step:Step)->Self{Self(std::sync::Arc::new(std::sync::Mutex::new(step)))}
    pub(super) fn set(&self,step:Step){*self.0.lock().unwrap_or_else(|error|error.into_inner())=step;}
    fn get(&self)->Step{*self.0.lock().unwrap_or_else(|error|error.into_inner())}
}
/// Phase-1 resume must consume old manifests before retry admission can prune their owners.
#[cfg(test)]
pub(crate) fn resume_phase1_locked(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks)->Result<super::Journal,StepError> {
    let journal=super::Journal::open(&roots.app,locks)?;
    super::rollover::cleanup_consumed_manifests(roots,locks,&journal)?;
    Ok(journal)
}
/// Container-start boundary only. Finish old recovery before manifest cleanup or admission.
/// Some(admission) parks an old refusal. Re-evaluate boot_plan after None: recovery may have parked this boot.
pub(crate) async fn resume_before_admission(roots:crate::db_upgrade::layout::Roots,locks:std::sync::Arc<super::StorageLocks>,mut j:super::Journal,candidate:super::journal::Candidate,label:Option<String>,space_sufficient:bool,now:chrono::DateTime<chrono::Utc>,identity:Option<String>)->Result<(super::Journal,Option<super::retry::Admission>),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app){return Err(preparation_error());}
    if identity.as_deref().is_some_and(|id|super::refusal::outcome(j.load(),id).is_some()){return Ok((j,None));}
    let r=roots.clone();let l=locks.clone();
    let committed=tokio::task::spawn_blocking(move|| {
        if !l.matches_roots(&r.db,&r.app){return Err(preparation_error());}
        super::replacement::is_committed(&r)
    }).await.map_err(|_|preparation_error())??;
    if committed{j=super::committed::resume(roots.clone(),locks.clone(),j).await?;}
    let backed=j.load().records.iter().any(|r|r.step==Step::P1Backup && r.state==StepState::Done);
    if j.load().branch==super::Branch::Upgrade && !committed && (j.load().outcome==Outcome::RestoreIncomplete || j.load().outcome==Outcome::Upgrading && backed) {
        let failure=super::journal::last_attempt_failure(&j.load().records).unwrap_or_else(||super::journal::PreviousFailure{step:j.load().records.iter().rev().find(|r|!matches!(r.step,Step::CmdRestorePg15|Step::CmdRemoveBackup|Step::CmdRemoveDiscarded)).map(|r|r.step).unwrap_or(Step::P2Preflight),code:"interrupted".into()});
        j=super::restore::recover_with_context(roots.clone(),locks.clone(),j,failure.step,&failure.code,identity.clone(),candidate.clone(),label.clone()).await?;
        // This boot has completed its recovery attempt; its atomic receipt parks it, never admission.
        return Ok((j,None));
    }
    // This also runs for active/committed journals, before any future fold can prune ownership.
    let r=roots.clone();let l=locks.clone();j=tokio::task::spawn_blocking(move||{super::rollover::cleanup_consumed_manifests(&r,&l,&j)?;Ok(j)}).await.map_err(|_|preparation_error())??;
    if j.load().branch!=super::Branch::Upgrade || committed || matches!(j.load().outcome,Outcome::Upgrading|Outcome::Pg17|Outcome::ActivationFailed|Outcome::RestoreIncomplete){return Ok((j,None));}
    let admission=match super::rollover::rollover(&roots,&locks,&mut j,candidate,label.as_deref(),space_sufficient,now).await? {
        super::rollover::RolloverOutcome::RolledOver=>None,
        super::rollover::RolloverOutcome::NotAdmitted(admission)=>Some(admission),
    };
    Ok((j,admission))
}

/// Missing identity (including a read error) must never skip phase 1.
pub(crate) fn boot_plan(j:&JournalV1,identity:Option<&str>)->BootPlan {
    let valid=identity.filter(|v|v.len()==64 && v.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    if valid.is_some_and(|id|super::refusal::outcome(j,id).is_some()){return BootPlan::ParkedRefusal;}
    let previous=j.records.iter().rev().find(|r|r.step==Step::Handoff);
    if j.handoff==Handoff::Done && valid.is_some()
        && previous.is_some_and(|r|r.state==StepState::Done && r.data.as_ref().and_then(|v|v.get("bootIdentity")).and_then(|v|v.as_str())==valid) {BootPlan::Phase2}else{BootPlan::Phase1}
}
/// Re-open under custody: the first read has no lock and is only a D-lock acquisition hint.
pub(crate) struct Startup {
    pub roots:crate::db_upgrade::layout::Roots,
    pub custody:super::custody::Custody,
    pub journal:Option<super::Journal>,
    pub identity:Option<String>,
    pub plan:BootPlan,
}
fn startup_error(code:&'static str,message:&'static str)->StepError {StepError{code,message,detail:None}}
fn initial_journal(app:&Path)->Result<Option<JournalV1>,StepError> {
    match std::fs::symlink_metadata(app.join(".nwui-auto/journal-v1.json")) {
        Ok(_)=>{
            let journal=super::journal::read_only(app)?;
            if !super::journal::commands_settled(&journal.records){return Err(startup_error("pair_mismatch","오프라인 관리 작업이 완료되지 않았습니다. 해당 restore-pg15 또는 remove 명령을 다시 실행해 완료하세요. 기존 데이터를 보존했습니다."));}
            Ok(Some(journal))
        },
        Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),
        Err(_)=>Err(startup_error("io_error","복구 기록을 읽지 못했습니다. 기존 데이터를 보존했습니다.")),
    }
}
fn same_plan(before:Option<&JournalV1>,after:Option<&JournalV1>,identity:Option<&str>)->Result<BootPlan,StepError> {
    let plan=|j:Option<&JournalV1>|j.map(|j|boot_plan(j,identity)).unwrap_or(BootPlan::Phase1);
    if plan(before)!=plan(after) || before.map(|j|j.attempt_id.as_str())!=after.map(|j|j.attempt_id.as_str()) {
        return Err(startup_error("storage_busy","잠금을 확보하는 동안 복구 기록이 변경되었습니다. 다시 시작해 확인하세요."));
    }
    Ok(plan(after))
}
/// P0/P0b run before F1 can create runtime secrets.
pub(crate) async fn start()->Result<Startup,StepError> {
    super::status::log_storage_warning();
    let roots=crate::db_upgrade::layout::Roots {
        db:crate::db_upgrade::paths::PGROOT.into(),app:crate::db_upgrade::paths::APPROOT.into(),
        architecture:crate::db_upgrade::engine::architecture().map_err(|e|startup_error(e.code,e.message))?.into(),
    };
    let read_roots=roots.clone();
    let (before,identity)=tokio::task::spawn_blocking(move|| {
        crate::db_upgrade::paths::validate_env().map_err(|e|startup_error(e.code,e.message))?;
        crate::db_upgrade::paths::startup_data_key_file(&read_roots.app).map_err(|e|startup_error(e.code,e.message))?;
        super::preflight::startup_ports()?;
        crate::db_upgrade::layout::validate_roots_unlocked(&read_roots).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})?;
        Ok::<_,StepError>((initial_journal(&read_roots.app)?,super::boot_identity::current().ok()))
    }).await.map_err(|_|startup_error("io_error","시작 전 검사를 완료하지 못했습니다."))??;
    let need_d=before.as_ref().map(|j|boot_plan(j,identity.as_deref())!=BootPlan::Phase2).unwrap_or(true);
    let mut custody=super::custody::Custody::acquire(roots.clone(),need_d).await?;
    let locks=custody.locks.clone();let owned_roots=roots.clone();let saved=before.clone();let boot=identity.clone();
    let (journal,plan)=custody.during(async move {
        tokio::task::spawn_blocking(move|| {
            let current=initial_journal(&owned_roots.app)?;
            let plan=same_plan(saved.as_ref(),current.as_ref(),boot.as_deref())?;
            let journal=if current.is_some(){Some(super::Journal::open(&owned_roots.app,&locks)?)}else{None};
            if plan==BootPlan::Phase1 {clear_runtime_records()?;} else {strict_roots(&owned_roots)?;}
            Ok((journal,plan))
        }).await.map_err(|_|startup_error("io_error","잠금 아래의 복구 기록 확인을 완료하지 못했습니다."))?
    }).await?;
    Ok(Startup{roots,custody,journal,identity,plan})
}
/// R4-4: resume, parked and Phase2 boots keep 2.0.0's strict root check under the locks;
/// only Phase1 normalizes (storage_modes), then its own gates check the roots strictly.
pub(super) fn strict_roots(roots:&crate::db_upgrade::layout::Roots)->Result<(),StepError> {
    crate::db_upgrade::layout::validate_roots(roots).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})
}
fn unsafe_run()->StepError {StepError {code:"io_error",message:"이전 실행의 상태 파일을 안전하게 정리하지 못했습니다. 서비스 시작을 보류했습니다.",detail:Some("runner_runtime_records".into())}}
/// Only these two root-owned regular leaves belong to W1. No wildcard or recursive removal.
pub(crate) fn clear_runtime_records()->Result<(),StepError> {clear_runtime_records_at(Path::new("/run"))}
fn clear_runtime_records_at(root:&Path)->Result<(),StepError> {
    crate::db_upgrade::files::real_path(root).map_err(|_|unsafe_run())?;
    let directory=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_DIRECTORY).open(root).map_err(|_|unsafe_run())?;
    let m=directory.metadata().map_err(|_|unsafe_run())?;
    if !m.is_dir() || m.uid()!=0 || m.mode()&0o022!=0 {return Err(unsafe_run());}
    let names=[c"nwui-storage.json",c"nwui-db-upgrade.json"];
    // Validate both first so an unsafe second leaf does not erase the first diagnostic.
    for name in names {
        let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe{libc::fstatat(directory.as_raw_fd(),name.as_ptr(),stat.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}!=0 {
            if std::io::Error::last_os_error().kind()==std::io::ErrorKind::NotFound {continue;}return Err(unsafe_run());
        }
        let stat=unsafe{stat.assume_init()};
        if stat.st_mode&libc::S_IFMT!=libc::S_IFREG || stat.st_uid!=0 || stat.st_mode&0o7777!=0o644 || stat.st_nlink!=1 {return Err(unsafe_run());}
    }
    for name in names {
        if unsafe{libc::unlinkat(directory.as_raw_fd(),name.as_ptr(),0)}!=0 && std::io::Error::last_os_error().kind()!=std::io::ErrorKind::NotFound {return Err(unsafe_run());}
    }
    directory.sync_all().map_err(|_|unsafe_run())
}
/// Finite F1/Y1/Y2 path. The caller handles old-attempt resume/admission and retains custody.
/// RuntimeConfig and admin inputs are read only after persisted secrets reach the s6 envdir.
pub(crate) async fn prepare_pg17(roots:crate::db_upgrade::layout::Roots,custody:&mut super::custody::Custody,journal:Option<super::Journal>,current:super::journal::Candidate,phase:&Phase1Step)->Result<super::ready::Prepared,StepError> {
    phase.set(Step::Y1);let inputs=crate::db_upgrade::fresh::Inputs::from_env().map_err(|e|startup_error(e.code,e.message))?;
    let locks=custody.locks.clone();
    let journal=custody.during(super::ready::enter(roots.clone(),locks,journal,current.clone(),inputs,"/run/s6/container_environment".into())).await?;
    phase.set(Step::Y2);let config=read_config()?;
    let locks=custody.locks.clone();
    custody.during(super::ready::prepare(roots,locks,journal,current,config,super::boot::Inputs::startup())).await
}

/// Complete the finite PG17 branch before any dependent starts. Monitoring remains phase 2.
/// The caller resolves old upgrades/retry admission first; enter refuses an uncommitted upgrade.
pub(crate) async fn prepare_and_handoff(startup:&mut Startup,current:super::journal::Candidate,phase:&Phase1Step)->Result<(),StepError> {
    if startup.plan!=BootPlan::Phase1 {return Err(preparation_error());}
    let identity=startup.identity.clone().ok_or_else(preparation_error)?;
    let prepared=prepare_pg17(startup.roots.clone(),&mut startup.custody,startup.journal.take(),current,phase).await?;
    let mut journal=prepared.journal;
    phase.set(Step::S4Activate);let result=finish_phase1(&startup.roots,&mut startup.custody,&mut journal,&prepared.selection,&prepared.privileges,&identity).await;
    startup.journal=Some(journal);
    if result.is_ok(){startup.plan=BootPlan::Phase2;}
    result
}


pub(crate) struct CleanupRetry {pub(crate) due:std::time::Instant,used:bool}
impl CleanupRetry {
    pub(crate) fn new(now:std::time::Instant)->Self{Self{due:now+std::time::Duration::from_secs(900),used:false}}
    fn take(&mut self,now:std::time::Instant)->bool{if self.used||now<self.due{false}else{self.used=true;true}}
}
/// `now` controls only the once-per-run scheduler; observation keeps its own real deadline.
/// None means not due/already used; Some(value) is the resulting cleanupPending state.
pub(crate) async fn retry_cleanup_after_handoff(startup:&mut Startup,config:&crate::config::RuntimeConfig,retry:&mut CleanupRetry,now:std::time::Instant,deadline:std::time::Instant)->Result<Option<bool>,StepError>{
    if !retry.take(now){return Ok(None);}
    monitor_after_handoff(startup,config,deadline).await.map(Some)
}
async fn wait_for_cleanup_retry(due:std::time::Instant)->bool{
    tokio::select!{_=tokio::time::sleep_until(tokio::time::Instant::from_std(due))=>true,_=super::signals::park()=>false}
}
/// The runner owns custody until shutdown; every finite operation is drained before parking.
pub(crate) async fn run()->Result<(),StepError>{
    protect_readiness_fd()?;let mut signals=super::signals::Signals::install()?;
    let result=match run_phases(&mut signals).await {
        Err(error) if unrecorded(Path::new(crate::db_upgrade::paths::APPROOT),Path::new("/run/nwui-storage.json"))=>{
            super::refusal::publish_unrecorded_at(&error,Path::new(crate::db_upgrade::paths::PGROOT),Path::new("/run/nwui-storage.json"),Path::new(super::status::STATUS_PATH),notify_readiness)?;
            super::signals::park().await
        },
        result=>result,
    };
    normal_stop(result,crate::db_upgrade::postgres::SIGNAL.load(std::sync::atomic::Ordering::Relaxed))
}
/// A requested stop is not a failure. `interrupted` with no signal came from EINTR and stays an error.
fn normal_stop(result:Result<(),StepError>,signal:i32)->Result<(),StepError>{
    match result {
        Err(error) if error.code=="interrupted" && signal!=0=>{eprintln!("[upgrade] 중지 요청을 받아 종료합니다.");Ok(())},
        result=>result,
    }
}
fn unrecorded(app:&Path,descriptor:&Path)->bool {
    crate::db_upgrade::postgres::SIGNAL.load(std::sync::atomic::Ordering::Relaxed)==0
        && matches!(initial_journal(app),Ok(None))
        && match super::types::read_selection_at(descriptor){Ok(value)=>value.outcome!=Outcome::Pg17,Err(error)=>error.code=="postponed"}
}
async fn run_phases(signals:&mut super::signals::Signals)->Result<(),StepError>{
    let policy=super::retry::Policy::startup()?;let current=super::journal::Candidate::current().await?;let mut startup=start().await?;
    let phase=Phase1Step::new(Step::P0b);
    signals.drain(async{
        if startup.plan==BootPlan::ParkedRefusal{resume_refusal(&startup,&current,policy.label.as_deref())?;return startup.custody.during(super::signals::park()).await;}
        if startup.plan==BootPlan::Phase2{
            resume_readiness(&startup.roots.app,&startup.custody.locks,startup.journal.as_ref().ok_or_else(preparation_error)?,startup.identity.as_deref())?;
        }else if let Err(error)=phase1(&mut startup,&current,&policy,&phase).await{
            reconcile_failure(&mut startup,&current,&policy,phase.get(),error).await?;
        }
        if startup.plan==BootPlan::ParkedRefusal{return startup.custody.during(super::signals::park()).await;}
        if startup.plan!=BootPlan::Phase2{return Err(preparation_error());}
        let config=read_config()?;
        // All activation observations share this one deadline; cleanup errors remain pending.
        let result=monitor_after_handoff(&mut startup,&config,std::time::Instant::now()+std::time::Duration::from_secs(120)).await;
        let pending=match result{Ok(pending)=>pending,Err(error)=>{eprintln!("[upgrade] activation failed: {}",error.code);true}};
        if let Some(j)=startup.journal.as_ref(){publish_step(&startup.roots,j,Step::Phase2Monitor,0,0,None,pending);}
        if pending&&startup.journal.as_ref().is_some_and(|j|j.load().outcome==Outcome::Pg17){
            let mut retry=CleanupRetry::new(std::time::Instant::now());let due=retry.due;
            let elapsed=startup.custody.during(async move{Ok(wait_for_cleanup_retry(due).await)}).await?;
            if elapsed{let now=std::time::Instant::now();let result=retry_cleanup_after_handoff(&mut startup,&config,&mut retry,now,now+std::time::Duration::from_secs(120)).await;
                let pending=match result{Ok(value)=>value,Err(error)=>{eprintln!("[upgrade] activation retry failed: {}",error.code);Some(true)}};
                if let (Some(pending),Some(j))=(pending,startup.journal.as_ref()){publish_step(&startup.roots,j,Step::Phase2Monitor,0,0,None,pending);}
            }
        }
        startup.custody.during(super::signals::park()).await
    }).await
}
async fn phase1(startup:&mut Startup,current:&super::journal::Candidate,policy:&super::retry::Policy,phase:&Phase1Step)->Result<(),StepError>{
    let identity=startup.identity.clone().ok_or_else(||startup_error("verification_failed","이 실행의 식별자를 확인하지 못해 시작을 보류했습니다."))?;
    phase.set(Step::Y1);
    // 2.0.1: under A+D, before the journal's own checks, legacy15, ready classify and P0 (F1, R1-5).
    // Here a refusal reaches reconcile_failure: parked and published with a journal (R4-2).
    let r=startup.roots.clone();let l=startup.custody.locks.clone();let journal=startup.journal.is_some();
    startup.custody.during(async move{tokio::task::spawn_blocking(move||super::storage_modes::normalize(&r,&l,journal).map(|_|())).await.map_err(|_|preparation_error())?}).await?;
    if let Some(j)=startup.journal.take(){
        let eligible=!matches!(j.load().outcome,Outcome::Upgrading|Outcome::Pg17|Outcome::ActivationFailed|Outcome::RestoreIncomplete);
        let sufficient=if eligible&&!policy.postponed{
            let r=startup.roots.clone();let l=startup.custody.locks.clone();let frame=j.load().clone();
            startup.custody.during(async move{tokio::task::spawn_blocking(move||{let j=super::Journal::open(&r.app,&l)?;if serde_json::to_value(j.load()).map_err(|_|preparation_error())?!=serde_json::to_value(frame).map_err(|_|preparation_error())?{return Err(preparation_error());}match super::preflight::legacy15(&r,&l)?{Some(legacy)=>Ok(super::preflight::assess_backup(&r,&l,&j,&legacy)?.1.space_sufficient),None=>Ok(false)}}).await.map_err(|_|preparation_error())?}).await?
        }else{false};
        let roots=startup.roots.clone();let locks=startup.custody.locks.clone();
        let(j,admission)=startup.custody.during(resume_before_admission(roots,locks,j,current.clone(),policy.label.clone(),sufficient,chrono::Utc::now(),Some(identity.clone()))).await?;
        startup.journal=Some(j);startup.plan=boot_plan(startup.journal.as_ref().ok_or_else(preparation_error)?.load(),Some(&identity));
        if startup.plan==BootPlan::ParkedRefusal{resume_refusal(startup,current,policy.label.as_deref())?;return Ok(());}
        if admission.is_some(){let j=startup.journal.as_mut().ok_or_else(preparation_error)?;finalize_refusal(&startup.roots,&startup.custody.locks,j,&identity,current,policy.label.as_deref())?;startup.plan=BootPlan::ParkedRefusal;return Ok(());}
    }
    let r=startup.roots.clone();let l=startup.custody.locks.clone();
    let legacy=startup.custody.during(async move{tokio::task::spawn_blocking(move||super::preflight::legacy15(&r,&l)).await.map_err(|_|preparation_error())?}).await?;
    let Some(legacy)=legacy else{return prepare_and_handoff(startup,current.clone(),phase).await;};
    phase.set(Step::P0b);if startup.journal.is_none(){let r=startup.roots.clone();let l=startup.custody.locks.clone();let c=current.clone();let label=policy.label.clone();startup.journal=Some(startup.custody.during(async move{tokio::task::spawn_blocking(move||super::Journal::begin_upgrade(&r.app,&l,c,label.as_deref())).await.map_err(|_|preparation_error())?}).await?);}
    if policy.postponed{
        let j=startup.journal.as_mut().ok_or_else(preparation_error)?;postpone(&startup.roots,&startup.custody.locks,j,&identity,current,policy.label.as_deref())?;startup.plan=BootPlan::ParkedRefusal;return Ok(());
    }
    upgrade_legacy(startup,current.clone(),legacy,phase).await
}
pub(super) fn committed_failure_outcome(j:&JournalV1,step:Step)->Outcome{
    let commit=j.records.iter().rev().find(|r|r.step==Step::P5Commit&&r.state==StepState::Done&&r.data.is_none());
    let current_upgrade=j.branch==super::Branch::Upgrade&&commit.is_some_and(|commit|!j.records.iter().any(|r|r.step==Step::Handoff&&r.state==StepState::Done&&r.seq>commit.seq));
    if step==Step::S4Activate||current_upgrade{Outcome::ActivationFailed}else{Outcome::RefusedPrivilege}
}
/// Physical commit authority, never the returned error code, chooses roll-forward or R-restore.
fn require_recovery_journal(app:&Path,error:&StepError)->Result<(),StepError>{
    if initial_journal(app)?.is_none(){return Err(StepError{code:error.code,message:error.message,detail:error.detail.clone()});}Ok(())
}
async fn reconcile_failure(startup:&mut Startup,current:&super::journal::Candidate,policy:&super::retry::Policy,step:Step,error:StepError)->Result<(),StepError>{
    // A pre-journal refusal must retain its cause; corrupt existing authority is still fatal.
    require_recovery_journal(&startup.roots.app,&error)?;
    let detail=error.detail.as_deref().unwrap_or("").chars().filter(|c|!c.is_control()).take(2048).collect::<String>();eprintln!("[upgrade] preparation failed: {} {detail}",error.code);
    let r=startup.roots.clone();let l=startup.custody.locks.clone();
    let(j,committed)=startup.custody.during(async move{tokio::task::spawn_blocking(move||Ok((super::Journal::open(&r.app,&l)?,super::replacement::is_committed(&r)?))).await.map_err(|_|preparation_error())?}).await?;
    if startup.identity.as_deref().is_some_and(|identity|super::refusal::outcome(j.load(),identity).is_some()){
        let mut j=j;let identity=startup.identity.as_deref().ok_or_else(preparation_error)?;finalize_refusal(&startup.roots,&startup.custody.locks,&mut j,identity,current,policy.label.as_deref())?;startup.journal=Some(j);startup.plan=BootPlan::ParkedRefusal;return Ok(());
    }
    if committed{
        let mut j=j;
        let Some(identity)=startup.identity.as_deref()else{j.record_retry_failure(step,error.code,chrono::Utc::now())?;super::refusal::publish_status_only(&startup.roots,&j,committed_failure_outcome(j.load(),step),current,policy.label.as_deref());return Err(error);};
        let outcome=committed_failure_outcome(j.load(),step);j.fail_attempt_with_identity(step,&error,outcome,chrono::Utc::now(),Some(identity))?;
        finalize_refusal(&startup.roots,&startup.custody.locks,&mut j,identity,current,policy.label.as_deref())?;
        startup.journal=Some(j);startup.plan=BootPlan::ParkedRefusal;return Ok(());
    }
    let backed=j.load().records.iter().any(|r|r.step==Step::P1Backup&&r.state==StepState::Done);
    let mut j=if j.load().branch==super::Branch::Upgrade&&backed{
        startup.custody.during(super::restore::recover_with_context(startup.roots.clone(),startup.custody.locks.clone(),j,step,error.code,startup.identity.clone(),current.clone(),policy.label.clone())).await?
    }else{
        let mut j=j;let identity=startup.identity.as_deref().ok_or_else(preparation_error)?;
        if j.pre_pool()?{let space=super::refusal::status_value(&startup.roots,j.load(),j.load().outcome,current,policy.label.as_deref())?.space;if let Err(publication)=j.finish_pre_pool(identity,Some(&error),space){super::refusal::publish_status_only(&startup.roots,&j,Outcome::RefusedPreflight,current,policy.label.as_deref());return Err(publication);}}else{
            let outcome=match j.load().branch{super::Branch::Fresh=>Outcome::RefusedFresh,super::Branch::Ready=>Outcome::RefusedPrivilege,super::Branch::Upgrade=>Outcome::RefusedPreflight};
            j.fail_attempt(step,&error,outcome,chrono::Utc::now())?;
        }j
    };
    let identity=startup.identity.clone().ok_or_else(preparation_error)?;finalize_refusal(&startup.roots,&startup.custody.locks,&mut j,&identity,current,policy.label.as_deref())?;startup.journal=Some(j);startup.plan=BootPlan::ParkedRefusal;Ok(())
}

/// One finite legacy attempt; errors are reconciled from disk by the outer coordinator.
async fn upgrade_legacy(startup:&mut Startup,current:super::journal::Candidate,legacy:super::preflight::Legacy15,phase:&Phase1Step)->Result<(),StepError>{
    use crate::db_upgrade::{fresh,key_file,paths,postgres};
    let roots=startup.roots.clone();let locks=startup.custody.locks.clone();let j=startup.journal.take().ok_or_else(preparation_error)?;
    let r=roots.clone();let l=locks.clone();
    phase.set(Step::P0b);let progress=phase.clone();
    let mut j=startup.custody.during(async move{tokio::task::spawn_blocking(move||{
        let mut j=j;let(prepared,assessment)=super::preflight::assess_backup(&r,&l,&j,&legacy)?;j.record(Step::P0b,StepState::Intent,Some(serde_json::json!({"space":assessment.space})))?;assessment.require()?;
        j.record(Step::P0b,StepState::Done,Some(serde_json::json!({"space":assessment.space})))?;publish_step(&r,&j,Step::P0b,0,prepared.source_bytes,None,false);super::reserves::ensure(&r,&l,&j)?;
        progress.set(Step::P1Backup);publish_step(&r,&j,Step::P1Backup,0,prepared.source_bytes,None,false);
        super::backup::create_verified(&r,&l,&mut j,&prepared)?;publish_step(&r,&j,Step::P1Backup,prepared.source_bytes,prepared.source_bytes,None,false);
        progress.set(Step::P2Preflight);fresh::legacy_environment_locked(&l,&r,Path::new("/run/s6/container_environment")).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})?;Ok(j)
    }).await.map_err(|_|preparation_error())?}).await?;
    phase.set(Step::P2Preflight);let config=read_config()?;let credentials:tokio_postgres::Config=config.database_url.parse().map_err(|_|preparation_error())?;
    let password=std::str::from_utf8(credentials.get_password().ok_or_else(preparation_error)?).map_err(|_|preparation_error())?.to_owned();
    let r=roots.clone();let l=locks.clone();
    let (journal,name,observed)=startup.custody.during(async move{tokio::task::spawn_blocking(move||{
        let header=super::backup::open_verified_manifest(&r,&j)?.header;let source=match header.source_location{super::backup_format::SourceLocation::Root=>r.db.clone(),super::backup_format::SourceLocation::Pgdata=>r.db.join("pgdata")};
        let control=postgres::read_control(15,&source).map_err(|e|startup_error(e.code,e.message))?;
        let path=paths::startup_data_key_file(&r.app).map_err(|e|startup_error(e.code,e.message))?;
        let observed=match key_file::read_key_file(&path,key_file::canonical_policy(&r.app,&path)){Ok(key)=>Some(key),Err(key_file::KeyFileError::Missing)=>None,Err(_)=>return Err(startup_error("key_path_invalid","기존 암호화 키 파일을 확인하지 못했습니다. 기존 파일을 보존했습니다."))};
        let name=super::check::begin_locked(&r,&l,&mut j,&control)?;Ok((j,name,observed))
    }).await.map_err(|_|preparation_error())?}).await?;
    j=journal;let key_required=observed.is_some();let observed=observed.map(|k|std::sync::Arc::new(k.secrets));
    let packaged=crate::migrations::read_migrations(&config.migrations_dir).await.map_err(|_|startup_error("migration_failed","적용할 변경 이력을 읽지 못했습니다."))?;
    let (source,schema)=startup.custody.during(super::check::inspect_source(roots.clone(),locks.clone(),&j,&name,&password,observed,&packaged)).await?;
    let(j,source)=startup.custody.during(super::check::finish(roots.clone(),locks.clone(),j,source,schema,&config,&name)).await?;
    let r=roots.clone();let l=locks.clone();
    phase.set(Step::P3Convert);let(j,source,id)=startup.custody.during(async move{tokio::task::spawn_blocking(move||{
        let mut j=j;let backup=super::backup::open_verified_manifest(&r,&j)?;let bytes=backup.summary.data_bytes;let header=backup.header;let original=match header.source_location{super::backup_format::SourceLocation::Root=>r.db.clone(),super::backup_format::SourceLocation::Pgdata=>r.db.join("pgdata")};
        let control=postgres::read_control(15,&original).map_err(|e|startup_error(e.code,e.message))?;publish_step(&r,&j,Step::P3Convert,0,bytes,None,false);
        let target=super::conversion::run_locked(&r,&l,&mut j,&control,&source.snapshot)?;publish_step(&r,&j,Step::P3Convert,bytes,bytes,None,false);Ok((j,source,target.system_identifier))
    }).await.map_err(|_|preparation_error())?}).await?;
    phase.set(Step::P4Target);let ensured=startup.custody.during(super::key_write::ensure_key_locked(roots.clone(),locks.clone(),j,key_required)).await?;
    let r=roots.clone();let l=locks.clone();let runtime=tokio::runtime::Handle::current();let progress=phase.clone();
    let(j,selection,privileges)=startup.custody.during(async move{tokio::task::spawn_blocking(move||{
        let mut j=ensured.journal;let source_key=ensured.source;let key=ensured.key;let proof=super::profile::proof(&j)?;
        let generation=match &source_key{super::KeySource::Staged{generation}=>generation.clone(),super::KeySource::Canonical=>uuid::Uuid::new_v4().to_string()};
        let mut pair=crate::db_upgrade::pair::Pair::fresh(&r.architecture);pair.mode="migration".into();pair.ready=true;pair.fresh_step=None;pair.system_identifier=Some(id.clone());pair.encryption_key_required=key_required;
        let binding=super::KeyBinding::Verified{pair_id:pair.pair_id.clone()};
        let selection=super::StorageSelection{version:1,epoch:j.load().records.last().ok_or_else(preparation_error)?.seq.checked_add(1).ok_or_else(preparation_error)?,outcome:Outcome::Upgrading,branch:super::Branch::Upgrade,pgdata:r.db.join(".nwui-auto-work").join(&j.load().attempt_id).join("data"),backup_id:Some(j.load().attempt_id.clone()),schema_profile:proof.profile_id.clone(),key_binding:binding.clone()};
        let name=j.load().records.iter().rev().find(|v|v.step==Step::P3Convert&&v.state==StepState::Done).and_then(|v|v.data.as_ref()).and_then(|v|v.get("logName")).and_then(|v|v.as_str()).ok_or_else(preparation_error)?;
        let log=super::diagnostics::path(&r.db,&j.load().attempt_id,name)?.join("boot");postgres::directory(&log).map_err(|e|startup_error(e.code,e.message))?;
        let server=postgres::Private::start(17,&selection.pgdata,&log).map_err(|e|startup_error(e.code,e.message))?;
        let pg=super::PrivatePg{socket_dir:server.socket.clone(),port:50432,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};
        publish_step(&r,&j,Step::P4Target,0,0,None,false);
        let prepared=runtime.block_on(super::target_boot::prepare(&r,&l,&mut j,&source,&current,&selection,&pg,&config,&key.secrets,super::boot::Inputs::startup()));
        let stopped=server.stop().map_err(|e|startup_error(e.code,e.message));let privileges=match prepared{Err(error)=>{stopped?;return Err(error)},Ok(value)=>{stopped?;value}};
        publish_step(&r,&j,Step::P4Target,0,0,None,false);
        let draft=super::generation::GenerationDraft{generation:&generation,origin:super::selector::Origin::Auto,source_profile:Some((&proof.profile_id,&proof.profile_sha256)),actual_id:&id,db_pair:&pair,app_pair:&pair,agreement:&binding,key_source:&source_key,key:&key};
        let selection=super::publication::publish_upgrade_locked(&r,&l,&mut j,&selection,draft,&progress)?;Ok((j,selection,privileges))
    }).await.map_err(|_|preparation_error())?}).await?;
    phase.set(Step::P5Commit);let mut j=startup.custody.during(super::committed::resume(roots,locks,j)).await?;
    let identity=startup.identity.clone().ok_or_else(preparation_error)?;phase.set(Step::S4Activate);let result=finish_phase1(&startup.roots,&mut startup.custody,&mut j,&selection,&privileges,&identity).await;startup.journal=Some(j);if result.is_ok(){startup.plan=BootPlan::Phase2;}result
}
fn read_config()->Result<crate::config::RuntimeConfig,StepError>{
    let invalid=||startup_error("boot_step_failed","실행 환경 설정을 확인하세요. 기존 데이터를 보존했습니다.");
    let auth=crate::nonroot::runner_value("AUTH_SECRET").filter(|s|!s.is_empty()).ok_or_else(invalid)?;
    let database=crate::nonroot::runner_value("DATABASE_URL").filter(|s|!s.is_empty()).ok_or_else(invalid)?;
    crate::config::RuntimeConfig::read(|name|match name{
        "AUTH_SECRET"=>Some(auth.clone()),"DATABASE_URL"=>Some(database.clone()),
        _=>crate::nonroot::runner_value(name),
    }).map_err(|_|invalid())
}

/// Phase 2 never acquires D or re-runs activation. Non-PG17 outcomes stay parked.
/// Returns cleanupPending (false for a parked outcome); P6 starts only after durable proof.
pub(crate) async fn monitor_after_handoff(startup:&mut Startup,config:&crate::config::RuntimeConfig,deadline:std::time::Instant)->Result<bool,StepError> {
    if startup.plan!=BootPlan::Phase2 || !startup.custody.locks.holds_a() || startup.custody.locks.holds_d() {return Err(preparation_error());}
    let j=startup.journal.as_mut().ok_or_else(preparation_error)?;
    if !j.belongs_to(&startup.roots.app) || boot_plan(j.load(),startup.identity.as_deref())!=BootPlan::Phase2 {return Err(preparation_error());}
    if j.load().outcome!=Outcome::Pg17 {return Ok(false);}
    let identity=startup.identity.clone().ok_or_else(preparation_error)?;
    let app=startup.roots.app.clone();let generation=tokio::task::spawn_blocking(move||super::selector::read_selector(&app)?.generation.ok_or_else(preparation_error)).await.map_err(|_|preparation_error())??;
    j.record(Step::Phase2Monitor,StepState::Intent,None)?;publish_step(&startup.roots,j,Step::Phase2Monitor,0,0,None,false);
    let observed=startup.custody.during(observe_until_stop(async {
        super::monitor::check(config,deadline).await?;
        if super::boot_identity::current()?!=identity{return Err(preparation_error());}
        Ok(wait_apply_uncertain(config,deadline).await)
    })).await;
    // Shutdown is not an activation failure, even if it raced a successful observation.
    if crate::db_upgrade::files::CANCELLED.load(std::sync::atomic::Ordering::Relaxed){return Ok(true);}
    let apply_uncertain=match &observed{Ok(value)=>*value,Err(_)=>false};
    let observed=observed.map(|_|());
    if !record_monitor_result(j,&identity,&generation,observed,apply_uncertain)?{publish_step(&startup.roots,j,Step::Phase2Monitor,0,0,None,true);return Ok(true);}
    if crate::db_upgrade::files::CANCELLED.load(std::sync::atomic::Ordering::Relaxed){return Ok(true);}
    cleanup_after_handoff(startup).await
}
/// Only read-only observations may be dropped; custody and mutation draining stay outside.
async fn observe_until_stop<T>(observation:impl std::future::Future<Output=Result<T,StepError>>)->Result<T,StepError>{
    tokio::select!{biased;
        _=super::signals::park()=>Err(startup_error("interrupted","시작 준비를 중단했습니다. 진행 중인 작업을 정리하고 복구 기록을 보존했습니다.")),
        result=observation=>result,
    }
}
async fn wait_apply_uncertain(config:&crate::config::RuntimeConfig,deadline:std::time::Instant)->bool {
    loop {
        // A8 remaining at the activation deadline defers proof; it is not an activation failure.
        if !tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),crate::control::confirmation::uncertain(config)).await.unwrap_or(true){return false;}
        let Some(remaining)=deadline.checked_duration_since(std::time::Instant::now()).filter(|d|!d.is_zero())else{return true;};
        tokio::time::sleep(remaining.min(std::time::Duration::from_millis(250))).await;
    }
}
/// A-only post-activation cleanup. Its errors remain cleanupPending, never a restore decision.
pub(crate) async fn cleanup_after_handoff(startup:&mut Startup)->Result<bool,StepError> {
    if startup.plan!=BootPlan::Phase2 || !startup.custody.locks.holds_a() || startup.custody.locks.holds_d() {return Err(preparation_error());}
    let identity=startup.identity.clone().ok_or_else(preparation_error)?;
    let roots=startup.roots.clone();let locks=startup.custody.locks.clone();
    let result=startup.custody.during(async move {
        tokio::task::spawn_blocking(move|| {
            let mut journal=super::Journal::open(&roots.app,&locks)?;
            cleanup_locked(&roots,&locks,&mut journal,&identity)
        }).await.map_err(|_|preparation_error())?
    }).await;
    // A daemon fault can hide a successful work result; reload only after during drained it.
    let app=startup.roots.app.clone();let locks=startup.custody.locks.clone();
    startup.journal=Some(tokio::task::spawn_blocking(move||super::Journal::open(&app,&locks)).await.map_err(|_|preparation_error())??);
    match result {Ok(())=>Ok(false),Err(error)=>{eprintln!("[upgrade] cleanup pending: {}",error.code);Ok(true)}}
}
fn cleanup_locked(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&mut super::Journal,identity:&str)->Result<(),StepError> {
    if !locks.holds_a() || locks.holds_d() || !locks.matches_app(&roots.app) || !j.belongs_to(&roots.app) || super::boot_identity::current()?!=identity {return Err(preparation_error());}
    if j.load().branch!=super::Branch::Upgrade {return Ok(());}
    let selector=super::selector::read_selector(&roots.app)?;
    let generation=selector.generation.as_deref().ok_or_else(preparation_error)?;
    if !activation_proof(j,identity,generation){return Err(preparation_error());}
    // The production postmaster is live: only its immutable identifier is inspected here.
    let control=crate::db_upgrade::postgres::read_control(17,&roots.db.join("17/data")).map_err(|e|startup_error(e.code,e.message))?;
    let path=crate::db_upgrade::paths::startup_data_key_file(&roots.app).map_err(|e|startup_error(e.code,e.message))?;
    let key=crate::db_upgrade::key_file::read_key_file(&path,crate::db_upgrade::key_file::canonical_policy(&roots.app,&path)).map_err(|_|preparation_error())?;
    let selected=super::selector::load_generation(&roots.db,&roots.app,&roots.architecture,selector,&control.system_identifier,&key.bytes)?;
    publish_step(roots,j,Step::P6Cleanup,0,0,None,true);let result=super::cleanup::cleanup(roots,locks,j,&selected);publish_step(roots,j,Step::P6Cleanup,0,0,None,result.is_err());result
}

#[derive(serde::Serialize,serde::Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct ActivationProof {boot_identity:String,generation:String,observed_at:String}
pub(crate) fn activation_proof(j:&super::Journal,identity:&str,generation:&str)->bool {
    if j.load().outcome!=Outcome::Pg17 || !super::types::canonical_generation(generation) || identity.len()!=64 || !identity.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)){return false;}
    let handoff=j.load().records.iter().rev().find(|r|r.step==Step::Handoff);
    if !handoff.is_some_and(|r|r.state==StepState::Done && r.data.as_ref().and_then(|d|d.get("bootIdentity")).and_then(|v|v.as_str())==Some(identity)){return false;}
    let record=j.load().records.iter().rev().find(|r|r.step==Step::Phase2Monitor);
    record.filter(|r|r.state==StepState::Done).and_then(|r|r.data.as_ref()).and_then(|data|serde_json::from_value::<ActivationProof>(data.clone()).ok()).is_some_and(|proof|proof.boot_identity==identity && proof.generation==generation && chrono::DateTime::parse_from_rfc3339(&proof.observed_at).is_ok())
}
pub(super) fn record_monitor_result(j:&mut super::Journal,identity:&str,generation:&str,result:Result<(),StepError>,apply_uncertain:bool)->Result<bool,StepError> {
    if crate::db_upgrade::files::CANCELLED.load(std::sync::atomic::Ordering::Relaxed){return Ok(false);}
    match result {
        Ok(())if apply_uncertain=>Ok(false),
        Ok(())=>j.record(Step::Phase2Monitor,StepState::Done,Some(serde_json::to_value(ActivationProof{boot_identity:identity.into(),generation:generation.into(),observed_at:chrono::Utc::now().to_rfc3339()}).map_err(|_|preparation_error())?)).map(|()|true),
        Err(error)=>{j.fail_attempt(Step::Phase2Monitor,&error,Outcome::ActivationFailed,chrono::Utc::now())?;Err(error)},
    }
}

fn preparation_error()->StepError {StepError {code:"privilege_prepare_failed",message:"서비스 실행 준비 기록이 반환된 핸들과 일치하지 않습니다. 서비스 시작을 보류했습니다.",detail:None}}
fn verify_prepared_return(j:&super::Journal,p:&crate::nonroot::Prepared)->Result<(),StepError> {
    verify_prepared_return_for(j,p,crate::nonroot::PRIVILEGE_SPLIT)
}
fn verify_prepared_return_for(j:&super::Journal,p:&crate::nonroot::Prepared,split:bool)->Result<(),StepError> {
    let saved=super::generation::completed_preparation_for(j,split).ok_or_else(preparation_error)?;
    if serde_json::to_value(&saved).map_err(|_|preparation_error())?!=serde_json::to_value(p).map_err(|_|preparation_error())? {return Err(preparation_error());}Ok(())
}
/// On every phase-1 S4 resume retry the owner, never jump past a partial publisher bootstrap.
pub(crate) async fn prepare_privileges(sel:&super::StorageSelection,locks:&super::StorageLocks,pg:&super::PrivatePg,j:&mut super::Journal,mode:crate::publisher::boot::BootMode)->Result<crate::nonroot::Prepared,StepError> {
    if !crate::nonroot::PRIVILEGE_SPLIT {
        if j.load().records.iter().any(|r|r.step==Step::S4Prepare) {return Err(preparation_error());}
        j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-split-off"})))?;
        let p=crate::nonroot::Prepared{payload:None,key_receipt:None};verify_prepared_return(j,&p)?;return Ok(p);
    }
    let result=crate::nonroot::prepare(sel,locks,pg,j,mode).await.and_then(|p|{verify_prepared_return(j,&p)?;Ok(p)});
    match result {Ok(p)=>Ok(p),Err(error)=>{discard_privileges(j).await?;Err(error)}}
}
/// Even Prepared(None,None) means a partial owner record: discard it before any R-restore step.
pub(crate) async fn discard_privileges(j:&mut super::Journal)->Result<(),StepError> {
    if !crate::nonroot::PRIVILEGE_SPLIT && j.load().records.iter().any(|r|r.step==Step::S4Prepare){return Err(preparation_error());}
    match crate::nonroot::prepared_from_journal(j) {
        Some(p)=>crate::nonroot::discard(p,j).await?,
        None if j.load().records.iter().any(|r|r.step==Step::S4Prepare)=>return Err(preparation_error()),
        None=>{},
    }
    j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-discarded"})))
}

fn handoff_inputs(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&super::Journal,selection:&super::StorageSelection,p:&crate::nonroot::Prepared,identity:&str)->Result<(),StepError> {
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) || !j.belongs_to(&roots.app)
        || selection.version!=1 || selection.outcome!=Outcome::Pg17 || selection.branch!=j.load().branch || selection.pgdata!=roots.db.join("17/data") || selection.epoch==0
        || selection.schema_profile.is_empty() || selection.schema_profile.len()>128 || selection.backup_id.as_deref().is_some_and(|id|!super::types::canonical_generation(id))
        || p.key_receipt.as_ref().is_some_and(|receipt|receipt.selection_epoch!=selection.epoch)
        || !matches!(&selection.key_binding,super::KeyBinding::Verified{pair_id} if super::types::canonical_generation(pair_id))
        || identity.len()!=64 || !identity.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !j.load().records.iter().any(|r|r.step==Step::P5Commit && r.state==StepState::Done && r.data.is_none()) {return Err(preparation_error());}
    verify_prepared_return(j,p)
}
fn required_live_pair(snapshot:&crate::db_upgrade::pair::Pair,live:&crate::db_upgrade::pair::Pair)->Result<(),StepError> {
    super::selector::same_pair_except_flag(snapshot,live)?;
    if !live.encryption_key_required{return Err(preparation_error());}Ok(())
}
/// Complete finite phase 1; the caller starts monitoring only after this handoff returns.
/// No status I/O failure is fed here: informational status publication remains non-fatal.
pub(crate) async fn finish_phase1(roots:&crate::db_upgrade::layout::Roots,custody:&mut super::custody::Custody,j:&mut super::Journal,selection:&super::StorageSelection,p:&crate::nonroot::Prepared,identity:&str)->Result<(),StepError> {
    handoff_inputs(roots,&custody.locks,j,selection,p,identity)?;
    let roots_check=roots.clone();let locks=custody.locks.clone();let descriptor=selection.clone();let boot_identity=identity.to_owned();
    custody.during(async move {
        tokio::task::spawn_blocking(move|| {
            if !locks.holds_d() || !locks.matches_roots(&roots_check.db,&roots_check.app) || super::boot_identity::current()?!=boot_identity{return Err(preparation_error());}
            let control=crate::db_upgrade::postgres::read_control(17,&roots_check.db.join("17/data")).map_err(|e|startup_error(e.code,e.message))?;
            control.require_cold().map_err(|e|startup_error(e.code,e.message))?;
            match std::fs::symlink_metadata(roots_check.db.join("17/data/postmaster.pid")){Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},_=>return Err(preparation_error())}
            let path=crate::db_upgrade::paths::startup_data_key_file(&roots_check.app).map_err(|e|startup_error(e.code,e.message))?;
            let key=crate::db_upgrade::key_file::read_key_file(&path,crate::db_upgrade::key_file::canonical_policy(&roots_check.app,&path)).map_err(|_|preparation_error())?;
            let selected=super::selector::read_selector(&roots_check.app)?;
            let generation=super::selector::load_generation(&roots_check.db,&roots_check.app,&roots_check.architecture,selected,&control.system_identifier,&key.bytes)?;
            if descriptor.backup_id!=generation.selector.backup_id || !matches!(&descriptor.key_binding,super::KeyBinding::Verified{pair_id} if *pair_id==generation.app_pair.pair_id){return Err(preparation_error());}
            let (live,_):(crate::db_upgrade::pair::Pair,_)=super::selector::read_record(&roots_check.app.join(".nwui/pair-v1.json"))?;
            required_live_pair(&generation.app_pair,&live)?;
            Ok(())
        }).await.map_err(|_|preparation_error())?
    }).await?;
    // Only the owner writes S4Activate; I1's no-op is intentionally inert.
    custody.during(crate::nonroot::activate(selection,p,j)).await?;
    j.set_outcome(Outcome::Pg17)?;
    let descriptor=selection.clone();
    custody.during(async move {
        tokio::task::spawn_blocking(move|| {
            super::types::write_selection_at(Path::new("/run/nwui-storage.json"),&descriptor)?;
            super::service::stop_status()
        }).await.map_err(|_|readiness_error())?
    }).await?;
    publish_step(roots,j,Step::Handoff,0,0,None,true);custody.handoff(j,identity).await?;publish_step(roots,j,Step::Handoff,0,0,None,true);Ok(())
}

/// A.19 same-boot restart only: custody already reconciled A and deliberately did not take D.
/// Re-notification does not re-run preparation or turn a recorded activation failure into PG17.
pub(crate) fn resume_readiness(app:&Path,locks:&super::StorageLocks,j:&super::Journal,identity:Option<&str>)->Result<(),StepError> {
    if !locks.holds_a() || locks.holds_d() || !locks.matches_app(app) || !j.belongs_to(app) || boot_plan(j.load(),identity)!=BootPlan::Phase2 {return Err(preparation_error());}
    let current=super::Journal::open(app,locks)?;
    if serde_json::to_value(current.load()).map_err(|_|preparation_error())?!=serde_json::to_value(j.load()).map_err(|_|preparation_error())? {return Err(preparation_error());}
    notify_readiness()
}

/// Run in an awaited blocking operation under A+D custody; the receipt precedes publication.
pub(crate) fn finalize_refusal(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&mut super::Journal,identity:&str,current:&super::journal::Candidate,label:Option<&str>)->Result<(),StepError> {
    super::refusal::record(roots,locks,j,identity)?;super::refusal::publish(roots,locks,j,identity,current,label)?;notify_readiness()
}
/// Operator opt-out is final for this boot, before P1/reserves/private PG startup.
pub(crate) fn postpone(roots:&crate::db_upgrade::layout::Roots,locks:&super::StorageLocks,j:&mut super::Journal,identity:&str,current:&super::journal::Candidate,label:Option<&str>)->Result<(),StepError>{
    if !locks.holds_d()||!locks.matches_roots(&roots.db,&roots.app)||!j.belongs_to(&roots.app)||j.load().branch!=super::Branch::Upgrade||j.load().records.iter().any(|r|matches!(r.step,Step::P1Backup|Step::P2Preflight|Step::P3Convert|Step::P5Commit)){return Err(preparation_error());}
    // Validate the identity before exposing the outcome; an unreadable identity never signals ready.
    super::refusal::receipt(Outcome::Postponed,identity)?;
    if j.pre_pool()?{if let Err(error)=j.finish_pre_pool(identity,None,None){super::refusal::publish_status_only(roots,j,Outcome::Postponed,current,label);return Err(error);}}else{j.record(Step::P0b,StepState::Done,Some(serde_json::json!({"upgradePolicy":"off"})))?;j.set_outcome(Outcome::Postponed)?;}
    eprintln!("[upgrade] 업그레이드를 보류했습니다 — NWUI_DB_UPGRADE 변수를 지우고 재시작하거나 1.19.11을 사용하세요.");
    finalize_refusal(roots,locks,j,identity,current,label)
}
/// Same-boot replay does not append records, re-run admission or release D.
pub(crate) fn resume_refusal(startup:&Startup,current:&super::journal::Candidate,label:Option<&str>)->Result<(),StepError> {
    if startup.plan!=BootPlan::ParkedRefusal{return Err(preparation_error());}
    let j=startup.journal.as_ref().ok_or_else(preparation_error)?;let identity=startup.identity.as_deref().ok_or_else(preparation_error)?;
    super::refusal::publish(&startup.roots,&startup.custody.locks,j,identity,current,label)?;notify_readiness()
}

/// Step lines describe the upgrade itself. An upgraded install keeps its journal, so finished
/// cleanup ends them; only the boot that just finished it still logs that closing line.
fn logs_step(j:&super::journal::JournalV1,step:Step)->bool {
    let finished=super::status::terminal_cleanup;
    j.branch==super::Branch::Upgrade && (!j.records.iter().any(finished) || step==Step::P6Cleanup && j.records.last().is_some_and(finished))
}
/// Informational only: status/log failures never decide a journal outcome or trigger restore.
pub(crate) fn publish_step(roots:&crate::db_upgrade::layout::Roots,j:&super::Journal,step:Step,bytes_done:u64,bytes_total:u64,space:Option<super::types::StatusSpace>,cleanup_pending:bool){
    let rendered=(||{
        let mut value=super::refusal::status_value(roots,j.load(),j.load().outcome,&j.load().candidate,None)?;
        let (index,total)=super::status::step_position(step);
        let now=chrono::Utc::now();let start=j.load().records.iter().rev().find(|r|r.step==step&&r.state==StepState::Intent).map(|r|r.at.clone()).unwrap_or_else(||now.to_rfc3339());
        let elapsed=chrono::DateTime::parse_from_rfc3339(&start).map(|at|(now-at.with_timezone(&chrono::Utc)).num_milliseconds().max(0)).unwrap_or(0);
        let state=match j.load().records.iter().rev().find(|r|r.step==step).map(|r|&r.state){Some(StepState::Done)=>"done",Some(StepState::Failed{..})=>"failed",_=>"start"};
        let name=serde_json::to_value(step).map_err(|_|preparation_error())?;let name=name.as_str().ok_or_else(preparation_error)?;
        if logs_step(j.load(),step) {eprintln!("[upgrade] step {index}/{total} {name} {state} (duration_ms={elapsed}, bytes={bytes_done}/{bytes_total})");}
        value.updated_at=now.to_rfc3339();value.step=super::types::StatusStep{id:step,index,total,started_at:start,bytes_done,bytes_total};if space.is_some(){value.space=space;}value.cleanup_pending=cleanup_pending;
        if value.outcome==Outcome::Pg17{(value.cleanup_pending,value.failure)=super::status::pg17_pending(j.load());}
        super::commands::publish_status(&value);Ok::<_,StepError>(())
    })();if let Err(error)=rendered{eprintln!("[upgrade] status render failed: {}",error.code);}
}
fn readiness_error()->StepError {StepError {code:"boot_step_failed",message:"서비스 시작 알림을 전달하지 못했습니다. 상태를 확인하세요.",detail:Some("runner_readiness".into())}}
/// Keep fd 3 in this runner, but never leak it into fdholder tools or private postmasters.
pub(crate) fn protect_readiness_fd()->Result<(),StepError> {
    let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe{libc::fstat(3,stat.as_mut_ptr())}!=0 {return Err(readiness_error());}
    let kind=unsafe{stat.assume_init()}.st_mode&libc::S_IFMT;
    let flags=unsafe{libc::fcntl(3,libc::F_GETFD)};let access=unsafe{libc::fcntl(3,libc::F_GETFL)};
    if !matches!(kind,libc::S_IFIFO|libc::S_IFSOCK) || flags<0 || access<0 || access&libc::O_ACCMODE==libc::O_RDONLY
        || unsafe{libc::fcntl(3,libc::F_SETFD,flags|libc::FD_CLOEXEC)}<0 {return Err(readiness_error());}Ok(())
}
/// Called once after the durable outcome/descriptor and, for PG17, D handoff/status-down.
pub(crate) fn notify_readiness()->Result<(),StepError> {
    loop {
        let written=unsafe{libc::write(3,b"\n".as_ptr().cast(),1)};
        if written==1 {return Ok(());}
        if written<0 && std::io::Error::last_os_error().kind()==std::io::ErrorKind::Interrupted {continue;}return Err(readiness_error());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,Journal,StorageLocks,journal::Candidate};
    use std::{fs,os::unix::fs::{PermissionsExt,symlink}};
    fn journal()->(tempfile::TempDir,StorageLocks,Journal) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let j=Journal::begin(&app,&locks,Branch::Ready,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();(t,locks,j)
    }
    #[test]
    fn f1_unrecorded_refusal_never_overwrites_journal_or_pg17_authority(){
        let(t,_locks,_j)=journal();let app=t.path().join("app");let descriptor=t.path().join("storage.json");
        assert!(!unrecorded(&app,&descriptor));
        let path=app.join(".nwui-auto/journal-v1.json");fs::write(&path,b"corrupt").unwrap();assert!(!unrecorded(&app,&descriptor));
        fs::remove_file(path).unwrap();assert!(unrecorded(&app,&descriptor));
        let selection=super::super::StorageSelection{version:1,epoch:1,outcome:Outcome::Pg17,branch:Branch::Ready,pgdata:crate::db_upgrade::paths::PGDATA.into(),backup_id:None,schema_profile:"none".into(),key_binding:super::super::KeyBinding::Unverified};
        super::super::types::write_selection_at(&descriptor,&selection).unwrap();assert!(!unrecorded(&app,&descriptor));
        fs::write(&descriptor,b"corrupt").unwrap();assert!(!unrecorded(&app,&descriptor));
        fs::remove_file(&descriptor).unwrap();let bad=t.path().join("bad");fs::write(&bad,b"not a directory").unwrap();assert!(!unrecorded(&bad,&descriptor));
    }
    #[test]
    fn c3_step_lines_end_once_the_upgrade_has_finished(){
        let(_t,_locks,j)=journal();let mut frame=j.load().clone();
        let record=|step,state,data|super::super::journal::Record{seq:0,step,state,at:String::new(),data};
        assert!(!logs_step(&frame,Step::Handoff),"a non-upgrade boot stays silent");
        frame.branch=Branch::Upgrade;frame.records.clear();
        assert!(logs_step(&frame,Step::Handoff),"the upgrade itself is logged");
        frame.records.push(record(Step::P6Cleanup,StepState::Done,Some(serde_json::json!({"entry":"x"}))));
        assert!(logs_step(&frame,Step::Handoff),"a partial cleanup record is not the end");
        frame.records.push(record(Step::P6Cleanup,StepState::Done,None));
        assert!(logs_step(&frame,Step::P6Cleanup),"the boot that finished keeps its closing line");
        assert!(!logs_step(&frame,Step::Phase2Monitor),"nothing after the closing line");
        frame.records.push(record(Step::Phase2Monitor,StepState::Done,None));
        for step in [Step::Handoff,Step::Phase2Monitor,Step::P6Cleanup] {assert!(!logs_step(&frame,step),"a later restart stays silent");}
    }
    #[test]
    fn c1_only_a_signalled_interrupted_is_a_normal_stop(){
        let failed=|code|Err(StepError{code,message:"fixture",detail:None});
        for signal in [2,15] {assert!(normal_stop(failed("interrupted"),signal).is_ok());}
        assert_eq!(normal_stop(failed("interrupted"),0).unwrap_err().code,"interrupted","EINTR without a stop request stays an error");
        assert_eq!(normal_stop(failed("pair_mismatch"),15).unwrap_err().code,"pair_mismatch","a real failure during a stop keeps its code");
        assert!(normal_stop(Ok(()),0).is_ok());
    }
    #[tokio::test]
    async fn f1_shutdown_cancels_only_observation_and_keeps_intent_even_after_success(){
        const CHILD:&str="NWUI_F1_MONITOR_STOP";
        if std::env::var_os(CHILD).is_none(){
            let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::runner::tests::f1_shutdown_cancels_only_observation_and_keeps_intent_even_after_success","--nocapture"]).env(CHILD,"1").output().unwrap();
            assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));return;
        }
        let(t,_locks,mut j)=journal();let identity="a".repeat(64);let generation=uuid::Uuid::new_v4().to_string();
        j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&identity).unwrap();j.handoff(Handoff::Done,&identity).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();
        let path=t.path().join("app/.nwui-auto/journal-v1.json");let before=fs::read(&path).unwrap();
        let mut signals=super::super::signals::Signals::install().unwrap();
        let stopper=tokio::spawn(async{tokio::time::sleep(std::time::Duration::from_millis(50)).await;assert_eq!(unsafe{libc::kill(libc::getpid(),libc::SIGTERM)},0);});
        let start=std::time::Instant::now();
        let result=signals.drain(observe_until_stop(std::future::pending::<Result<(),StepError>>())).await;
        stopper.await.unwrap();assert!(start.elapsed()<std::time::Duration::from_secs(1));assert_eq!(result.as_ref().unwrap_err().code,"interrupted");
        assert!(!record_monitor_result(&mut j,&identity,&generation,result,false).unwrap());
        assert!(!record_monitor_result(&mut j,&identity,&generation,Ok(()),false).unwrap());
        assert_eq!(fs::read(path).unwrap(),before);assert_eq!(j.load().outcome,Outcome::Pg17);assert_eq!(j.load().records.last().unwrap().state,StepState::Intent);
        assert!(!unrecorded(t.path(),&t.path().join("missing")));assert!(!j.load().records.iter().any(|r|r.step==Step::P6Cleanup));
    }
    #[tokio::test]
    async fn g45_a8_removed_during_activation_records_done_inside_same_deadline(){
        let(t,_locks,mut j)=journal();let mut config=crate::config::RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@127.0.0.1/nginx_web_ui".into()),_=>None}).unwrap();config.nginx_conf_dir=t.path().to_owned();
        let marker=config.nginx_conf_dir.join(".nwui-apply-uncertain");fs::write(&marker,b"fixture").unwrap();
        let identity="a".repeat(64);let generation=uuid::Uuid::new_v4().to_string();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&identity).unwrap();j.handoff(Handoff::Done,&identity).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();
        let remove=tokio::spawn(async move{tokio::time::sleep(std::time::Duration::from_millis(300)).await;tokio::fs::remove_file(marker).await.unwrap();});
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);let uncertain=wait_apply_uncertain(&config,deadline).await;remove.await.unwrap();
        assert!(record_monitor_result(&mut j,&identity,&generation,Ok(()),uncertain).unwrap());assert!(activation_proof(&j,&identity,&generation));assert!(std::time::Instant::now()<deadline);assert_eq!(j.load().outcome,Outcome::Pg17);
    }
    #[tokio::test]
    async fn g45_a8_kept_waits_deadline_without_failed_record(){
        let(t,_locks,mut j)=journal();let mut config=crate::config::RuntimeConfig::read(|name|match name{"AUTH_SECRET"=>Some("fixture-only".into()),"DATABASE_URL"=>Some("postgres://nginx:fixture-only@127.0.0.1/nginx_web_ui".into()),_=>None}).unwrap();config.nginx_conf_dir=t.path().to_owned();
        let marker=config.nginx_conf_dir.join(".nwui-apply-uncertain");fs::write(&marker,b"fixture").unwrap();let before=fs::read(&marker).unwrap();
        j.set_outcome(Outcome::Pg17).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();let count=j.load().records.len();let deadline=std::time::Instant::now()+std::time::Duration::from_millis(320);
        let uncertain=wait_apply_uncertain(&config,deadline).await;assert!(std::time::Instant::now()>=deadline,"A8 must be observed until the existing deadline");assert!(uncertain);
        assert!(!record_monitor_result(&mut j,&"a".repeat(64),&uuid::Uuid::new_v4().to_string(),Ok(()),uncertain).unwrap());assert_eq!(j.load().records.len(),count);assert_eq!(j.load().outcome,Outcome::Pg17);assert!(!j.load().records.iter().any(|r|matches!(r.state,StepState::Failed{..})));assert_eq!(fs::read(marker).unwrap(),before);
    }
    #[test]
    fn g34_kept_a8_defers_proof_without_failing_activation(){
        let(t,_locks,mut j)=journal();let identity="a".repeat(64);let generation=uuid::Uuid::new_v4().to_string();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&identity).unwrap();j.handoff(Handoff::Done,&identity).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();let before=j.load().records.len();
        assert!(!record_monitor_result(&mut j,&identity,&generation,Ok(()),true).unwrap());assert_eq!(j.load().records.len(),before);assert_eq!(j.load().outcome,Outcome::Pg17);assert!(!activation_proof(&j,&identity,&generation));assert!(!j.load().records.iter().any(|r|matches!(r.state,StepState::Failed{..})||r.step==Step::P6Cleanup));
        let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};assert!(super::super::refusal::status_value(&roots,j.load(),Outcome::Pg17,&j.load().candidate,None).unwrap().failure.is_none());assert!(!super::super::boot::notices(&roots,&j).unwrap().iter().any(|n|n.kind=="system.db_upgrade_activation_failed"));
    }
    #[test]
    fn g34_retry_and_next_boot_reobserve_before_creating_cleanup_proof(){
        for next_boot in [false,true]{let(_t,_locks,mut j)=journal();let first="a".repeat(64);let current=if next_boot{"b".repeat(64)}else{first.clone()};let generation=uuid::Uuid::new_v4().to_string();j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&first).unwrap();j.handoff(Handoff::Done,&first).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();assert!(!record_monitor_result(&mut j,&first,&generation,Ok(()),true).unwrap());
            let start=std::time::Instant::now();let mut retry=CleanupRetry::new(start);assert!(!retry.take(start+std::time::Duration::from_secs(899)));assert!(!activation_proof(&j,&current,&generation));assert!(retry.take(start+std::time::Duration::from_secs(900)));assert!(!retry.take(start+std::time::Duration::from_secs(1800)));
            if next_boot{j.handoff(Handoff::Intent,&current).unwrap();j.handoff(Handoff::Done,&current).unwrap();}j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();assert!(record_monitor_result(&mut j,&current,&generation,Ok(()),false).unwrap());assert!(activation_proof(&j,&current,&generation));assert_eq!(j.load().outcome,Outcome::Pg17);
        }
    }
    #[test]
    fn g34_activation_errors_still_fail_independently_of_a8(){
        for step in ["s6","api-health","owned-generation"]{let(_t,_locks,mut j)=journal();j.set_outcome(Outcome::Pg17).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();let error=StepError{code:"activation_timeout",message:"fixture",detail:Some(step.into())};assert_eq!(record_monitor_result(&mut j,&"a".repeat(64),&uuid::Uuid::new_v4().to_string(),Err(error),true).unwrap_err().code,"activation_timeout");assert_eq!(j.load().outcome,Outcome::ActivationFailed);assert!(matches!(j.load().records.last().unwrap().state,StepState::Failed{..}));assert!(!j.load().records.iter().any(|r|r.step==Step::P6Cleanup||r.step==Step::P5Publish));}
    }
    #[test]
    fn w1_phase2_cleanup_proof_is_bound_to_current_handoff_generation_and_latest_observation() {
        let (_t,_locks,mut j)=journal();let id="a".repeat(64);let generation=uuid::Uuid::new_v4().to_string();j.set_outcome(Outcome::Pg17).unwrap();
        record_monitor_result(&mut j,&id,&generation,Ok(()),false).unwrap();assert!(!activation_proof(&j,&id,&generation));
        j.handoff(Handoff::Intent,&id).unwrap();j.handoff(Handoff::Done,&id).unwrap();assert!(activation_proof(&j,&id,&generation));assert!(!activation_proof(&j,&"b".repeat(64),&generation));assert!(!activation_proof(&j,&id,&uuid::Uuid::new_v4().to_string()));
        j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();assert!(!activation_proof(&j,&id,&generation));record_monitor_result(&mut j,&id,&generation,Ok(()),false).unwrap();assert!(activation_proof(&j,&id,&generation));j.record_retry_failure(Step::Phase2Monitor,"activation_timeout",chrono::Utc::now()).unwrap();assert!(!activation_proof(&j,&id,&generation));
    }
    #[test]
    fn w1_phase2_cleanup_requires_a_only_current_boot_and_never_changes_outcome_on_refusal() {
        let (t,mut locks,mut j)=journal();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        j.set_outcome(Outcome::Pg17).unwrap();let before=serde_json::to_value(j.load()).unwrap();
        let identity=super::super::boot_identity::current().unwrap();
        assert_eq!(cleanup_locked(&roots,&locks,&mut j,&identity).unwrap_err().code,"privilege_prepare_failed");
        locks.release_d();assert_eq!(cleanup_locked(&roots,&locks,&mut j,&"0".repeat(64)).unwrap_err().code,"privilege_prepare_failed");
        // Ready/fresh branches have no old PG15 entries, so no selector/tool/key access is needed.
        cleanup_locked(&roots,&locks,&mut j,&identity).unwrap();
        assert_eq!(serde_json::to_value(j.load()).unwrap(),before);assert!(!roots.db.join("17").exists());
    }
    #[test]
    fn w1_phase2_records_failure_before_outcome_and_never_turns_it_into_restore() {
        let (_t,_locks,mut j)=journal();j.set_outcome(Outcome::Pg17).unwrap();j.record(Step::Phase2Monitor,StepState::Intent,None).unwrap();
        let e=StepError{code:"activation_timeout",message:"fixture",detail:None};assert_eq!(record_monitor_result(&mut j,&"a".repeat(64),&uuid::Uuid::new_v4().to_string(),Err(e),false).unwrap_err().code,"activation_timeout");
        assert_eq!(j.load().outcome,Outcome::ActivationFailed);assert!(matches!(&j.load().records.last().unwrap().state,StepState::Failed{code}if code=="activation_timeout"));assert!(!j.load().records.iter().any(|r|r.step==Step::RRestore));
        j.set_outcome(Outcome::Pg17).unwrap();record_monitor_result(&mut j,&"a".repeat(64),&uuid::Uuid::new_v4().to_string(),Ok(()),false).unwrap();assert_eq!(j.load().outcome,Outcome::Pg17);let r=j.load().records.last().unwrap();assert_eq!(r.step,Step::Phase2Monitor);assert_eq!(r.state,StepState::Done);assert!(r.data.as_ref().unwrap().get("observedAt").is_some());
    }
    #[test]
    fn w1_runner_startup_revalidates_the_attempt_and_phase_under_custody() {
        let (_t,_locks,j)=journal();let id="a".repeat(64);let first=j.load().clone();
        assert_eq!(same_plan(Some(&first),Some(&first),Some(&id)).unwrap(),BootPlan::Phase1);
        assert_eq!(same_plan(None,None,None).unwrap(),BootPlan::Phase1);
        assert_eq!(same_plan(None,Some(&first),Some(&id)).unwrap_err().code,"storage_busy");
        let mut changed=first.clone();changed.attempt_id=uuid::Uuid::new_v4().to_string();assert!(same_plan(Some(&first),Some(&changed),Some(&id)).is_err());
        changed=first.clone();changed.handoff=Handoff::Done;changed.outcome=Outcome::ActivationFailed;
        changed.records.push(super::super::journal::Record{seq:1,step:Step::Handoff,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(serde_json::json!({"bootIdentity":id}))});
        assert!(same_plan(Some(&first),Some(&changed),Some(&id)).is_err());
        assert_eq!(same_plan(Some(&changed),Some(&changed),Some(&id)).unwrap(),BootPlan::Phase2);
        assert_eq!(same_plan(Some(&changed),Some(&changed),None).unwrap(),BootPlan::Phase1);
    }
    #[test]
    fn w1_runner_blocks_unfinished_operator_intent_before_readiness_or_admission_without_journal_changes() {
        let (t,_locks,mut j)=journal();let app=t.path().join("app");let data=serde_json::json!({"discardId":uuid::Uuid::new_v4().to_string()});j.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(data.clone())).unwrap();let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();let error=initial_journal(&app).err().unwrap();assert_eq!(error.code,"pair_mismatch");assert!(error.message.contains("다시 실행"));assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);j.record(Step::CmdRemoveDiscarded,StepState::Done,Some(data)).unwrap();assert!(initial_journal(&app).unwrap().is_some());
    }
    #[test]
    fn w1_runner_initial_read_distinguishes_missing_from_corrupt_without_mutation() {
        let (t,_locks,j)=journal();let app=t.path().join("app");let path=app.join(".nwui-auto/journal-v1.json");
        assert_eq!(initial_journal(&app).unwrap().unwrap().attempt_id,j.load().attempt_id);
        std::fs::write(&path,b"corrupt").unwrap();assert!(initial_journal(&app).is_err());assert_eq!(std::fs::read(&path).unwrap(),b"corrupt");
        std::fs::remove_file(path).unwrap();assert!(initial_journal(&app).unwrap().is_none());
    }
    #[test]
    fn w1_runner_handoff_reads_the_monotonic_live_flag_without_rewriting_the_snapshot() {
        let mut snapshot=crate::db_upgrade::pair::Pair::fresh("arm64");snapshot.ready=true;snapshot.fresh_step=None;snapshot.system_identifier=Some("123".into());
        assert!(required_live_pair(&snapshot,&snapshot).is_err());let mut live=snapshot.clone();live.encryption_key_required=true;required_live_pair(&snapshot,&live).unwrap();assert!(!snapshot.encryption_key_required);
        live.pair_id=uuid::Uuid::new_v4().to_string();assert!(required_live_pair(&snapshot,&live).is_err());
    }
    #[test]
    fn w1_runner_handoff_refuses_uncommitted_unverified_or_wrong_attempt_inputs_without_mutation() {
        let (t,locks,mut j)=journal();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        let mut selection=super::super::StorageSelection{version:1,epoch:1,outcome:Outcome::Pg17,branch:Branch::Ready,pgdata:roots.db.join("17/data"),backup_id:None,schema_profile:"none".into(),key_binding:super::super::KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()}};
        j.record(Step::P5Publish,StepState::Done,Some(serde_json::json!({"authorityEvent":"privilege-split-off"}))).unwrap();let p=crate::nonroot::Prepared{payload:None,key_receipt:None};let id="a".repeat(64);
        assert!(handoff_inputs(&roots,&locks,&j,&selection,&p,&id).is_err());j.record(Step::P5Commit,StepState::Done,None).unwrap();let before=std::fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        if !crate::nonroot::PRIVILEGE_SPLIT {handoff_inputs(&roots,&locks,&j,&selection,&p,&id).unwrap();}
        selection.key_binding=super::super::KeyBinding::Unverified;assert!(handoff_inputs(&roots,&locks,&j,&selection,&p,&id).is_err());selection.key_binding=super::super::KeyBinding::Verified{pair_id:uuid::Uuid::new_v4().to_string()};
        assert!(handoff_inputs(&roots,&locks,&j,&selection,&p,"invalid").is_err());selection.version=2;assert!(handoff_inputs(&roots,&locks,&j,&selection,&p,&id).is_err());selection.version=1;selection.branch=Branch::Upgrade;assert!(handoff_inputs(&roots,&locks,&j,&selection,&p,&id).is_err());
        assert_eq!(std::fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert!(locks.holds_a() && locks.holds_d());
    }
    #[test]
    fn w1_runner_resume_consumes_old_manifests_before_the_next_admission_can_prune_previous() {
        let (t,locks,j)=journal();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:crate::db_upgrade::engine::architecture().unwrap().into()};
        let old=uuid::Uuid::new_v4().to_string();let mut state=j.load().clone();
        state.previous.push(super::super::journal::PreviousAttempt{attempt_id:old.clone(),outcome:Outcome::Restored,candidate:state.candidate.clone(),last_failure:None,generation:None,discard_id:None,ended_at:chrono::Utc::now().to_rfc3339()});
        let path=roots.app.join(".nwui-auto/journal-v1.json");let before=serde_json::to_vec(&state).unwrap();fs::write(&path,&before).unwrap();
        let root=roots.app.join(".nwui-auto");let old_manifest=root.join(format!("backup-{old}.jsonl"));let current=root.join(format!("backup-{}.jsonl",state.attempt_id));
        for p in [&old_manifest,&current] {fs::write(p,b"manifest fixture").unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o600)).unwrap();}
        let resumed=resume_phase1_locked(&roots,&locks).unwrap();assert!(!old_manifest.exists());assert!(current.exists());assert_eq!(resumed.load().previous.len(),1);assert_eq!(fs::read(&path).unwrap(),before);
        resume_phase1_locked(&roots,&locks).unwrap();assert_eq!(fs::read(&current).unwrap(),b"manifest fixture");
    }
    #[test]
    fn w1_runner_phase2_requires_this_boot_completed_handoff_even_after_activation_failure() {
        let (_t,_locks,j)=journal();let id="a".repeat(64);let mut state=j.load().clone();
        assert_eq!(boot_plan(&state,Some(&id)),BootPlan::Phase1);
        state.handoff=Handoff::Done;state.outcome=Outcome::Pg17;
        state.records.push(super::super::journal::Record{seq:1,step:Step::Handoff,state:StepState::Done,at:chrono::Utc::now().to_rfc3339(),data:Some(serde_json::json!({"bootIdentity":id}))});
        assert_eq!(boot_plan(&state,Some(&id)),BootPlan::Phase2);
        for current in [None,Some(""),Some("invalid"),Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")] {assert_eq!(boot_plan(&state,current),BootPlan::Phase1);}
        state.records.last_mut().unwrap().state=StepState::Intent;assert_eq!(boot_plan(&state,Some(&id)),BootPlan::Phase1);
        state.records.last_mut().unwrap().state=StepState::Done;state.outcome=Outcome::ActivationFailed;assert_eq!(boot_plan(&state,Some(&id)),BootPlan::Phase2);
    }
    #[tokio::test]
    async fn cleanup_retry_shutdown_does_not_wait_out_the_fifteen_minute_delay(){
        const CHILD:&str="NWUI_CLEANUP_TIMER_CHILD";
        if std::env::var_os(CHILD).is_none(){let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::runner::tests::cleanup_retry_shutdown_does_not_wait_out_the_fifteen_minute_delay","--nocapture"]).env(CHILD,"1").output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));return;}
        use std::sync::atomic::Ordering;crate::db_upgrade::files::CANCELLED.store(false,Ordering::Relaxed);let signal=tokio::spawn(async{tokio::time::sleep(std::time::Duration::from_millis(20)).await;crate::db_upgrade::files::CANCELLED.store(true,Ordering::Relaxed);});let start=std::time::Instant::now();assert!(!wait_for_cleanup_retry(start+std::time::Duration::from_secs(900)).await);assert!(start.elapsed()<std::time::Duration::from_secs(2));signal.await.unwrap();
    }
    #[test]
    fn committed_failure_uses_the_callers_step_and_only_the_current_unhanded_upgrade(){
        let(_t,_l,mut j)=journal();j.record(Step::P5Commit,StepState::Done,None).unwrap();let mut frame=j.load().clone();assert_eq!(committed_failure_outcome(&frame,Step::Y2),Outcome::RefusedPrivilege);assert_eq!(committed_failure_outcome(&frame,Step::S4Activate),Outcome::ActivationFailed);frame.branch=Branch::Upgrade;assert_eq!(committed_failure_outcome(&frame,Step::P5Publish),Outcome::ActivationFailed);
        j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&"a".repeat(64)).unwrap();j.handoff(Handoff::Done,&"a".repeat(64)).unwrap();let mut frame=j.load().clone();frame.branch=Branch::Upgrade;assert_eq!(committed_failure_outcome(&frame,Step::Y2),Outcome::RefusedPrivilege);assert_eq!(committed_failure_outcome(&frame,Step::S4Activate),Outcome::ActivationFailed);
        let phase=Phase1Step::new(Step::Y1);let worker=phase.clone();std::thread::spawn(move||worker.set(Step::Y2)).join().unwrap();assert_eq!(phase.get(),Step::Y2);assert_eq!(committed_failure_outcome(&frame,phase.get()),Outcome::RefusedPrivilege);
    }
    #[test]
    fn a_new_boots_privilege_refusal_replaces_an_old_activation_receipt_atomically(){
        let(t,l,mut j)=journal();let error=StepError{code:"boot_step_failed",message:"fixture",detail:None};let old="a".repeat(64);j.fail_attempt_with_identity(Step::S4Activate,&error,Outcome::ActivationFailed,chrono::Utc::now(),Some(&old)).unwrap();let seq=j.load().records.last().unwrap().seq;let identity="b".repeat(64);j.fail_attempt_with_identity(Step::Y2,&error,Outcome::RefusedPrivilege,chrono::Utc::now(),Some(&identity)).unwrap();let opened=Journal::open(&t.path().join("app"),&l).unwrap();assert_eq!(opened.load().records.last().unwrap().seq,seq+2);assert_eq!(boot_plan(opened.load(),Some(&identity)),BootPlan::ParkedRefusal);assert_eq!(boot_plan(opened.load(),Some(&old)),BootPlan::Phase1);
        let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};let status=super::super::refusal::status_value(&roots,opened.load(),Outcome::RefusedPrivilege,&opened.load().candidate,None).unwrap();assert_eq!(status.failure.as_ref().unwrap().message,"PG17 데이터는 변경되지 않았습니다 — 원인을 해결하고 재시작하세요");assert!(!serde_json::to_string(&status).unwrap().contains("restore-pg15"));assert!(super::super::boot::notices(&roots,&opened).unwrap().is_empty());
    }
    #[test]
    fn cleanup_retry_with_injected_clock_waits_fifteen_minutes_and_can_be_taken_only_once(){
        let start=std::time::Instant::now();let mut retry=CleanupRetry::new(start);assert!(!retry.take(start));assert!(!retry.take(start+std::time::Duration::from_secs(899)));assert!(retry.take(start+std::time::Duration::from_secs(900)));assert!(!retry.take(start+std::time::Duration::from_secs(901)));assert!(!retry.take(start+std::time::Duration::from_secs(3600)));
    }
    #[test]
    fn pre_handoff_activation_failure_has_one_atomic_receipt_but_phase2_failure_does_not(){
        let(t,l,mut j)=journal();let id="a".repeat(64);let error=StepError{code:"privilege_activate_failed",message:"fixture",detail:None};j.set_outcome(Outcome::Pg17).unwrap();let seq=j.load().records.last().map(|r|r.seq).unwrap_or(0);
        j.fail_attempt_with_identity(Step::S4Activate,&error,Outcome::ActivationFailed,chrono::Utc::now(),Some(&id)).unwrap();let reopened=Journal::open(&t.path().join("app"),&l).unwrap();assert_eq!(reopened.load().records.last().unwrap().seq,seq+2);assert_eq!(boot_plan(reopened.load(),Some(&id)),BootPlan::ParkedRefusal);assert_eq!(boot_plan(reopened.load(),Some(&"b".repeat(64))),BootPlan::Phase1);
        let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};let status=super::super::refusal::status_value(&roots,j.load(),Outcome::ActivationFailed,&j.load().candidate,None).unwrap();assert_eq!(status.failure.unwrap().message,"업그레이드는 완료됐지만 서비스 시작에 실패했습니다");
        j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&id).unwrap();j.handoff(Handoff::Done,&id).unwrap();j.fail_attempt(Step::Phase2Monitor,&error,Outcome::ActivationFailed,chrono::Utc::now()).unwrap();assert_eq!(boot_plan(j.load(),Some(&id)),BootPlan::Phase2);assert!(super::super::refusal::outcome(j.load(),&id).is_none());
    }
    #[test]
    fn w1_runner_handoff_identity_and_outcome_persist_atomically_for_restart() {
        let (t,locks,mut j)=journal();let id="a".repeat(64);assert!(j.handoff(Handoff::Intent,&id).is_err());
        j.set_outcome(Outcome::Pg17).unwrap();assert!(j.handoff(Handoff::Done,&id).is_err());j.handoff(Handoff::Intent,&id).unwrap();
        assert_eq!(boot_plan(j.load(),Some(&id)),BootPlan::Phase1);assert!(j.handoff(Handoff::Done,&"b".repeat(64)).is_err());
        j.handoff(Handoff::Done,&id).unwrap();let reopened=Journal::open(&t.path().join("app"),&locks).unwrap();assert_eq!(boot_plan(reopened.load(),Some(&id)),BootPlan::Phase2);
        let mut j=reopened;j.handoff(Handoff::Intent,&"b".repeat(64)).unwrap();assert_eq!(boot_plan(j.load(),Some(&id)),BootPlan::Phase1);
    }
    #[test]
    fn w1_runner_adopts_custody_without_unlocking_duplicates_and_releases_only_d() {
        use std::{fs::File,os::fd::{AsFd,OwnedFd}};
        let (t,locks,j)=journal();let app=t.path().join("app");let db=t.path().join("db");
        let a=locks.a_fd().try_clone_to_owned().unwrap();let d=locks.d_fd().unwrap().try_clone_to_owned().unwrap();drop(j);drop(locks);
        let mut adopted=StorageLocks::from_custody(&db,&app,a,Some(d)).unwrap();let external_a=File::open(&app).unwrap();let external_d=File::open(&db).unwrap();assert!(external_a.try_lock().is_err());assert!(external_d.try_lock().is_err());
        adopted.release_d();assert!(!adopted.holds_d());external_d.try_lock().unwrap();assert!(external_a.try_lock().is_err());
        let copy:OwnedFd=adopted.a_fd().try_clone_to_owned().unwrap();drop(adopted);assert!(external_a.try_lock().is_err());drop(copy);external_a.try_lock().unwrap();
        let wrong=File::open(&db).unwrap();assert!(StorageLocks::from_custody(&db,&app,wrong.as_fd().try_clone_to_owned().unwrap(),None).is_err());
    }
    #[test]
    fn w1_runner_readiness_fd_survives_in_parent_but_not_exec_children() {
        const CHILD:&str="NWUI_RUNNER_READY_FD_FIXTURE";
        if std::env::var_os(CHILD).is_some() {
            protect_readiness_fd().unwrap();assert_ne!(unsafe{libc::fcntl(3,libc::F_GETFD)}&libc::FD_CLOEXEC,0);
            assert!(std::process::Command::new("/bin/sh").args(["-c","test ! -e /proc/self/fd/3"]).status().unwrap().success());
            let (t,mut locks,mut j)=journal();let app=t.path().join("app");let identity="a".repeat(64);j.set_outcome(Outcome::Pg17).unwrap();j.handoff(Handoff::Intent,&identity).unwrap();j.handoff(Handoff::Done,&identity).unwrap();
            assert_eq!(resume_readiness(&app,&locks,&j,Some(&identity)).unwrap_err().code,"privilege_prepare_failed");locks.release_d();
            assert!(resume_readiness(&app,&locks,&j,Some(&"b".repeat(64))).is_err());resume_readiness(&app,&locks,&j,Some(&identity)).unwrap();
            j.fail_attempt(Step::Phase2Monitor,&StepError{code:"activation_timeout",message:"fixture",detail:None},Outcome::ActivationFailed,chrono::Utc::now()).unwrap();let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();resume_readiness(&app,&locks,&j,Some(&identity)).unwrap();assert_eq!(j.load().outcome,Outcome::ActivationFailed);assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);assert!(fs::File::open(&app).unwrap().try_lock().is_err());fs::File::open(t.path().join("db")).unwrap().try_lock().unwrap();return;
        }
        use std::{io::Read,os::{fd::AsRawFd,unix::process::CommandExt}};
        let (mut read,write)=std::os::unix::net::UnixStream::pair().unwrap();read.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut command=std::process::Command::new(std::env::current_exe().unwrap());command.args(["--exact","upgrade::runner::tests::w1_runner_readiness_fd_survives_in_parent_but_not_exec_children","--nocapture"]).env_clear().env(CHILD,"1");
        unsafe{command.pre_exec(move||{if libc::dup2(write.as_raw_fd(),3)<0{return Err(std::io::Error::last_os_error());}Ok(())});}
        let mut child=command.spawn().unwrap();let mut bytes=[0;2];read.read_exact(&mut bytes).unwrap();assert_eq!(bytes,[b'\n';2]);assert!(child.wait().unwrap().success());
    }
    #[test]
    fn w1_runner_s4_return_must_equal_the_real_owner_done_and_partial_is_never_complete() {
        let (_t,_locks,mut j)=journal();let selection=super::super::StorageSelection {version:1,epoch:1,outcome:Outcome::Pg17,branch:Branch::Ready,pgdata:"/fixture/data".into(),backup_id:None,schema_profile:"none".into(),key_binding:super::super::KeyBinding::Verified {pair_id:"00000000-0000-4000-8000-000000000001".into()}};
        for r in crate::nonroot::fixture_prepare_records(&selection,&j.load().attempt_id,1,true) {j.record(r.step,r.state,r.data).unwrap();}
        let mut p=crate::nonroot::prepared_from_journal(&j).unwrap();verify_prepared_return_for(&j,&p,true).unwrap();p.key_receipt.as_mut().unwrap().selection_epoch+=1;assert!(verify_prepared_return_for(&j,&p,true).is_err());
        for r in crate::nonroot::fixture_prepare_records(&selection,&j.load().attempt_id,3,false) {j.record(r.step,r.state,r.data).unwrap();}
        assert!(verify_prepared_return_for(&j,&p,true).is_err());let partial=crate::nonroot::prepared_from_journal(&j).unwrap();assert!(partial.payload.is_none() && partial.key_receipt.is_none());assert!(verify_prepared_return_for(&j,&partial,true).is_err());
    }
    #[tokio::test]
    async fn w1_runner_split_off_records_only_its_own_event_and_refuses_stray_s4() {
        if crate::nonroot::PRIVILEGE_SPLIT {return;}
        let (_t,locks,mut j)=journal();let selection=super::super::StorageSelection{version:1,epoch:1,outcome:Outcome::Pg17,branch:Branch::Ready,pgdata:"/not-opened/data".into(),backup_id:None,schema_profile:"none".into(),key_binding:super::super::KeyBinding::Verified{pair_id:"00000000-0000-4000-8000-000000000001".into()}};
        let pg=super::super::PrivatePg{socket_dir:"/not-opened/socket".into(),port:50433,dbname:"nginx_web_ui".into(),role:"nginx".into(),major:17};
        let p=prepare_privileges(&selection,&locks,&pg,&mut j,crate::publisher::boot::BootMode::Conversion).await.unwrap();assert!(p.payload.is_none() && p.key_receipt.is_none());
        assert!(!j.load().records.iter().any(|r|r.step==Step::S4Prepare));assert!(super::super::generation::completed_preparation(&j).is_some());
        assert_eq!(j.load().records.last().unwrap().data,Some(serde_json::json!({"authorityEvent":"privilege-split-off"})));
        for r in crate::nonroot::fixture_prepare_records(&selection,&j.load().attempt_id,j.load().records.last().unwrap().seq+1,false) {j.record(r.step,r.state,r.data).unwrap();}
        let count=j.load().records.len();assert_eq!(prepare_privileges(&selection,&locks,&pg,&mut j,crate::publisher::boot::BootMode::Conversion).await.unwrap_err().code,"privilege_prepare_failed");assert_eq!(j.load().records.len(),count);
    }
    #[test]
    fn w1_runner_new_boot_cleanup_removes_only_its_two_records_and_is_idempotent() {
        let t=tempfile::tempdir().unwrap();for name in ["nwui-storage.json","nwui-db-upgrade.json","foreign.json"] {let p=t.path().join(name);fs::write(&p,"old").unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o644)).unwrap();}
        clear_runtime_records_at(t.path()).unwrap();clear_runtime_records_at(t.path()).unwrap();assert_eq!(fs::read(t.path().join("foreign.json")).unwrap(),b"old");
        assert!(!t.path().join("nwui-storage.json").exists());assert!(!t.path().join("nwui-db-upgrade.json").exists());
    }
    #[test]
    fn w1_runner_unsafe_runtime_leaf_or_parent_preserves_both_records() {
        for kind in ["symlink","dir","mode","hardlink","parent"] {let t=tempfile::tempdir().unwrap();let first=t.path().join("nwui-storage.json");fs::write(&first,"keep").unwrap();fs::set_permissions(&first,fs::Permissions::from_mode(0o644)).unwrap();let other=t.path().join("nwui-db-upgrade.json");
            match kind {"symlink"=>symlink(&first,&other).unwrap(),"dir"=>fs::create_dir(&other).unwrap(),"hardlink"=>fs::hard_link(&first,&other).unwrap(),_=>{fs::write(&other,"keep").unwrap();fs::set_permissions(&other,fs::Permissions::from_mode(if kind=="mode"{0o666}else{0o644})).unwrap();}}
            if kind=="parent" {fs::set_permissions(t.path(),fs::Permissions::from_mode(0o777)).unwrap();}
            assert!(clear_runtime_records_at(t.path()).is_err());assert_eq!(fs::read(&first).unwrap(),b"keep");assert!(fs::symlink_metadata(&other).is_ok());
        }
    }
}

#[cfg(test)] #[path="credential_tests.rs"] mod credential_tests;

#[cfg(test)]mod desktop_shared_tests{
 use super::*;
 #[test]fn desktop_shared_prejournal_failure_keeps_original_code_and_creates_nothing(){
  let t=tempfile::tempdir().unwrap();let error=StepError{code:"key_missing",message:"original",detail:Some("original detail".into())};
  let got=require_recovery_journal(t.path(),&error).unwrap_err();assert_eq!(got.code,"key_missing");assert_eq!(got.detail,error.detail);assert!(!t.path().join(".nwui-auto").exists());
  std::fs::create_dir(t.path().join(".nwui-auto")).unwrap();std::fs::write(t.path().join(".nwui-auto/journal-v1.json"),b"corrupt").unwrap();assert_ne!(require_recovery_journal(t.path(),&error).unwrap_err().code,"key_missing");
 }
}
