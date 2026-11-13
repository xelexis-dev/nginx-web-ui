//! Retry admission happens only at container start; never while an installation serves traffic.
use super::{ErrorClass,StepError,StepState,class_of,journal::{Candidate,JournalV1}};
#[cfg(test)]
use super::{Journal,Step};
#[derive(Debug,PartialEq,Eq)]
pub(crate) enum Admission {Attempt,SameCandidateRefused,Backoff(String)}
fn same(a:&Candidate,b:&Candidate)->bool {a.revision==b.revision && a.pins_sha256==b.pins_sha256 && a.migrations_sha256==b.migrations_sha256}
fn invalid()->StepError {StepError {code:"invalid_arguments",message:"재시도 표시가 비어 있거나 너무 깁니다. 설정을 확인하세요.",detail:None}}
fn valid_label(label:Option<&str>)->Result<Option<&str>,StepError> {
    if label.is_some_and(|s|s.is_empty() || s.len()>128 || s.contains('\0')) {return Err(invalid());}Ok(label)
}
/// Trusted startup settings only; labels are never printed or taken from a socket request.
pub(crate) struct Policy {pub postponed:bool,pub label:Option<String>}
impl Policy {
    pub(crate) fn startup()->Result<Self,StepError>{Self::read(crate::nonroot::startup_value)}
    fn read(get:impl Fn(&str)->Option<String>)->Result<Self,StepError>{
        let label=get("NWUI_DB_UPGRADE_RETRY");valid_label(label.as_deref())?;
        Ok(Self{postponed:get("NWUI_DB_UPGRADE").as_deref()==Some("off"),label})
    }
}
pub(crate) fn admit(j:&JournalV1,candidate:&Candidate,label:Option<&str>,space_sufficient:bool,now:chrono::DateTime<chrono::Utc>)->Result<Admission,StepError> {
    let label=valid_label(label)?;
    if !same(&j.candidate,candidate) || label.is_some_and(|s|!j.retry.consumed_labels.iter().any(|v|v==s)) {return Ok(Admission::Attempt);}
    let failure=j.records.iter().rev().find_map(|r|if let StepState::Failed{code}=&r.state {Some(code.as_str())}else{None});
    let Some(code)=failure else{return Ok(Admission::Attempt);};
    if class_of(code)==ErrorClass::Deterministic {return Ok(Admission::SameCandidateRefused);}
    if matches!(code,"insufficient_space"|"insufficient_inodes") && space_sufficient {return Ok(Admission::Attempt);}
    if let Some(until)=&j.retry.backoff_until {
        let until_time=chrono::DateTime::parse_from_rfc3339(until).map_err(|_|invalid())?;
        if now<until_time {return Ok(Admission::Backoff(until.clone()));}
    }
    Ok(Admission::Attempt)
}
/// Both the offline CLI and status use the same admission decision; CLI supplies label=None to admit.
pub(crate) fn restored_next_action(admission:&Admission)->&'static str{
    match admission{
        Admission::Attempt=>"PG15로 복원했습니다. 1.19.11로 시작하거나, 2.0 이미지를 다시 시작하면 새 이전을 진행합니다.",
        Admission::Backoff(_)=>"PG15로 복원했습니다. 1.19.11로 시작하거나, 표시된 다음 재시도 가능 시각 이후 2.0 이미지를 다시 시작하면 새 이전을 진행합니다.",
        Admission::SameCandidateRefused=>"PG15로 복원했습니다. 1.19.11로 시작하세요. 이 이미지는 같은 원인으로 다시 이전하지 않습니다. 원인을 해결했다면 NWUI_DB_UPGRADE_RETRY에 새 값을 지정하고 다시 시작하세요.",
    }
}
/// The real step/code remain authoritative; this never invents a successful outcome.
#[cfg(test)]
pub(crate) fn failed(j:&mut Journal,step:Step,code:&'static str,now:chrono::DateTime<chrono::Utc>)->Result<(),StepError> {
    j.record_retry_failure(step,code,now)
}
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Branch,StorageLocks};
    use std::{fs,os::unix::fs::PermissionsExt};
    fn fixture()->(tempfile::TempDir,StorageLocks,Journal) {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let j=Journal::begin(&app,&locks,Branch::Ready,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();(t,locks,j)
    }
    fn now()->chrono::DateTime<chrono::Utc> {"2026-10-28T00:00:00Z".parse().unwrap()}
    #[test]
    fn startup_policy_reads_only_documented_settings_and_validates_retry_labels(){
        let default=Policy::read(|_|None).unwrap();assert!(!default.postponed);assert!(default.label.is_none());
        for value in ["off","on",""]{let policy=Policy::read(|name|match name{"NWUI_DB_UPGRADE"=>Some(value.into()),"NWUI_DB_UPGRADE_RETRY"=>Some("operator-fix".into()),_=>panic!("unexpected setting")}).unwrap();assert_eq!(policy.postponed,value=="off");assert_eq!(policy.label.as_deref(),Some("operator-fix"));}
        for label in ["",&"x".repeat(129),"x\0y"]{assert!(Policy::read(|name|(name=="NWUI_DB_UPGRADE_RETRY").then(||label.into())).is_err());}
    }
    #[test]
    fn restored_copy_follows_real_admission_for_success_failure_label_and_capacity(){
        let (_t,_locks,mut j)=fixture();let candidate=j.load().candidate.clone();j.set_outcome(super::super::Outcome::Pg17).unwrap();let choice=admit(j.load(),&candidate,None,true,now()).unwrap();assert_eq!(choice,Admission::Attempt);assert!(restored_next_action(&choice).contains("새 이전을 진행합니다."));
        failed(&mut j,Step::S4Activate,"privilege_activate_failed",now()).unwrap();j.set_outcome(super::super::Outcome::ActivationFailed).unwrap();let choice=admit(j.load(),&candidate,None,true,now()).unwrap();assert_eq!(choice,Admission::SameCandidateRefused);assert!(restored_next_action(&choice).contains("이 이미지는 같은 원인으로 다시 이전하지 않습니다."));let retry=admit(j.load(),&candidate,Some("fixed"),true,now()).unwrap();assert_eq!(retry,Admission::Attempt);
        failed(&mut j,Step::P3Convert,"interrupted",now()).unwrap();failed(&mut j,Step::P3Convert,"interrupted",now()).unwrap();let choice=admit(j.load(),&candidate,None,true,now()).unwrap();assert!(matches!(choice,Admission::Backoff(_)));assert!(restored_next_action(&choice).contains("표시된 다음 재시도 가능 시각 이후"));
        failed(&mut j,Step::P2Preflight,"insufficient_space",now()).unwrap();let mut state=j.load().clone();state.retry.backoff_until=Some((now()+chrono::Duration::hours(24)).to_rfc3339());let choice=admit(&state,&candidate,None,true,now()).unwrap();assert_eq!(choice,Admission::Attempt);
    }
    #[test]
    fn w1_retry_deterministic_failure_requires_new_candidate_or_an_unconsumed_label() {
        let (_t,_locks,mut j)=fixture();failed(&mut j,Step::P2Preflight,"ledger_unknown",now()).unwrap();let candidate=j.load().candidate.clone();
        assert_eq!(admit(j.load(),&candidate,None,false,now()).unwrap(),Admission::SameCandidateRefused);
        assert_eq!(admit(j.load(),&candidate,Some("operator-1"),false,now()).unwrap(),Admission::Attempt);
        let mut state=j.load().clone();state.retry.consumed_labels.push("operator-1".into());assert_eq!(admit(&state,&candidate,Some("operator-1"),false,now()).unwrap(),Admission::SameCandidateRefused);
        let mut changed=candidate.clone();changed.migrations_sha256="f".repeat(64);assert_eq!(admit(&state,&changed,None,false,now()).unwrap(),Admission::Attempt);
    }
    #[test]
    fn w1_retry_two_consecutive_interruptions_back_off_but_a_real_capacity_fix_retries() {
        let (_t,_locks,mut j)=fixture();let candidate=j.load().candidate.clone();failed(&mut j,Step::P1Backup,"interrupted",now()).unwrap();assert_eq!(admit(j.load(),&candidate,None,false,now()).unwrap(),Admission::Attempt);
        failed(&mut j,Step::P1Backup,"interrupted",now()).unwrap();assert!(matches!(admit(j.load(),&candidate,None,false,now()).unwrap(),Admission::Backoff(_)));
        assert_eq!(admit(j.load(),&candidate,None,false,now()+chrono::Duration::hours(24)).unwrap(),Admission::Attempt);
        failed(&mut j,Step::P1Backup,"insufficient_space",now()).unwrap();assert_eq!(j.load().retry.interruptions,0);assert_eq!(admit(j.load(),&candidate,None,true,now()).unwrap(),Admission::Attempt);
    }
    #[test]
    fn w1_retry_invalid_labels_never_advance_the_journal() {
        let (_t,_locks,j)=fixture();let candidate=j.load().candidate.clone();let count=j.load().records.len();for label in ["",&"x".repeat(129),"a\0b"]{assert!(admit(j.load(),&candidate,Some(label),false,now()).is_err());}assert_eq!(count,j.load().records.len());
    }
}
