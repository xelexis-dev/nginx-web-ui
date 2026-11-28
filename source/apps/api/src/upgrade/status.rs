//! Informational status-file I/O. Runner wiring and failure policy belong to W1.
use super::types::{UpgradeStatus, StepError};
use std::{ffi::CString, fs::File, io::{self, Read, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}, path::{Component, Path}};

pub const STATUS_PATH: &str = "/run/nwui-db-upgrade.json";
pub const STATUS_MAX_BYTES: usize = 65536;
fn error(code: &'static str, path: &Path) -> StepError {
    StepError { code, message: "업그레이드 상태 파일을 확인하세요.", detail: Some(format!("{code}: {}", path.display()).chars().filter(|c| !c.is_control()).take(256).collect()) }
}
fn io_error(e: io::Error, path: &Path) -> StepError {
    error(match e.raw_os_error() { Some(libc::EROFS)=>"storage_read_only", Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space", Some(libc::EINTR)=>"interrupted", _=>"io_error" },path)
}
fn name(value: &std::ffi::OsStr, path: &Path) -> Result<CString,StepError> {
    CString::new(value.as_encoded_bytes()).map_err(|_|error("status_unsafe",path))
}
fn open_at(parent: &File, name: &CString, flags: i32, mode: u32) -> io::Result<File> {
    let fd=unsafe {libc::openat(parent.as_raw_fd(),name.as_ptr(),flags|libc::O_CLOEXEC,mode as libc::mode_t)};
    if fd<0 {Err(io::Error::last_os_error())} else {Ok(unsafe {File::from_raw_fd(fd)})}
}
fn safe_parent(file: &File, path: &Path) -> Result<(),StepError> {
    let m=file.metadata().map_err(|e|io_error(e,path))?;
    let mode=m.mode();
    if m.uid()!=0 || !m.is_dir() || (mode&0o022!=0 && !(mode&0o002!=0 && mode&0o1000!=0)) {return Err(error("status_unsafe",path));}
    Ok(())
}
fn parent(path: &Path) -> Result<(File,CString),StepError> {
    if !path.is_absolute() {return Err(error("status_unsafe",path));}
    let leaf=name(path.file_name().ok_or_else(||error("status_unsafe",path))?,path)?;
    let mut directory=File::open("/").map_err(|e|io_error(e,path))?;
    safe_parent(&directory,path)?;
    for component in path.parent().unwrap().components() {
        let Component::Normal(part)=component else {if component==Component::RootDir {continue;}return Err(error("status_unsafe",path));};
        directory=open_at(&directory,&name(part,path)?,libc::O_RDONLY|libc::O_DIRECTORY|libc::O_NOFOLLOW,0).map_err(|e| {
            if matches!(e.raw_os_error(),Some(libc::ELOOP|libc::ENOTDIR)) {error("status_unsafe",path)} else {io_error(e,path)}
        })?;
        safe_parent(&directory,path)?;
    }
    Ok((directory,leaf))
}
fn regular_or_absent(parent: &File, leaf: &CString, path: &Path) -> Result<(),StepError> {
    let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {libc::fstatat(parent.as_raw_fd(),leaf.as_ptr(),stat.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}<0 {
        let e=io::Error::last_os_error();
        return if e.raw_os_error()==Some(libc::ENOENT) {Ok(())} else {Err(io_error(e,path))};
    }
    if unsafe {stat.assume_init()}.st_mode&libc::S_IFMT!=libc::S_IFREG {return Err(error("status_unsafe",path));}
    Ok(())
}
pub fn publish_at(path: &Path, status: &UpgradeStatus) -> Result<(),StepError> {
    let bytes=serde_json::to_vec(status).map_err(|_|error("status_invalid",path))?;
    if status.version!=1 {return Err(error("status_invalid",path));}
    write_runtime_json(path,&bytes)
}
pub(super) fn write_runtime_json(path:&Path,bytes:&[u8])->Result<(),StepError> {
    if bytes.len()>STATUS_MAX_BYTES {return Err(error("status_invalid",path));}
    if unsafe {libc::geteuid()}!=0 {return Err(error("status_unsafe",path));}
    let (directory,leaf)=parent(path)?;
    regular_or_absent(&directory,&leaf,path)?;
    let temp=CString::new(format!(".nwui-status-{}",uuid::Uuid::new_v4())).unwrap();
    let mut file=open_at(&directory,&temp,libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW,0o600).map_err(|e|io_error(e,path))?;
    let result=(|| {
        if file.metadata().map_err(|e|io_error(e,path))?.uid()!=0 {return Err(error("status_unsafe",path));}
        if unsafe {libc::fchmod(file.as_raw_fd(),0o644)}<0 {return Err(io_error(io::Error::last_os_error(),path));}
        file.write_all(bytes).map_err(|e|io_error(e,path))?;
        #[cfg(test)]
        if FAIL_AFTER_WRITE.with(|f|f.replace(false)) {return Err(io_error(io::Error::from_raw_os_error(libc::ENOSPC),path));}
        file.sync_all().map_err(|e|io_error(e,path))?;
        regular_or_absent(&directory,&leaf,path)?;
        if unsafe {libc::renameat(directory.as_raw_fd(),temp.as_ptr(),directory.as_raw_fd(),leaf.as_ptr())}<0 {return Err(io_error(io::Error::last_os_error(),path));}
        directory.sync_all().map_err(|e|io_error(e,path))
    })();
    if result.is_err() {unsafe {libc::unlinkat(directory.as_raw_fd(),temp.as_ptr(),0)};}
    result
}
pub fn read_at(path: &Path) -> Result<UpgradeStatus,StepError> {
    let file=open_runtime_json(path)?;
    read_open_file(file,path)
}
pub(super) fn open_runtime_json(path:&Path)->Result<File,StepError> {
    let (directory,leaf)=parent(path)?;
    open_at(&directory,&leaf,libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK,0).map_err(|e|match e.raw_os_error() {
        Some(libc::ENOENT)=>error("status_missing",path), Some(libc::ELOOP|libc::ENXIO)=>error("status_unsafe",path), _=>io_error(e,path)
    })
}
fn read_open_file(file:File,path:&Path)->Result<UpgradeStatus,StepError> {
    let (bytes,_)=read_runtime_json(&file,path)?;
    let status:UpgradeStatus=serde_json::from_slice(&bytes).map_err(|_|error("status_invalid",path))?;
    if status.version!=1 {return Err(error("status_invalid",path));}
    Ok(status)
}
pub(super) fn read_runtime_json(file:&File,path:&Path)->Result<(Vec<u8>,std::fs::Metadata),StepError> {
    let m=file.metadata().map_err(|e|io_error(e,path))?;
    // Atomic replacement can unlink an already-open whole publication; only extra links are unsafe.
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o644 || m.nlink()>1 {return Err(error("status_unsafe",path));}
    if m.len()>STATUS_MAX_BYTES as u64 {return Err(error("status_invalid",path));}
    let mut bytes=Vec::new();file.take(STATUS_MAX_BYTES as u64+1).read_to_end(&mut bytes).map_err(|e|io_error(e,path))?;
    if bytes.len()>STATUS_MAX_BYTES {return Err(error("status_invalid",path));}
    Ok((bytes,m))
}
pub(crate) fn source_guidance(too_old: bool) -> Option<&'static str> {
    too_old.then_some(super::legacy::GUIDANCE)
}
pub(crate) fn retained_backup_notice(backup: &super::types::RetainedCopy) -> String {
    format!("보관: {} · {} 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: {}", backup.id, backup.bytes, backup.remove_command)
}
/// The terminal P6 Done follows entries, staged key, work directory and reserve cleanup.
pub(crate) fn terminal_cleanup(r:&super::journal::Record)->bool {r.step==super::Step::P6Cleanup && r.state==super::StepState::Done && r.data.is_none()}
pub(crate) fn pg17_pending(j:&super::journal::JournalV1)->(bool,Option<super::types::StatusFailure>) {
    use super::{Branch,Outcome,Step,StepState,types::StatusFailure};
    let pending=j.outcome==Outcome::Pg17 && j.branch==Branch::Upgrade && !j.records.iter().any(terminal_cleanup);
    let failure=if pending {j.records.iter().rev().find_map(|r|match &r.state {
        StepState::Failed{code} if r.step==Step::P6Cleanup=>Some(StatusFailure{class:super::class_of(code),code:code.clone(),message:cleanup_pending_notice(code),next_action:"컨테이너를 다시 시작하면 정리를 다시 시도합니다.".into()}),
        _=>None,
    })}else{None};
    (pending,failure)
}
pub(crate) fn cleanup_pending_notice(code: &str) -> String {
    let notice="이전 PG15 데이터 정리가 끝나지 않았습니다 — 서비스는 정상이며 다음 시작 때 다시 정리합니다";
    if code.is_empty() {notice.into()} else {format!("{notice} ({code})")}
}
#[derive(Debug, PartialEq)]
pub(crate) struct UpgradeNotification {
    pub kind: &'static str, pub severity: &'static str, pub title: &'static str,
    pub body: String, pub dedupe_key: String, pub data: serde_json::Value,
}
pub(super) const BACKUP_KEPT_KIND: &str = "system.db_upgrade_backup_kept";

pub(crate) fn backup_kept_notification(backup: &super::types::RetainedCopy, location: &str) -> UpgradeNotification {
    UpgradeNotification {
        kind:BACKUP_KEPT_KIND, severity:"info", title:"PG15 백업을 보관하고 있습니다",
        body:format!("업그레이드 전 PG15 데이터 백업({} 바이트)을 {location}에 보관하고 있습니다. 자동으로 지우지 않습니다. 1.19.11로 되돌리려면 컨테이너를 중지하고 2.0.3 이미지의 오프라인 진입점으로 restore-pg15를 실행하세요. 복원하면 업그레이드 이후의 변경은 사라집니다. 2.0.3 이미지를 정상 시작해 2.1 이상에서 사용한 데이터를 열지 마세요. 되돌릴 필요가 없으면 컨테이너를 중지하고 같은 볼륨으로 {} 명령을 실행해 지울 수 있습니다. 자세한 절차는 업그레이드 안내를 참고하세요.",backup.bytes,backup.remove_command),
        dedupe_key:format!("db-upgrade-backup:{}",backup.id),data:serde_json::json!({"backupId":backup.id,"bytes":backup.bytes}),
    }
}
pub(crate) fn activation_failed_notification(attempt_id: &str, code: &str) -> UpgradeNotification {
    UpgradeNotification {
        kind:"system.db_upgrade_activation_failed",severity:"error",title:"업그레이드 후 서비스 시작에 실패했습니다",
        body:format!("PG17 업그레이드는 완료됐지만 이전 시작에서 서비스가 준비되지 않았습니다({code}). 이번 시작에서 다시 시도합니다. 계속 실패하면 nwui-db-upgrade auto-status 명령의 안내를 따르세요."),
        dedupe_key:format!("db-upgrade-activation:{attempt_id}:{code}"),data:serde_json::Value::Null,
    }
}
pub(crate) async fn insert_notification(client: &impl tokio_postgres::GenericClient, row: &UpgradeNotification) -> Result<bool, StepError> {
    client.execute("INSERT INTO notifications(id,type,severity,title,body,link,data,dedupe_key) VALUES($1,$2,$3::text::notification_severity,$4,$5,$6,$7,$8) ON CONFLICT(dedupe_key) DO UPDATE SET body=excluded.body,data=excluded.data WHERE notifications.type='system.db_upgrade_backup_kept' AND (notifications.body IS DISTINCT FROM excluded.body OR notifications.data IS DISTINCT FROM excluded.data)",
        &[&uuid::Uuid::new_v4().to_string(),&row.kind,&row.severity,&row.title,&row.body,&Option::<String>::None,&row.data,&row.dedupe_key])
        .await.map(|count|count==1).map_err(|_|StepError{code:"io_error",message:"업그레이드 알림을 저장하지 못했습니다.",detail:None})
}
pub(crate) fn storage_warnings()->Vec<super::types::StorageWarning>{
    let mut warnings=warnings_for(crate::db_upgrade::shared_folder::scopes());
    warnings.extend(super::storage_modes::warning(Path::new(super::storage_modes::REPORT_PATH),super::boot_identity::current().ok().as_deref()));
    warnings
}
fn warnings_for(mut scopes:Vec<String>)->Vec<super::types::StorageWarning>{
    scopes.retain(|s|matches!(s.as_str(),"db"|"app"|"nginx-log"|"app-log"));scopes.sort();scopes.dedup();if scopes.is_empty(){return vec![];}
    let mut assets=Vec::new();let mut labels=Vec::new();
    if scopes.iter().any(|s|s=="db"){assets.push("DB 파일");labels.push("DB 데이터");}
    if scopes.iter().any(|s|s=="app"){assets.push("암호화 키·비밀 값·설정 게시 파일");labels.push("앱 데이터");}
    if scopes.iter().any(|s|s=="nginx-log"||s=="app-log"){assets.push("로그");}
    if scopes.iter().any(|s|s=="nginx-log"){labels.push("nginx 로그");}
    if scopes.iter().any(|s|s=="app-log"){labels.push("앱 로그");}
    let escalation=if scopes.iter().any(|s|s=="db"||s=="app"){" 이 프로세스 중 하나가 침해되면 컨테이너 안의 root 권한까지 이어질 수 있습니다."}else{""};
    let message=format!("감지된 Docker Desktop·OrbStack Mac 공유 폴더에서는 사용자 간 파일 권한 제한이 적용되지 않습니다. 관리 요청을 처리하는 API·웹 화면·nginx 작업 프로세스도 이 저장소의 {}에 접근해 읽고 바꿀 수 있어 권한 분리가 약해집니다.{escalation} 컨테이너 내부 프로세스 간 권한 분리가 필요하면 Docker 관리 볼륨을 사용하세요. macOS에서는 가능하면 OrbStack과 Docker 관리 볼륨 사용을 권장합니다. 이 경고만으로 설치나 업그레이드가 중단되지는 않습니다. 현재 해당 저장소: {}",assets.join("·"),labels.join(", "));
    vec![super::types::StorageWarning{code:"desktop_shared_folder_isolation".into(),message,scopes}]
}
pub(crate) fn storage_notification(w:&super::types::StorageWarning)->UpgradeNotification{
    if w.code==super::storage_modes::WARNING_CODE {
        use sha2::{Digest,Sha256};let key=hex::encode(Sha256::digest(w.message.as_bytes()));
        return UpgradeNotification{kind:"system.storage_modes_normalized",severity:"warning",title:"저장소 권한을 정리했습니다",body:w.message.clone(),dedupe_key:format!("storage-modes-normalized:v1:{}",&key[..16]),data:serde_json::json!({"scopes":w.scopes})};
    }
    UpgradeNotification{kind:"system.desktop_shared_folder_isolation",severity:"warning",title:"호스트 폴더 저장소에서는 권한 분리가 약해집니다",body:w.message.clone(),dedupe_key:format!("desktop-shared-folder-isolation:v1:{}",w.scopes.join(",")),data:serde_json::json!({"scopes":w.scopes})}
}
fn storage_log_line(w:&super::types::StorageWarning)->String{
    if w.code==super::storage_modes::WARNING_CODE {return format!("[storage] warning {} scopes={} | {}",w.code,w.scopes.join(","),w.message);}
    let escalation=if w.scopes.iter().any(|s|s=="db"||s=="app"){ " Compromise of one of these processes can lead to root privileges inside the container." }else{""};
    let mut assets=Vec::new();if w.scopes.iter().any(|s|s=="db"){assets.push("database files");}if w.scopes.iter().any(|s|s=="app"){assets.push("encryption keys, secrets and published configuration");}if w.scopes.iter().any(|s|s=="nginx-log"||s=="app-log"){assets.push("logs");}
    let labels=w.scopes.iter().map(|s|match s.as_str(){"db"=>"DB data","app"=>"app data","nginx-log"=>"nginx logs",_=>"app logs"}).collect::<Vec<_>>().join(", ");
    format!("[storage] warning {} scopes={} {} | Detected Docker Desktop/OrbStack Mac shared folders do not enforce file permissions between container users. The API, web server and nginx workers can read and change {}; privilege separation is weakened.{} Use Docker-managed volumes for isolation between container processes; on macOS prefer OrbStack with Docker-managed volumes. This warning alone does not stop installation or upgrade. Current affected storage: {labels}",w.code,w.scopes.join(","),w.message,assets.join(", "),escalation)
}
pub(crate) fn log_storage_warning(){for w in storage_warnings(){eprintln!("{}",storage_log_line(&w));}}
/// Upgrade-branch order shown as 「단계 n/m」. Steps outside it have index 0.
pub(crate) fn step_position(step:super::Step)->(u32,u32) {
    use super::Step::*;
    const ORDER:&[super::Step]=&[P0,P0b,P1Backup,P2Preflight,P3Convert,P4Target,S4Prepare,P5Publish,P5Guard,P5Commit,S4Activate,Handoff,Phase2Monitor,P6Cleanup];
    (ORDER.iter().position(|s|*s==step).map(|n|n as u32+1).unwrap_or(0),ORDER.len() as u32)
}
pub(crate) fn outcome_label(outcome:super::Outcome)->&'static str {
    use super::Outcome;
    match outcome {
        Outcome::Upgrading=>"데이터베이스를 준비하고 있습니다", Outcome::Pg17=>"데이터베이스 준비를 마쳤습니다", Outcome::Restored=>"원본 PG15 데이터를 검증 후 복원했습니다", Outcome::RestoreIncomplete=>"원본 복원을 끝내지 못했습니다", Outcome::ActivationFailed=>"업그레이드는 완료됐지만 서비스 시작에 실패했습니다", Outcome::RefusedPreflight=>"업그레이드 전 검사에서 중단했습니다", Outcome::RefusedFresh=>"새 설치 준비를 끝내지 못했습니다", Outcome::RefusedPrivilege=>"PG17 데이터는 변경되지 않았습니다", Outcome::Postponed=>"업그레이드를 보류했습니다"
    }
}
pub(crate) fn step_label(step:super::Step)->&'static str {
    use super::Step::*;
    match step {
        P0=>"저장소 확인",P0b=>"시작 설정 확인",F1=>"새 저장소 준비",Y1=>"기존 저장소 확인",Y2=>"실행 준비",Y3=>"시작 준비",P1Backup=>"원본 보관",P2Preflight=>"업그레이드 전 검사",P3Convert=>"PG17 변환",P4Target=>"변환 결과 검증",S4Prepare=>"서비스 권한 준비",P5Publish=>"실행 파일 준비",P5Guard=>"설정 검사",P5Commit=>"PG17 선택",S4Activate=>"서비스 활성화",Handoff=>"서비스 시작",Phase2Monitor=>"시작 상태 확인",P6Cleanup=>"이전 데이터 정리",RRestore|CmdRestorePg15=>"원본 복원",CmdRemoveBackup=>"보관 데이터 제거",CmdRemoveDiscarded=>"보존 데이터 제거"
    }
}
pub fn format_human(s: &UpgradeStatus) -> String {
    let outcome=if super::unrecorded::journal_refusal(s) {"시작 준비 기록을 저장하지 못했습니다"} else {outcome_label(s.outcome)};
    let label=step_label(s.step.id);
    let elapsed=chrono::DateTime::parse_from_rfc3339(&s.updated_at).ok().zip(chrono::DateTime::parse_from_rfc3339(&s.step.started_at).ok()).map(|(end,start)|format!("{}초",(end-start).num_seconds().max(0))).unwrap_or_else(||"확인할 수 없음".into());
    let mut lines=vec![outcome.to_string()];
    // Total 0 is the placeholder of a final outcome or an unrecorded refusal: no step is known.
    if s.step.total>0 {lines.push(if s.step.index>0 {format!("단계 {}/{}, {label} · 경과 {elapsed}",s.step.index,s.step.total)}else{label.to_string()});}
    if s.step.bytes_total>0 {lines.push(format!("{} / {} 바이트",s.step.bytes_done,s.step.bytes_total));}
    if let Some(f)=&s.failure {lines.push(format!("{}: {} {}",f.code,f.message,f.next_action));}
    if let Some(p)=&s.space {
        let inode=if p.inodes_required==0&&p.inodes_available==0{"inode: 파일 시스템이 개수를 제공하지 않아 확인하지 않음".to_owned()}else{format!("inode: {} / {}",p.inodes_available,p.inodes_required)};
        lines.push(format!("공간: {} / {} 바이트, {inode}",p.available_bytes,p.required_bytes));
    }
    for location in &s.locations {lines.push(format!("보존 위치: {location}"));}
    if let Some(b)=&s.backup {lines.push(retained_backup_notice(b));}
    for b in &s.discarded {lines.push(format!("분리 보존: {} · {} 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: {}",b.id,b.bytes,b.remove_command));}
    for warning in &s.warnings {lines.push(format!("저장소 경고: {}",warning.message));}
    lines.push(if s.cleanup_pending {cleanup_pending_notice(s.failure.as_ref().map(|f|f.code.as_str()).unwrap_or_default())}else{"정리 대기 없음".into()});
    if let Some(at)=&s.retry.next_eligible_at {lines.push(format!("다음 재시도 가능: {at}"));}
    if let Some(reason)=&s.retry.reason {lines.push(format!("재시도: {reason}"));}
    if s.retry.next_eligible_at.is_none() && s.retry.reason.is_none() {lines.push("재시도 시각: 미지정".into());}
    lines.into_iter().map(|line|line.chars().filter(|c|!c.is_control()).collect::<String>()).collect::<Vec<_>>().join("\n")
}
pub fn publish(status: &UpgradeStatus) -> Result<(), StepError> {publish_at(Path::new(STATUS_PATH),status)}
pub fn read() -> Result<UpgradeStatus,StepError> {read_at(Path::new(STATUS_PATH))}
#[cfg(test)]
thread_local! {static FAIL_AFTER_WRITE:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{PermissionsExt,symlink}, sync::{Arc,Barrier,atomic::{AtomicBool,Ordering}}};
    const A8_LITERAL:&str=r#"{"version":1,"updatedAt":"2026-10-28T00:00:06Z","outcome":"upgrading","branch":"upgrade","step":{"id":"p1-backup","index":3,"total":11,"startedAt":"2026-10-28T00:00:00Z","bytesDone":1024,"bytesTotal":4096},"failure":{"code":"insufficient_space","class":"transient","message":"공간을 확인하세요.","nextAction":"여유 공간을 확보하세요."},"space":{"requiredBytes":4096,"availableBytes":1024,"inodesRequired":8,"inodesAvailable":4},"locations":["/var/lib/postgresql/data/.nwui-pg15-backup/kept"],"backup":{"id":"kept","bytes":2048,"removeCommand":"nwui-db-upgrade remove-pg15-backup --confirm kept"},"discarded":[{"id":"aside","bytes":512,"removeCommand":"nwui-db-upgrade remove-discarded --confirm aside"}],"cleanupPending":true,"retry":{"nextEligibleAt":"2026-10-29T00:00:00Z","reason":"다음 시작 때 재시도합니다."}}"#;
    pub(crate) fn sample()->UpgradeStatus {serde_json::from_str(A8_LITERAL).unwrap()}
    #[test] fn g22_pure_notices_preserve_design_and_existing_backup_line() {
        assert_eq!(source_guidance(true),Some(super::super::legacy::GUIDANCE));
        assert_eq!(source_guidance(false),None);
        let value=sample();
        assert_eq!(retained_backup_notice(value.backup.as_ref().unwrap()),"보관: kept · 2048 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: nwui-db-upgrade remove-pg15-backup --confirm kept");
        assert_eq!(cleanup_pending_notice(""),"이전 PG15 데이터 정리가 끝나지 않았습니다 — 서비스는 정상이며 다음 시작 때 다시 정리합니다");
        assert_eq!(cleanup_pending_notice("io_error"),"이전 PG15 데이터 정리가 끝나지 않았습니다 — 서비스는 정상이며 다음 시작 때 다시 정리합니다 (io_error)");
        let mut value=value;value.failure=None;
        assert!(format_human(&value).contains(&cleanup_pending_notice("")));
        value.cleanup_pending=false;
        assert!(!format_human(&value).contains(&cleanup_pending_notice("")));
    }
    #[test] fn g22_notification_rows_use_frozen_types_keys_titles_and_bodies() {
        let value=sample();let backup=value.backup.as_ref().unwrap();let kept=backup_kept_notification(backup,"/var/lib/postgresql/data/.nwui-pg15-backup/kept");
        assert_eq!((kept.kind,kept.severity,kept.title),("system.db_upgrade_backup_kept","info","PG15 백업을 보관하고 있습니다"));
        assert_eq!(kept.dedupe_key,"db-upgrade-backup:kept");assert_eq!(kept.data,serde_json::json!({"backupId":"kept","bytes":2048}));
        assert_eq!(kept.body,"업그레이드 전 PG15 데이터 백업(2048 바이트)을 /var/lib/postgresql/data/.nwui-pg15-backup/kept에 보관하고 있습니다. 자동으로 지우지 않습니다. 1.19.11로 되돌리려면 컨테이너를 중지하고 2.0.3 이미지의 오프라인 진입점으로 restore-pg15를 실행하세요. 복원하면 업그레이드 이후의 변경은 사라집니다. 2.0.3 이미지를 정상 시작해 2.1 이상에서 사용한 데이터를 열지 마세요. 되돌릴 필요가 없으면 컨테이너를 중지하고 같은 볼륨으로 nwui-db-upgrade remove-pg15-backup --confirm kept 명령을 실행해 지울 수 있습니다. 자세한 절차는 업그레이드 안내를 참고하세요.");
        let failed=activation_failed_notification("attempt","activation_failed");
        assert_eq!((failed.kind,failed.severity,failed.title),("system.db_upgrade_activation_failed","error","업그레이드 후 서비스 시작에 실패했습니다"));
        assert_eq!(failed.dedupe_key,"db-upgrade-activation:attempt:activation_failed");
        assert_eq!(failed.body,"PG17 업그레이드는 완료됐지만 이전 시작에서 서비스가 준비되지 않았습니다(activation_failed). 이번 시작에서 다시 시도합니다. 계속 실패하면 nwui-db-upgrade auto-status 명령의 안내를 따르세요.");
    }
    #[tokio::test]
    #[ignore = "native PG17 notification-store proof"]
    async fn g22_native_notification_dedupe_and_target_transaction_rollback() {
        if std::env::var("NWUI_TEST_PROFILE_NATIVE").as_deref()!=Ok("1"){return;}
        use crate::db_upgrade::postgres;
        use std::{process::Stdio,time::Duration};
        let root=tempfile::tempdir().unwrap();fs::set_permissions(root.path(),fs::Permissions::from_mode(0o755)).unwrap();
        let data=root.path().join("data");let work=root.path().join("work");postgres::directory(&data).unwrap();postgres::directory(&work).unwrap();
        let mut init=postgres::command(17,"initdb").unwrap();init.arg("-D").arg(&data).args(["--locale=C","--encoding=UTF8","--auth=trust"]).stdout(Stdio::null()).stderr(Stdio::null());
        postgres::wait(init.spawn().unwrap(),Duration::from_secs(30)).unwrap();
        let pg=postgres::Private::start(17,&data,&work).unwrap();
        let (mut db,connection)=tokio_postgres::Config::new().host_path(&pg.socket).port(50432).user("postgres").dbname("postgres").connect(tokio_postgres::NoTls).await.unwrap();
        let task=tokio::spawn(connection);
        db.batch_execute("SET default_transaction_read_only=off").await.unwrap();
        for sql in include_str!("../../../../packages/db/drizzle/0010_young_shockwave.sql").split("--> statement-breakpoint").take(2) {db.batch_execute(sql).await.unwrap();}
        db.batch_execute("ALTER TABLE notifications ADD COLUMN dedupe_key text; ALTER TABLE notifications ADD CONSTRAINT notifications_dedupe_key_unique UNIQUE(dedupe_key)").await.unwrap();
        let backup=sample().backup.unwrap();let kept=backup_kept_notification(&backup,"/destination/db/backup");
        let tx=db.transaction().await.unwrap();assert!(insert_notification(&tx,&kept).await.unwrap());tx.rollback().await.unwrap();
        assert_eq!(db.query_one("SELECT count(*)::bigint FROM notifications",&[]).await.unwrap().get::<_,i64>(0),0);
        assert!(insert_notification(&db,&kept).await.unwrap());assert!(!insert_notification(&db,&kept).await.unwrap());
        let original_id:String=db.query_one("SELECT id FROM notifications WHERE dedupe_key=$1",&[&kept.dedupe_key]).await.unwrap().get(0);
        db.execute("UPDATE notifications SET body='old restore guidance',data='{}'::jsonb WHERE dedupe_key=$1",&[&kept.dedupe_key]).await.unwrap();
        assert!(insert_notification(&db,&kept).await.unwrap());assert!(!insert_notification(&db,&kept).await.unwrap());
        assert_eq!(db.query_one("SELECT id FROM notifications WHERE dedupe_key=$1",&[&kept.dedupe_key]).await.unwrap().get::<_,String>(0),original_id);
        let failed=activation_failed_notification("attempt","activation_failed");assert!(insert_notification(&db,&failed).await.unwrap());assert!(!insert_notification(&db,&failed).await.unwrap());
        let rows=db.query("SELECT type,severity::text,title,body,data,dedupe_key FROM notifications ORDER BY type",&[]).await.unwrap();assert_eq!(rows.len(),2);
        for (row,expected) in rows.iter().zip([&failed,&kept]) {
            assert_eq!(row.get::<_,String>(0),expected.kind);assert_eq!(row.get::<_,String>(1),expected.severity);assert_eq!(row.get::<_,String>(2),expected.title);assert_eq!(row.get::<_,String>(3),expected.body);assert_eq!(row.get::<_,serde_json::Value>(4),expected.data);assert_eq!(row.get::<_,String>(5),expected.dedupe_key);
        }
        drop(db);task.await.unwrap().unwrap();drop(pg);
    }
    #[test] fn g22_pending_uses_exact_design_notice_and_code() {
        let value=sample();
        assert!(format_human(&value).contains("이전 PG15 데이터 정리가 끝나지 않았습니다 — 서비스는 정상이며 다음 시작 때 다시 정리합니다 (insufficient_space)"));
    }

    fn raw(path:&Path,bytes:impl AsRef<[u8]>) {fs::write(path,bytes).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(0o644)).unwrap();}
    fn code<T>(result:Result<T,StepError>,expected:&str) {assert_eq!(result.err().expect("expected refusal").code,expected);}
    #[test] fn literal_round_trip_and_root_mode() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");let value=sample();
        publish_at(&path,&value).unwrap();assert_eq!(serde_json::to_value(read_at(&path).unwrap()).unwrap(),serde_json::from_str::<serde_json::Value>(A8_LITERAL).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode()&0o7777,0o644);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
    }
    #[test] fn missing_and_untrusted_leaves_refuse_without_blocking() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");code(read_at(&path),"status_missing");
        raw(&path,A8_LITERAL);
        for mode in [0o600,0o666,0o1644] {fs::set_permissions(&path,fs::Permissions::from_mode(mode)).unwrap();code(read_at(&path),"status_unsafe");}
        raw(&path,A8_LITERAL);let link=temp.path().join("link");symlink(&path,&link).unwrap();code(read_at(&link),"status_unsafe");
        let hard=temp.path().join("hard");fs::hard_link(&path,&hard).unwrap();code(read_at(&path),"status_unsafe");fs::remove_file(hard).unwrap();
        code(read_at(temp.path()),"status_unsafe");
        let socket=temp.path().join("socket");let _listener=std::os::unix::net::UnixListener::bind(&socket).unwrap();code(read_at(&socket),"status_unsafe");
        let fifo=temp.path().join("fifo");let name=std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o644)},0);code(read_at(&fifo),"status_unsafe");
        let name=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::chown(name.as_ptr(),10001,10001)},0);code(read_at(&path),"status_unsafe");
    }
    #[test] fn parent_rules_include_sticky_root_and_refuse_symlinks() {
        let temp=tempfile::tempdir().unwrap();let parent=temp.path().join("parent");fs::create_dir(&parent).unwrap();let path=parent.join("status.json");raw(&path,A8_LITERAL);
        fs::set_permissions(&parent,fs::Permissions::from_mode(0o777)).unwrap();code(read_at(&path),"status_unsafe");code(publish_at(&path,&sample()),"status_unsafe");
        fs::set_permissions(&parent,fs::Permissions::from_mode(0o1777)).unwrap();read_at(&path).unwrap();publish_at(&path,&sample()).unwrap();
        let alias=temp.path().join("alias");symlink(&parent,&alias).unwrap();code(read_at(&alias.join("status.json")),"status_unsafe");
        code(publish_at(&alias.join("status.json"),&sample()),"status_unsafe");
        let name=std::ffi::CString::new(parent.as_os_str().as_encoded_bytes()).unwrap();assert_eq!(unsafe{libc::chown(name.as_ptr(),10001,10001)},0);code(read_at(&path),"status_unsafe");
    }
    #[test] fn invalid_shapes_bytes_and_version_have_no_content_in_errors() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");
        for bytes in [vec![b' ';STATUS_MAX_BYTES+1],b"{PRIVATE_SECRET".to_vec(),b"{}".to_vec(),A8_LITERAL.replace("\"version\":1","\"version\":2").into_bytes(),vec![b'"',0xff,b'"']] {
            raw(&path,bytes);let error=read_at(&path).err().unwrap();assert_eq!(error.code,"status_invalid");let detail=error.detail.unwrap_or_default();assert!(detail.chars().count()<=256);assert!(!detail.contains("PRIVATE_SECRET"));
        }
    }
    #[test] fn invalid_publish_does_not_touch_parent_or_old_status() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");raw(&path,A8_LITERAL);let before=fs::read(&path).unwrap();
        let mut large=sample();large.failure.as_mut().unwrap().message="x".repeat(STATUS_MAX_BYTES);code(publish_at(&path,&large),"status_invalid");
        let mut bad=sample();bad.version=2;code(publish_at(&path,&bad),"status_invalid");assert_eq!(fs::read(&path).unwrap(),before);assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
        code(publish_at(&temp.path().join("absent").join("file"),&large),"status_invalid");assert!(!temp.path().join("absent").exists());
        code(publish_at(temp.path(),&sample()),"status_unsafe");
    }
    #[test] fn atomic_replacement_never_accepts_partial_content() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");let a=sample();let mut b=a.clone();b.failure.as_mut().unwrap().message="다른 완전한 상태".repeat(256);publish_at(&path,&a).unwrap();
        let start=Arc::new(Barrier::new(2));let done=Arc::new(AtomicBool::new(false));let p=path.clone();let s=start.clone();let d=done.clone();let av=serde_json::to_value(&a).unwrap();let bv=serde_json::to_value(&b).unwrap();
        let reader=std::thread::spawn(move||{read_at(&p).unwrap();s.wait();let mut accepted=1;while !d.load(Ordering::Acquire){match read_at(&p){Ok(value)=>{let value=serde_json::to_value(value).unwrap();assert!(value==av||value==bv);accepted+=1;},Err(error)=>panic!("atomic replacement must preserve the opened publication: {error:?}")}}accepted});
        start.wait();for i in 0..32 {publish_at(&path,if i%2==0 {&b}else{&a}).unwrap();}done.store(true,Ordering::Release);assert!(reader.join().unwrap()>0);
    }
    #[test] fn opened_publication_survives_atomic_replace_before_read() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");let old=sample();publish_at(&path,&old).unwrap();
        let (directory,leaf)=parent(&path).unwrap();let opened=open_at(&directory,&leaf,libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK,0).unwrap();
        let mut new=sample();new.outcome=super::super::Outcome::Pg17;publish_at(&path,&new).unwrap();
        assert_eq!(opened.metadata().unwrap().nlink(),0);
        assert_eq!(serde_json::to_value(read_open_file(opened,&path).unwrap()).unwrap(),serde_json::to_value(old).unwrap());
        assert_eq!(serde_json::to_value(read_at(&path).unwrap()).unwrap(),serde_json::to_value(new).unwrap());
    }
    #[test] fn root_status_can_be_read_by_an_unprivileged_uid() {
        const CHILD:&str="NWUI_STATUS_READER_CHILD";
        if let Some(path)=std::env::var_os(CHILD) {assert_eq!(unsafe{libc::geteuid()},10001);read_at(Path::new(&path)).unwrap();code(publish_at(Path::new(&path),&sample()),"status_unsafe");return;}
        use std::os::unix::process::CommandExt;
        let temp=tempfile::tempdir().unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o755)).unwrap();let path=temp.path().join("status.json");publish_at(&path,&sample()).unwrap();
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::status::tests::root_status_can_be_read_by_an_unprivileged_uid","--nocapture"]).env(CHILD,&path).uid(10001).gid(10001).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stdout));
    }
    #[test] fn publisher_never_emits_duplicate_keys() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");publish_at(&path,&sample()).unwrap();
        let text=fs::read_to_string(&path).unwrap();
        // Every serialized field is known to this typed decoder, which rejects duplicates.
        serde_json::from_str::<UpgradeStatus>(&text).unwrap();
        let duplicate=text.replacen("\"version\":1","\"version\":1,\"version\":1",1);
        assert_ne!(duplicate,text);assert!(serde_json::from_str::<UpgradeStatus>(&duplicate).is_err());
        raw(&path,duplicate);code(read_at(&path),"status_invalid");
    }
    #[test] fn failed_publish_preserves_old_file_and_removes_temp() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");raw(&path,A8_LITERAL);
        FAIL_AFTER_WRITE.with(|f|f.set(true));code(publish_at(&path,&sample()),"insufficient_space");
        assert_eq!(fs::read(&path).unwrap(),A8_LITERAL.as_bytes());assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
        publish_at(&path,&sample()).unwrap();
        for (errno,expected) in [(libc::EROFS,"storage_read_only"),(libc::ENOSPC,"insufficient_space"),(libc::EDQUOT,"insufficient_space"),(libc::EINTR,"interrupted"),(libc::EIO,"io_error")] {assert_eq!(io_error(io::Error::from_raw_os_error(errno),&path).code,expected);}
    }
    #[test] fn reader_uses_frozen_types_without_unwritten_semantic_limits() {
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("status.json");let mut value=serde_json::from_str::<serde_json::Value>(A8_LITERAL).unwrap();
        value["updatedAt"]=serde_json::json!("not a timestamp");value["step"]["startedAt"]=serde_json::json!("");value["step"]["total"]=serde_json::json!(0);value["step"]["bytesTotal"]=serde_json::json!(u64::MAX);value["locations"]=serde_json::json!(["relative"]);
        value.as_object_mut().unwrap().remove("backup");value["retry"]=serde_json::json!({});raw(&path,serde_json::to_vec(&value).unwrap());let result=read_at(&path).unwrap();assert!(result.backup.is_none());assert_eq!(result.step.bytes_total,u64::MAX);
        for bad in [serde_json::json!(-1),serde_json::json!(1.5),serde_json::json!("1")] {value["step"]["bytesTotal"]=bad;raw(&path,serde_json::to_vec(&value).unwrap());code(read_at(&path),"status_invalid");}
    }
    #[test] fn human_format_distinguishes_unknown_inodes_from_a_known_zero(){
        let mut value=sample();let p=value.space.as_mut().unwrap();p.inodes_required=0;p.inodes_available=0;
        let text=format_human(&value);assert!(text.contains("공간: 1024 / 4096 바이트, inode: 파일 시스템이 개수를 제공하지 않아 확인하지 않음"));assert!(!text.contains("inode: 0 / 0"));
        value.space.as_mut().unwrap().inodes_required=8;let text=format_human(&value);assert!(text.contains("inode: 0 / 8"));assert!(!text.contains("개수를 제공하지 않아"));
        let p=value.space.as_mut().unwrap();p.inodes_required=0;p.inodes_available=4;assert!(format_human(&value).contains("inode: 4 / 0"));
    }
    #[test] fn c2_a_step_without_a_position_shows_only_its_label_and_a_placeholder_shows_nothing() {
        let mut value=sample();value.step.index=0;value.step.total=14;let text=format_human(&value);
        assert_eq!(text.lines().nth(1),Some("원본 보관"),"{text}");assert!(!text.contains("경과"),"{text}");
        value.step.total=0;let text=format_human(&value);
        assert!(!text.contains("원본 보관")&&!text.contains("0/0")&&!text.contains("경과"),"{text}");
        assert_eq!(step_position(super::super::Step::P3Convert),(5,14));assert_eq!(step_position(super::super::Step::Y1),(0,14));
    }
    #[test] fn human_format_uses_status_time_and_all_frozen_fields() {
        let text=format_human(&sample());for expected in ["데이터베이스를 준비하고 있습니다","단계 3/11, 원본 보관","6초","1024 / 4096 바이트","insufficient_space","여유 공간을 확보하세요.","kept","2048 바이트","nwui-db-upgrade remove-pg15-backup --confirm kept","aside","512 바이트","nwui-db-upgrade remove-discarded --confirm aside","2026-10-29T00:00:00Z"] {assert!(text.contains(expected),"missing {expected}: {text}");}
        let snapshot="데이터베이스를 준비하고 있습니다\n단계 3/11, 원본 보관 · 경과 6초\n1024 / 4096 바이트\ninsufficient_space: 공간을 확인하세요. 여유 공간을 확보하세요.\n공간: 1024 / 4096 바이트, inode: 4 / 8\n보존 위치: /var/lib/postgresql/data/.nwui-pg15-backup/kept\n보관: kept · 2048 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: nwui-db-upgrade remove-pg15-backup --confirm kept\n분리 보존: aside · 512 바이트 · 컨테이너를 중지하고 같은 볼륨으로 실행: nwui-db-upgrade remove-discarded --confirm aside\n이전 PG15 데이터 정리가 끝나지 않았습니다 — 서비스는 정상이며 다음 시작 때 다시 정리합니다 (insufficient_space)\n다음 재시도 가능: 2026-10-29T00:00:00Z\n재시도: 다음 시작 때 재시도합니다.";
        assert_eq!(text,snapshot);assert_eq!(text,format_human(&sample()));assert!(!text.contains('\u{1b}'));
        for (outcome,label) in [(super::super::Outcome::Restored,"원본 PG15 데이터를 검증 후 복원했습니다"),(super::super::Outcome::RefusedPreflight,"업그레이드 전 검사에서 중단했습니다"),(super::super::Outcome::Pg17,"데이터베이스 준비를 마쳤습니다"),(super::super::Outcome::RefusedFresh,"새 설치 준비를 끝내지 못했습니다"),(super::super::Outcome::RefusedPrivilege,"PG17 데이터는 변경되지 않았습니다")] {let mut value=sample();value.outcome=outcome;assert_eq!(format_human(&value),snapshot.replacen("데이터베이스를 준비하고 있습니다",label,1));
            let browser=include_str!("../../../../docker/upgrade-status/status.mjs");let key=serde_json::to_value(outcome).unwrap();let key=key.as_str().unwrap();let key=if key.contains('-'){format!("'{key}'")}else{key.into()};assert!(browser.contains(&format!("{key}:['{label}'")),"browser/CLI headline parity: {key}");
        }
    }
}

#[cfg(test)]mod desktop_shared_tests{
 use super::*;
 #[test]fn desktop_shared_scope_text_and_notice_key_are_canonical_and_not_failures(){
  assert!(warnings_for(vec![]).is_empty());
  let a=warnings_for(vec!["app".into(),"db".into(),"app".into()]);let b=warnings_for(vec!["db".into(),"app".into()]);assert_eq!(a,b);let line=storage_log_line(&a[0]);assert_eq!(line.lines().count(),1);assert!(line.contains(&a[0].message)&&line.contains("root privileges"));let n=storage_notification(&a[0]);assert_eq!(n.dedupe_key,"desktop-shared-folder-isolation:v1:app,db");assert_eq!(n.body,a[0].message);assert!(n.body.contains("root 권한"));assert!(!n.body.contains("로그를"));
  let log=warnings_for(vec!["app-log".into()]);assert!(!storage_log_line(&log[0]).contains("root privileges"));assert!(!log[0].message.contains("root 권한"));assert!(!log[0].message.contains("DB 파일"));assert!(log[0].message.contains("현재 해당 저장소: 앱 로그"));
  for w in a.iter().chain(log.iter()){assert!(!w.message.contains("/run/")&&!w.message.contains("/var/"));assert!(w.message.contains("OrbStack과 Docker 관리 볼륨"));assert!(w.message.contains("Docker Desktop·OrbStack Mac 공유 폴더"));assert!(storage_log_line(w).contains("Docker Desktop/OrbStack Mac shared folders"));}
 }
 #[test]fn desktop_shared_old_status_literal_is_unchanged_and_warning_is_additive(){
  let mut value=super::tests::sample();let before=serde_json::to_value(&value).unwrap();assert!(before.get("warnings").is_none());value.warnings=warnings_for(vec!["db".into()]);let mut after=serde_json::to_value(&value).unwrap();assert!(after.as_object_mut().unwrap().remove("warnings").is_some());assert_eq!(before,after);assert!(format_human(&value).contains(&value.warnings[0].message));
 }
}

#[cfg(test)]mod desktop_shared_database_tests{
 use super::*;
 #[tokio::test]
 #[ignore = "real DB: set NWUI_TEST_DATABASE_URL"]
 async fn storage_notice_dedupe_and_insert_failure_are_nonfatal(){
  let Ok(url)=std::env::var("NWUI_TEST_DATABASE_URL")else{eprintln!("SKIP storage_notice_dedupe: database profile absent");return};
  let(mut db,connection)=tokio_postgres::connect(&url,tokio_postgres::NoTls).await.unwrap();let task=tokio::spawn(connection);
  let tx=db.transaction().await.unwrap();let schema=format!("shared_{}",uuid::Uuid::new_v4().simple());
  tx.batch_execute(&format!("CREATE SCHEMA {schema}; SET LOCAL search_path={schema}; CREATE TYPE notification_severity AS ENUM('warning'); CREATE TABLE notifications(id text,type text,severity notification_severity,title text,body text,link text,data jsonb,dedupe_key text UNIQUE)")).await.unwrap();
  let warning=warnings_for(vec!["app-log".into()]);let notice=storage_notification(&warning[0]);
  assert!(insert_notification(&tx,&notice).await.unwrap());assert!(!insert_notification(&tx,&notice).await.unwrap());
  let row=tx.query_one("SELECT count(*)::bigint FROM notifications",&[]).await.unwrap();assert_eq!(row.get::<_,i64>(0),1);
  let mut normalized = super::super::types::StorageWarning {
   code: super::super::storage_modes::WARNING_CODE.into(),
   message: "시작 전에 저장소 권한을 정리했습니다. 그룹·기타 사용자의 쓰기 권한을 빼고, 비밀 값·인증서 파일과 폴더는 검사가 허용하는 권한으로 맞췄습니다. 파일 내용과 소유자는 바꾸지 않았습니다. 바뀐 항목: auth_secret 0644 → 0600. 이 권한은 1.19.11로 되돌려도 그대로 사용할 수 있습니다.".into(),
   scopes: vec!["app".into()],
  };
  let historical = storage_notification(&normalized);
  assert!(insert_notification(&tx, &historical).await.unwrap());
  normalized.message = "시작 전에 저장소 접근 권한을 줄였습니다. 파일 내용과 소유자는 그대로입니다. 바뀐 항목: auth_secret 0644 → 0600.".into();
  let current = storage_notification(&normalized);
  assert!(insert_notification(&tx, &current).await.unwrap());
  assert!(!insert_notification(&tx, &storage_notification(&normalized)).await.unwrap());
  let rows = tx.query("SELECT body FROM notifications WHERE type = $1", &[&current.kind]).await.unwrap();
  let bodies: std::collections::HashSet<&str> = rows.iter().map(|row| row.get(0)).collect();
  assert_eq!(rows.len(), 2);
  assert_eq!(bodies, [historical.body.as_str(), current.body.as_str()].into_iter().collect());
  tx.batch_execute("DROP TABLE notifications").await.unwrap();assert!(insert_notification(&tx,&notice).await.is_err());
  tx.rollback().await.unwrap();drop(db);task.await.unwrap().unwrap();
 }
}
