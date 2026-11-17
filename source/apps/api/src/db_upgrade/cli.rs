//! Offline status and backup management; legacy migration commands never open storage.
pub const LEGACY_GUIDANCE:&str="이 명령은 2.1에서 실행할 수 없습니다. 컨테이너를 중지한 뒤 2.0.x 최신(2.0.3 이상) 이미지의 offline entrypoint(nwui-db-upgrade)로 같은 명령을 실행하세요. 2.0 이미지를 정상 시작해 2.1 데이터를 열지 마세요. 기존 데이터는 변경하지 않았습니다.";
pub async fn run(args:Vec<String>)->i32 {
    match args.first().map(String::as_str) {
        Some("auto-status")=>crate::upgrade::auto_status(&args[1..]).await,
        Some("check"|"run"|"status"|"verify"|"restore-pg15")=>{eprintln!("requires_2_0_first: {LEGACY_GUIDANCE}");2},
        Some(command @ ("remove-pg15-backup"|"remove-discarded"))=>match crate::upgrade::operator_command(command,&args[1..]).await {
            Ok(message)=>{println!("{message}");0},Err(error)=>{eprintln!("{}: {}",error.code,error.message);2},
        },
        _=>{eprintln!("Usage: nwui-db-upgrade auto-status | remove-pg15-backup --confirm <id> | remove-discarded --confirm <id>");2},
    }
}
