//! A runner shutdown requests cancellation but never drops an in-flight mutation future.
use super::StepError;
use std::sync::atomic::Ordering;
fn interrupted()->StepError {StepError{code:"interrupted",message:"시작 준비를 중단했습니다. 진행 중인 작업을 정리하고 복구 기록을 보존했습니다.",detail:None}}
pub(crate) struct Signals {watcher:tokio::task::JoinHandle<()>,observed:bool}
impl Signals {
    pub(crate) fn install()->Result<Self,StepError> {
        use tokio::signal::unix::{signal,SignalKind};
        let mut int=signal(SignalKind::interrupt()).map_err(|_|interrupted())?;
        let mut term=signal(SignalKind::terminate()).map_err(|_|interrupted())?;
        crate::db_upgrade::postgres::SIGNAL.store(0,Ordering::Relaxed);crate::db_upgrade::files::CANCELLED.store(false,Ordering::Relaxed);
        let watcher=tokio::spawn(async move {
            let number=tokio::select!{_=int.recv()=>2,_=term.recv()=>15};
            crate::db_upgrade::postgres::SIGNAL.store(number,Ordering::Relaxed);crate::db_upgrade::files::CANCELLED.store(true,Ordering::Relaxed);
        });
        Ok(Self{watcher,observed:false})
    }
    async fn wait(&mut self) {
        if !self.observed {
            if (&mut self.watcher).await.is_err(){crate::db_upgrade::postgres::SIGNAL.store(15,Ordering::Relaxed);crate::db_upgrade::files::CANCELLED.store(true,Ordering::Relaxed);}
            self.observed=true;
        }
    }
    /// On a signal the work is still awaited, so held FDs, blocking tasks and private PGs
    /// are not abandoned by a select branch. A real operation error keeps its own code.
    pub(crate) async fn drain<T>(&mut self,work:impl std::future::Future<Output=Result<T,StepError>>)->Result<T,StepError> {
        tokio::pin!(work);
        let result=tokio::select!{biased;_=self.wait()=>work.as_mut().await,result=work.as_mut()=>result};
        match result {Err(e)=>Err(e),Ok(value)=>if crate::db_upgrade::postgres::SIGNAL.load(Ordering::Relaxed)!=0{Err(interrupted())}else{Ok(value)}}
    }
}
impl Drop for Signals {fn drop(&mut self){self.watcher.abort();}}
/// Parked outcomes retain custody until container shutdown, without retrying admission.
pub(crate) async fn park()->Result<(),StepError> {
    while !crate::db_upgrade::files::CANCELLED.load(Ordering::Relaxed){tokio::time::sleep(std::time::Duration::from_millis(100)).await;}Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn w1_signal_real_term_drains_the_blocking_mutation_before_returning_and_preserves_real_errors() {
        const CHILD:&str="NWUI_RUNNER_SIGNAL_FIXTURE";
        let Ok(mode)=std::env::var(CHILD) else {
            for mode in ["normal","term","watcher-failed"] {let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","upgrade::signals::tests::w1_signal_real_term_drains_the_blocking_mutation_before_returning_and_preserves_real_errors","--nocapture"]).env(CHILD,mode).output().unwrap();assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));}return;
        };
        let mut signals=Signals::install().unwrap();
        if mode=="normal" {assert_eq!(signals.drain(async{Ok(17)}).await.unwrap(),17);return;}
        if mode=="watcher-failed" {signals.watcher.abort();signals.wait().await;assert!(crate::db_upgrade::files::CANCELLED.load(Ordering::Relaxed));assert_eq!(signals.drain(async{Ok(17)}).await.unwrap_err().code,"interrupted");return;}
        let t=tempfile::tempdir().unwrap();let root=t.path().to_owned();let proof=root.join("finished");let worker_proof=proof.clone();let guard=std::fs::File::open(&root).unwrap();guard.try_lock().unwrap();let independent=std::fs::File::open(&root).unwrap();let(started,ready)=tokio::sync::oneshot::channel();let(release,held)=std::sync::mpsc::channel();
        let sender=tokio::spawn(async move {ready.await.unwrap();assert_eq!(unsafe{libc::kill(libc::getpid(),libc::SIGTERM)},0);while !crate::db_upgrade::files::CANCELLED.load(Ordering::Relaxed){tokio::task::yield_now().await;}assert!(independent.try_lock().is_err());release.send(()).unwrap();});
        let work=async move {tokio::task::spawn_blocking(move|| {
            started.send(()).unwrap();while !crate::db_upgrade::files::CANCELLED.load(Ordering::Relaxed){std::thread::sleep(std::time::Duration::from_millis(1));}
            held.recv_timeout(std::time::Duration::from_secs(5)).unwrap();std::thread::sleep(std::time::Duration::from_millis(80));std::fs::write(worker_proof,b"drained").unwrap();drop(guard);Ok(())
        }).await.unwrap()};
        let start=std::time::Instant::now();assert_eq!(signals.drain(work).await.unwrap_err().code,"interrupted");assert!(start.elapsed()>=std::time::Duration::from_millis(80));assert_eq!(std::fs::read(proof).unwrap(),b"drained");sender.await.unwrap();
        let error=StepError{code:"pair_mismatch",message:"fixture",detail:None};assert_eq!(signals.drain::<()>(async{Err(error)}).await.unwrap_err().code,"pair_mismatch");park().await.unwrap();
    }
}
