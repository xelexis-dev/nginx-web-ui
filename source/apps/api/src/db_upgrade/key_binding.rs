use super::{layout::{self,Decision,Roots},files,pair::Pair,error::{Error,Result}};
use crate::crypto::SecretBox;
use std::path::Path;

pub fn mark_key_required(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,loaded:&SecretBox)->Result<()> {
    layout::classify(roots,read_id)?;
    let _locks=layout::lock_key_roots(roots)?;
    mark_under_lock(roots,read_id,loaded,None)
}
pub fn mark_key_required_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,loaded:&SecretBox)->Result<()> {
    if !locks.matches_roots(&roots.db,&roots.app){return Err(Error::refusal("storage_busy"));}
    mark_under_lock(roots,read_id,loaded,Some(locks))
}
/// Read-only A.13 agreement; the persistent required flag is deliberately separate.
pub fn verify_key_locked(locks:&crate::upgrade::StorageLocks,roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,loaded:&SecretBox)->Result<Pair> {
    if !locks.matches_roots(&roots.db,&roots.app){return Err(Error::refusal("storage_busy"));}
    agreement(roots,read_id,loaded,Some(locks))
}
fn agreement(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,loaded:&SecretBox,locks:Option<&crate::upgrade::StorageLocks>)->Result<Pair> {
    let mismatch=||Error::refusal("pair_mismatch");
    let Decision::Ready(pair)=layout::classify_for(locks,roots,read_id)? else {return Err(mismatch());};
    let path=super::paths::startup_data_key_file(&roots.app).map_err(|e|Error::refusal(e.code))?;
    let persisted=super::key_file::read_key_file(&path,super::key_file::canonical_policy(&roots.app,&path)).map_err(|_|mismatch())?.secrets;
    // Confirm the AppState key is the persisted key without exposing either key.
    let probe="nwui-db-upgrade key agreement";
    let sealed=loaded.seal(probe).map_err(|_|mismatch())?;
    if persisted.open(Some(&sealed)).as_deref()!=Some(probe){return Err(mismatch());}
    Ok(pair)
}
fn mark_under_lock(roots:&Roots,read_id:&impl Fn(&Path)->Result<String>,loaded:&SecretBox,locks:Option<&crate::upgrade::StorageLocks>)->Result<()> {
    let mismatch=||Error::refusal("pair_mismatch");
    let mut pair=agreement(roots,read_id,loaded,locks)?;
    let mut complete=pair.encryption_key_required;
    for root in [&roots.db,&roots.app] {
        let saved:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;
        complete &= saved==pair;
    }
    if complete{return Ok(());}
    pair.encryption_key_required=true;
    // A flag-only interruption is completed on the next boot with the same bound key.
    for root in [&roots.db,&roots.app] {
        files::atomic_json(&root.join(".nwui/pair-v1.json"),&pair).map_err(|_|mismatch())?;
    }
    for root in [&roots.db,&roots.app] {
        let saved:Pair=files::read_json(&root.join(".nwui/pair-v1.json")).map_err(|_|mismatch())?;
        if saved!=pair{return Err(mismatch());}
    }
    Ok(())
}
