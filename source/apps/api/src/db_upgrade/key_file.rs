//! D1 canonical/staged key validation. Raw bytes and the SecretBox come from one open file.
use crate::crypto::SecretBox;
use std::{ffi::CString, fs::{self,File,OpenOptions}, io::{self,Read}, os::{fd::{AsRawFd,FromRawFd},unix::fs::{MetadataExt,OpenOptionsExt}}, path::{Path,PathBuf}, time::{Duration,Instant}};

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum KeyPolicy { Strict, Override }
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum TrustedInput { Key(KeyPolicy), Executable, Module, Config }
impl TrustedInput {
    fn parent_policy(self)->KeyPolicy {match self {Self::Key(policy)=>policy,_=>KeyPolicy::Override}}
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,thiserror::Error)]
pub enum KeyFileError {
    #[error("암호화 키 파일이 없습니다.")] Missing,
    #[error("암호화 키 파일의 경로 또는 권한을 확인하세요.")] Unsafe,
    #[error("암호화 키 파일 형식을 확인하세요.")] Invalid,
    #[error("암호화 키 파일 검증을 완료하지 못했습니다.")] Read,
}
impl KeyFileError {
    pub fn trusted_input_failure(self,path:&Path)->crate::upgrade::StepError {
        crate::upgrade::StepError {
            code:"trusted_input_unsafe",
            message:"nginx가 읽거나 실행하는 파일은 API 계정이 바꿀 수 없어야 합니다. chown root:root <path>, chmod go-w <path>를 적용하고 UID/GID 10001 또는 nginx/adm 그룹의 ACL 항목을 제거하세요.",
            detail:Some(path.display().to_string()),
        }
    }
}
// No Debug/Serialize: callers may copy these bytes, never log them.
pub struct ValidatedKeyFile { pub secrets: SecretBox, pub bytes: Vec<u8> }
pub fn canonical_policy(app_root:&Path,path:&Path)->KeyPolicy {
    if path==super::paths::resolve_data_key_file(app_root,None) { KeyPolicy::Strict } else { KeyPolicy::Override }
}
/// The source kind, never its pathname, decides whether a staged key is strict.
/// canonical_path must come from KeySource::Canonical.path with the already-resolved trusted override.
pub fn policy_for(source:&crate::upgrade::KeySource,app_root:&Path,canonical_path:&Path)->KeyPolicy {
    match source {
        crate::upgrade::KeySource::Staged {..}=>KeyPolicy::Strict,
        crate::upgrade::KeySource::Canonical=>canonical_policy(app_root,canonical_path),
    }
}
fn io_error(e:io::Error)->KeyFileError {
    match e.raw_os_error() {
        Some(libc::ENOENT)=>KeyFileError::Missing,
        Some(libc::ELOOP|libc::ENOTDIR)=>KeyFileError::Unsafe,
        _=>KeyFileError::Read,
    }
}
fn world_unsafe(m:&fs::Metadata)->bool { m.mode()&0o002!=0 && (!m.is_dir() || m.mode()&0o1000==0) }
fn api_owned(m:&fs::Metadata)->bool { m.uid()==10001 || m.gid()==10001 }
#[cfg(test)]
thread_local!{static SHARED_FIXTURE:std::cell::RefCell<Option<PathBuf>>=const{std::cell::RefCell::new(None)};}
fn shared(file:&File)->bool{
    #[cfg(test)]
    if SHARED_FIXTURE.with(|p|p.borrow().as_ref().is_some_and(|p|fs::read_link(format!("/proc/self/fd/{}",file.as_raw_fd())).is_ok_and(|f|f.starts_with(p)))){return true;}
    super::shared_folder::is_shared(file)
}
struct Check { file:File, allow_write:bool, shared:bool }
fn directories(parent:&Path,immediate:bool,policy:KeyPolicy,checks:&mut Vec<Check>)->Result<(PathBuf,bool),KeyFileError> {
    let mut paths=parent.ancestors().map(Path::to_path_buf).collect::<Vec<_>>();paths.reverse();
    if paths.len()>128 { return Err(KeyFileError::Unsafe); }
    let mut last=PathBuf::from("/");
    for (index,path) in paths.iter().enumerate() {
        let link=match fs::symlink_metadata(path) {Ok(m)=>m,Err(e)if e.kind()==io::ErrorKind::NotFound=>return Ok((last,true)),Err(e)=>return Err(io_error(e))};
        if policy==KeyPolicy::Strict && link.file_type().is_symlink() {return Err(KeyFileError::Unsafe);}
        let flags=libc::O_DIRECTORY|libc::O_NONBLOCK|if policy==KeyPolicy::Strict {libc::O_NOFOLLOW}else{0};
        let file=OpenOptions::new().read(true).custom_flags(flags).open(path).map_err(io_error)?;
        let m=file.metadata().map_err(io_error)?;let is_parent=immediate && index+1==paths.len();
        if !m.is_dir() || world_unsafe(&m) || m.uid()==10001 || is_parent && api_owned(&m)
            || policy==KeyPolicy::Strict && (m.uid()!=0 || m.mode()&0o022!=0 && m.mode()&0o1000==0) { return Err(KeyFileError::Unsafe); }
        let child_uid=match paths.get(index+1) {
            Some(child)=>match fs::symlink_metadata(child) {Ok(m)=>m.uid(),Err(e)if e.kind()==io::ErrorKind::NotFound=>0,Err(e)=>return Err(io_error(e))},
            None=>0,
        };
        // Sticky ancestors protect an existing root-owned child; the immediate parent must deny writes.
        let allow_write=!is_parent && m.mode()&0o1000!=0 && m.uid()!=10001 && child_uid!=10001;
        let shared=shared(&file);checks.push(Check {file,allow_write,shared});last=path.clone();
    }
    Ok((last,false))
}
fn api_groups()->Result<Vec<libc::gid_t>,KeyFileError> {
    let mut text=String::new();File::open("/etc/group").map_err(io_error)?.take(65537).read_to_string(&mut text).map_err(io_error)?;
    if text.len()>65536 {return Err(KeyFileError::Read);}
    groups_from(&text)
}
fn groups_from(text:&str)->Result<Vec<libc::gid_t>,KeyFileError> {
    let mut groups=vec![10001];
    for name in ["nginx","adm"] {
        let gid=text.lines().find_map(|line| {
            let fields=line.split(':').collect::<Vec<_>>();
            (fields.len()==4 && fields[0]==name).then(||fields[2].parse::<u32>().ok()).flatten()
        }).ok_or(KeyFileError::Read)?;
        if !groups.contains(&gid) {groups.push(gid);}
    }
    Ok(groups)
}

fn deny_api_writes(checks:&[Check],key:Option<&File>,groups:&[libc::gid_t],relax:bool)->Result<(),KeyFileError> {
    if unsafe{libc::geteuid()}!=0 {return Err(KeyFileError::Unsafe);}
    let control=File::open("/dev/null").map_err(io_error)?;
    let control_path=CString::new(format!("/proc/self/fd/{}",control.as_raw_fd())).map_err(|_|KeyFileError::Read)?;
    let mut probes=checks.iter().map(|c|(CString::new(format!("/proc/self/fd/{}",c.file.as_raw_fd())).unwrap(),c.allow_write,relax && c.shared)).collect::<Vec<_>>();
    if let Some(file)=key {probes.push((CString::new(format!("/proc/self/fd/{}",file.as_raw_fd())).unwrap(),false,relax && shared(file)));}
    let pid=unsafe{libc::fork()};if pid<0 {return Err(KeyFileError::Read);}
    if pid==0 {unsafe{
        // access(2) checks REAL IDs with no root capabilities. Keep effective/saved root so another
        // UID-10001 process can never ptrace this fork's inherited root memory or descriptors.
        let mut blocked=std::mem::zeroed::<libc::sigset_t>();
        if libc::sigfillset(&mut blocked)!=0 || libc::sigprocmask(libc::SIG_SETMASK,&blocked,std::ptr::null_mut())!=0
            || libc::setgroups(groups.len(),groups.as_ptr())!=0 || libc::setresgid(10001,0,0)!=0 || libc::setresuid(10001,0,0)!=0 {libc::_exit(2);}
        // A positive control proves proc-fd lookup works; a lookup failure is never "denied safely".
        if libc::access(control_path.as_ptr(),libc::W_OK)!=0 {libc::_exit(2);}
        for (path,allowed,shared) in &probes {
            if *shared {continue;}
            if libc::access(path.as_ptr(),libc::W_OK)==0 {if !*allowed {libc::_exit(1);}}
            else {let code=*libc::__errno_location();if !matches!(code,libc::EACCES|libc::EPERM|libc::EROFS) {libc::_exit(2);}}
        }
        libc::_exit(0);
    }}
    let deadline=Instant::now()+Duration::from_secs(2);
    loop {
        let mut status=0;let result=unsafe{libc::waitpid(pid,&mut status,libc::WNOHANG)};
        if result==pid {return if libc::WIFEXITED(status) {match libc::WEXITSTATUS(status) {0=>Ok(()),1=>Err(KeyFileError::Unsafe),_=>Err(KeyFileError::Read)}} else {Err(KeyFileError::Read)};}
        if result<0 && io::Error::last_os_error().raw_os_error()!=Some(libc::EINTR) {return Err(KeyFileError::Read);}
        if Instant::now()>=deadline {unsafe{libc::kill(pid,libc::SIGKILL);libc::waitpid(pid,std::ptr::null_mut(),0);}return Err(KeyFileError::Read);}
        std::thread::sleep(Duration::from_millis(1));
    }
}
/// Root-only, read-only validation; no key creation, chmod, decode/re-encode or path reopen.
pub fn read_key_file(path:&Path,policy:KeyPolicy)->Result<ValidatedKeyFile,KeyFileError> {
    read_with_groups(path,policy,&api_groups()?)
}
/// Read-only parent capability for K. The key itself is never opened or parsed.
pub fn validate_parent(path:&Path,policy:KeyPolicy)->Result<File,KeyFileError> {
    parent_with_groups(path,policy,&api_groups()?)
}
/// Configuration/PATH parents never inherit the Key-purpose shared-storage exception.
pub fn validate_config_parent(path:&Path)->Result<File,KeyFileError>{
    parent_scoped_with_groups(path,KeyPolicy::Override,&api_groups()?,false)
}
fn parent_with_groups(path:&Path,policy:KeyPolicy,groups:&[libc::gid_t])->Result<File,KeyFileError> {
    parent_scoped_with_groups(path,policy,groups,true)
}
fn parent_scoped_with_groups(path:&Path,policy:KeyPolicy,groups:&[libc::gid_t],relax:bool)->Result<File,KeyFileError> {
    let (checks,parent,_)=parent_checks(path,policy)?;
    deny_api_writes(&checks,None,groups,relax)?;
    parent.ok_or(KeyFileError::Missing)
}
fn parent_checks(path:&Path,policy:KeyPolicy)->Result<(Vec<Check>,Option<File>,usize),KeyFileError> {
    if !path.is_absolute() || path.file_name().is_none() {return Err(KeyFileError::Unsafe);}
    let parent=path.parent().ok_or(KeyFileError::Unsafe)?;
    let mut checks=Vec::new();let (last,missing_parent)=directories(parent,true,policy,&mut checks)?;
    let selected=if missing_parent {None} else {Some(checks.last().ok_or(KeyFileError::Read)?.file.try_clone().map_err(io_error)?)};
    let canonical=fs::canonicalize(&last).map_err(io_error)?;
    let physical_start=checks.len();directories(&canonical,!missing_parent,policy,&mut checks)?;
    if let Some(selected)=&selected {
        let logical=selected.metadata().map_err(io_error)?;
        let physical=checks.last().ok_or(KeyFileError::Read)?.file.metadata().map_err(io_error)?;
        if logical.dev()!=physical.dev() || logical.ino()!=physical.ino() {return Err(KeyFileError::Unsafe);}
    }
    Ok((checks,selected,physical_start))
}
/// Validate root-selected input once and return that exact open descriptor.
pub fn validate_trusted_input(path:&Path,kind:TrustedInput)->Result<File,KeyFileError> {
    validate_input_with_groups(path,kind,&api_groups()?)
}
fn validate_input_with_groups(path:&Path,kind:TrustedInput,groups:&[libc::gid_t])->Result<File,KeyFileError> {
    validate_input_scoped(path,kind,groups,None)
}
/// Config-purpose exception requires a root-held store directory and physical, mount-bound ancestry.
pub(crate) fn validate_publisher_input(path:&Path,root:&File)->Result<File,KeyFileError>{
    validate_input_scoped(path,TrustedInput::Config,&api_groups()?,Some(root))
}
fn same_identity(a:(u64,u64,u64),b:(u64,u64,u64))->bool{a==b}
fn store_ancestor(file:&File,root:&File,physical:&[Check])->bool{
    let Ok(r)=root.metadata()else{return false};
    if !r.is_dir() || r.uid()!=0 || r.mode()&0o7777!=0o700{return false;}
    let (Ok(rid),Ok(fid))=(super::shared_folder::mnt_id(root),super::shared_folder::mnt_id(file))else{return false};
    rid==fid && physical.iter().any(|c|c.file.metadata().ok().zip(super::shared_folder::mnt_id(&c.file).ok()).is_some_and(|(m,id)|same_identity((r.dev(),r.ino(),rid),(m.dev(),m.ino(),id))))
}
fn validate_input_scoped(path:&Path,kind:TrustedInput,groups:&[libc::gid_t],root:Option<&File>)->Result<File,KeyFileError>{
    let (checks,parent,physical_start)=parent_checks(path,kind.parent_policy())?;
    let file=if let Some(parent)=parent {
        let name=CString::new(path.file_name().ok_or(KeyFileError::Unsafe)?.as_encoded_bytes()).map_err(|_|KeyFileError::Unsafe)?;
        let fd=unsafe{libc::openat(parent.as_raw_fd(),name.as_ptr(),libc::O_RDONLY|libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC)};
        if fd>=0 {Some(unsafe{File::from_raw_fd(fd)})} else {
            let error=io::Error::last_os_error();if error.kind()==io::ErrorKind::NotFound {None} else {return Err(io_error(error));}
        }
    } else {None};
    if let Some(file)=&file {
        let m=file.metadata().map_err(io_error)?;
        if api_owned(&m) || world_unsafe(&m) {return Err(KeyFileError::Unsafe);}
        match kind {
            TrustedInput::Key(policy) if !m.is_file() || m.len()>4096
                || policy==KeyPolicy::Strict && (m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()!=1)=>return Err(KeyFileError::Unsafe),
            TrustedInput::Executable if !m.is_file() || m.mode()&0o111==0=>return Err(KeyFileError::Unsafe),
            TrustedInput::Config|TrustedInput::Module if !m.is_file()=>return Err(KeyFileError::Unsafe),
            _=>{},
        }
    }
    let relax=matches!(kind,TrustedInput::Key(_)) || matches!(kind,TrustedInput::Config) && root.zip(file.as_ref()).is_some_and(|(root,file)|store_ancestor(file,root,&checks[physical_start..]) && shared(file));
    deny_api_writes(&checks,file.as_ref(),groups,relax)?;
    file.ok_or(KeyFileError::Missing)
}
fn read_with_groups(path:&Path,policy:KeyPolicy,groups:&[libc::gid_t])->Result<ValidatedKeyFile,KeyFileError> {
    let file=validate_input_with_groups(path,TrustedInput::Key(policy),groups)?;
    let mut bytes=Vec::new();file.take(4097).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len()>4096 {return Err(KeyFileError::Unsafe);}
    let text=std::str::from_utf8(&bytes).map_err(|_|KeyFileError::Invalid)?;
    let secrets=crate::crypto::parse_key_file(path,text).map_err(|_|KeyFileError::Invalid)?;
    Ok(ValidatedKeyFile {secrets,bytes})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{ffi::OsStrExt,fs::{symlink,PermissionsExt}};
    const RAW:&[u8]=b"0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF\n";
    fn checked(path:&Path,policy:KeyPolicy)->Result<ValidatedKeyFile,KeyFileError> {
        read_with_groups(path,policy,&groups_from("nginx:x:101:\nadm:x:4:\n").unwrap())
    }
    #[test]
    fn key_file_missing_or_invalid_supplementary_group_is_an_error() {
        for text in ["", "adm:x:4:", "nginx:x:101:", "nginx:x:nope:\nadm:x:4:", "nginx:x:101:\nadm:x:nope:"] {
            assert_eq!(groups_from(text),Err(KeyFileError::Read));
        }
        assert_eq!(groups_from("nginx:x:101:\nadm:x:4:\n").unwrap(),vec![10001,101,4]);
    }
    fn fixture()->(tempfile::TempDir,PathBuf) {
        assert_eq!(unsafe{libc::geteuid()},0,"key-file trust tests require the root CI container");
        let dir=tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(),fs::Permissions::from_mode(0o700)).unwrap();
        let path=dir.path().join("data_key");fs::write(&path,RAW).unwrap();mode(&path,0o600);(dir,path)
    }
    fn mode(path:&Path,value:u32) {fs::set_permissions(path,fs::Permissions::from_mode(value)).unwrap();}
    fn owner(path:&Path,uid:u32,gid:u32) {
        let path=CString::new(path.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::chown(path.as_ptr(),uid,gid)},0);
    }
    fn reject(path:&Path,policy:KeyPolicy,error:KeyFileError) {
        assert!(matches!(checked(path,policy),Err(e) if e==error));
    }
    #[test]
    fn key_file_strict_preserves_exact_bytes_and_parser() {
        let (_dir,path)=fixture();let checked=checked(&path,KeyPolicy::Strict).unwrap();
        assert_eq!(checked.bytes,RAW);
        let sealed=checked.secrets.seal("fixture").unwrap();assert_eq!(checked.secrets.open(Some(&sealed)).as_deref(),Some("fixture"));
    }
    #[test]
    fn key_file_default_policy_is_strict() {
        let dir=tempfile::tempdir().unwrap();
        assert_eq!(canonical_policy(dir.path(),&dir.path().join("secrets/data_key")),KeyPolicy::Strict);
        assert_eq!(canonical_policy(dir.path(),&dir.path().join("override")),KeyPolicy::Override);
    }
    #[test]
    fn key_file_staged_source_is_always_strict_even_with_a_canonical_override() {
        use crate::upgrade::KeySource;
        let (dir,path)=fixture();let source=KeySource::Staged {generation:uuid::Uuid::new_v4().to_string()};
        let canonical_override=dir.path().join("mounted-secret");
        let staged=source.path(dir.path(),None);fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&staged,RAW).unwrap();mode(&staged,0o444);
        let policy=policy_for(&source,dir.path(),&canonical_override);assert_eq!(policy,KeyPolicy::Strict);
        reject(&staged,policy,KeyFileError::Unsafe);mode(&staged,0o600);assert_eq!(checked(&staged,policy).unwrap().bytes,RAW);
        assert_eq!(policy_for(&KeySource::Canonical,dir.path(),&path),KeyPolicy::Override);
        assert_eq!(policy_for(&KeySource::Canonical,dir.path(),&dir.path().join("secrets/data_key")),KeyPolicy::Strict);
    }
    #[test]
    fn key_file_override_allows_readonly_secrets_but_strict_requires_0600() {
        let (_dir,path)=fixture();
        for permissions in [0o400,0o444,0o640,0o660] {
            mode(&path,permissions);assert_eq!(checked(&path,KeyPolicy::Override).unwrap().bytes,RAW);
            reject(&path,KeyPolicy::Strict,KeyFileError::Unsafe);
        }
    }
    #[test]
    fn key_file_rejects_leaf_symlink_and_strict_hardlink() {
        let (dir,path)=fixture();let alias=dir.path().join("link");symlink(&path,&alias).unwrap();
        for policy in [KeyPolicy::Strict,KeyPolicy::Override] {reject(&alias,policy,KeyFileError::Unsafe);}
        fs::hard_link(&path,dir.path().join("second-name")).unwrap();reject(&path,KeyPolicy::Strict,KeyFileError::Unsafe);
    }
    #[test]
    fn key_file_rejects_nonregular_invalid_and_oversized_content() {
        let (dir,path)=fixture();reject(dir.path(),KeyPolicy::Override,KeyFileError::Unsafe);
        for bytes in [b"not-a-key".as_slice(),b"\xff".as_slice(),b"".as_slice()] {
            fs::write(&path,bytes).unwrap();reject(&path,KeyPolicy::Override,KeyFileError::Invalid);
        }
        fs::write(&path,vec![b'0';4097]).unwrap();reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
    }
    #[test]
    fn key_file_rejects_api_owned_file_or_parent_even_when_readonly() {
        for (uid,gid) in [(10001,0),(0,10001)] {
            let (_dir,path)=fixture();owner(&path,uid,gid);mode(&path,0o400);reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
            let (dir,path)=fixture();owner(dir.path(),uid,gid);mode(dir.path(),0o500);reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
        }
    }
    #[test]
    fn key_file_rejects_world_write_and_unsafe_ancestors() {
        let (dir,path)=fixture();mode(&path,0o602);reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
        mode(&path,0o600);mode(dir.path(),0o777);reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
        let nested=dir.path().join("nested");fs::create_dir(&nested).unwrap();mode(&nested,0o700);
        let nested_key=nested.join("key");fs::write(&nested_key,RAW).unwrap();mode(&nested_key,0o600);
        reject(&nested_key,KeyPolicy::Override,KeyFileError::Unsafe);
        mode(dir.path(),0o1700);assert_eq!(checked(&nested_key,KeyPolicy::Override).unwrap().bytes,RAW);
    }
    #[test]
    fn key_file_checks_actual_supplementary_group_writes() {
        let (_dir,path)=fixture();owner(&path,0,12345);mode(&path,0o660);
        assert!(read_with_groups(&path,KeyPolicy::Override,&[10001]).is_ok());
        assert!(matches!(read_with_groups(&path,KeyPolicy::Override,&[10001,12345]),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_checks_actual_parent_group_writes() {
        let (dir,path)=fixture();owner(dir.path(),0,12345);mode(dir.path(),0o770);
        assert!(read_with_groups(&path,KeyPolicy::Override,&[10001]).is_ok());
        assert!(matches!(read_with_groups(&path,KeyPolicy::Override,&[10001,12345]),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_acl_write_negative_control_is_caught() {
        let (_dir,path)=fixture();let file=File::open(&path).unwrap();
        let mut acl=2u32.to_le_bytes().to_vec();
        for (tag,permission,id) in [(1u16,6u16,u32::MAX),(2,2,10001),(4,0,u32::MAX),(16,2,u32::MAX),(32,0,u32::MAX)] {
            acl.extend(tag.to_le_bytes());acl.extend(permission.to_le_bytes());acl.extend(id.to_le_bytes());
        }
        let name=CString::new("system.posix_acl_access").unwrap();
        assert_eq!(unsafe{libc::fsetxattr(file.as_raw_fd(),name.as_ptr(),acl.as_ptr().cast(),acl.len(),0)},0,"ACL negative control must be supported");
        assert_eq!(file.metadata().unwrap().gid(),0);
        reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
    }
    #[test]
    fn key_file_secure_ancestor_alias_override_only() {
        let (dir,path)=fixture();let alias=dir.path().join("alias");symlink(dir.path(),&alias).unwrap();let key=alias.join("data_key");
        assert_eq!(checked(&key,KeyPolicy::Override).unwrap().bytes,RAW);
        reject(&key,KeyPolicy::Strict,KeyFileError::Unsafe);
        assert!(path.exists());
    }
    #[test]
    fn key_file_missing_never_skips_unsafe_parent_check() {
        let (dir,path)=fixture();fs::remove_file(&path).unwrap();reject(&path,KeyPolicy::Override,KeyFileError::Missing);
        mode(dir.path(),0o777);reject(&path,KeyPolicy::Override,KeyFileError::Unsafe);
        reject(Path::new("relative/key"),KeyPolicy::Override,KeyFileError::Unsafe);
    }
    #[test]
    fn key_file_parent_validation_never_opens_or_parses_the_key() {
        let (dir,path)=fixture();fs::remove_file(&path).unwrap();
        let name=CString::new(path.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);
        let before=fs::symlink_metadata(&path).unwrap();
        let parent=parent_with_groups(&path,KeyPolicy::Strict,&[10001]).unwrap();
        assert_eq!(parent.metadata().unwrap().ino(),fs::metadata(dir.path()).unwrap().ino());
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(),before.ino());
        fs::remove_file(&path).unwrap();fs::write(&path,b"invalid-key").unwrap();
        assert!(parent_with_groups(&path,KeyPolicy::Strict,&[10001]).is_ok());
        assert_eq!(fs::read(&path).unwrap(),b"invalid-key");
    }
    #[test]
    fn key_file_parent_validation_is_readonly_and_rejects_missing_unsafe_paths() {
        let (dir,path)=fixture();let missing=dir.path().join("missing/data_key");
        assert!(matches!(parent_with_groups(&missing,KeyPolicy::Override,&[10001]),Err(KeyFileError::Missing)));
        assert!(!dir.path().join("missing").exists());
        for (uid,gid) in [(10001,0),(0,10001)] {
            owner(dir.path(),uid,gid);
            assert!(matches!(parent_with_groups(&path,KeyPolicy::Override,&[10001]),Err(KeyFileError::Unsafe)));
            owner(dir.path(),0,0);
        }
        mode(dir.path(),0o777);
        assert!(matches!(parent_with_groups(&path,KeyPolicy::Override,&[10001]),Err(KeyFileError::Unsafe)));
        assert_eq!(fs::read(&path).unwrap(),RAW);
    }
    #[test]
    fn key_file_parent_validation_checks_supplementary_groups_and_acl() {
        let (dir,path)=fixture();owner(dir.path(),0,12345);mode(dir.path(),0o770);
        assert!(parent_with_groups(&path,KeyPolicy::Override,&[10001]).is_ok());
        assert!(matches!(parent_with_groups(&path,KeyPolicy::Override,&[10001,12345]),Err(KeyFileError::Unsafe)));
        owner(dir.path(),0,0);mode(dir.path(),0o700);let parent=File::open(dir.path()).unwrap();
        let mut acl=2u32.to_le_bytes().to_vec();
        for (tag,permission,id) in [(1u16,7u16,u32::MAX),(2,3,10001),(4,0,u32::MAX),(16,3,u32::MAX),(32,0,u32::MAX)] {
            acl.extend(tag.to_le_bytes());acl.extend(permission.to_le_bytes());acl.extend(id.to_le_bytes());
        }
        assert_eq!(unsafe{libc::fsetxattr(parent.as_raw_fd(),c"system.posix_acl_access".as_ptr(),acl.as_ptr().cast(),acl.len(),0)},0);
        assert!(matches!(parent_with_groups(&path,KeyPolicy::Override,&[10001]),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_parent_capability_stays_on_the_validated_directory() {
        let (dir,path)=fixture();let parent=parent_with_groups(&path,KeyPolicy::Strict,&[10001]).unwrap();
        let container=tempfile::tempdir().unwrap();let moved=container.path().join("original");
        fs::rename(dir.path(),&moved).unwrap();fs::create_dir(dir.path()).unwrap();owner(dir.path(),10001,10001);
        assert_eq!(parent.metadata().unwrap().ino(),fs::metadata(&moved).unwrap().ino());
        assert_ne!(parent.metadata().unwrap().ino(),fs::metadata(dir.path()).unwrap().ino());
        assert_eq!(fs::read(moved.join("data_key")).unwrap(),RAW);
    }

    fn trusted(path:&Path,kind:TrustedInput)->Result<File,KeyFileError> {
        validate_input_with_groups(path,kind,&groups_from("nginx:x:101:\nadm:x:4:\n").unwrap())
    }
    #[test]
    fn key_file_trusted_config_accepts_readonly_without_parsing_key_content() {
        let (_dir,path)=fixture();fs::write(&path,b"events {}\nhttp {}\n").unwrap();mode(&path,0o444);
        let mut file=trusted(&path,TrustedInput::Config).unwrap();let mut text=String::new();file.read_to_string(&mut text).unwrap();
        assert_eq!(text,"events {}\nhttp {}\n");assert!(matches!(checked(&path,KeyPolicy::Override),Err(KeyFileError::Invalid)));
    }
    #[test]
    fn key_file_trusted_executable_requires_regular_file_and_exec_bit() {
        let (dir,path)=fixture();
        assert!(matches!(trusted(&path,TrustedInput::Executable),Err(KeyFileError::Unsafe)));
        mode(&path,0o500);assert!(trusted(&path,TrustedInput::Executable).is_ok());
        assert!(matches!(trusted(dir.path(),TrustedInput::Executable),Err(KeyFileError::Unsafe)));
        let link=dir.path().join("linked");symlink(&path,&link).unwrap();
        assert!(matches!(trusted(&link,TrustedInput::Executable),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_trusted_inputs_refuse_api_ownership_and_actual_group_writes() {
        for kind in [TrustedInput::Config,TrustedInput::Executable] {
            let (_dir,path)=fixture();mode(&path,0o500);
            for (uid,gid) in [(10001,0),(0,10001)] {
                owner(&path,uid,gid);assert!(matches!(trusted(&path,kind),Err(KeyFileError::Unsafe)));
            }
            owner(&path,0,12345);mode(&path,0o770);
            assert!(validate_input_with_groups(&path,kind,&[10001]).is_ok());
            assert!(matches!(validate_input_with_groups(&path,kind,&[10001,12345]),Err(KeyFileError::Unsafe)));
        }
    }
    #[test]
    fn key_file_trusted_executable_acl_negative_control_is_caught() {
        let (_dir,path)=fixture();mode(&path,0o700);let file=File::open(&path).unwrap();
        let mut acl=2u32.to_le_bytes().to_vec();
        for (tag,permission,id) in [(1u16,7u16,u32::MAX),(2,2,10001),(4,0,u32::MAX),(16,2,u32::MAX),(32,0,u32::MAX)] {
            acl.extend(tag.to_le_bytes());acl.extend(permission.to_le_bytes());acl.extend(id.to_le_bytes());
        }
        assert_eq!(unsafe{libc::fsetxattr(file.as_raw_fd(),c"system.posix_acl_access".as_ptr(),acl.as_ptr().cast(),acl.len(),0)},0);
        assert!(matches!(trusted(&path,TrustedInput::Executable),Err(KeyFileError::Unsafe)));
        assert!(matches!(trusted(&path,TrustedInput::Config),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_trusted_input_returns_the_validated_inode() {
        let (dir,path)=fixture();fs::write(&path,b"original config").unwrap();
        let mut file=trusted(&path,TrustedInput::Config).unwrap();fs::rename(&path,dir.path().join("original")).unwrap();
        fs::write(&path,b"replacement config").unwrap();owner(&path,10001,10001);
        let mut text=String::new();file.read_to_string(&mut text).unwrap();assert_eq!(text,"original config");
    }
    #[test]
    fn key_file_trusted_input_failure_names_the_path_and_exact_guidance() {
        let path=Path::new("/fixture/nginx.conf");let error=KeyFileError::Unsafe.trusted_input_failure(path);
        assert_eq!(error.code,"trusted_input_unsafe");assert_eq!(error.detail.as_deref(),Some("/fixture/nginx.conf"));
        for guidance in ["chown root:root <path>","chmod go-w <path>","UID/GID 10001","nginx/adm","ACL"] {assert!(error.message.contains(guidance));}
    }

    #[test]
    fn key_file_trusted_config_refuses_a_directory() {
        let (dir,_path)=fixture();let directory=dir.path().join("included-directory");fs::create_dir(&directory).unwrap();
        assert!(parent_with_groups(&directory,KeyPolicy::Override,&[10001,101,4]).is_ok());
        assert!(matches!(trusted(&directory,TrustedInput::Config),Err(KeyFileError::Unsafe)));
    }
    #[test]
    fn key_file_trusted_config_refuses_a_fifo() {
        let (_dir,path)=fixture();fs::remove_file(&path).unwrap();
        let name=CString::new(path.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);
        assert!(parent_with_groups(&path,KeyPolicy::Override,&[10001,101,4]).is_ok());
        assert!(matches!(trusted(&path,TrustedInput::Config),Err(KeyFileError::Unsafe)));
    }

}

#[cfg(test)]mod desktop_shared_tests{
 use super::*;use std::os::unix::fs::{PermissionsExt,symlink};
 #[test]fn desktop_shared_missing_and_existing_key_probe_decisions_precede_fork(){
  let check=Check{file:File::open("/dev/null").unwrap(),allow_write:false,shared:true};let key=File::open("/dev/null").unwrap();let groups=vec![10001];
  // Writable descriptor models fakeowner enforcement; missing key still probes its parent.
  assert_eq!(deny_api_writes(&[check],None,&groups,false),Err(KeyFileError::Unsafe));
  let check=Check{file:File::open("/dev/null").unwrap(),allow_write:false,shared:true};assert!(deny_api_writes(&[check],None,&groups,true).is_ok());
  // A native writable key remains strict even when another descriptor was classified shared.
  assert_eq!(deny_api_writes(&[],Some(&key),&groups,true),Err(KeyFileError::Unsafe));
 }
 #[test]fn desktop_shared_store_ancestry_uses_physical_chain_and_mount_identity(){
  let t=tempfile::tempdir().unwrap();let root=t.path().join(".nwui-publisher");fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
  let inside=root.join("stage");fs::create_dir(&inside).unwrap();let file=inside.join("nginx.conf");fs::write(&file,b"# fixture").unwrap();let r=File::open(&root).unwrap();
  let(checks,_,physical)=parent_checks(&file,KeyPolicy::Override).unwrap();assert!(store_ancestor(&File::open(&file).unwrap(),&r,&checks[physical..]));
  let outside=t.path().join("outside");fs::create_dir(&outside).unwrap();fs::write(outside.join("foreign.conf"),b"# outside").unwrap();symlink(&outside,root.join("alias")).unwrap();let foreign=root.join("alias/foreign.conf");
  let(checks,_,physical)=parent_checks(&foreign,KeyPolicy::Override).unwrap();assert!(!store_ancestor(&File::open(&foreign).unwrap(),&r,&checks[physical..]));
  assert!(same_identity((52,10,1),(52,10,1)));assert!(!same_identity((52,10,1),(52,10,2)));
 }
 #[test]fn desktop_shared_roles_do_not_relax_module_config_or_metadata(){
  let t=tempfile::tempdir().unwrap();let p=t.path().join("key");fs::write(&p,b"0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF").unwrap();fs::set_permissions(&p,fs::Permissions::from_mode(0o666)).unwrap();
  for kind in [TrustedInput::Key(KeyPolicy::Strict),TrustedInput::Module,TrustedInput::Config]{assert!(matches!(validate_input_with_groups(&p,kind,&[10001]),Err(KeyFileError::Unsafe)));}
  let checks=[Check{file:File::open("/dev/null").unwrap(),allow_write:false,shared:true}];assert_eq!(deny_api_writes(&checks,None,&[10001],false),Err(KeyFileError::Unsafe));
 }
}

#[cfg(test)]mod desktop_shared_closure_tests{
 use super::*;use std::os::unix::fs::PermissionsExt;
 struct Classification;impl Drop for Classification{fn drop(&mut self){SHARED_FIXTURE.with(|p|*p.borrow_mut()=None);}}
 fn writable_acl(file:&File){
  let mut acl=2u32.to_le_bytes().to_vec();
  for(tag,permission,id)in[(1u16,6u16,u32::MAX),(2,2,10001),(4,0,u32::MAX),(16,2,u32::MAX),(32,0,u32::MAX)]{acl.extend(tag.to_le_bytes());acl.extend(permission.to_le_bytes());acl.extend(id.to_le_bytes());}
  assert_eq!(unsafe{libc::fsetxattr(file.as_raw_fd(),c"system.posix_acl_access".as_ptr(),acl.as_ptr().cast(),acl.len(),0)},0);
 }
 #[test]fn desktop_shared_each_closure_file_requires_shared_physical_store_ancestry(){
  let t=tempfile::tempdir().unwrap();let root=t.path().join(".nwui-publisher");fs::create_dir(&root).unwrap();fs::set_permissions(&root,fs::Permissions::from_mode(0o700)).unwrap();
  let inside=root.join("staged.conf");let outside=t.path().join("outside.conf");for p in [&inside,&outside]{fs::write(p,b"# fixture").unwrap();fs::set_permissions(p,fs::Permissions::from_mode(0o600)).unwrap();writable_acl(&File::open(p).unwrap());}
  let r=File::open(&root).unwrap();
  assert!(validate_publisher_input(&inside,&r).is_err(),"native named-volume input remains strict");
  SHARED_FIXTURE.with(|p|*p.borrow_mut()=Some(t.path().to_owned()));let _reset=Classification;
  assert!(validate_publisher_input(&inside,&r).is_ok(),"shared staged config is admitted");
  assert!(validate_publisher_input(&outside,&r).is_err(),"shared include outside store stays strict");
  for kind in [TrustedInput::Config,TrustedInput::Module,TrustedInput::Executable]{assert!(validate_input_with_groups(&inside,kind,&[10001]).is_err(),"strict role {kind:?}");}
  let parent=t.path().join("admission-parent");fs::create_dir(&parent).unwrap();fs::set_permissions(&parent,fs::Permissions::from_mode(0o700)).unwrap();
  let dir=File::open(&parent).unwrap();let mut acl=2u32.to_le_bytes().to_vec();
  for(tag,permission,id)in[(1u16,7u16,u32::MAX),(2,7,10001),(4,0,u32::MAX),(16,7,u32::MAX),(32,0,u32::MAX)]{acl.extend(tag.to_le_bytes());acl.extend(permission.to_le_bytes());acl.extend(id.to_le_bytes());}
  assert_eq!(unsafe{libc::fsetxattr(dir.as_raw_fd(),c"system.posix_acl_access".as_ptr(),acl.as_ptr().cast(),acl.len(),0)},0);
  let absent=parent.join("not-created");assert!(validate_parent(&absent,KeyPolicy::Override).is_ok());assert!(matches!(validate_config_parent(&absent),Err(KeyFileError::Unsafe)));assert!(!absent.exists());
 }
}
