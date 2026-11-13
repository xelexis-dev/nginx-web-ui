//! Fixed-root control records and independent copies; no symlinks or special files.
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write}, path::{Component, Path}};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

pub static CANCELLED:std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(false);
fn checkpoint()->io::Result<()> {
    if CANCELLED.load(std::sync::atomic::Ordering::Relaxed){return Err(io::Error::new(io::ErrorKind::Interrupted,"storage operation interrupted"));}Ok(())
}
fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

pub fn real_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() { return Err(invalid("storage path is not absolute")); }
    let mut cursor=std::path::PathBuf::new();
    for component in path.components() {
        match component { Component::RootDir|Component::Normal(_) => cursor.push(component), _ => return Err(invalid("unsafe storage path")) }
        if fs::symlink_metadata(&cursor)?.file_type().is_symlink() { return Err(invalid("symlink in storage path")); }
    }
    Ok(())
}

pub fn private_dir(path: &Path) -> io::Result<()> {
    let parent=path.parent().ok_or_else(||invalid("missing control parent"))?;real_path(parent)?;
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => File::open(parent)?.sync_all()?,
        Err(e) if e.kind()==io::ErrorKind::AlreadyExists => {},
        Err(e) => return Err(e),
    }
    private_parent(path)
}
fn private_parent(path:&Path)->io::Result<()> {
    real_path(path)?;let metadata=fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid()!=unsafe{libc::geteuid()} || metadata.mode()&0o7777!=0o700 { return Err(invalid("unsafe control directory owner or mode")); }
    Ok(())
}
fn control_file(path:&Path)->io::Result<File> {
    private_parent(path.parent().ok_or_else(||invalid("missing control parent"))?)?;
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
    let metadata=file.metadata()?;
    if !metadata.is_file() || metadata.uid()!=unsafe{libc::geteuid()} || metadata.mode()&0o7777!=0o600 || metadata.nlink()!=1 || metadata.len()>1_048_576 {
        return Err(invalid("unsafe control file"));
    }
    Ok(file)
}
pub fn read_json<T:DeserializeOwned>(path:&Path)->io::Result<T> {
    let mut bytes=Vec::new();control_file(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len()>1_048_576 { return Err(invalid("oversized control record")); }
    serde_json::from_slice(&bytes).map_err(|_|invalid("invalid control record"))
}
pub fn atomic_json<T:Serialize>(path:&Path,value:&T)->io::Result<()> {
    let parent=path.parent().ok_or_else(||invalid("missing control parent"))?;private_parent(parent)?;
    match fs::symlink_metadata(path) {
        Ok(_) => { control_file(path)?; },
        Err(e) if e.kind()==io::ErrorKind::NotFound => {},
        Err(e) => return Err(e),
    }
    let bytes=serde_json::to_vec(value).map_err(|_|invalid("invalid control record"))?;
    if bytes.len()>1_048_576 { return Err(invalid("oversized control record")); }
    let temporary=parent.join(format!(".nwui-{}.tmp",uuid::Uuid::new_v4()));
    let result=(||{
        let mut file=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
        file.write_all(&bytes)?;file.sync_all()?;fs::rename(&temporary,path)?;File::open(parent)?.sync_all()
    })();
    if temporary.exists() { let _=fs::remove_file(&temporary); }
    result
}

#[derive(Debug)]
pub struct TreeInfo { pub sha256:String, pub bytes:u64, pub files:u64 }
pub fn snapshot(path:&Path)->io::Result<String> { Ok(tree_info(path)?.sha256) }
pub fn tree_info(path:&Path)->io::Result<TreeInfo> {
    real_path(path)?;
    let mut digest=Sha256::new();let mut bytes=0u64;let mut count=0u64;
    fn walk(root:&Path, path:&Path, depth:u8, count:&mut u64, bytes:&mut u64, digest:&mut Sha256)->io::Result<()> {
        checkpoint()?;*count+=1;if depth>64 || *count>1_000_000 { return Err(invalid("storage traversal limit exceeded")); }
        let meta=fs::symlink_metadata(path)?;
        if !meta.is_file() && !meta.is_dir() { return Err(invalid("symlink or special file in storage")); }
        let relative=path.strip_prefix(root).map_err(|_|invalid("storage escaped root"))?.to_str().ok_or_else(||invalid("non-UTF8 storage path"))?;
        let mut content=None;
        if meta.is_file() {
            let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path)?;
            let opened=file.metadata()?;
            if opened.dev()!=meta.dev() || opened.ino()!=meta.ino() || !opened.is_file() { return Err(invalid("storage changed while reading")); }
            let mut file=file.take(meta.len().saturating_add(1));let mut buffer=[0u8;65536];let mut hash=Sha256::new();let mut read=0u64;
            loop { checkpoint()?;let n=file.read(&mut buffer)?;if n==0{break;}read+=n as u64;hash.update(&buffer[..n]); }
            if read!=meta.len() { return Err(invalid("storage changed while reading")); }
            *bytes=bytes.checked_add(read).ok_or_else(||invalid("storage size overflow"))?;
            content=Some(hex::encode(hash.finalize()));
        }
        let record=serde_json::to_vec(&(relative,meta.is_dir(),meta.mode()&0o7777,meta.uid(),meta.gid(),if meta.is_file(){meta.len()}else{0},content)).map_err(|_|invalid("invalid storage metadata"))?;
        digest.update((record.len()as u64).to_be_bytes());digest.update(record);
        if meta.is_dir() {
            let mut children=fs::read_dir(path)?.map(|entry|entry.map(|entry|entry.path())).collect::<io::Result<Vec<_>>>()?;children.sort();
            for child in children { walk(root,&child,depth+1,count,bytes,digest)?; }
        }
        Ok(())
    }
    walk(path,path,0,&mut count,&mut bytes,&mut digest)?;
    Ok(TreeInfo {sha256:hex::encode(digest.finalize()),bytes,files:count})
}

pub fn copy_tree(source:&Path,destination:&Path)->io::Result<()> {
    let before=snapshot(source)?;let parent=destination.parent().ok_or_else(||invalid("missing destination parent"))?;real_path(parent)?;
    if parent.starts_with(source) || source.starts_with(destination) { return Err(invalid("nested source and destination")); }
    if fs::symlink_metadata(destination).is_ok() { return Err(invalid("copy destination already exists")); }
    fn copy(source:&Path,destination:&Path,depth:u8)->io::Result<()> {
        checkpoint()?;if depth>64 { return Err(invalid("storage traversal limit exceeded")); }
        let meta=fs::symlink_metadata(source)?;
        if meta.is_dir() {
            fs::DirBuilder::new().mode(0o700).create(destination)?;
            let mut children=fs::read_dir(source)?.map(|entry|entry.map(|entry|entry.path())).collect::<io::Result<Vec<_>>>()?;children.sort();
            for child in children { copy(&child,&destination.join(child.file_name().ok_or_else(||invalid("missing file name"))?),depth+1)?; }
        } else if meta.is_file() {
            let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(source)?;
            let opened=file.metadata()?;
            if opened.dev()!=meta.dev() || opened.ino()!=meta.ino() || !opened.is_file() { return Err(invalid("storage changed during copy")); }
            let mut output=OpenOptions::new().write(true).create_new(true).mode(0o600).open(destination)?;
            let mut file=file.take(meta.len().saturating_add(1));let mut copied=0u64;let mut buffer=[0u8;65536];
            loop {checkpoint()?;let n=file.read(&mut buffer)?;if n==0{break;}output.write_all(&buffer[..n])?;copied+=n as u64;}
            if copied!=meta.len() { return Err(invalid("storage changed during copy")); }
            output.sync_all()?;
        } else { return Err(invalid("symlink or special file in storage")); }
        let copied=fs::symlink_metadata(destination)?;
        if copied.uid()!=meta.uid() || copied.gid()!=meta.gid() { std::os::unix::fs::chown(destination,Some(meta.uid()),Some(meta.gid()))?; }
        fs::set_permissions(destination,fs::Permissions::from_mode(meta.mode()&0o7777))?;
        File::open(destination)?.sync_all()
    }
    copy(source,destination,0)?;
    if snapshot(source)?!=before || snapshot(destination)?!=before { return Err(invalid("source or independent copy changed")); }
    Ok(())
}
