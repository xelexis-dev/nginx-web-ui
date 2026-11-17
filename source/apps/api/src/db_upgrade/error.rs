use std::fmt;
#[derive(Debug,Clone)]
pub struct Error { pub code:&'static str, pub message:&'static str, pub exit:i32, pub detail:Option<&'static str> }
impl Error {
    pub fn refusal(code:&'static str)->Self {
        let message=match code {
            "requires_2_0_first"=>"1.x(PostgreSQL 15) 데이터입니다. 먼저 2.0.x 최신(2.0.3 이상)에서 이전을 완료하세요. 기존 데이터는 변경하지 않았습니다.",
            "migration_incomplete"=>"이전이 완료되지 않았습니다. 원본은 보존하고 nwui-db-upgrade status로 상태를 확인하세요.",
            "pair_mismatch"=>"DB와 앱 데이터가 같은 백업 쌍이 아닙니다. 새 키를 만들지 않았습니다. 올바른 두 볼륨을 연결하세요.",
            "key_path_invalid"=>"DATA_KEY_FILE은 비어 있지 않은 절대 경로여야 합니다. 키나 저장소를 변경하지 않았습니다.",
            "key_missing"=>"암호화 키가 없습니다. 새 키를 만들지 않았습니다. 원래 앱 볼륨의 키를 복원하세요.",
            "storage_override"=>"기존 PGDATA 설정이 새 저장 경로와 다릅니다. 데이터를 옮기지 말고 2.0 업그레이드 안내를 확인하세요.",
            "invalid_arguments"=>"이전 명령 인자가 올바르지 않습니다. check|run에는 백업 ID와 서로 다른 두 대상 볼륨 또는 바인드 경로를 명시하세요.",
            "source_changed"=>"원본 데이터가 작업 중 변경됐습니다. 기존 배포와 모든 원본 쓰기를 멈춘 뒤 새 백업과 새 대상 볼륨 쌍을 선택하세요.",
            "insufficient_space"=>"대상 공간 또는 inode가 부족합니다. 원본·백업을 보존하고 충분한 공간의 새 대상 볼륨 쌍을 선택하세요.",
            "storage_busy"=>"다른 저장소 작업이 실행 중입니다. 볼륨을 바꾸지 않았습니다. 해당 작업이 끝나거나 같은 저장소를 쓰는 컨테이너를 중지한 뒤 다시 시도하세요.",
            _=>"지원하지 않는 데이터 경로 또는 PostgreSQL 버전입니다. 초기화하지 않았습니다. 마운트와 이전 보고서를 확인하세요.",
        };
        Self{code,message,exit:2,detail:None}
    }
    pub fn existing_app()->Self { Self{code:"pair_mismatch",message:"기존 앱 데이터와 빈 DB 볼륨이 연결돼 있습니다. 새 설치는 두 볼륨이 모두 비어 있어야 합니다. 새 키나 DB를 만들지 않았습니다.",exit:2,detail:None} }
}
impl fmt::Display for Error { fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{write!(f,"{}: {}",self.code,self.message)} }
impl std::error::Error for Error {}
pub type Result<T>=std::result::Result<T,Error>;
