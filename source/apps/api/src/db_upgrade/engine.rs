//! Fixed-root, read-only status and preflight. Conversion never takes paths from JSON metadata.
use super::{mounts,files,layout::{self,Roots,Decision},state::State,postgres,error::{Error,Result}};
use std::{fs,path::{Path,PathBuf}};
use std::os::unix::fs::MetadataExt;
#[derive(Debug)]
pub struct Outcome {pub state:State,pub notice:Option<&'static str>}
pub const COPY_PRESENT_NOTICE:&str="working_copy_present: PG15 working copy remains. 준비 완료된 사본은 run 재실행으로 정리할 수 있습니다.";
pub const COPY_REMOVED_NOTICE:&str="working_copy_removed: working copy removed. 저장된 이전 기준과 PG17로 검증합니다.";
pub const REMOVED_CHANGED_NOTICE:&str="working_copy_removed: working copy removed. changed_since_conversion: 이전 기준과 데이터가 달라졌습니다. 전환 후 데이터 변경은 정상일 수 있습니다.";
pub const BASELINE_NOTICE:&str="changed_since_conversion: 이전 기준과 데이터가 달라졌습니다. 전환 후 데이터 변경은 정상일 수 있습니다.";
pub struct Engine {pub source_db:PathBuf,pub source_app:PathBuf,pub roots:Roots}
fn unsupported()->Error{Error::refusal("unsupported_layout")}
pub fn architecture()->Result<&'static str>{match std::env::consts::ARCH{"aarch64"=>Ok("arm64"),"x86_64"=>Ok("amd64"),_=>Err(unsupported())}}
impl Engine {
    pub async fn execute(&self,args:&super::arguments::Arguments)->Result<Outcome> {
        use super::{arguments::Operation as Command,state::{Phase,Operation},migration};
        let state=self.status()?;
        if args.operation==Command::Status{let notice=self.copy_notice(&state,false)?;return Ok(Outcome{state,notice});}
        if state.ready {
            let _locks=migration::lock(&[&self.roots.db,&self.roots.app])?;
            let state=self.status()?;
            let changed=if args.operation==Command::Verify{self.verify_mounts()?;migration::diagnostic(self,&state).await?}else{migration::validate_arguments(self,&state,args)?;if args.operation==Command::Run{super::cleanup::cleanup_ready(self,&state)?;}false};
            let notice=self.copy_notice(&state,changed)?;
            return Ok(Outcome{state,notice});
        }
        if args.operation==Command::Verify {
            self.verify_mounts()?;
            let _locks=migration::lock(&[&self.roots.db,&self.roots.app])?;
            let state=self.status()?;
            migration::diagnostic(self,&state).await?;return Ok(Outcome{state,notice:None});
        }
        if state.operation!=Operation::Idle||!matches!(state.phase,Phase::NotStarted|Phase::Checked){return Err(Error::refusal("migration_incomplete"));}
        self.validate_mounts()?;
        let _locks=migration::lock(&[&self.source_db,&self.source_app,&self.roots.db,&self.roots.app])?;
        let state=self.status()?;
        if state.operation!=Operation::Idle||!matches!(state.phase,Phase::NotStarted|Phase::Checked){return Err(Error::refusal("migration_incomplete"));}
        let checked=if state.phase==Phase::NotStarted{migration::check_new(self,args).await?}else{migration::resume_checked(self,&state,args)?;state};
        let state=if args.operation==Command::Check{checked}else{migration::convert(self,args,checked).await?};
        if args.operation==Command::Run{super::cleanup::cleanup_ready(self,&state)?;}
        let notice=self.copy_notice(&state,false)?;
        Ok(Outcome{state,notice})
    }
    fn copy_notice(&self,state:&State,changed:bool)->Result<Option<&'static str>> {
        if !state.ready||state.mode!="migration"{return Ok(None);}
        let present=super::cleanup::working_copy_present(self,state)?;
        Ok(Some(match(present,changed){(true,true)=>BASELINE_NOTICE,(true,false)=>COPY_PRESENT_NOTICE,(false,true)=>REMOVED_CHANGED_NOTICE,(false,false)=>COPY_REMOVED_NOTICE}))
    }
    pub fn fixed()->Result<Self>{super::paths::validate_env()?;Ok(Self{source_db:"/source/db".into(),source_app:"/source/app".into(),roots:Roots{db:"/destination/db".into(),app:"/destination/app".into(),architecture:architecture()?.into()}})}
    pub fn status(&self)->Result<State> {
        for root in [&self.roots.db,&self.roots.app]{directory(root)?;}
        let control=self.roots.db.join(".nwui");
        match fs::symlink_metadata(&control) {
            Ok(_)=>directory(&control)?,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(_)=>return Err(unsupported()),
        }
        let path=control.join("migration-v1.json");
        let state:State=match files::read_json(&path) {
            Ok(state)=>state,
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(State::not_started(&self.roots.architecture)),
            Err(_)=>return Err(unsupported()),
        };
        state.validate().map_err(|_|unsupported())?;
        if state.target.architecture!=self.roots.architecture{return Err(unsupported());}
        if state.ready {
            if !matches!(layout::classify(&self.roots,&|data|Ok(postgres::read_control(17,data)?.system_identifier))?,Decision::Ready(_)){return Err(Error::refusal("pair_mismatch"));}
        }
        Ok(state)
    }
    pub fn validate_roots(&self)->Result<()> {
        let roots=[&self.source_db,&self.source_app,&self.roots.db,&self.roots.app];
        for (i,path) in roots.iter().enumerate() {
            directory(path)?;
            for other in &roots[..i] {
                let a=fs::metadata(path).map_err(|_|unsupported())?;let b=fs::metadata(other).map_err(|_|unsupported())?;
                if path.starts_with(other)||other.starts_with(path)||(a.dev()==b.dev()&&a.ino()==b.ino()){return Err(unsupported());}
            }
        }
        layout::validate_roots(&self.roots)?;
        mounts::Table::read()?.distinct(&[&self.source_db,&self.source_app,&self.roots.db,&self.roots.app])?;
        Ok(())
    }
    fn verify_mounts(&self)->Result<()> {
        layout::validate_roots(&self.roots)?;let table=mounts::Table::read()?;
        for path in [&self.roots.db,&self.roots.app]{table.require_mount(path,false)?;}Ok(())
    }
    pub fn validate_mounts(&self)->Result<()> {
        self.validate_roots()?;
        let table=mounts::Table::read()?;
        for (path,read_only) in [(&self.source_db,true),(&self.source_app,true),(&self.roots.db,false),(&self.roots.app,false)] {table.require_mount(path,read_only)?;}
        Ok(())
    }
}
fn directory(path:&Path)->Result<()> {files::real_path(path).map_err(|_|unsupported())?;if !fs::symlink_metadata(path).map_err(|_|unsupported())?.is_dir(){return Err(unsupported());}Ok(())}
pub fn required_bytes(db:u64,app:u64,largest_index:u64)->Result<u64> {
    // Two retained DB copies plus two source-sized WAL/rewrite allowances (including ANALYZE).
    // Largest-index rebuild/sort space and the existing reserve add >=20% over all three G6 peaks.
    db.checked_mul(4).and_then(|n|largest_index.checked_mul(2).and_then(|index|n.checked_add(index)))
        .and_then(|n|n.checked_add(std::cmp::max(1_073_741_824,db/5))).and_then(|n|n.checked_add(app)).ok_or_else(unsupported)
}
