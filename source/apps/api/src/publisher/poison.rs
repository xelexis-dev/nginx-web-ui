//! An unreaped child keeps every publisher mutation closed across a restart.
use super::store::PublisherStore;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File}, io};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
struct Poison { pid:u32, start_ticks:u64 }

pub(super) fn start_ticks(pid:u32) -> Result<Option<u64>> {
    ensure!(pid>1,"Invalid nginx child PID");
    let stat=match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat)=>stat,
        Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(error.into()),
    };
    let fields=stat.rsplit_once(") ").context("Invalid child stat")?.1;
    Ok(Some(fields.split_whitespace().nth(19).context("Missing child start ticks")?.parse()?))
}

pub(super) fn mark(store:&PublisherStore,pid:u32,start_ticks:u64) -> Result<()> {
    ensure!(pid>1 && start_ticks>0,"Invalid nginx child identity");
    crate::db_upgrade::files::atomic_json(&store.root.join("poison.json"),&Poison { pid,start_ticks })?;
    Ok(())
}

pub(super) fn check(store:&PublisherStore) -> Result<()> {
    let path=store.root.join("poison.json");
    match fs::symlink_metadata(&path) {
        Ok(_)=>{},
        Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
        Err(error)=>return Err(error.into()),
    }
    let poison:Poison=crate::db_upgrade::files::read_json(&path)?;
    ensure!(start_ticks(poison.pid)?!=Some(poison.start_ticks),"Publisher child remains unreaped");
    fs::remove_file(path)?;
    File::open(&store.root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n1_poison_blocks_a_live_identity_but_never_blocks_a_reused_pid() {
        let temp=tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o700)).unwrap();
        let store=PublisherStore::at(temp.path().into());
        let pid=std::process::id();
        let ticks=start_ticks(pid).unwrap().unwrap();
        mark(&store,pid,ticks).unwrap();
        assert!(check(&store).is_err());
        assert!(store.root.join("poison.json").exists());
        mark(&store,pid,ticks+1).unwrap();
        check(&store).unwrap();
        assert!(!store.root.join("poison.json").exists());
    }
}
