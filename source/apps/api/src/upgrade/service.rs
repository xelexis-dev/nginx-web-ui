//! Closed s6 service operations; callers await them on a blocking thread under custody.
use super::StepError;
use std::{process::{Command,Stdio},os::unix::process::CommandExt,time::{Duration,Instant}};
const PATH:&str="/command:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const STATUS:&str="/run/service/upgrade-status";
// Units without notification-fd need up; the later HTTP/generation smokes prove they serve.
struct Dependent {service:&'static str,notification:bool,phase2:bool}
static DEPENDENTS:[Dependent;5]=[
    Dependent{service:"/run/service/postgres",notification:false,phase2:false},
    Dependent{service:"/run/service/nginx",notification:false,phase2:true},
    Dependent{service:"/run/service/api",notification:true,phase2:true},
    Dependent{service:"/run/service/web",notification:false,phase2:false},
    Dependent{service:"/run/service/publisher",notification:true,phase2:true},
];
fn dependents()->impl Iterator<Item=&'static Dependent> {
    DEPENDENTS.iter().filter(|d|d.service!="/run/service/publisher"||crate::nonroot::PRIVILEGE_SPLIT)
}
fn error(code:&'static str)->StepError {StepError{code,message:"서비스 전환을 제한 시간 안에 확인하지 못했습니다. 잠금과 진단 기록을 보존합니다.",detail:None}}
fn command(tool:&str)->Command {
    let mut c=Command::new(tool);c.env_clear().env("PATH",PATH).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);c
}
fn invoke(mut c:Command,bound:Duration)->Result<(),StepError> {
    let child=c.spawn().map_err(|_|error("io_error"))?;
    crate::db_upgrade::postgres::wait(child,bound).map_err(|e|error(if e.code=="interrupted"{"interrupted"}else{"activation_timeout"}))
}
fn status_commands()->[Command;2] {
    let mut down=command("s6-svc");down.args(["-d",STATUS]);
    let mut confirmed=command("s6-svwait");confirmed.args(["-D","-t","5000",STATUS]);[down,confirmed]
}
/// Only PG17 handoff calls this. A refusal leaves the status responder running.
pub(crate) fn stop_status()->Result<(),StepError> {
    let [down,confirmed]=status_commands();invoke(down,Duration::from_secs(5))?;invoke(confirmed,Duration::from_millis(5250))
}
fn supervisor_fields(service:&str,fields:&str)->Result<String,StepError>{
    let mut c=command("s6-svstat");c.args(["-o",fields,service]);
    let (out,_)=crate::db_upgrade::postgres::output(c,Duration::from_secs(1)).map_err(|_|error("activation_timeout"))?;
    Ok(out)
}
pub(super) fn restore_status_responder()->Result<(),StepError>{
    if supervisor_fields(STATUS,"up,wantedup")?.split_whitespace().eq(["false","false"])
        && supervisor_fields("/run/service/nginx","up")?.trim()=="false" {
        let mut up=command("s6-svc");up.args(["-u",STATUS]);
        invoke(up,Duration::from_secs(1))?;
    }
    invoke(dependent_command(STATUS,true,Duration::from_secs(5)),Duration::from_millis(5250))
}
pub(super) async fn status_shows_refusal(expected:&super::types::StatusFailure)->bool{
    let Ok(client)=reqwest::Client::builder()
        .no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(2)).build() else{return false;};
    let Ok(mut response)=client.get("http://127.0.0.1:8080/status.json").send().await else{return false;};
    if response.status()!=reqwest::StatusCode::OK{return false;}
    let mut bytes=Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len()+chunk.len()<=super::status::STATUS_MAX_BYTES=>bytes.extend_from_slice(&chunk),
            Ok(None)=>break,
            _=>return false,
        }
    }
    serde_json::from_slice::<super::types::UpgradeStatus>(&bytes).is_ok_and(|status|
        super::unrecorded::journal_refusal(&status)
            && status.failure.is_some_and(|value|value.code==expected.code&&value.message==expected.message&&value.next_action==expected.next_action)
    )
}
pub(super) fn halt_overlay(){
    let _=invoke(command("/run/s6/basedir/bin/halt"),Duration::from_secs(2));
}
fn dependent_command(service:&str,notification:bool,bound:Duration)->Command {
    let mut c=command("s6-svwait");c.args([if notification{"-U"}else{"-u"},"-t"]).arg(bound.as_millis().max(1).to_string()).arg(service);c
}
/// The bound covers the whole graph observation, not a fresh timeout for every service.
pub(crate) fn wait_dependents(bound:Duration)->Result<(),StepError> {
    let deadline=Instant::now().checked_add(bound).ok_or_else(||error("activation_timeout"))?;
    for dependent in dependents() {
        let remaining=deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or_else(||error("activation_timeout"))?;
        invoke(dependent_command(dependent.service,dependent.notification,remaining),remaining)?;
    }
    Ok(())
}
/// A.24's read-only readiness observation; no service command or notification is sent.
pub(crate) fn phase2_ready(bound:Duration)->Result<bool,StepError> {
    let deadline=Instant::now().checked_add(bound).ok_or_else(||error("activation_timeout"))?;
    for dependent in dependents().filter(|d|d.phase2) {
        let remaining=deadline.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or_else(||error("activation_timeout"))?;
        let c=phase2_command(dependent);
        let (out,_)=crate::db_upgrade::postgres::output(c,remaining).map_err(|_|error("activation_timeout"))?;
        if !ready_fields(&out,dependent.notification){return Ok(false);}
    }Ok(true)
}
fn phase2_command(dependent:&Dependent)->Command {
    let mut c=command("s6-svstat");c.args(["-o",if dependent.notification{"up,ready"}else{"up"},dependent.service]);c
}
fn ready_fields(output:&str,notification:bool)->bool {
    let expected:&[&str]=if notification{&["true","true"]}else{&["true"]};
    output.split_whitespace().eq(expected.iter().copied())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn w1_phase2_s6_requires_both_explicit_up_and_ready() {
        assert!(ready_fields("true true\n",true));for raw in ["true false","false true","false false","true","true true extra","up ready",""]{assert!(!ready_fields(raw,true));}
    }

    #[test]
    fn w1_g44_nginx_without_notification_accepts_up() {
        assert!(ready_fields("true\n",false));
        for raw in ["false","true false","true true","true extra","", "up"]{assert!(!ready_fields(raw,false));}
    }
    #[test]
    fn w1_g44_service_flags_match_source_notification_files() {
        let units=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker/s6-rc.d");
        for dependent in &DEPENDENTS {
            let name=dependent.service.strip_prefix("/run/service/").unwrap();let unit=units.join(name);
            assert!(unit.is_dir(),"missing service unit: {name}");
            assert_eq!(dependent.notification,unit.join("notification-fd").is_file(),"notification flag: {name}");
        }
        assert!(DEPENDENTS.iter().any(|d|d.service=="/run/service/publisher"),"publisher must share the readiness table");
        let phase2=dependents().filter(|d|d.phase2).map(|d|d.service).collect::<Vec<_>>();
        assert_eq!(phase2,if crate::nonroot::PRIVILEGE_SPLIT{vec!["/run/service/nginx","/run/service/api","/run/service/publisher"]}else{vec!["/run/service/nginx","/run/service/api"]});
        for dependent in dependents() {
            let c=phase2_command(dependent);
            assert_eq!(c.get_args().collect::<Vec<_>>(),["-o",if dependent.notification{"up,ready"}else{"up"},dependent.service]);
        }
    }

    #[test]
    fn w1_service_status_handoff_requests_down_then_confirms_definite_down() {
        let [down,confirmed]=status_commands();
        assert_eq!(down.get_program(),"s6-svc");assert_eq!(down.get_args().collect::<Vec<_>>(),["-d",STATUS]);
        assert_eq!(confirmed.get_program(),"s6-svwait");assert_eq!(confirmed.get_args().collect::<Vec<_>>(),["-D","-t","5000",STATUS]);
        assert!(down.get_envs().all(|(k,v)|k=="PATH" && v==Some(std::ffi::OsStr::new(PATH))));
    }
    #[test]
    fn w1_service_waits_for_notifications_only_on_units_that_declare_them() {
        for dependent in dependents() {
            let service=dependent.service;let c=dependent_command(service,dependent.notification,Duration::from_millis(321));let args=c.get_args().map(|a|a.to_str().unwrap()).collect::<Vec<_>>();
            assert_eq!(args,[if matches!(service,"/run/service/api"|"/run/service/publisher"){"-U"}else{"-u"},"-t","321",service]);
        }
    }
    #[test]
    fn w1_service_timeout_kills_and_reaps_the_actual_child() {
        let t=tempfile::tempdir().unwrap();let pid=t.path().join("pid");
        let mut c=Command::new("/bin/sh");c.args(["-c","echo $$ > \"$1\"; exec sleep 30","service-test"]).arg(&pid).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
        let start=Instant::now();assert_eq!(invoke(c,Duration::from_millis(150)).unwrap_err().code,"activation_timeout");assert!(start.elapsed()<Duration::from_secs(3));
        let child=std::fs::read_to_string(pid).unwrap().trim().parse::<i32>().unwrap();assert_eq!(unsafe{libc::kill(child,0)},-1);assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
    }
    #[test]
    fn w1_service_zero_deadline_never_starts_a_dependent_wait() {assert_eq!(wait_dependents(Duration::ZERO).unwrap_err().code,"activation_timeout");}
}
