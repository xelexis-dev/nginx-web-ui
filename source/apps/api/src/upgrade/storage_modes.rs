//! 2.0.1: narrow the storage modes the upgrade gates refuse, under both storage locks and before P0,
//! so P1 manifests record the narrowed modes. Mode bits only: never content, owner, times, links or
//! another mount. Gates stay strict; this only removes what they would refuse (final.md F1–F12, R1).
use super::{StepError,StorageLocks,types::StorageWarning};
use crate::db_upgrade::{layout::Roots,shared_folder};
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,ffi::{CString,OsStr},fs::{self,File,OpenOptions},io::{self,Read,Write},os::{fd::{AsRawFd,BorrowedFd,FromRawFd,RawFd},unix::{ffi::OsStrExt,fs::{MetadataExt,OpenOptionsExt,PermissionsExt}}},path::Path};

pub(crate) const REPORT_PATH:&str="/run/nwui-storage-modes.json";
pub(crate) const WARNING_CODE:&str="storage_modes_normalized";
/// R4-2: a Phase1 refusal's detail for the journal-based status projection, bound to this boot.
pub(crate) const REFUSAL_PATH:&str="/run/nwui-storage-modes-refusal.json";
const MESSAGE:&str="저장소 권한을 자동으로 정리하지 못했습니다. 파일 내용과 소유자는 변경하지 않았습니다.";
const MAX_ENTRIES:u64=1_000_000;
const MAX_DEPTH:u32=64;
const MAX_MESSAGE:usize=2048;
const FIXED_LINES:usize=12;
const DEEP_PAIRS:usize=8;
const LOG_PAIRS:usize=64;
const REPORT_MAX:u64=65536;

fn refused(detail:String)->String {let detail=detail.chars().filter(|c|!c.is_control()).take(2048).collect::<String>();eprintln!("[storage] refused: {detail}");detail}
pub(crate) type Chmod<'a>=&'a dyn Fn(RawFd,u32)->io::Result<()>;
/// Test seams for permission changes and cancellation.
pub(crate) struct Seams<'a> {pub chmod:Chmod<'a>,pub cancelled:&'a dyn Fn()->bool,pub readonly:Option<bool>}

#[derive(Clone,Copy,PartialEq,Eq,Debug,Serialize,Deserialize,PartialOrd,Ord)]
#[serde(rename_all="lowercase")]
enum Scope {App,Db}
impl Scope {fn label(self)->&'static str{match self{Scope::App=>"app",Scope::Db=>"db"}}}
#[derive(Clone,Copy,PartialEq,Eq)]
enum Kind {Dir,File}
#[derive(Clone,Copy)]
enum Owner {Root,RootOrPostgres}
#[derive(Clone,Copy)]
enum Rule {Strip,DataDir,Exact(&'static [u32],u32)}
impl Rule {
    /// None: the gate already accepts this mode, so the entry is never touched.
    fn target(self,mode:u32)->Option<u32> {
        match self {
            Rule::Strip=>(mode&0o022!=0).then_some(mode&!0o022),
            // PostgreSQL refuses a data directory with S_IWGRP|S_IRWXO; 0755 would fail after P1.
            Rule::DataDir=>(mode&0o022!=0).then_some(mode&!0o027),
            Rule::Exact(accepted,target)=>(!accepted.contains(&mode)).then_some(target),
        }
    }
}

#[derive(Serialize,Deserialize,Debug,Clone,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
struct FixedChange {scope:Scope,path:String,old:u32,new:u32,exact:bool}
#[derive(Serialize,Deserialize,Debug,Clone,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
struct DeepChange {path:String,old:u32,new:u32,count:u64}
#[derive(Serialize,Deserialize,Debug,Clone,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub(crate) struct Report {
    version:u8,
    boot_identity:Option<String>,
    fixed:Vec<FixedChange>,
    deep:Vec<DeepChange>,
    deep_other:u64,
    deep_other_pairs:u64,
}
impl Report {
    fn empty()->Self {Self{version:1,boot_identity:None,fixed:vec![],deep:vec![],deep_other:0,deep_other_pairs:0}}
    pub(crate) fn changed(&self)->u64 {self.fixed.len()as u64+self.deep.iter().map(|d|d.count).sum::<u64>()+self.deep_other}
}

struct Root {file:File,dev:u64,mnt:u64,scope:Scope,readonly:bool}
#[derive(Clone,Copy)]
struct Layout {legacy_modes:bool,canonical_key:bool}

fn open_child(parent:&File,name:&OsStr)->io::Result<Option<File>> {
    let name=CString::new(name.as_bytes()).map_err(|_|io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),libc::O_PATH|libc::O_NOFOLLOW|libc::O_CLOEXEC)};
    if fd<0 {let e=io::Error::last_os_error();return if e.kind()==io::ErrorKind::NotFound{Ok(None)}else{Err(e)};}
    Ok(Some(unsafe{File::from_raw_fd(fd)}))
}
fn real_dir(file:&File)->bool {file.metadata().is_ok_and(|m|m.is_dir()&&!m.file_type().is_symlink())}
fn present(parent:&File,name:&str)->bool {matches!(open_child(parent,OsStr::new(name)),Ok(Some(_)))}
fn present_in(parent:&File,directory:&str,name:&str)->bool {
    matches!(open_child(parent,OsStr::new(directory)),Ok(Some(d))if real_dir(&d)&&present(&d,name))
}
fn names(directory:&File)->io::Result<Vec<std::ffi::OsString>> {
    // The magic link resolves to the held directory inode, never to a path a peer can swap.
    let mut names=fs::read_dir(format!("/proc/self/fd/{}",directory.as_raw_fd()))?.map(|e|e.map(|e|e.file_name())).collect::<io::Result<Vec<_>>>()?;
    names.sort();Ok(names)
}
/// Same technique as glibc fchmodat(AT_SYMLINK_NOFOLLOW): the O_PATH descriptor names the inode.
pub(crate) fn proc_chmod(fd:RawFd,mode:u32)->io::Result<()> {
    let path=CString::new(format!("/proc/self/fd/{fd}")).map_err(|_|io::Error::from(io::ErrorKind::InvalidInput))?;
    if unsafe{libc::chmod(path.as_ptr(),mode as libc::mode_t)}!=0 {return Err(io::Error::last_os_error());}
    Ok(())
}
fn reopen(parent:&File,name:&OsStr,dir:bool,expected:&fs::Metadata)->io::Result<File> {
    let name=CString::new(name.as_bytes()).map_err(|_|io::Error::from(io::ErrorKind::InvalidInput))?;
    let flags=libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC|if dir{libc::O_DIRECTORY}else{0};
    let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),flags)};
    if fd<0 {return Err(io::Error::last_os_error());}
    let file=unsafe{File::from_raw_fd(fd)};let m=file.metadata()?;
    if m.dev()!=expected.dev() || m.ino()!=expected.ino() {return Err(io::Error::other("entry replaced"));}
    Ok(file)
}
fn root(fd:BorrowedFd<'_>,scope:Scope)->io::Result<Root> {
    // A duplicate shares the held lock description; closing it never unlocks (no LOCK_UN here).
    let file=File::from(fd.try_clone_to_owned()?);let m=file.metadata()?;let mnt=shared_folder::mnt_id(&file)?;
    let mut s=std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe{libc::fstatvfs(file.as_raw_fd(),s.as_mut_ptr())}!=0 {return Err(io::Error::last_os_error());}
    let readonly=unsafe{s.assume_init()}.f_flag&libc::ST_RDONLY!=0;
    Ok(Root{file,dev:m.dev(),mnt,scope,readonly})
}
fn layout(db:&File,roots:&Roots)->Layout {
    let canonical=crate::db_upgrade::paths::startup_data_key_file(&roots.app).is_ok_and(|p|p==roots.app.join("secrets/data_key"));
    // layout.rs:56 accepts a legacy 0644 secret on the 1.19 path and on migration pairs (their state file).
    Layout{legacy_modes:present_in(db,".nwui","migration-v1.json"),canonical_key:canonical}
}

struct Run<'a> {
    roots:&'a Roots,layout:Layout,pg:Option<(u32,u32)>,chmod:Chmod<'a>,cancelled:&'a dyn Fn()->bool,
    apply:bool,seen:u64,planned:u64,applied:u64,fixed:Vec<FixedChange>,deep:BTreeMap<(u32,u32),u64>,
}
impl Run<'_> {
    fn host(&self,scope:Scope,path:&Path)->String {
        let (root,label)=match scope{Scope::App=>(&self.roots.app,"<앱 데이터 호스트 폴더>"),Scope::Db=>(&self.roots.db,"<DB 호스트 폴더>")};
        match path.strip_prefix(root){Ok(r)if r.as_os_str().is_empty()=>label.into(),Ok(r)=>format!("{label}/{}",r.display()),Err(_)=>path.display().to_string()}
    }
    fn refusal(&self,text:String)->StepError {
        let mut detail=text;
        if self.applied>0 {detail.push_str(&format!(" 이미 정리한 {}개 항목은 권한을 줄인 상태로 남으며 1.19.11에서도 그대로 사용할 수 있습니다.",self.applied));}
        let detail=detail.chars().filter(|c|!c.is_control()).take(2048).collect::<String>();
        eprintln!("[storage] refused: {detail}");
        StepError{code:"storage_permissions",message:MESSAGE,detail:Some(detail)}
    }
    fn unreadable(&self,path:&Path,error:&io::Error)->StepError {
        self.refusal(format!("{}: 저장소 항목을 확인하지 못했습니다({error}). 마운트와 폴더 상태를 확인한 뒤 다시 시작하세요.",path.display()))
    }
    fn chmod_hint(&self,scope:Scope,path:&Path,new:u32)->String {format!("컨테이너를 멈추고 호스트에서 sudo chmod {new:o} '{}'를 실행한 뒤 다시 시작하세요.",self.host(scope,path))}
    fn owner_ok(&self,owner:Owner,uid:u32)->bool {uid==0 || matches!(owner,Owner::RootOrPostgres)&&self.pg.is_some_and(|(pg,_)|pg==uid)}
    /// F5: a candidate that needs a change must be safe to change, otherwise refuse before any change.
    fn owner_refusal(&self,root:&Root,meta:&fs::Metadata,path:&Path,owner:Owner)->StepError {
        let (need,own)=match (owner,self.pg) {(Owner::RootOrPostgres,Some((uid,gid)))=>(format!("root 또는 postgres(UID {uid})"),format!("{uid}:{gid}")),_=>("root".to_owned(),"root:root".to_owned())};
        self.refusal(format!("{}: 소유자 UID {}는 허용되지 않습니다(필요: {need}). 컨테이너를 멈추고 호스트에서 sudo chown {own} '{}'를 실행한 뒤 다시 시작하세요.",path.display(),meta.uid(),self.host(root.scope,path)))
    }
    /// R4-6: stop between entries; a rerun recomputes every predicate and finishes the job.
    fn interrupted(&self)->Result<(),StepError> {
        if (self.cancelled)() {return Err(StepError{code:"interrupted",message:"저장소 권한 정리를 중단했습니다. 다시 시작하면 이어서 정리합니다.",detail:None});}
        Ok(())
    }
    fn candidate(&self,root:&Root,meta:&fs::Metadata,path:&Path,old:u32,new:u32,owner:Owner,same_mount:bool)->Result<(),StepError> {
        let shown=path.display();
        if !self.owner_ok(owner,meta.uid()) {return Err(self.owner_refusal(root,meta,path,owner));}
        if meta.is_file() && meta.nlink()!=1 {return Err(self.refusal(format!("{shown}: 다른 이름과 하드 링크로 연결된 파일이라 권한(현재 {old:04o})을 바꾸면 다른 위치의 파일도 바뀝니다. 원본 설치에서 이 파일을 확인하세요.")));}
        if !same_mount {return Err(self.refusal(format!("{shown}: 다른 마운트에 있어 권한(현재 {old:04o})을 자동으로 바꾸지 않습니다. {}",self.chmod_hint(root.scope,path,new))));}
        if new&!old!=0 {return Err(self.refusal(format!("{shown}: 권한 {old:04o}은 허용되지 않고, 필요한 {new:04o}로 바꾸려면 권한을 넓혀야 해 자동으로 바꾸지 않습니다. {}",self.chmod_hint(root.scope,path,new))));}
        if root.readonly {return Err(self.refusal(format!("{shown}: 읽기 전용으로 연결돼 권한(현재 {old:04o}, 필요 {new:04o})을 바꿀 수 없습니다. {}",self.chmod_hint(root.scope,path,new))));}
        Ok(())
    }
    fn change(&mut self,root:&Root,file:&File,parent:Option<(&File,&OsStr)>,path:&Path,dir:bool,old:u32,new:u32)->Result<(),StepError> {
        let shown=path.display();
        if let Err(e)=(self.chmod)(file.as_raw_fd(),new) {
            let reason=match e.raw_os_error(){Some(libc::EROFS)=>"읽기 전용으로 연결돼",Some(libc::EPERM|libc::EACCES)=>"저장소가 권한 변경을 허용하지 않아",_=>"권한을 바꾸는 중 오류가 나서"};
            return Err(self.refusal(format!("{shown}: {reason} 권한(현재 {old:04o}, 필요 {new:04o})을 바꾸지 못했습니다({e}). {}",self.chmod_hint(root.scope,path,new))));
        }
        let meta=file.metadata().map_err(|e|self.unreadable(path,&e))?;
        if meta.mode()&0o7777!=new {
            // The measured mode decides; the share kind only picks the wording (F12).
            let text=if shared_folder::kind(file).is_some() {
                format!("{shown}: 권한 변경({old:04o} → {new:04o})이 반영되지 않았습니다. Mac 공유 폴더는 컨테이너 안의 권한 변경을 반영하지 않을 수 있습니다. Mac에서 chmod {new:o} '{}'를 실행하거나 Docker 관리 볼륨을 사용한 뒤 다시 시작하세요.",self.host(root.scope,path))
            }else{format!("{shown}: 권한 변경({old:04o} → {new:04o})이 저장소에 반영되지 않았습니다. {}",self.chmod_hint(root.scope,path,new))};
            return Err(self.refusal(text));
        }
        // R4-5: the inode has changed; count it before the fsync so a failed sync still reports it.
        self.applied+=1;
        // R1-2: P1 manifests record this mode, so it must be durable before P0b/P1.
        let synced=match parent {None=>file.sync_all(),Some((parent,name))=>reopen(parent,name,dir,&meta).and_then(|f|f.sync_all())};
        synced.map_err(|e|self.refusal(format!("{shown}: 바꾼 권한을 디스크에 기록하지 못했습니다({e}). 저장소 상태를 확인한 뒤 다시 시작하세요.")))?;
        Ok(())
    }
    fn fixed(&mut self,root:&Root,parent:Option<(&File,&OsStr)>,file:&File,path:&Path,kind:Kind,owner:Owner,rule:Rule)->Result<(),StepError> {
        self.interrupted()?;
        let meta=file.metadata().map_err(|e|self.unreadable(path,&e))?;let ft=meta.file_type();
        // R4-3: every gate refuses another type too; refuse here first, before any change, with the path.
        if ft.is_symlink() || !(kind==Kind::Dir&&ft.is_dir() || kind==Kind::File&&ft.is_file()) {
            return Err(self.refusal(format!("{}: {} 아니라 사용할 수 없습니다(심볼릭 링크·특수 파일 또는 다른 형식). 원본 설치에서 이 항목을 확인하세요.",path.display(),if kind==Kind::Dir{"폴더가"}else{"일반 파일이"})));
        }
        // R4-1: the gates check these owners whether or not a mode changes; name the fix here.
        if !self.owner_ok(owner,meta.uid()) {return Err(self.owner_refusal(root,&meta,path,owner));}
        let old=meta.mode()&0o7777;let Some(new)=rule.target(old) else {return Ok(())};
        let same_mount=parent.is_none() || meta.dev()==root.dev && shared_folder::mnt_id(file).map_err(|e|self.unreadable(path,&e))?==root.mnt;
        self.candidate(root,&meta,path,old,new,owner,same_mount)?;
        if !self.apply {self.planned+=1;return Ok(());}
        self.change(root,file,parent,path,ft.is_dir(),old,new)?;
        eprintln!("[storage] mode {} {} {old:04o}->{new:04o}",root.scope.label(),path.display());
        self.fixed.push(FixedChange{scope:root.scope,path:path.display().to_string(),old,new,exact:matches!(rule,Rule::Exact(..))});Ok(())
    }
    /// Returns the child only when it is a real directory, so callers never descend through a link.
    fn child(&mut self,root:&Root,parent:&File,parent_path:&Path,name:&str,kind:Kind,owner:Owner,rule:Rule)->Result<Option<File>,StepError> {
        let path=parent_path.join(name);
        let Some(file)=open_child(parent,OsStr::new(name)).map_err(|e|self.unreadable(&path,&e))? else {return Ok(None)};
        self.fixed(root,Some((parent,OsStr::new(name))),&file,&path,kind,owner,rule)?;
        Ok(real_dir(&file).then_some(file))
    }
    fn pass(&mut self,app:&Root,db:&Root)->Result<(),StepError> {
        self.seen=0;self.deep.clear();
        let roots=self.roots;let secrets=roots.app.join("secrets");let tls=roots.app.join("tls");
        let private=Rule::Exact(&[0o700],0o700);let key=Rule::Exact(&[0o600],0o600);
        let accepted:&'static [u32]=if self.layout.legacy_modes{&[0o600,0o644]}else{&[0o600]};
        // App side in every branch (F4 A1–A7).
        self.fixed(app,None,&app.file,&roots.app,Kind::Dir,Owner::Root,Rule::Strip)?;
        if let Some(directory)=self.child(app,&app.file,&roots.app,"secrets",Kind::Dir,Owner::Root,private)? {
            for name in ["auth_secret","db_password","admin_password"] {self.child(app,&directory,&secrets,name,Kind::File,Owner::Root,Rule::Exact(accepted,0o600))?;}
            if self.layout.canonical_key {self.child(app,&directory,&secrets,"data_key",Kind::File,Owner::Root,key)?;}
        }
        if let Some(directory)=self.child(app,&app.file,&roots.app,"tls",Kind::Dir,Owner::Root,private)? {
            self.child(app,&directory,&tls,"default.key",Kind::File,Owner::Root,key)?;
            self.child(app,&directory,&tls,"default.crt",Kind::File,Owner::Root,Rule::Exact(&[0o644],0o644))?;
        }
        self.fixed(db,None,&db.file,&roots.db,Kind::Dir,Owner::RootOrPostgres,Rule::Strip)
    }

    fn report(&self,l_path:&Path)->Report {
        let mut pairs=self.deep.iter().map(|(&(old,new),&count)|(old,new,count)).collect::<Vec<_>>();
        pairs.sort_by(|a,b|b.2.cmp(&a.2).then((a.0,a.1).cmp(&(b.0,b.1))));
        let rest=pairs.iter().skip(LOG_PAIRS);
        Report{version:1,boot_identity:None,fixed:self.fixed.clone(),
            deep:pairs.iter().take(LOG_PAIRS).map(|&(old,new,count)|DeepChange{path:l_path.display().to_string(),old,new,count}).collect(),
            deep_other:rest.clone().map(|p|p.2).sum(),deep_other_pairs:rest.count()as u64}
    }
}

/// Phase-1 boot only, under A+D custody, before journal-driven work, PG17 classification and ready classify.
pub(crate) fn normalize(roots:&Roots,locks:&StorageLocks,journal_present:bool)->Result<Report,StepError> {
    super::legacy::admit(roots)?;
    let cancelled=||crate::db_upgrade::files::CANCELLED.load(std::sync::atomic::Ordering::Relaxed);
    let result=normalize_seams(roots,locks,journal_present,&Seams{chmod:&proc_chmod,cancelled:&cancelled,readonly:None});
    let mut report=match result {
        Ok(report)=>report,
        Err(error)=>{
            if let (Some(detail),Ok(identity))=(error.detail.as_ref().filter(|_|error.code=="storage_permissions"),super::boot_identity::current()) {
                let record=serde_json::json!({"version":1,"bootIdentity":identity,"detail":detail});
                if let Err(e)=write_json(Path::new(REFUSAL_PATH),&record) {eprintln!("[storage] refusal record not written: {e}");}
            }
            return Err(error);
        }
    };
    if report.changed()>0 {
        for d in &report.deep {eprintln!("[storage] mode db {} entries={} {:04o}->{:04o}",d.path,d.count,d.old,d.new);}
        if report.deep_other>0 {eprintln!("[storage] mode db {} other entries={} in {} more mode pairs",roots.db.display(),report.deep_other,report.deep_other_pairs);}
        report.boot_identity=super::boot_identity::current().ok();
        if let Err(e)=write_report(Path::new(REPORT_PATH),&report) {eprintln!("[storage] mode report not written: {e}");}
    }
    Ok(report)
}
pub(crate) fn normalize_seams(roots:&Roots,locks:&StorageLocks,_journal_present:bool,seams:&Seams<'_>)->Result<Report,StepError> {
    let chmod=seams.chmod;
    let busy=||StepError{code:"storage_busy",message:"저장소 잠금이 유지되지 않았습니다.",detail:None};
    if !locks.holds_d() || !locks.matches_roots(&roots.db,&roots.app) {return Err(busy());}
    let d=locks.d_fd().ok_or_else(busy)?;
    let opened=|fd,scope,path:&Path|root(fd,scope).map_err(|e|StepError{code:"storage_permissions",message:MESSAGE,detail:Some(format!("{}: 저장소 루트를 확인하지 못했습니다({e}).",path.display()))});
    let mut app=opened(locks.a_fd(),Scope::App,&roots.app)?;let mut db=opened(d,Scope::Db,&roots.db)?;
    if let Some(readonly)=seams.readonly {app.readonly=readonly;db.readonly=readonly;}
    // R4-1: every Phase1 boot writes the journal, status and PostgreSQL files; refuse a read-only root first.
    for (root,path) in [(&app,&roots.app),(&db,&roots.db)] {
        if root.readonly {return Err(StepError{code:"storage_permissions",message:MESSAGE,detail:Some(refused(format!("{}: 읽기 전용으로 연결돼 있습니다. 시작할 때 이 저장소에 기록해야 하므로 쓰기 가능하게 연결한 뒤 다시 시작하세요.",path.display())))});}
    }
    let layout=layout(&db.file,roots);
    let mut run=Run{roots,layout,pg:crate::db_upgrade::postgres::postgres_ids().ok(),chmod,cancelled:seams.cancelled,apply:false,seen:0,planned:0,applied:0,fixed:vec![],deep:BTreeMap::new()};
    run.pass(&app,&db)?;
    if run.planned==0 {return Ok(Report::empty());}
    let l_path=roots.db.clone();
    run.apply=true;run.pass(&app,&db)?;
    Ok(run.report(&l_path))
}

pub(crate) fn write_report(path:&Path,report:&Report)->io::Result<()> {write_json(path,report)}
fn write_json<T:Serialize>(path:&Path,value:&T)->io::Result<()> {
    let parent=path.parent().ok_or_else(||io::Error::from(io::ErrorKind::InvalidInput))?;
    let bytes=serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len()as u64>REPORT_MAX {return Err(io::Error::other("oversized report"));}
    let temporary=parent.join(format!(".nwui-storage-modes-{}.tmp",uuid::Uuid::new_v4()));
    let result=(||{
        let mut file=OpenOptions::new().write(true).create_new(true).mode(0o644).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;file.write_all(&bytes)?;file.sync_all()?;
        fs::rename(&temporary,path)?;File::open(parent)?.sync_all()
    })();
    if result.is_err() {let _=fs::remove_file(&temporary);}
    result
}
fn read_report(path:&Path)->io::Result<Report> {
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path)?;let m=file.metadata()?;
    if !m.is_file() || m.uid()!=unsafe{libc::geteuid()} || m.mode()&0o022!=0 || m.nlink()!=1 || m.len()>REPORT_MAX {return Err(io::Error::other("unsafe report"));}
    let mut bytes=Vec::new();file.take(REPORT_MAX+1).read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn refusal_path()->std::path::PathBuf {
    #[cfg(test)] if let Some(p)=REFUSAL_FIXTURE.with(|p|p.borrow().clone()) {return p;}
    std::path::PathBuf::from(REFUSAL_PATH)
}
#[cfg(test)]
thread_local!{pub(crate) static REFUSAL_FIXTURE:std::cell::RefCell<Option<std::path::PathBuf>>=const{std::cell::RefCell::new(None)};}
/// R4-2: the journal records only the code; this boot's normalizer detail becomes nextAction.
pub(crate) fn refusal_detail(code:&str)->Option<String> {
    if code!="storage_permissions" {return None;}
    let identity=super::boot_identity::current().ok()?;
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(refusal_path()).ok()?;let m=file.metadata().ok()?;
    if !m.is_file() || m.uid()!=unsafe{libc::geteuid()} || m.mode()&0o022!=0 || m.nlink()!=1 || m.len()>REPORT_MAX {return None;}
    let mut bytes=Vec::new();file.take(REPORT_MAX+1).read_to_end(&mut bytes).ok()?;let value:serde_json::Value=serde_json::from_slice(&bytes).ok()?;
    if value.get("bootIdentity").and_then(|v|v.as_str())!=Some(identity.as_str()) {return None;}
    value.get("detail").and_then(|v|v.as_str()).map(|d|d.chars().filter(|c|!c.is_control()).take(2048).collect())
}
/// R1-4: only this container start's report is shown; a missing identity shows nothing.
pub(crate) fn warning(path:&Path,identity:Option<&str>)->Option<StorageWarning> {
    let identity=identity?;let report=read_report(path).ok()?;
    if report.version!=1 || report.boot_identity.as_deref()!=Some(identity) || report.changed()==0 {return None;}
    let mut scopes=report.fixed.iter().map(|f|f.scope).collect::<Vec<_>>();
    if !report.deep.is_empty() || report.deep_other>0 {scopes.push(Scope::Db);}
    scopes.sort();scopes.dedup();
    Some(StorageWarning{code:WARNING_CODE.into(),message:message(&report),scopes:scopes.into_iter().map(|s|s.label().to_owned()).collect()})
}
/// R1-3: at most 2048 bytes; at most 12 fixed lines and 8 deep pairs, then 「그 밖에 N개 항목」.
fn message(report:&Report)->String {
    let items=report.fixed.iter().take(FIXED_LINES).map(|f|(format!("{} {:04o} → {:04o}",f.path,f.old,f.new),1))
        .chain(report.deep.iter().take(DEEP_PAIRS).map(|d|(format!("{} 아래 {}개 {:04o} → {:04o}",d.path,d.count,d.old,d.new),d.count)));
    let reserve=", 그 밖에 18446744073709551615개 항목.".len();
    let mut out=String::from("시작 전에 저장소 접근 권한을 줄였습니다. 파일 내용과 소유자는 그대로입니다. 바뀐 항목: ");let mut listed=0u64;let mut first=true;
    for (text,count) in items {
        let separator=if first{""}else{", "};
        if out.len()+separator.len()+text.len()+reserve>MAX_MESSAGE {continue;}
        out.push_str(separator);out.push_str(&text);listed+=count;first=false;
    }
    let other=report.changed().saturating_sub(listed);
    if other>0 {out.push_str(&format!("{}그 밖에 {other}개 항목",if first{""}else{", "}));}
    out.push('.');out
}

#[cfg(test)] mod restored_tests {use super::*;use std::{collections::BTreeMap,os::unix::fs::{symlink,chown},path::PathBuf};struct Fixture{_t:tempfile::TempDir,roots:Roots,locks:StorageLocks,outside:PathBuf}
    /// The 1.19.11 legacy root layout from the field record, before any host mode damage.
fn fixture(_nested:bool)->Fixture {
 let t=tempfile::tempdir().unwrap();set(t.path(),0o711);let roots=Roots{db:t.path().join("db"),app:t.path().join("app"),architecture:"arm64".into()};for p in [&roots.db,&roots.app]{dir(p,0o700);}for p in [roots.app.join("secrets"),roots.app.join("tls")]{dir(&p,0o700);}
 write(&roots.app.join("secrets/auth_secret"),b"fixture-auth",0o600);write(&roots.app.join("secrets/db_password"),b"nginx",0o600);
 let key=rcgen::KeyPair::generate().unwrap();let cert=rcgen::CertificateParams::new(vec!["unconfigured.invalid".into()]).unwrap().self_signed(&key).unwrap();write(&roots.app.join("tls/default.crt"),cert.pem().as_bytes(),0o644);write(&roots.app.join("tls/default.key"),key.serialize_pem().as_bytes(),0o600);
 let outside=t.path().join("outside");dir(&outside,0o755);let locks=StorageLocks::acquire(&roots.db,&roots.app).unwrap();Fixture{_t:t,roots,locks,outside}
}
    fn set(path:&Path,mode:u32) {fs::set_permissions(path,fs::Permissions::from_mode(mode)).unwrap();}
    fn mode(path:&Path)->u32 {fs::symlink_metadata(path).unwrap().mode()&0o7777}
    fn write(path:&Path,bytes:&[u8],m:u32) {fs::write(path,bytes).unwrap();set(path,m);}
    fn dir(path:&Path,m:u32) {fs::create_dir(path).unwrap();set(path,m);}
    /// The field modes: app root 0775, auth_secret 0674, PG dirs 0770 and files 0670 (root kept).
fn damage(f:&Fixture,_nested:bool){set(&f.roots.app,0o775);set(&f.roots.app.join("secrets/auth_secret"),0o674);}
    fn contents(root:&Path)->BTreeMap<PathBuf,Vec<u8>> {
        fn walk(p:&Path,out:&mut BTreeMap<PathBuf,Vec<u8>>){let m=fs::symlink_metadata(p).unwrap();if m.is_file(){out.insert(p.into(),fs::read(p).unwrap());}else if m.is_dir(){for e in fs::read_dir(p).unwrap(){walk(&e.unwrap().path(),out);}}}
        let mut out=BTreeMap::new();walk(root,&mut out);out
    }
    /// Mode map that tolerates symlinks (files::snapshot refuses them).
    fn modes(root:&Path)->BTreeMap<PathBuf,u32> {
        fn walk(p:&Path,out:&mut BTreeMap<PathBuf,u32>){let m=fs::symlink_metadata(p).unwrap();out.insert(p.into(),m.mode());if m.is_dir(){for e in fs::read_dir(p).unwrap(){walk(&e.unwrap().path(),out);}}}
        let mut out=BTreeMap::new();walk(root,&mut out);out
    }
    fn run(f:&Fixture,journal:bool)->Result<Report,StepError> {normalize_seams(&f.roots,&f.locks,journal,&Seams{chmod:&proc_chmod,cancelled:&||false,readonly:None})}
    #[test]
    fn r4_1_owner_mismatch_refuses_with_a_command_even_without_a_mode_change() {
        let f=fixture(false);chown(&f.roots.app,Some(1000),None).unwrap();
        let detail=run(&f,false).unwrap_err().detail.unwrap();assert!(detail.contains("소유자 UID 1000")&&detail.contains("sudo chown root:root '<앱 데이터 호스트 폴더>'")&&!detail.contains('`'),"{detail}");
        chown(&f.roots.app,Some(0),None).unwrap();chown(f.roots.app.join("secrets"),Some(1000),None).unwrap();
        let detail=run(&f,false).unwrap_err().detail.unwrap();assert!(detail.contains("secrets")&&detail.contains("sudo chown root:root '<앱 데이터 호스트 폴더>/secrets'"),"{detail}");
    }
    #[test]
    fn r4_1_p0_keeps_the_root_detail_and_a_read_only_root_refuses_first() {
        let f=fixture(false);set(&f.roots.app,0o775);
        let snapshot=crate::db_upgrade::files::snapshot(f._t.path()).unwrap();
        let error=normalize_seams(&f.roots,&f.locks,false,&Seams{chmod:&proc_chmod,cancelled:&||false,readonly:Some(true)}).unwrap_err();
        assert_eq!(error.code,"storage_permissions");
        assert_eq!(error.message,"저장소 권한을 자동으로 정리하지 못했습니다. 파일 내용과 소유자는 변경하지 않았습니다.");
        let detail=error.detail.unwrap();
        assert!(detail.contains("읽기 전용으로 연결돼")&&detail.contains(&f.roots.app.display().to_string()),"{detail}");
        assert_eq!(crate::db_upgrade::files::snapshot(f._t.path()).unwrap(),snapshot);
    }
    #[test]
    fn r4_2_a_journal_refusal_projects_this_boots_detail_as_next_action() {
        use super::super::{Branch,Journal,Outcome,Step,journal::Candidate};
        let f=fixture(false);let candidate=Candidate{revision:"0".repeat(40),pins_sha256:"1".repeat(64),migrations_sha256:"2".repeat(64)};
        let mut j=Journal::begin(&f.roots.app,&f.locks,Branch::Ready,candidate.clone()).unwrap();
        let detail="/var/lib/nginx-web-ui/tls/default.crt: 권한 0600은 허용되지 않고 ...";
        j.fail_attempt(Step::Y1,&StepError{code:"storage_permissions",message:MESSAGE,detail:Some(detail.into())},Outcome::RefusedPrivilege,chrono::Utc::now()).unwrap();
        let record=f.outside.join("refusal.json");REFUSAL_FIXTURE.with(|p|*p.borrow_mut()=Some(record.clone()));
        let project=||super::super::refusal::status_value(&f.roots,j.load(),Outcome::RefusedPrivilege,&candidate,None).unwrap().failure.unwrap().next_action;
        assert_ne!(project(),detail,"no record, no detail");
        write_json(&record,&serde_json::json!({"version":1,"bootIdentity":"f".repeat(64),"detail":detail})).unwrap();assert_ne!(project(),detail,"another boot's record is ignored");
        write_json(&record,&serde_json::json!({"version":1,"bootIdentity":super::super::boot_identity::current().unwrap(),"detail":detail})).unwrap();assert_eq!(project(),detail);
        REFUSAL_FIXTURE.with(|p|*p.borrow_mut()=None);
    }
    #[test]
    fn r4_4_non_phase1_boots_keep_the_strict_root_check() {
        let f=fixture(false);set(&f.roots.app,0o777);
        assert_eq!(super::super::runner::strict_roots(&f.roots).unwrap_err().detail.as_deref(),Some("app_root_mode"));assert_eq!(mode(&f.roots.app),0o777,"never normalized there");
    }
    #[test]
    fn r4_5_a_change_whose_fsync_fails_is_still_counted() {
        let f=fixture(false);set(&f.roots.app.join("secrets/auth_secret"),0o674);
        // Replace the entry right after its chmod, so the reopen-and-fsync step sees another inode.
        let replace=|fd:RawFd,m:u32|{proc_chmod(fd,m)?;let p=fs::read_link(format!("/proc/self/fd/{fd}"))?;if p.ends_with("auth_secret"){fs::rename(&p,p.with_extension("moved"))?;fs::write(&p,b"other")?;}Ok(())};
        let detail=normalize_seams(&f.roots,&f.locks,false,&Seams{chmod:&replace,cancelled:&||false,readonly:None}).unwrap_err().detail.unwrap();
        assert!(detail.contains("디스크에 기록하지 못했습니다")&&detail.contains("이미 정리한 1개 항목"),"{detail}");
    }
    #[test]
    fn r4_6_cancellation_stops_between_entries_and_a_rerun_finishes() {
        let f=fixture(false);damage(&f,false);let calls=std::cell::Cell::new(0);let cancelled=||{calls.set(calls.get()+1);calls.get()>3};
        let e=normalize_seams(&f.roots,&f.locks,false,&Seams{chmod:&proc_chmod,cancelled:&cancelled,readonly:None}).unwrap_err();assert_eq!(e.code,"interrupted");
        run(&f,false).unwrap();assert_eq!(mode(&f.roots.app.join("secrets/auth_secret")),0o600);assert_eq!(run(&f,false).unwrap().changed(),0);
    }
    #[test]
    fn u2_second_run_changes_nothing() {
        let f=fixture(false);damage(&f,false);run(&f,false).unwrap();
        let snapshot=crate::db_upgrade::files::snapshot(f._t.path()).unwrap();
        assert_eq!(run(&f,false).unwrap().changed(),0);assert_eq!(crate::db_upgrade::files::snapshot(f._t.path()).unwrap(),snapshot);
    }
    #[test] fn u3_r4_3_a_symlink_in_the_scanned_set_refuses_before_any_change_and_is_never_followed(){let f=fixture(false);damage(&f,false);let target=f.outside.join("target");write(&target,b"outside",0o666);let path=f.roots.app.join("secrets/admin_password");symlink(&target,&path).unwrap();let before=modes(f._t.path());let detail=run(&f,false).unwrap_err().detail.unwrap();assert!(detail.contains("secrets/admin_password")&&detail.contains("일반 파일이 아니라"),"{detail}");assert_eq!(modes(f._t.path()),before);assert_eq!(mode(&target),0o666);}
    #[test]
    fn u5_fixed_hard_link_refuses_only_when_a_change_is_needed() {
        let f=fixture(false);fs::hard_link(f.roots.app.join("secrets/auth_secret"),f.outside.join("version")).unwrap();
        assert_eq!(run(&f,false).unwrap().changed(),0,"accepted entries are not newly rejected");
        set(&f.roots.app.join("secrets/auth_secret"),0o620);let snapshot=crate::db_upgrade::files::snapshot(f._t.path()).unwrap();
        let e=run(&f,false).unwrap_err();assert!(e.detail.unwrap().contains("auth_secret"));
        assert_eq!(crate::db_upgrade::files::snapshot(f._t.path()).unwrap(),snapshot);
    }
    #[test]
    fn u6_foreign_owner_refuses_with_path_condition_and_chown_command() {
        let f=fixture(false);set(&f.roots.app,0o775);chown(&f.roots.app,Some(1000),None).unwrap();
        let detail=run(&f,false).unwrap_err().detail.unwrap();
        assert!(detail.contains(&f.roots.app.display().to_string())&&detail.contains("소유자 UID 1000")&&detail.contains("sudo chown root:root '<앱 데이터 호스트 폴더>'"),"{detail}");
        assert_eq!(mode(&f.roots.app),0o775);
    }
    #[test]
    fn u7_ignored_chmod_refuses_and_reports_nothing_changed() {
        let f=fixture(false);damage(&f,false);
        let e=normalize_seams(&f.roots,&f.locks,false,&Seams{chmod:&|_,_|Ok(()),cancelled:&||false,readonly:None}).unwrap_err();let detail=e.detail.unwrap();
        assert!(detail.contains("반영되지 않았습니다")&&detail.contains("sudo chmod 755 '<앱 데이터 호스트 폴더>'")&&!detail.contains("이미 정리한"),"{detail}");
        assert_eq!(mode(&f.roots.app),0o775);
    }
    #[test]
    fn u8_read_only_error_names_the_condition() {
        let f=fixture(false);damage(&f,false);
        let detail=normalize_seams(&f.roots,&f.locks,false,&Seams{chmod:&|_,_|Err(io::Error::from_raw_os_error(libc::EROFS)),cancelled:&||false,readonly:None}).unwrap_err().detail.unwrap();
        assert!(detail.contains("읽기 전용"),"{detail}");
    }
    #[test]
    fn u9_entries_the_gates_accept_are_never_touched() {
        let f=fixture(false);set(&f.roots.db,0o750);
        let snapshot=crate::db_upgrade::files::snapshot(f._t.path()).unwrap();
        assert_eq!(run(&f,false).unwrap().changed(),0);assert_eq!(crate::db_upgrade::files::snapshot(f._t.path()).unwrap(),snapshot);
    }
    #[test]
    fn storage_warning_describes_read_only_write_only_and_mixed_mode_changes() {
        for initial_mode in [0o644, 0o622, 0o666] {
            let f = fixture(false);
            let secret = f.roots.app.join("secrets/auth_secret");
            set(&secret, initial_mode);
            let before = fs::metadata(&secret).unwrap();
            let mut report = run(&f, false).unwrap();
            assert_eq!(report.changed(), 1);
            assert_eq!(mode(&secret), 0o600);
            assert_eq!(fs::read(&secret).unwrap(), b"fixture-auth");
            let after = fs::metadata(&secret).unwrap();
            assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));

            let path = f.outside.join("report.json");
            report.boot_identity = Some("a".repeat(64));
            write_report(&path, &report).unwrap();
            let warning = warning(&path, report.boot_identity.as_deref()).unwrap();
            let notice = super::super::status::storage_notification(&warning);
            assert_eq!(notice.kind, "system.storage_modes_normalized");
            assert_eq!(notice.severity, "warning");
            assert_eq!(notice.body, format!(
                "시작 전에 저장소 접근 권한을 줄였습니다. 파일 내용과 소유자는 그대로입니다. 바뀐 항목: {} {initial_mode:04o} → 0600.",
                secret.display()
            ));
        }
    }
    #[test]
    fn unchanged_storage_boot_does_not_replay_the_previous_warning() {
        let f = fixture(false);
        set(&f.roots.app, 0o722);
        let path = f.outside.join("report.json");
        let mut first = run(&f, false).unwrap();
        first.boot_identity = Some("a".repeat(64));
        write_report(&path, &first).unwrap();
        assert!(warning(&path, first.boot_identity.as_deref()).is_some());

        let next_boot = "b".repeat(64);
        let mut second = run(&f, false).unwrap();
        assert_eq!(second.changed(), 0);
        assert!(warning(&path, Some(&next_boot)).is_none());
        second.boot_identity = Some(next_boot);
        write_report(&path, &second).unwrap();
        assert!(warning(&path, second.boot_identity.as_deref()).is_none());
    }
    #[test]
    fn u12_warning_is_bounded_and_bound_to_this_container_start() {
        let f=fixture(false);let mut report=Report::empty();for i in 0..LOG_PAIRS{report.deep.push(DeepChange{path:"historical/db".into(),old:0o770+i as u32,new:0o750,count:1});}report.deep_other=3000-LOG_PAIRS as u64;report.deep_other_pairs=report.deep_other;
        report.boot_identity=Some("a".repeat(64));let path=f.outside.join("report.json");write_report(&path,&report).unwrap();
        assert!(warning(&path,Some(&"b".repeat(64))).is_none());assert!(warning(&path,None).is_none());
        let w=warning(&path,Some(&"a".repeat(64))).unwrap();assert!(w.message.len()<=MAX_MESSAGE,"{}",w.message.len());
        assert!(w.message.contains("그 밖에"),"{}",w.message);assert_eq!(w.scopes,["db"]);
        let mut status=super::super::status::tests::sample();status.warnings=vec![w];
        assert!(serde_json::to_vec(&status).unwrap().len()<=super::super::status::STATUS_MAX_BYTES);
    }
    #[test]
    fn a5_only_the_canonical_data_key_is_narrowed_to_its_exact_mode() {
        let f=fixture(false);let key=f.roots.app.join("secrets/data_key");write(&key,&[7u8;32],0o620);
        let report=run(&f,false).unwrap();assert_eq!(mode(&key),0o600);assert_eq!(report.fixed.len(),1);assert!(report.fixed[0].exact);
        set(&f.roots.app.join("tls/default.crt"),0o664);set(&f.roots.app.join("tls"),0o750);run(&f,false).unwrap();
        assert_eq!(mode(&f.roots.app.join("tls/default.crt")),0o644);assert_eq!(mode(&f.roots.app.join("tls")),0o700);
    }
    #[test]
    fn a4_a_0644_secret_is_kept_only_where_the_gate_accepts_it() {
        // Ready layout without migration state: layout.rs:56 accepts only 0600 there.
        let f=fixture(false);set(&f.roots.app.join("secrets/db_password"),0o644);
        dir(&f.roots.db.join("17"),0o700);run(&f,false).unwrap();assert_eq!(mode(&f.roots.app.join("secrets/db_password")),0o600);
        set(&f.roots.app.join("secrets/db_password"),0o644);dir(&f.roots.db.join(".nwui"),0o700);write(&f.roots.db.join(".nwui/migration-v1.json"),b"{}",0o600);
        assert_eq!(run(&f,false).unwrap().changed(),0);assert_eq!(mode(&f.roots.app.join("secrets/db_password")),0o644);
    }
    #[test]
    fn u13_a_mode_that_needs_widening_refuses_with_a_command() {
        let f=fixture(false);set(&f.roots.app.join("secrets/auth_secret"),0o400);
        let detail=run(&f,false).unwrap_err().detail.unwrap();assert!(detail.contains("sudo chmod 600 '<앱 데이터 호스트 폴더>/secrets/auth_secret'")&&!detail.contains('`'),"{detail}");
        assert_eq!(mode(&f.roots.app.join("secrets/auth_secret")),0o400);
    }
}
