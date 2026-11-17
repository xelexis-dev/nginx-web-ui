//! Early boot filesystem gates. No configuration, database connection or key generation.
use super::{paths,postgres,layout::{self,Roots},key_binding,error::{Error,Result}};
pub fn roots()->Result<Roots> {
    paths::validate_env()?;
    Ok(Roots{db:paths::PGROOT.into(),app:paths::APPROOT.into(),architecture:paths::architecture()?.into()})
}
pub fn classify()->Result<()> {
    layout::prepare(&roots()?,&|data|Ok(postgres::read_control(17,data)?.system_identifier))?;Ok(())
}
/// Phase-1 agreement is read-only; upgrade callers persist the flag only after commit.
pub fn verify_key_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,key:&crate::crypto::SecretBox)->Result<crate::upgrade::KeyBinding> {
    let pair=key_binding::verify_key_locked(locks,roots,&|data|Ok(postgres::read_control(17,data)?.system_identifier),key)?;
    Ok(crate::upgrade::KeyBinding::Verified {pair_id:pair.pair_id})
}

/// Internal s6 commands share the offline signal/cleanup boundary, without AppState.
pub async fn run(args:&[String])->i32 {
    let parts=args.iter().map(String::as_str).collect::<Vec<_>>();
    let result=cancellable(async {
        match parts.as_slice() {
            ["storage","classify"]=>classify(),
            ["storage-secrets"]=>super::fresh::secrets(&roots()?,&super::fresh::Inputs::from_env()?,std::path::Path::new("/run/s6/container_environment")),
            ["storage-provision"]=>super::fresh::provision(&roots()?).await,
            _=>Err(Error::refusal("invalid_arguments")),
        }
    }).await;
    match result {Ok(())=>0,Err(error)=>{let error=postgres::interrupted().unwrap_or(error);eprintln!("{error}");error.exit}}
}

pub async fn cancellable<T>(operation:impl std::future::Future<Output=Result<T>>)->Result<T> {
    use tokio::signal::unix::{signal,SignalKind};
    let mut int=signal(SignalKind::interrupt()).map_err(|_|Error::refusal("unsupported_layout"))?;
    let mut term=signal(SignalKind::terminate()).map_err(|_|Error::refusal("unsupported_layout"))?;
    postgres::SIGNAL.store(0,std::sync::atomic::Ordering::Relaxed);super::files::CANCELLED.store(false,std::sync::atomic::Ordering::Relaxed);
    // A separate task sets the flag even while the command runner is synchronously waiting.
    let mut watcher=tokio::spawn(async move {
        let number=tokio::select!{_=int.recv()=>2,_=term.recv()=>15};
        postgres::SIGNAL.store(number,std::sync::atomic::Ordering::Relaxed);super::files::CANCELLED.store(true,std::sync::atomic::Ordering::Relaxed);number
    });
    let result=tokio::select! {
        result=operation=>result,
        signal=&mut watcher=>{let number=signal.unwrap_or(15);Err(Error{code:"interrupted",message:"이전 작업이 중단됐습니다. 원본과 현재 작업을 보존했습니다.",exit:128+number,detail:None})},
    };
    watcher.abort();result
}
