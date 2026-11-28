//! Runner-owned lock policy; transport is exclusively the frozen A.20 s6 client.
use super::{StepError,StorageLocks,fdholder::{self,CustodyId,DaemonWatch}};
use crate::db_upgrade::layout::Roots;
use std::{collections::BTreeSet,fs::OpenOptions,os::{fd::{BorrowedFd,OwnedFd},unix::fs::OpenOptionsExt},path::Path,sync::Arc,time::Duration};
fn error(code:&'static str)->StepError {StepError {code,message:"저장소 잠금 보관을 확인하지 못했습니다. 기존 데이터와 잠금을 유지한 채 서비스 시작을 보류합니다.",detail:None}}
async fn blocking<T:Send+'static>(work:impl FnOnce()->Result<T,StepError>+Send+'static)->Result<T,StepError> {
    tokio::task::spawn_blocking(work).await.map_err(|_|error("io_error"))?
}
fn lock_root(root:&Path)->Result<OwnedFd,StepError> {
    let file=OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(root).map_err(|_|error("io_error"))?;
    file.try_lock().map_err(|_|error("storage_busy"))?;Ok(file.into())
}
fn publish_missing<'a>(locks:&'a StorageLocks,present:&BTreeSet<CustodyId>,retain_d:bool,mut store:impl FnMut(CustodyId,BorrowedFd<'a>)->Result<(),StepError>)->Result<(),StepError> {
    if retain_d && !locks.holds_d() {return Err(error("storage_busy"));}
    if !retain_d && present.contains(&CustodyId::D) {return Err(error("custody_protocol"));}
    for (id,fd) in [(CustodyId::A,Some(locks.a_fd())),(CustodyId::D,if retain_d{locks.d_fd()}else{None})] {
        if let Some(fd)=fd {if !present.contains(&id){store(id,fd)?;}}
    }
    Ok(())
}
pub(super) fn wait_and_publish(locks:&StorageLocks,retain_d:bool)->Result<(),StepError> {
    let mut last=error("custody_unavailable");
    for attempt in 0..10 {
        let result=(||{fdholder::wait_ready(Duration::from_millis(100))?;let present=fdholder::list()?;publish_missing(locks,&present,retain_d,fdholder::store)})();
        match result {Ok(())=>return Ok(()),Err(e)if matches!(e.code,"custody_unavailable"|"io_error")=>last=e,Err(e)=>return Err(e)}
        if attempt<9 {std::thread::sleep(Duration::from_millis(100));}
    }
    Err(last)
}
pub(crate) struct Custody {pub locks:Arc<StorageLocks>,watch:Option<DaemonWatch>,pub(super) d_deletion_attempted:bool}
impl Custody {
    /// need_d is false only for a same-boot completed handoff (A.19), never for a new boot.
    pub(crate) async fn acquire(roots:Roots,need_d:bool)->Result<Self,StepError> {
        let locks=blocking(move|| {
            crate::db_upgrade::layout::validate_roots_unlocked(&roots).map_err(|e|StepError{code:e.code,message:e.message,detail:e.detail.map(str::to_owned)})?;
            fdholder::wait_ready(Duration::from_secs(2))?;let present=fdholder::list()?;
            if !need_d && present.contains(&CustodyId::D) {return Err(error("custody_protocol"));}
            let a=if present.contains(&CustodyId::A){fdholder::retrieve(CustodyId::A)?}else{lock_root(&roots.app)?};
            let d=if need_d{Some(if present.contains(&CustodyId::D){fdholder::retrieve(CustodyId::D)?}else{lock_root(&roots.db)?})}else{None};
            let locks=StorageLocks::from_custody(&roots.db,&roots.app,a,d)?;
            wait_and_publish(&locks,need_d)?;Ok(locks)
        }).await?;
        Ok(Self {locks:Arc::new(locks),watch:Some(DaemonWatch::start()?),d_deletion_attempted:false})
    }
    pub(super) fn refusal_locks(&self,roots:&Roots,present:&BTreeSet<CustodyId>)->bool {
        !self.d_deletion_attempted && Arc::strong_count(&self.locks)==1
            && self.locks.holds_a() && self.locks.holds_d() && self.locks.matches_roots(&roots.db,&roots.app)
            && present.contains(&CustodyId::A) && present.contains(&CustodyId::D)
    }
    async fn repopulate(&mut self)->Result<(),StepError> {
        let locks=self.locks.clone();let retain_d=locks.holds_d()&&!self.d_deletion_attempted;blocking(move||wait_and_publish(&locks,retain_d)).await?;
        self.watch=Some(DaemonWatch::start()?);Ok(())
    }
    /// A custody fault never drops a running mutation future: finish/reap it under our own FDs.
    /// The caller keeps this holder alive while handling an error or parking a refused outcome.
    pub(crate) async fn during<T>(&mut self,work:impl std::future::Future<Output=Result<T,StepError>>)->Result<T,StepError> {
        tokio::pin!(work);let mut fault=None;
        loop {
            let down=async {match self.watch.as_mut(){Some(w)=>w.down().await,None=>{tokio::time::sleep(Duration::from_millis(100)).await;Err(error("custody_unavailable"))}}};
            tokio::select! {
                result=work.as_mut()=>return match result{Err(e)=>Err(e),Ok(value)=>match fault{Some(e)=>Err(e),None=>Ok(value)}},
                observed=down=>{
                    self.watch=None;
                    if let Err(e)=observed {eprintln!("[upgrade] lock daemon observation failed: {}",e.code);}
                    match self.repopulate().await {Ok(())=>fault=None,Err(e)=>{eprintln!("[upgrade] lock custody recovery pending: {}",e.code);fault=Some(e);}}
                }
            }
        }
    }
    /// Descriptor fsync and upgrade-status down must already be complete. Never LOCK_UN.
    pub(crate) async fn handoff(&mut self,j:&mut super::Journal,boot_identity:&str)->Result<(),StepError> {
        if Arc::strong_count(&self.locks)!=1 {return Err(error("storage_busy"));}
        j.handoff(super::journal::Handoff::Intent,boot_identity)?;
        self.d_deletion_attempted=true;
        blocking(||fdholder::delete(CustodyId::D)).await?;
        // Do not re-store D after its deletion, even though our own D copy is still held.
        let locks=self.locks.clone();blocking(move||wait_and_publish(&locks,false)).await?;
        Arc::get_mut(&mut self.locks).ok_or_else(||error("storage_busy"))?.release_d();
        j.handoff(super::journal::Handoff::Done,boot_identity)?;
        super::runner::notify_readiness()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File,os::unix::fs::PermissionsExt};
    fn fixture()->(tempfile::TempDir,Roots,StorageLocks) {
        let t=tempfile::tempdir().unwrap();let roots=Roots {db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app] {std::fs::create_dir(p).unwrap();std::fs::set_permissions(p,std::fs::Permissions::from_mode(0o700)).unwrap();}let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();(t,roots,locks)
    }
    #[test]
    fn refusal_requires_both_daemon_locks_and_no_d_deletion_attempt() {
        let (_t,roots,locks)=fixture();
        let mut custody=Custody{locks:Arc::new(locks),watch:None,d_deletion_attempted:false};
        let present=BTreeSet::from([CustodyId::A,CustodyId::D]);
        assert!(custody.refusal_locks(&roots,&present));
        assert!(!custody.refusal_locks(&roots,&BTreeSet::from([CustodyId::A])));
        let outstanding=custody.locks.clone();
        assert!(!custody.refusal_locks(&roots,&present));
        drop(outstanding);
        custody.d_deletion_attempted=true;
        assert!(!custody.refusal_locks(&roots,&present));
    }
    #[test]
    fn w1_custody_publication_copies_keep_a_across_delete_d_then_close_and_do_not_unlock() {
        let (_t,roots,mut locks)=fixture();let mut daemon=Vec::new();publish_missing(&locks,&BTreeSet::new(),true,|id,fd|{daemon.push((id,fd.try_clone_to_owned().unwrap()));Ok(())}).unwrap();assert_eq!(daemon.iter().map(|(id,_)|*id).collect::<Vec<_>>(),[CustodyId::A,CustodyId::D]);
        let a=File::open(&roots.app).unwrap();let d=File::open(&roots.db).unwrap();assert!(a.try_lock().is_err());assert!(d.try_lock().is_err());
        daemon.retain(|(id,_)|*id!=CustodyId::D);assert!(d.try_lock().is_err());locks.release_d();d.try_lock().unwrap();drop(locks);assert!(a.try_lock().is_err());drop(daemon);a.try_lock().unwrap();
    }
    #[test]
    fn w1_custody_phase2_and_handoff_never_republish_d_and_unknown_d_refuses_before_store() {
        let (_t,_roots,mut locks)=fixture();let mut published=Vec::new();publish_missing(&locks,&BTreeSet::new(),false,|id,_|{published.push(id);Ok(())}).unwrap();assert_eq!(published,[CustodyId::A]);
        locks.release_d();published.clear();publish_missing(&locks,&BTreeSet::from([CustodyId::A]),false,|id,_|{published.push(id);Ok(())}).unwrap();assert!(published.is_empty());
        assert_eq!(publish_missing(&locks,&BTreeSet::from([CustodyId::D]),false,|_,_|panic!("unexpected D must refuse first")).unwrap_err().code,"custody_protocol");
        assert_eq!(publish_missing(&locks,&BTreeSet::new(),true,|_,_|panic!("missing own D must refuse first")).unwrap_err().code,"storage_busy");
    }
}
