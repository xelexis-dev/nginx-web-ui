use super::{state::{State,Operation,LastError,Checks,Destination},error::{Error,Result}};
fn failed()->Error{Error::refusal("unsupported_layout")}
pub fn failure(mut state:State,error:&Error)->State {
    state.operation=Operation::Failed;state.next_phase=None;state.ready=false;state.checks=Checks::default();
    state.last_error=Some(LastError{code:error.code.into(),message:error.message.into()});state
}
fn yaml(value:&str)->Result<String>{serde_json::to_string(value).map_err(|_|failed())}
pub fn human(state:&State)->Result<String> {
    let phase=serde_json::to_string(&state.phase).map_err(|_|failed())?;
    let mut out=format!("PostgreSQL 15 → 17 | 단계: {} | 준비: {}\n",phase.trim_matches('"'),if state.ready{"완료"}else{"미완료"});
    if state.ready{out.push_str("전환 후 원래 볼륨 쌍으로 되돌리면 전환 이후의 기록은 사라집니다.\n");}
    else{out.push_str("대상을 전환하지 마세요. 원본과 실패한 대상은 보존하세요.\n");}
    if let Some(bytes)=state.summary.required_bytes{out.push_str(&format!("예상 대상 공간: {bytes} bytes. 호스트·VM 여유 공간도 확인하세요.\n"));}
    if let Some(tables)=&state.summary.table_counts{for (name,count) in tables{out.push_str(&format!("{name}: {count}\n"));}}
    if let Some(targets)=&state.destinations {
        out.push_str("\nservices:\n  nginx-web-ui:\n    volumes:\n");
        let mut external=String::new();
        for (name,target,path) in [("nwui_db",&targets.db,"/var/lib/postgresql/data"),("nwui_app",&targets.app,"/var/lib/nginx-web-ui")] {
            target.validate().map_err(|_|failed())?;
            match target {
                Destination::Volume{source}=>{out.push_str(&format!("      - type: volume\n        source: {name}\n        target: {}\n",yaml(path)?));external.push_str(&format!("  {name}:\n    external: true\n    name: {}\n",yaml(source)?));},
                Destination::Bind{source}=>out.push_str(&format!("      - type: bind\n        source: {}\n        target: {}\n",yaml(source)?,yaml(path)?)),
            }
        }
        if !external.is_empty(){out.push_str("volumes:\n");out.push_str(&external);}
    }
    if let Some(error)=&state.last_error{out.push_str(&format!("{}: {}\n",error.code,error.message));}Ok(out)
}
