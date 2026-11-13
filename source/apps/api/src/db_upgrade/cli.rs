//! Offline entry point: no AppState, migrations, HTTP clients or background workers.
use super::{arguments::Arguments,engine::{self,Engine},error::{Error,Result},output,postgres,state::State};
use std::sync::atomic::Ordering;
const USAGE:&str="Usage: nwui-db-upgrade check|run --backup-id <id> --destination-db-volume <name>|--destination-db-bind <path> --destination-app-volume <name>|--destination-app-bind <path> [--json]\n       nwui-db-upgrade status|verify [--json]\n       nwui-db-upgrade auto-status [--json]\n       nwui-db-upgrade restore-pg15 --confirm-loss <backup-id>\n       nwui-db-upgrade remove-pg15-backup --confirm <backup-id>\n       nwui-db-upgrade remove-discarded --confirm <discard-id>";
fn emit(state:&State,json:bool)->Result<()> {
    if json {println!("{}",serde_json::to_string(state).map_err(|_|Error::refusal("unsupported_layout"))?);}
    else {print!("{}",output::human(state)?);}Ok(())
}
pub async fn run(args:Vec<String>)->i32 {
    if args.first().is_some_and(|arg|arg=="auto-status") {return crate::upgrade::auto_status(&args[1..]).await;}
    if let Some(command)=args.first().filter(|arg|matches!(arg.as_str(),"restore-pg15"|"remove-pg15-backup"|"remove-discarded")) {
        return match crate::upgrade::operator_command(command,&args[1..]).await {
            Ok(message)=>{println!("{message}");0},
            Err(error)=>{eprintln!("{}: {}",error.code,error.message);2},
        };
    }
    let json=args.iter().any(|a|a=="--json");
    let result=Arguments::parse(args).and_then(|args|Engine::fixed().map(|engine|(args,engine)));
    let (args,engine)=match result {Ok(value)=>value,Err(error)=>{if error.code=="invalid_arguments" {eprintln!("{USAGE}");}return report(None,json,error)}};
    let result=cancellable(engine.execute(&args)).await;
    match result {
        Ok(outcome)=>{if let Some(notice)=outcome.notice{eprintln!("{notice}");}match emit(&outcome.state,json){Ok(())=>0,Err(error)=>report(Some(&engine),json,error)}},
        Err(error)=>report(Some(&engine),json,error),
    }
}
fn report(engine:Option<&Engine>,json:bool,error:Error)->i32 {
    let error=postgres::interrupted().unwrap_or(error);
    eprintln!("{error}");
    let state=engine.and_then(|engine|engine.status().ok()).or_else(||engine::architecture().ok().map(State::not_started));
    if let Some(state)=state {let _=emit(&output::failure(state,&error),json);}error.exit
}
pub async fn cancellable<T>(operation:impl std::future::Future<Output=Result<T>>)->Result<T> {
    use tokio::signal::unix::{signal,SignalKind};
    let mut int=signal(SignalKind::interrupt()).map_err(|_|Error::refusal("unsupported_layout"))?;
    let mut term=signal(SignalKind::terminate()).map_err(|_|Error::refusal("unsupported_layout"))?;
    postgres::SIGNAL.store(0,Ordering::Relaxed);super::files::CANCELLED.store(false,Ordering::Relaxed);
    // A separate task sets the flag even while the command runner is synchronously waiting.
    let mut watcher=tokio::spawn(async move {
        let number=tokio::select!{_=int.recv()=>2,_=term.recv()=>15};
        postgres::SIGNAL.store(number,Ordering::Relaxed);super::files::CANCELLED.store(true,Ordering::Relaxed);number
    });
    let result=tokio::select! {
        result=operation=>result,
        signal=&mut watcher=>{let number=signal.unwrap_or(15);Err(Error{code:"interrupted",message:"이전 작업이 중단됐습니다. 원본과 현재 작업을 보존했습니다.",exit:128+number,detail:None})},
    };
    watcher.abort();result
}
