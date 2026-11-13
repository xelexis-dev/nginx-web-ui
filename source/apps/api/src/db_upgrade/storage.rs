//! Early boot filesystem gates. No configuration, database connection or key generation.
use super::{engine,paths,postgres,layout::{self,Roots},key_binding,error::{Error,Result}};
pub fn roots()->Result<Roots> {
    paths::validate_env()?;
    Ok(Roots{db:paths::PGROOT.into(),app:paths::APPROOT.into(),architecture:engine::architecture()?.into()})
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
    let result=super::cli::cancellable(async {
        match parts.as_slice() {
            ["storage","classify"]=>classify(),
            ["storage-secrets"]=>super::fresh::secrets(&roots()?,&super::fresh::Inputs::from_env()?,std::path::Path::new("/run/s6/container_environment")),
            ["storage-provision"]=>super::fresh::provision(&roots()?).await,
            _=>Err(Error::refusal("invalid_arguments")),
        }
    }).await;
    match result {Ok(())=>0,Err(error)=>{let error=postgres::interrupted().unwrap_or(error);eprintln!("{error}");error.exit}}
}
