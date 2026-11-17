//! Read-only operator diagnostics, independent of AppState and the offline copy-tool mounts.
use super::{StepError,journal,selector,status,types::UpgradeStatus};
use std::path::Path;
fn io_error()->StepError {StepError {code:"io_error",message:"업그레이드 기록을 읽지 못했습니다. 기존 내용을 보존했습니다.",detail:None}}
fn present(path:&Path)->Result<bool,StepError> {
    match std::fs::symlink_metadata(path) {Ok(_)=>Ok(true),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(_)=>Err(io_error())}
}
fn bounded_code(value:&str)->String {value.chars().filter(|c|c.is_ascii_alphanumeric() || matches!(c,'_'|'-'|'.')).take(128).collect()}
fn mismatch()->StepError {StepError{code:"pair_mismatch",message:"선택된 데이터와 보관된 검증 기록이 일치하지 않습니다. 기존 내용을 보존했습니다.",detail:None}}
fn command_done(j:&journal::JournalV1,step:super::Step,field:&str,id:&str)->bool {
    j.records.iter().any(|r|r.step==step && r.state==super::StepState::Done && r.data.as_ref().and_then(|d|d.get(field)).and_then(|v|v.as_str())==Some(id))
}
/// Project only journal-authorized current and prior discard ids; never discover ids on disk.
pub(crate) fn retained_discards(roots:&crate::db_upgrade::layout::Roots,j:&journal::JournalV1)->Result<Vec<super::types::RetainedCopy>,StepError> {
    use super::{Step,types::RetainedCopy};
    let mut ids=Vec::new();
    if let Some(id)=super::rollover::retention(j)?.1 {ids.push(id);}
    for old in &j.previous {if let Some(id)=&old.discard_id {if old.generation.as_ref()!=Some(id){return Err(mismatch());}ids.push(id.clone());}}
    let mut discarded=Vec::new();let mut seen=std::collections::HashSet::new();
    for id in ids {
        if !super::types::canonical_generation(&id){return Err(mismatch());}if !seen.insert(id.clone()){continue;}
        if command_done(&j,Step::CmdRemoveDiscarded,"discardId",&id) {continue;}
        let path=roots.db.join(".nwui-discard").join(&id);
        let bytes=if present(&path)? {
            crate::db_upgrade::files::real_path(&path).map_err(|_|mismatch())?;
            use std::os::unix::fs::MetadataExt;
            let device=std::fs::symlink_metadata(&roots.db).map_err(|_|io_error())?.dev();
            super::refusal::bytes(&path,device,0,&mut 0)?
        }else{0};
        discarded.push(RetainedCopy{remove_command:format!("nwui-db-upgrade remove-discarded --confirm {id}"),id,bytes});
    }
    Ok(discarded)
}
pub(crate) fn offline_status(roots:&crate::db_upgrade::layout::Roots,candidate:&journal::Candidate,now:chrono::DateTime<chrono::Utc>)->Result<Option<UpgradeStatus>,StepError> {
    use super::{Outcome,Step,StepState,types::*};
    let selected=if present(&roots.app.join(".nwui-auto/active-v1.json"))? {Some(selector::read_selector(&roots.app)?)}else{None};
    if !present(&roots.app.join(".nwui-auto/journal-v1.json"))? {return Ok(None);}
    let j=journal::read_only(&roots.app)?;
    let final_status=if super::refusal::final_outcome(j.outcome) {
        Some(super::refusal::status_value(roots,&j,j.outcome,candidate,None)?)
    }else{None};
    let last=j.records.last();
    let updated_at=last.map(|r|r.at.clone()).unwrap_or_else(||now.to_rfc3339());
    let mut backup=None;
    let consumed=matches!(j.outcome,Outcome::Restored) || j.records.iter().any(|r|matches!(r.step,Step::RRestore|Step::CmdRestorePg15) && r.state==StepState::Done)
        || command_done(&j,Step::CmdRemoveBackup,"backupId",&j.attempt_id);
    if final_status.is_none() && !consumed {
        if let Some(saved)=j.records.iter().rev().find(|r|r.step==Step::P1Backup && r.state==StepState::Done) {
            if selected.as_ref().is_some_and(|s|s.active_major==17 && s.backup_id.as_deref()!=Some(j.attempt_id.as_str())) {return Err(mismatch());}
            let bytes=saved.data.as_ref().and_then(|v|v.get("sourceBytes")).and_then(|v|v.as_u64()).ok_or_else(mismatch)?;
            let path=roots.db.join(".nwui-pg15-backup").join(&j.attempt_id);
            if present(&path)? {super::target::directory(&path)?;backup=Some(RetainedCopy{id:j.attempt_id.clone(),bytes,remove_command:format!("nwui-db-upgrade remove-pg15-backup --confirm {}",j.attempt_id)});}
        }
    }
    if let Some(value)=final_status {return Ok(Some(value));}
    let discarded=retained_discards(roots,&j)?;
    // Operator command failures must not override the attempt's admission decision.
    let mut admission=j.clone();admission.records.retain(|r|!matches!(r.step,Step::CmdRestorePg15|Step::CmdRemoveBackup|Step::CmdRemoveDiscarded));
    let decision=super::retry::admit(&admission,candidate,None,true,now)?;
    let next=match decision {
        super::retry::Admission::SameCandidateRefused=>"원인을 해결했다면 NWUI_DB_UPGRADE_RETRY에 새 값을 지정하고 다시 시작하면 한 번 더 시도합니다.",
        super::retry::Admission::Backoff(_)=>"원인을 해결한 뒤 표시된 다음 재시도 가능 시각 이후에 컨테이너를 다시 시작하면 자동으로 다시 시도합니다.",
        super::retry::Admission::Attempt=>"기록된 원인을 해결한 뒤 컨테이너를 다시 시작하세요.",
    };
    let mut failure=journal::last_attempt_failure(&j.records).map(|f| {
        let message="시작 준비를 보류했습니다. 원본과 복구 기록은 보존했습니다.".into();
        let next_action=super::storage_modes::refusal_detail(&f.code).unwrap_or_else(||next.into());
        StatusFailure{class:super::class_of(&f.code),code:f.code,message,next_action}
    });
    let (cleanup_pending,pg17_failure)=status::pg17_pending(&j);
    if j.outcome==Outcome::Pg17 {failure=pg17_failure;}
    let retry=StatusRetry{next_eligible_at:match &decision{super::retry::Admission::Backoff(at)=>Some(at.clone()),_=>None},reason:failure.is_some().then(||next.into())};
    let locations=backup.as_ref().map(|b|vec![roots.db.join(".nwui-pg15-backup").join(&b.id).display().to_string()]).unwrap_or_default();
    let step=last.map(|r|r.step).unwrap_or(Step::P0);let (index,total)=status::step_position(step);
    let started_at=j.records.iter().rev().find(|r|r.step==step&&r.state==StepState::Intent).map(|r|r.at.clone()).unwrap_or_else(||updated_at.clone());
    Ok(Some(UpgradeStatus{warnings:super::status::storage_warnings(),version:1,updated_at,outcome:j.outcome,branch:j.branch,step:StatusStep{id:step,index,total,started_at,bytes_done:0,bytes_total:0},failure,space:None,locations,backup,discarded,cleanup_pending,retry}))
}
#[cfg(test)]
fn render_at(status_path:&Path,app_root:&Path)->Result<String,StepError> {
    let roots=crate::db_upgrade::layout::Roots{app:app_root.into(),db:app_root.parent().unwrap().join("db"),architecture:"arm64".into()};
    let candidate=if present(&app_root.join(".nwui-auto/journal-v1.json"))? {journal::read_only(app_root)?.candidate}else{journal::Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}};
    render(status::read_at(status_path),&roots,&candidate,chrono::Utc::now())
}
fn render(value:Result<UpgradeStatus,StepError>,roots:&crate::db_upgrade::layout::Roots,candidate:&journal::Candidate,now:chrono::DateTime<chrono::Utc>)->Result<String,StepError> {
    let live=match value {Ok(v)=>Some(v),Err(e)if e.code=="status_missing"=>None,Err(e)=>return Err(e)};
    if let Some(value)=live.as_ref().filter(|v|v.failure.as_ref().is_some_and(|f|f.code=="requires_2_0_first")) {return Ok(status::format_human(value));}
    let running=live.is_some();
    let durable=offline_status(roots,candidate,now)?;
    let mut status=match (durable,live) {
        (Some(mut saved),Some(live))=>{if saved.outcome==live.outcome && saved.branch==live.branch && saved.step.id==live.step.id && live.updated_at>=saved.updated_at {saved.step=live.step;saved.updated_at=live.updated_at;saved.space=live.space;}Some(saved)},
        (Some(saved),_)=>Some(saved),
        (None,_)=>None,
    };
    // A stopped upgrade publishes nothing, so only a running one keeps counting.
    if let Some(value)=status.as_mut().filter(|v|running&&v.outcome==super::Outcome::Upgrading){value.updated_at=now.to_rfc3339();}
    let warnings=status::storage_warnings();
    if let Some(value)=status.as_mut(){value.warnings=warnings.clone();}
    let mut lines=match status {Some(value)=>status::format_human(&value).lines().map(String::from).collect(),None=>vec!["업그레이드 상태가 아직 게시되지 않았습니다. 시작을 기다리세요.".into()]};
    if !warnings.is_empty() && !lines.iter().any(|s|s.starts_with("저장소 경고:")){for w in warnings{lines.push(format!("저장소 경고: {}",w.message));}}
    let app_root=&roots.app;
    if present(&app_root.join(".nwui-auto/active-v1.json"))? {
        let selected=selector::read_selector(app_root)?;
        lines.push(format!("선택 기록: PG{} · 세대 {} · 식별자 {}",selected.active_major,selected.generation.as_deref().unwrap_or("없음"),selected.system_identifier));
        if let Some(reason)=selected.operator_reason {lines.push(format!("선택 사유: {}",if reason=="restore-pg15"{"PG15 복원"}else{reason.as_str()}));}
    }else{lines.push("선택 기록: 아직 없음".into());}
    if present(&app_root.join(".nwui-auto/journal-v1.json"))? {
        let journal=journal::read_only(app_root)?;
        lines.push(format!("복구 기록: 시도 {} · 결과: {}",journal.attempt_id,status::outcome_label(journal.outcome)));
        // A parked refusal stores its readiness receipt as a P5Publish record, but no publish ran.
        if let Some(record)=journal.records.iter().rev().find(|r|r.data.as_ref().and_then(|d|d.get("authorityEvent")).and_then(|v|v.as_str())!=Some("refusal-readiness")) {
            let state=match &record.state {super::StepState::Intent=>"진행 중".into(),super::StepState::Done=>"완료".into(),super::StepState::Failed {code}=>format!("실패 ({})",bounded_code(code))};
            lines.push(format!("마지막 단계 기록: {} / {state}",status::step_label(record.step)));
        }
        for old in &journal.previous {lines.push(format!("이전 시도: {} · 결과: {}",old.attempt_id,status::outcome_label(old.outcome)));if let Some(f)=&old.last_failure {lines.push(format!("{}: {}",status::step_label(f.step),bounded_code(&f.code)));}}
        lines.push(format!("이동 기록: {}개 · 인계 {}",journal.moves.len(),match journal.handoff {journal::Handoff::None=>"없음",journal::Handoff::Intent=>"진행 중",journal::Handoff::Done=>"완료"}));
    }else{lines.push("복구 기록: 아직 없음".into());}
    Ok(lines.into_iter().map(|s|s.chars().filter(|c|!c.is_control()).collect::<String>()).collect::<Vec<_>>().join("\n")+"\n")
}
/// Runtime refusal is diagnostic only: never accept PG17 or bypass an existing journal.
fn unrecorded_status_at(app:&Path,status_path:&Path,descriptor:&Path)->Result<Option<String>,StepError>{
    use super::{Branch,Outcome,Step,KeyBinding};
    let value=match status::read_at(status_path){Ok(v)=>v,Err(e)if e.code=="status_missing"=>return Ok(None),Err(e)=>return Err(e)};
    let live_legacy=value.outcome==Outcome::RefusedPreflight && value.step.id==Step::P0;
    if !live_legacy && present(&app.join(".nwui-auto/journal-v1.json"))?{return Ok(None);}
    let refused=matches!((value.branch,value.outcome),(Branch::Upgrade,Outcome::RefusedPreflight)|(Branch::Fresh,Outcome::RefusedFresh)|(Branch::Ready,Outcome::RefusedPrivilege));
    if !refused || value.step.id!=Step::P0 || value.failure.is_none() || value.backup.is_some() || !value.discarded.is_empty() || !value.locations.is_empty() || value.cleanup_pending || value.retry.next_eligible_at.is_some() || value.retry.reason.is_some(){return Ok(None);}
    let selection=super::types::read_selection_at(descriptor)?;
    if selection.epoch!=1 || selection.outcome!=value.outcome || selection.branch!=value.branch || selection.pgdata!=Path::new(crate::db_upgrade::paths::PGDATA) || selection.backup_id.is_some() || selection.schema_profile!="none" || !matches!(selection.key_binding,KeyBinding::Unverified){return Err(mismatch());}
    // Recheck absence after reading the live projection; it never substitutes for recovery authority.
    if !live_legacy && present(&app.join(".nwui-auto/journal-v1.json"))?{return Ok(None);}
    if live_legacy{return Ok(Some(status::format_human(&value)));}
    Ok(Some(status::format_human(&value).lines().map(|line|line.chars().filter(|c|!c.is_control()).collect::<String>()).collect::<Vec<_>>().join("\n")+"\n복구 기록: 아직 없음\n"))
}
pub(crate) async fn auto_status(args:&[String])->i32 {
    if !args.is_empty(){eprintln!("invalid_arguments: auto-status에는 추가 인자가 필요하지 않습니다.");return 2;}
    match unrecorded_status_at(Path::new(crate::db_upgrade::paths::APPROOT),Path::new(status::STATUS_PATH),Path::new("/run/nwui-storage.json")){
        Ok(Some(text))=>{print!("{text}");return 0;},Ok(None)=>{},Err(error)=>{eprintln!("{}: {}",error.code,error.message);return 1;},
    }
    let candidate=match super::journal::Candidate::current().await {Ok(v)=>v,Err(e)=>{eprintln!("{}: {}",e.code,e.message);return 1;}};
    let result=tokio::task::spawn_blocking(move|| {
        let roots=crate::db_upgrade::storage::roots().map_err(|e|StepError{code:e.code,message:e.message,detail:None})?;
        render(status::read(),&roots,&candidate,chrono::Utc::now())
    }).await;
    match result {Ok(Ok(text))=>{print!("{text}");0},Ok(Err(error))=>{eprintln!("{}: {}",error.code,error.message);1},Err(_)=>1}
}
/// Informational publication has no Result to propagate into a step/restore decision.
pub(crate) fn publish_status(value:&UpgradeStatus) {
    publish_with(value,status::publish,|error|eprintln!("[upgrade] status publish failed: {}",error.code));
}
fn publish_with(value:&UpgradeStatus,publish:impl FnOnce(&UpgradeStatus)->Result<(),StepError>,log:impl FnOnce(&StepError)) {
    if let Err(error)=publish(value){log(&error);}
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::{Branch,Journal,Step,StepState,StorageLocks,journal::Candidate};
    use std::{fs,os::unix::fs::PermissionsExt};
    fn sample()->UpgradeStatus {
        serde_json::from_value(serde_json::json!({"version":1,"updatedAt":"2026-10-28T00:00:01Z","outcome":"pg17","branch":"ready","step":{"id":"phase2-monitor","index":1,"total":1,"startedAt":"2026-10-28T00:00:00Z","bytesDone":0,"bytesTotal":0},"failure":null,"space":null,"locations":[],"backup":null,"discarded":[],"cleanupPending":false,"retry":{"nextEligibleAt":null,"reason":null}})).unwrap()
    }
    #[test]
    fn f1_live_refusal_diagnostics_require_absent_journal_and_matching_safe_descriptor(){
        let t=tempfile::tempdir().unwrap();let app=t.path().join("app");fs::create_dir(&app).unwrap();let status=t.path().join("status.json");let descriptor=t.path().join("storage.json");
        assert!(unrecorded_status_at(&app,&status,&descriptor).unwrap().is_none());
        super::super::refusal::publish_unrecorded_at(&StepError{code:"storage_override",message:"fixture cause",detail:None},&t.path().join("missing-db"),&descriptor,&status,||Ok(())).unwrap();
        let text=unrecorded_status_at(&app,&status,&descriptor).unwrap().unwrap();assert!(text.starts_with("새 설치 준비를 끝내지 못했습니다"));assert!(text.contains("storage_override: fixture cause"));assert!(text.contains("다시 확인합니다"));
        fs::create_dir(app.join(".nwui-auto")).unwrap();fs::write(app.join(".nwui-auto/journal-v1.json"),b"corrupt").unwrap();assert!(unrecorded_status_at(&app,&status,&descriptor).unwrap().is_none());
        fs::remove_file(app.join(".nwui-auto/journal-v1.json")).unwrap();
        let mut value=status::read_at(&status).unwrap();value.outcome=super::super::Outcome::Pg17;status::publish_at(&status,&value).unwrap();assert!(unrecorded_status_at(&app,&status,&descriptor).unwrap().is_none());
        value.outcome=super::super::Outcome::RefusedFresh;status::publish_at(&status,&value).unwrap();fs::write(&descriptor,b"{}").unwrap();assert!(unrecorded_status_at(&app,&status,&descriptor).is_err());
        fs::set_permissions(&status,fs::Permissions::from_mode(0o666)).unwrap();assert_eq!(unrecorded_status_at(&app,&status,&descriptor).unwrap_err().code,"status_unsafe");
    }
    #[test]
    fn f10_auto_status_shows_a_live_storage_permission_refusal_with_its_detail(){
        let t=tempfile::tempdir().unwrap();let app=t.path().join("app");fs::create_dir(&app).unwrap();let db=t.path().join("db");fs::create_dir(&db).unwrap();fs::write(db.join("PG_VERSION"),"15").unwrap();
        let status=t.path().join("status.json");let descriptor=t.path().join("storage.json");
        let detail="/var/lib/nginx-web-ui: 소유자 UID 1000는 허용되지 않습니다(필요: root). 컨테이너를 멈추고 호스트에서 sudo chown root:root '<앱 데이터 호스트 폴더>'를 실행한 뒤 다시 시작하세요.";
        super::super::refusal::publish_unrecorded_at(&StepError{code:"storage_permissions",message:"저장소 권한을 자동으로 정리하지 못했습니다. 파일 내용과 소유자는 변경하지 않았습니다.",detail:Some(detail.into())},&db,&descriptor,&status,||Ok(())).unwrap();
        let text=unrecorded_status_at(&app,&status,&descriptor).unwrap().unwrap();
        assert!(text.starts_with("업그레이드 전 검사에서 중단했습니다"),"{text}");assert!(text.contains("storage_permissions: 저장소 권한을 자동으로 정리하지 못했습니다")&&text.contains(detail),"{text}");
    }
    #[test]
    fn c2_c4_a_journal_newer_than_the_live_status_keeps_the_real_step_in_plain_words(){
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for path in [&roots.db,&roots.app] {fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
        let mut journal=Journal::begin(&roots.app,&locks,Branch::Upgrade,candidate.clone()).unwrap();journal.ensure_app_pool().unwrap();journal.record(Step::P3Convert,StepState::Intent,None).unwrap();
        let mut live=sample();live.outcome=super::super::Outcome::Upgrading;live.branch=Branch::Upgrade;live.updated_at="2020-01-01T00:00:00Z".into();
        let text=render(Ok(live),&roots,&candidate,chrono::Utc::now()+chrono::Duration::seconds(7)).unwrap();
        assert!(text.contains("단계 5/14, PG17 변환 · 경과 7초"),"{text}");assert!(text.contains("마지막 단계 기록: PG17 변환 / 진행 중"),"{text}");assert!(text.contains("결과: 데이터베이스를 준비하고 있습니다"),"{text}");assert!(text.contains("인계 없음"),"{text}");
        for raw in ["intent","done","upgrading","none","pg17"] {assert!(!text.split(|c:char|!c.is_ascii_alphanumeric()).any(|word|word==raw),"{raw}: {text}");}
        // Stopped container: no live status, so the elapsed time ends at the last journal write.
        let stopped=render(Err(StepError{code:"status_missing",message:"fixture",detail:None}),&roots,&candidate,chrono::Utc::now()+chrono::Duration::seconds(7)).unwrap();
        assert!(stopped.contains("단계 5/14, PG17 변환 · 경과 0초"),"{stopped}");
    }
    #[test]
    fn auto_status_does_not_report_the_refusal_receipt_as_a_publish_step(){
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        j.record_retry_failure(Step::P2Preflight,"ledger_unknown",chrono::Utc::now()).unwrap();j.set_outcome(super::super::Outcome::RefusedPreflight).unwrap();
        let roots=crate::db_upgrade::layout::Roots{db:db.clone(),app:app.clone(),architecture:"arm64".into()};
        super::super::refusal::record(&roots,&locks,&mut j,&"a".repeat(64)).unwrap();assert_eq!(j.load().records.last().unwrap().step,Step::P5Publish);
        let text=render_at(&t.path().join("missing-run"),&app).unwrap();
        assert!(text.contains("마지막 단계 기록: 업그레이드 전 검사 / 실패 (ledger_unknown)"),"{text}");assert!(!text.contains("실행 파일 준비"),"{text}");
    }
    #[test]
    fn w1_auto_status_pg17_keeps_outcome_and_step_on_separate_lines() {
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for path in [&roots.db,&roots.app] {fs::create_dir(path).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
        let mut journal=Journal::begin(&roots.app,&locks,Branch::Ready,candidate.clone()).unwrap();journal.set_outcome(super::super::Outcome::Pg17).unwrap();journal.record(Step::Phase2Monitor,StepState::Done,None).unwrap();
        let text=render(Ok(sample()),&roots,&candidate,chrono::Utc::now()).unwrap();let mut lines=text.lines();
        assert_eq!(lines.next(),Some("데이터베이스 준비를 마쳤습니다"));
        let step=lines.next().unwrap();assert!(step.starts_with("단계 ")&&step.contains("시작 상태 확인"),"step must occupy its own line: {step}");
    }
    #[test]
    fn w1_auto_status_reads_without_relocking_and_never_prints_journal_payloads() {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");for p in [t.path(),&db,&app] {if p!=t.path(){fs::create_dir(p).unwrap();}fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let mut journal=Journal::begin(&app,&locks,Branch::Ready,Candidate {revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        journal.record(Step::Y1,StepState::Done,Some(serde_json::json!({"neverPrint":"private-test-payload"}))).unwrap();
        let path=t.path().join("status.json");status::publish_at(&path,&sample()).unwrap();
        let text=render_at(&path,&app).unwrap();assert!(text.contains("데이터베이스를 준비하고 있습니다"));assert!(text.contains(&journal.load().attempt_id));assert!(!text.contains("private-test-payload"));assert!(!text.contains("neverPrint"));
        assert!(locks.holds_a() && locks.holds_d());
    }
    #[tokio::test]
    async fn w1_auto_status_missing_is_truthful_and_unsafe_status_is_not_silently_ignored() {
        let t=tempfile::tempdir().unwrap();fs::set_permissions(t.path(),fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("missing");
        assert!(render_at(&path,t.path()).unwrap().contains("아직 게시되지 않았습니다"));
        fs::write(&path,b"bad").unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o644)).unwrap();assert_eq!(render_at(&path,t.path()).unwrap_err().code,"status_invalid");assert_eq!(auto_status(&["--unknown".into()]).await,2);
    }
    #[test]
    fn g34_pg17_pending_offline_has_no_stale_failure_before_p6_and_clears_after_completion() {
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        j.record_retry_failure(Step::P2Preflight,"interrupted",chrono::Utc::now()).unwrap();j.set_outcome(super::super::Outcome::Pg17).unwrap();
        let offline=offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();
        assert!(offline.cleanup_pending);assert!(offline.failure.is_none());
        let runtime=status::pg17_pending(j.load());assert!(runtime.0);assert!(runtime.1.is_none());
        j.record(Step::P6Cleanup,StepState::Intent,Some(serde_json::json!({"entry":"base"}))).unwrap();j.record(Step::P6Cleanup,StepState::Done,Some(serde_json::json!({"entry":"base"}))).unwrap();
        assert!(status::pg17_pending(j.load()).0);
        j.record_retry_failure(Step::P6Cleanup,"io_error",chrono::Utc::now()).unwrap();
        let offline=offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();let runtime=status::pg17_pending(j.load());
        assert!(runtime.0&&offline.cleanup_pending);assert_eq!(runtime.1.as_ref().unwrap().code,"io_error");assert_eq!(serde_json::to_value(&runtime.1).unwrap(),serde_json::to_value(&offline.failure).unwrap());
        j.record(Step::P6Cleanup,StepState::Done,None).unwrap();
        let offline=offline_status(&roots,&j.load().candidate,chrono::Utc::now()).unwrap().unwrap();
        assert!(!offline.cleanup_pending);assert!(offline.failure.is_none());let runtime=status::pg17_pending(j.load());assert!(!runtime.0);assert!(runtime.1.is_none());
    }
    #[test]
    fn g22_offline_without_run_keeps_backup_and_failure_guidance() {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":1234}))).unwrap();
        j.record_retry_failure(Step::P2Preflight,"ledger_unknown",chrono::Utc::now()).unwrap();j.set_outcome(super::super::Outcome::RefusedPreflight).unwrap();
        fs::create_dir_all(db.join(".nwui-pg15-backup").join(&j.load().attempt_id)).unwrap();
        let text=render_at(&t.path().join("missing-run"),&app).unwrap();
        assert!(text.contains(&format!("보관: {} · 1234 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: nwui-db-upgrade remove-pg15-backup --confirm {}",j.load().attempt_id,j.load().attempt_id)),"{text}");
        assert!(text.contains("ledger_unknown"));assert!(text.contains("기록된 원인을 해결한 뒤 컨테이너를 다시 시작하세요."));
        // A new attempt can still have the previous restored-15 selector until P5.
        let restored=selector::Selector{version:1,active_major:15,generation:None,system_identifier:"123456789".into(),origin:selector::Origin::Auto,backup_id:Some(uuid::Uuid::new_v4().to_string()),profile_id:Some("legacy-28".into()),profile_sha256:Some("a".repeat(64)),manifest_sha256:None,operator_reason:Some("restore-pg15".into())};
        crate::db_upgrade::files::atomic_json(&app.join(".nwui-auto/active-v1.json"),&restored).unwrap();
        assert!(render_at(&t.path().join("missing-run"),&app).unwrap().contains("1234 바이트"));
        let bad=app.join(".nwui-auto/active-v1.json");fs::write(&bad,b"{}").unwrap();fs::set_permissions(&bad,fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(render_at(&t.path().join("missing-run"),&app).unwrap_err().code,"pair_mismatch");

    }
    #[test]
    fn g22_removed_backup_is_not_resurrected_by_a_stale_run_file() {
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&db,&app).unwrap();let mut j=Journal::begin(&app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        j.record(Step::P1Backup,StepState::Done,Some(serde_json::json!({"sourceBytes":1234}))).unwrap();
        let data=serde_json::json!({"backupId":j.load().attempt_id});j.record(Step::CmdRemoveBackup,StepState::Intent,Some(data.clone())).unwrap();j.record(Step::CmdRemoveBackup,StepState::Done,Some(data)).unwrap();
        let path=t.path().join("stale-run");let mut value=sample();value.backup=Some(super::super::types::RetainedCopy{id:j.load().attempt_id.clone(),bytes:1234,remove_command:"stale-remove".into()});status::publish_at(&path,&value).unwrap();
        let text=render_at(&path,&app).unwrap();assert!(!text.contains("보관:"),"{text}");assert!(!text.contains("stale-remove"));
        assert!(locks.holds_a() && locks.holds_d());
    }
    #[test]
    fn g22_run_file_alone_never_supplies_backup_authority() {
        let t=tempfile::tempdir().unwrap();let path=t.path().join("run");let mut value=sample();value.backup=Some(super::super::types::RetainedCopy{id:"unverified".into(),bytes:99,remove_command:"unverified-remove".into()});status::publish_at(&path,&value).unwrap();
        let text=render_at(&path,t.path()).unwrap();assert!(!text.contains("unverified"));assert!(text.contains("아직 게시되지 않았습니다"));
    }
    #[test]
    fn g22_previous_discard_is_measured_without_reading_or_following_links() {
        use std::os::unix::fs::symlink;
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Ready,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();
        j.record(Step::Y1,StepState::Done,None).unwrap();
        let id=uuid::Uuid::new_v4().to_string();let mut state=j.load().clone();
        state.previous.push(journal::PreviousAttempt{attempt_id:uuid::Uuid::new_v4().to_string(),outcome:super::super::Outcome::Restored,candidate:state.candidate.clone(),last_failure:None,generation:Some(id.clone()),discard_id:Some(id.clone()),ended_at:"2026-10-29T00:00:00Z".into()});
        crate::db_upgrade::files::atomic_json(&roots.app.join(".nwui-auto/journal-v1.json"),&state).unwrap();
        let path=roots.db.join(".nwui-discard").join(&id);fs::create_dir_all(&path).unwrap();fs::write(path.join("data"),b"private-value").unwrap();
        let value=offline_status(&roots,&state.candidate,chrono::Utc::now()).unwrap().unwrap();assert_eq!(value.discarded.len(),1);assert_eq!(value.discarded[0].bytes,13);assert_eq!(value.discarded[0].remove_command,format!("nwui-db-upgrade remove-discarded --confirm {id}"));
        let online=super::super::refusal::status_value(&roots,&state,state.outcome,&state.candidate,None).unwrap();assert_eq!(serde_json::to_value(&online.discarded).unwrap(),serde_json::to_value(&value.discarded).unwrap());
        let mut duplicate=state.clone();let mut old=duplicate.previous[0].clone();old.attempt_id=uuid::Uuid::new_v4().to_string();duplicate.previous.push(old);assert_eq!(retained_discards(&roots,&duplicate).unwrap().len(),1);
        let text=render_at(&t.path().join("missing-run"),&roots.app).unwrap();assert!(!text.contains("private-value"));assert!(text.contains(&state.previous[0].attempt_id));
        let outside=t.path().join("outside");fs::write(&outside,b"unchanged").unwrap();symlink(&outside,path.join("link")).unwrap();assert!(offline_status(&roots,&state.candidate,chrono::Utc::now()).is_err());assert_eq!(fs::read(&outside).unwrap(),b"unchanged");
        fs::remove_file(path.join("link")).unwrap();fs::remove_file(path.join("data")).unwrap();fs::remove_dir(&path).unwrap();
        let mut j=Journal::open(&roots.app,&locks).unwrap();j.record(Step::CmdRemoveDiscarded,StepState::Intent,Some(serde_json::json!({"discardId":id}))).unwrap();
        let value=offline_status(&roots,&state.candidate,chrono::Utc::now()).unwrap().unwrap();assert_eq!(value.discarded.len(),1);assert_eq!(value.discarded[0].bytes,0);assert!(value.discarded[0].remove_command.ends_with(&id));
        assert!(locks.holds_a() && locks.holds_d());
    }
    #[test]
    fn g22_offline_restored_action_requires_20_for_same_or_changed_candidate() {
        let t=tempfile::tempdir().unwrap();let roots=crate::db_upgrade::layout::Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};
        for p in [&roots.db,&roots.app] {fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();let mut j=Journal::begin(&roots.app,&locks,Branch::Upgrade,Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)}).unwrap();j.ensure_app_pool().unwrap();
        let now=chrono::Utc::now();for _ in 0..2 {j.record_retry_failure(Step::P3Convert,"interrupted",now).unwrap();}j.set_outcome(super::super::Outcome::Restored).unwrap();
        let before=fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap();
        let value=offline_status(&roots,&j.load().candidate,now).unwrap().unwrap();assert!(value.retry.next_eligible_at.is_none());assert!(value.retry.reason.is_none());assert!(value.failure.as_ref().unwrap().next_action.contains("2.0.x 최신(2.0.3 이상)"));assert!(value.backup.is_none());
        let mut next=j.load().candidate.clone();next.revision="f".repeat(40);let value=offline_status(&roots,&next,now).unwrap().unwrap();assert!(value.retry.next_eligible_at.is_none());assert!(value.retry.reason.is_none());assert!(value.failure.unwrap().next_action.contains("2.0.x 최신(2.0.3 이상)"));
        assert_eq!(fs::read(roots.app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
    }
    #[test]
    fn w1_informational_enospc_does_not_change_the_step_outcome_or_request_restore() {
        let before=sample();let original=serde_json::to_value(&before).unwrap();let mut logged=None;
        publish_with(&before,|_|Err(StepError {code:"insufficient_space",message:"injected ENOSPC",detail:None}),|error|logged=Some(error.code));
        assert_eq!(logged,Some("insufficient_space"));assert_eq!(serde_json::to_value(before).unwrap(),original);
    }
}

#[cfg(test)]
mod legacy_refusal_tests {
    use super::*;
    #[test]
    fn live_legacy_refusal_overrides_an_existing_durable_journal_without_writes(){
        use std::{fs,os::unix::fs::PermissionsExt};
        let t=tempfile::tempdir().unwrap();let db=t.path().join("db");let app=t.path().join("app");
        for p in [&db,&app]{fs::create_dir(p).unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o700)).unwrap();}
        let locks=crate::upgrade::StorageLocks::acquire(&db,&app).unwrap();
        let candidate=journal::Candidate{revision:"a".repeat(40),pins_sha256:"b".repeat(64),migrations_sha256:"c".repeat(64)};
        let _j=crate::upgrade::Journal::begin(&app,&locks,crate::upgrade::Branch::Upgrade,candidate).unwrap();
        let before=fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap();let descriptor=t.path().join("selection.json");let status=t.path().join("status.json");
        crate::upgrade::refusal::publish_unrecorded_at(&crate::upgrade::legacy::required(),&db,&descriptor,&status,||Ok(())).unwrap();
        let text=unrecorded_status_at(&app,&status,&descriptor).unwrap().unwrap();
        assert!(text.contains("requires_2_0_first"));assert!(text.contains("2.0.x 최신(2.0.3 이상)"));assert!(text.contains("변경하지 않았습니다"));
        assert_eq!(fs::read(app.join(".nwui-auto/journal-v1.json")).unwrap(),before);
        assert_eq!(crate::upgrade::status::read_at(&status).unwrap().outcome,crate::upgrade::Outcome::RefusedPreflight);
    }
}
