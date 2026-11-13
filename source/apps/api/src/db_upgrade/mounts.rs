//! Kernel mount roots expose bind aliases hidden by different container pathnames.
use super::error::{Error,Result};
use std::{fs::File,io::Read,path::{Path,PathBuf,Component}};
use std::os::unix::ffi::OsStringExt;
fn invalid()->Error{Error::refusal("unsupported_layout")}
pub(crate) struct Mount{pub(crate) id:u64,device:String,root:PathBuf,pub(crate) point:PathBuf,pub(crate) readonly:bool,pub(crate) valid:bool,pub(crate) fstype:String,pub(crate) source:String,pub(crate) super_options:String}
pub struct Table{mounts:Vec<Mount>}
pub(crate) fn path(bytes:&[u8])->Result<(PathBuf,bool)> {
    let mut decoded=Vec::new();let mut i=0;
    while i<bytes.len() {
        if bytes[i]==b'\\' {
            let escape=bytes.get(i+1..i+4).ok_or_else(invalid)?;
            decoded.push(match escape{b"040"=>b' ',b"011"=>b'\t',b"012"=>b'\n',b"134"=>b'\\',_=>return Err(invalid())});i+=4;
        }else{decoded.push(bytes[i]);i+=1;}
    }
    let path=PathBuf::from(std::ffi::OsString::from_vec(decoded.clone()));
    let valid=std::str::from_utf8(&decoded).is_ok()&&path.is_absolute()&&path.components().all(|c|matches!(c,Component::RootDir|Component::Normal(_)))
        &&(decoded.as_slice()==b"/"||!decoded.split(|b|*b==b'/').skip(1).any(|s|s.is_empty()||s==b"."||s==b".."));
    Ok((path,valid))
}
fn readonly(options:&str)->Result<bool>{let flags=options.split(',').collect::<Vec<_>>();match(flags.contains(&"ro"),flags.contains(&"rw")){(true,false)=>Ok(true),(false,true)=>Ok(false),_=>Err(invalid())}}
impl Table {
    pub fn read()->Result<Self> {
        let mut bytes=Vec::new();File::open("/proc/self/mountinfo").map_err(|_|invalid())?.take(1_048_577).read_to_end(&mut bytes).map_err(|_|invalid())?;Self::parse_bytes(&bytes)
    }
    pub fn parse(text:&str)->Result<Self> {Self::parse_bytes(text.as_bytes())}
    pub fn parse_bytes(text:&[u8])->Result<Self> {
        if text.is_empty()||text.len()>1_048_576{return Err(invalid());}
        let mut mounts=Vec::new();
        for line in text.split(|b|*b==b'\n').filter(|line|!line.is_empty()) {
            if mounts.len()>=4096{return Err(invalid());}
            let fields=line.split(|b|b.is_ascii_whitespace()).filter(|s|!s.is_empty()).collect::<Vec<_>>();
            let (point,point_valid)=path(fields.get(4).ok_or_else(invalid)?)?;
            if !point.is_absolute()||point.components().any(|c|matches!(c,Component::ParentDir)){return Err(invalid());}
            // Keep the mountpoint even if the rest is opaque, so relevant ancestors still refuse.
            let parsed=(||->Result<Mount>{
                let line=std::str::from_utf8(line).map_err(|_|invalid())?;let fields=line.split_ascii_whitespace().collect::<Vec<_>>();
                let sep=fields.iter().position(|s|*s=="-").ok_or_else(invalid)?;
                if sep<6||fields.len()!=sep+4||fields[0].parse::<u32>().is_err()||fields[1].parse::<u32>().is_err(){return Err(invalid());}
                let (major,minor)=fields[2].split_once(':').ok_or_else(invalid)?;
                if major.parse::<u32>().is_err()||minor.parse::<u32>().is_err(){return Err(invalid());}
                let (root,root_valid)=path(fields[3].as_bytes())?;
                let mode=readonly(fields[5]).and_then(|a|readonly(fields[sep+3]).map(|b|a|b));
                let valid=root_valid&&point_valid&&mode.is_ok();
                Ok(Mount{id:fields[0].parse().map_err(|_|invalid())?,fstype:fields[sep+1].into(),source:fields[sep+2].into(),super_options:fields[sep+3].into(),device:fields[2].into(),root,point:point.clone(),readonly:mode.unwrap_or(false),valid})
            })();
            let mount=parsed.unwrap_or(Mount{id:0,fstype:String::new(),source:String::new(),super_options:String::new(),device:String::new(),root:PathBuf::new(),point,readonly:false,valid:false});
            if mount.point==Path::new("/")&&!mount.valid{return Err(invalid());}
            mounts.push(mount);
        }
        if mounts.is_empty(){return Err(invalid());}
        Ok(Self{mounts})
    }
    pub(crate) fn by_id(&self,id:u64)->Option<&Mount> {
        let mut ids=std::collections::BTreeSet::new();
        if self.mounts.iter().any(|m|!m.valid || !ids.insert(m.id)){return None;}
        self.mounts.iter().find(|m|m.id==id)
    }
    pub(crate) fn points(&self)->impl Iterator<Item=&Path> {self.mounts.iter().map(|m|m.point.as_path())}
    fn containing(&self,path:&Path)->Result<&Mount> {
        let mut candidates=self.mounts.iter().filter(|m|path.starts_with(&m.point)).collect::<Vec<_>>();
        // A deleted unrelated bind must not prevent boot; every relevant ancestor stays strict.
        if candidates.iter().any(|m|!m.valid){return Err(invalid());}
        candidates.sort_by_key(|m|std::cmp::Reverse(m.point.components().count()));
        let selected=candidates.first().ok_or_else(invalid)?;
        if candidates.get(1).is_some_and(|other|other.point==selected.point){return Err(invalid());}
        Ok(selected)
    }
    pub fn physical(&self,path:&Path)->Result<(String,PathBuf)> {
        let mount=self.containing(path)?;let relative=path.strip_prefix(&mount.point).map_err(|_|invalid())?;
        Ok((mount.device.clone(),mount.root.join(relative)))
    }
    pub fn distinct(&self,roots:&[&Path])->Result<()> {
        for (index,root) in roots.iter().enumerate() {
            if self.mounts.iter().any(|m|m.point!=*root&&m.point.starts_with(root)){return Err(invalid());}
            let (device,physical)=self.physical(root)?;
            for previous in &roots[..index] {
                let (other_device,other)=self.physical(previous)?;
                if device==other_device&&(physical.starts_with(&other)||other.starts_with(&physical)){return Err(invalid());}
            }
        }
        Ok(())
    }
    pub fn require_mount(&self,path:&Path,read_only:bool)->Result<()> {
        let mount=self.containing(path)?;if mount.point!=path||mount.readonly!=read_only{return Err(invalid());}Ok(())
    }
}
