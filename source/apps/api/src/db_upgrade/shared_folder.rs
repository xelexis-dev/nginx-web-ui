//! Root decides each exception from the held object's kernel mount identity, never stored state.
use super::mounts::Table;
use std::{fs::{File,OpenOptions},io::Read,os::{fd::AsRawFd,unix::fs::OpenOptionsExt},path::Path};
const ROOTS:[(&str,&str);4]=[("/var/lib/postgresql/data","db"),("/var/lib/nginx-web-ui","app"),("/var/log/nginx","nginx-log"),("/var/log/nginx-web-ui","app-log")];
fn scope(path:&Path)->Option<&'static str>{ROOTS.iter().find_map(|(root,scope)|path.starts_with(root).then_some(*scope))}
pub(crate) fn mnt_id(file:&File)->std::io::Result<u64>{
    let mut s=String::new();File::open(format!("/proc/self/fdinfo/{}",file.as_raw_fd()))?.take(65537).read_to_string(&mut s)?;
    if s.len()>65536{return Err(std::io::Error::other("fdinfo too large"));}
    parse_mnt_id(&s)
}
fn parse_mnt_id(s:&str)->std::io::Result<u64>{
    let mut ids=s.lines().filter_map(|l|l.strip_prefix("mnt_id:"));
    let value=ids.next().ok_or_else(||std::io::Error::other("missing mount id"))?.trim().parse().map_err(|_|std::io::Error::other("invalid mount id"))?;
    if ids.next().is_some(){return Err(std::io::Error::other("ambiguous mount id"));}Ok(value)
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(crate) enum SharedKind {DockerDesktop,OrbStackMac}
fn orbstack_release(release:&str)->bool{
    let release=release.strip_suffix('\n').unwrap_or(release);
    let Some((version,build))=release.split_once("-orbstack-")else{return false};
    let mut version=version.split('.');
    let decimal=|s:&str|!s.is_empty()&&s.bytes().all(|b|b.is_ascii_digit());
    if !(0..3).all(|_|version.next().is_some_and(decimal))||version.next().is_some(){return false;}
    let Some((build,hash))=build.split_once("-g")else{return false};
    decimal(build)&&!hash.is_empty()&&hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))
}
fn decide(release:&str,magic:u64,id:u64,table:&Table)->Option<SharedKind>{
    let m=table.by_id(id)?;
    if m.readonly||scope(&m.point).is_none(){return None;}
    if orbstack_release(release)&&magic==0x65735546&&m.fstype=="virtiofs"&&m.source=="mac"{return Some(SharedKind::OrbStackMac);}
    let Ok((source,true))=super::mounts::path(m.source.as_bytes())else{return None};
    (release.trim_end().ends_with("-linuxkit") && magic==0x6a656a63 && m.fstype=="fakeowner" && !m.readonly
        && m.super_options.split(',').any(|s|s=="fakeowner") && source.starts_with("/run/host_mark") && source.as_path()!=Path::new("/run/host_mark")
        && !source.components().any(|c|matches!(c,std::path::Component::ParentDir|std::path::Component::CurDir)) && scope(&m.point).is_some()).then_some(SharedKind::DockerDesktop)
}
pub(crate) fn kind(file:&File)->Option<SharedKind>{
    if unsafe{libc::geteuid()}!=0{return None;}
    let release=std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    if !release.trim_end().ends_with("-linuxkit")&&!orbstack_release(&release){return None;}
    let mut stat=std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe{libc::fstatfs(file.as_raw_fd(),stat.as_mut_ptr())}!=0{return None;}
    let magic=unsafe{stat.assume_init()}.f_type as u64;
    if !(release.trim_end().ends_with("-linuxkit")&&magic==0x6a656a63||orbstack_release(&release)&&magic==0x65735546){return None;}
    let id=mnt_id(file).ok()?;let table=Table::read().ok()?;
    decide(&release,magic,id,&table)
}
pub(crate) fn is_shared(file:&File)->bool{kind(file).is_some()}
pub(crate) fn path_is_shared(path:&Path)->bool{
    OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK|libc::O_CLOEXEC).open(path).is_ok_and(|f|is_shared(&f))
}
/// Missing children inherit only the nearest existing ancestor's decision.
pub(crate) fn nearest_is_shared(path:&Path)->bool{
    let mut path=path;
    loop{match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY|libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path){
        Ok(f)=>return is_shared(&f),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>match path.parent(){Some(p)=>path=p,None=>return false},Err(_)=>return false,
    }}
}
/// Include effective descendant mounts; warnings describe scopes, never host paths.
pub(crate) fn scopes()->Vec<String>{
    let Ok(table)=Table::read()else{return vec![]};
    scopes_in(&table,path_is_shared)
}
fn scopes_in(table:&Table,shared:impl Fn(&Path)->bool)->Vec<String>{
    let mut scopes=std::collections::BTreeSet::new();
    for p in table.points(){if let Some(s)=scope(p){if shared(p){scopes.insert(s.to_owned());}}}
    scopes.into_iter().collect()
}
#[cfg(test)]mod tests{
    use super::*;
    fn table(point:&str,extra:&str)->Table{Table::parse(&format!("1 0 0:1 / / rw - ext4 /dev/root rw\n2 1 0:49 /host {point} rw - fakeowner /run/host_mark/private rw,fakeowner\n{extra}")).unwrap()}
    #[test]fn desktop_shared_kernel_signature_and_every_lookalike_fail_closed(){
        let text="1 0 0:1 / / rw - ext4 /dev/root rw\n2 1 0:49 /host /var/lib/postgresql/data rw - fakeowner /run/host_mark/private rw,fakeowner\n";
        assert!(decide("7.0.14-linuxkit\n",0x6a656a63,2,&Table::parse(text).unwrap()).is_some());
        for (a,b)in [("fakeowner","virtiofs"),("/run/host_mark/private","/run/host_marker/private"),("rw,fakeowner","rw"),("rw,fakeowner","ro,fakeowner"),("/var/lib/postgresql/data","/etc/nginx"),("/var/lib/postgresql/data","/var/lib")] {
            assert!(decide("7-linuxkit",0x6a656a63,2,&Table::parse(&text.replace(a,b)).unwrap()).is_none(),"{a}->{b}");
        }
        let t=Table::parse(text).unwrap();assert!(decide("7-generic",0x6a656a63,2,&t).is_none());assert!(decide("7-linuxkit",0xef53,2,&t).is_none());assert!(decide("7-linuxkit",0x6a656a63,3,&t).is_none());
        assert!(decide("7-linuxkit",0x6a656a63,2,&Table::parse(&(text.to_owned()+text)).unwrap()).is_none());
    }
    #[test]fn desktop_shared_mixed_nested_and_descendant_mounts_keep_effective_roles(){
        let t=table("/var/lib/postgresql/data","3 1 0:2 /app /var/lib/nginx-web-ui rw - ext4 /dev/data rw\n4 2 0:3 /nested /var/lib/postgresql/data/private rw - ext4 /dev/data rw\n");
        assert!(decide("7-linuxkit",0x6a656a63,2,&t).is_some());for id in [1,3,4]{assert!(decide("7-linuxkit",0x6a656a63,id,&t).is_none());}
        let t=table("/var/log/nginx-web-ui/custom/auth.log","");assert!(decide("7-linuxkit",0x6a656a63,2,&t).is_some());assert_eq!(scope(Path::new("/var/log/nginx-web-ui/custom/auth.log")),Some("app-log"));
        for p in ["/run", "/proc", "/var/log/nginx-unrelated", "/"]{assert!(scope(Path::new(p)).is_none());}
    }
}

#[cfg(test)]mod desktop_shared_parse_tests{
 use super::*;
 #[test]fn desktop_shared_fdinfo_and_descendant_warning_discovery_fail_closed(){
  assert_eq!(parse_mnt_id("pos:\t0\nmnt_id:\t123\n").unwrap(),123);
  for text in ["","mnt_id: nope","mnt_id: -1","mnt_id: 18446744073709551616","mnt_id: 2\nmnt_id: 2"]{assert!(parse_mnt_id(text).is_err());}
  let t=Table::parse("1 0 0:1 / / rw - ext4 /dev/root rw\n2 1 0:49 /host /var/log/nginx-web-ui/custom/auth.log rw - fakeowner /run/host_mark/private rw,fakeowner\n3 1 0:2 /db /var/lib/postgresql/data rw - ext4 /dev/root rw\n").unwrap();
  assert_eq!(scopes_in(&t,|p|p==Path::new("/var/log/nginx-web-ui/custom/auth.log")),vec!["app-log"]);
  let malformed=Table::parse("1 0 0:1 / / rw - ext4 /dev/root rw\n2 1 0:49 /host /var/lib/postgresql/data rw - fakeowner /run/host_mark/private rw,fakeowner\nbad 1 0:2 /opaque /opaque rw - ext4 /dev/root rw\n").unwrap();
  assert!(decide("7-linuxkit",0x6a656a63,2,&malformed).is_none());
 }
}

#[cfg(test)]mod orbstack_tests{
 use super::*;
 const RELEASE:&str="7.2.12-orbstack-00394-gbecdd6130068\n";
 const MAGIC:u64=0x65735546;
 const TEXT:&str="1 0 0:1 / / rw - ext4 /dev/root rw\n2 1 0:35 /private/tmp/db /var/lib/postgresql/data rw,relatime - virtiofs mac rw\n";
 #[test]fn measured_orbstack_signature_and_each_independent_near_miss(){
  let table=Table::parse(TEXT).unwrap();assert_eq!(decide(RELEASE,MAGIC,2,&table),Some(SharedKind::OrbStackMac));
  assert_eq!(decide(RELEASE.trim_end(),MAGIC,2,&table),Some(SharedKind::OrbStackMac));
  for release in ["","7-generic","7.2.12-linuxkit","6.6.0-microsoft-standard-WSL2","7.2.12-colima","7.2.12-rancher","prefix7.2.12-orbstack-00394-gbecdd6130068","7.2-orbstack-00394-gbecdd6130068","7..12-orbstack-00394-gbecdd6130068","7.2.12-orbstack--gabc","7.2.12-orbstack-00394-g","7.2.12-orbstack-00394-gABC","7.2.12-orbstack-00394-gabc-extra","7.2.12-orbstack-00394-gabc\n\n","7.2.12-orbstack-00394-gabc "]{assert_eq!(decide(release,MAGIC,2,&table),None,"{release:?}");}
  for magic in [0,0x6a656a63,0x9123683e,0x794c7630]{assert_eq!(decide(RELEASE,magic,2,&table),None);}
  for (from,to)in [("virtiofs mac","virtiofs mac2"),("virtiofs mac","virtiofs /mac"),("virtiofs mac","virtiofs orbstack"),("virtiofs mac","virtiofs m\\\\141c"),("virtiofs mac","fuse mac"),("rw,relatime","ro,relatime"),("mac rw","mac ro"),("mac rw","mac rw,ro"),("rw,relatime","rw,ro"),("/var/lib/postgresql/data","/var/lib"),("/var/lib/postgresql/data","/etc/nginx"),("/var/lib/postgresql/data","/var/log/nginx-unrelated"),("virtiofs mac","btrfs /dev/vdb1"),("virtiofs mac","overlay orbstack")]{assert_eq!(decide(RELEASE,MAGIC,2,&Table::parse(&TEXT.replace(from,to)).unwrap()),None,"{from}->{to}");}
  assert_eq!(decide(RELEASE,MAGIC,3,&table),None);
  assert_eq!(decide(RELEASE,MAGIC,2,&Table::parse(&(TEXT.to_owned()+TEXT)).unwrap()),None);
  let malformed=TEXT.to_owned()+"bad 1 0:3 / /opaque rw - ext4 /dev/root rw\n";
  assert_eq!(decide(RELEASE,MAGIC,2,&Table::parse(&malformed).unwrap()),None);
 }
 #[test]fn scopes_and_mount_ids_are_independent_even_on_one_superblock(){
  for (root,label)in ROOTS{
   let text=TEXT.replace("/var/lib/postgresql/data",root);let table=Table::parse(&text).unwrap();assert_eq!(decide(RELEASE,MAGIC,2,&table),Some(SharedKind::OrbStackMac));assert_eq!(scope(Path::new(root)),Some(label));
  }
  let text=TEXT.to_owned()+"3 1 0:35 /private/tmp/log /var/log/nginx-web-ui/custom/auth.log rw - virtiofs mac rw\n4 3 0:36 / /var/log/nginx-web-ui/custom/managed rw - btrfs /dev/vdb1 rw\n";
  let table=Table::parse(&text).unwrap();assert_eq!(decide(RELEASE,MAGIC,3,&table),Some(SharedKind::OrbStackMac));assert_eq!(decide(RELEASE,MAGIC,4,&table),None);
  assert_eq!(scopes_in(&table,|p|p==Path::new("/var/log/nginx-web-ui/custom/auth.log")),vec!["app-log"]);
 }
 #[cfg(target_os="linux")]
 #[test]fn nonroot_live_fd_never_grants_a_shared_exception(){
  if unsafe{libc::geteuid()}!=0{return;}
  let file=File::open("/").unwrap();let pid=unsafe{libc::fork()};assert!(pid>=0);
  if pid==0{unsafe{if libc::setgid(10001)!=0||libc::setuid(10001)!=0{libc::_exit(2);}libc::_exit(if kind(&file).is_none(){0}else{1});}}
  let mut status=0;assert_eq!(unsafe{libc::waitpid(pid,&mut status,0)},pid);assert_eq!(status,0);
 }
}
