//! Frozen A.16 streamed backup authority; never collects a file-entry list.
use super::{StepError,types::canonical_generation};
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use std::{cmp::Ordering,collections::BTreeSet,fs::{File,OpenOptions},io::{self,BufRead,BufReader,Read,Seek,SeekFrom,Write},os::unix::fs::{MetadataExt,OpenOptionsExt},path::Path};

pub(crate) const MAX_ENTRIES:u64=1_000_000;
pub(crate) const MAX_LINE:usize=4096;
pub(crate) const MAX_FILE:u64=512*1024*1024;
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub(crate) enum SourceLocation {Root,Pgdata}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct Root {pub mode:u32,pub uid:u32,pub gid:u32}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct Header {
    pub version:u8,pub backup_id:String,pub source:String,pub created_at:String,
    pub source_location:SourceLocation,pub control_state:String,pub system_identifier:String,pub checkpoint_sha256:String,pub root:Root,
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="lowercase")]
pub(crate) enum Kind {File,Dir}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub kind:Kind,pub path:String,pub bytes:u64,pub mode:u32,pub uid:u32,pub gid:u32,
    #[serde(default,skip_serializing_if="Option::is_none")] pub sha256:Option<String>,
}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Trailer {entries:u64,sha256:String}
#[derive(Debug,Clone,PartialEq,Eq)]
pub(crate) struct Summary {pub entries:u64,pub sha256:String,pub bytes:u64,pub data_bytes:u64}
pub(crate) fn too_large()->StepError {StepError {code:"backup_manifest_too_large",message:"자동 이전의 파일 수·경로 길이·보관 기록 크기 한도를 넘었습니다. 원본을 변경하지 않았습니다. 한도를 확인한 뒤 다시 준비하세요.",detail:None}}
fn failed()->StepError {StepError {code:"verification_failed",message:"보관 기록의 형식·항목 수·검증값이 일치하지 않습니다. 기존 데이터를 보존했습니다.",detail:None}}
pub(crate) fn io_error(e:io::Error)->StepError {
    let code=match e.raw_os_error() {Some(libc::EROFS)=>"storage_read_only",Some(libc::ENOSPC|libc::EDQUOT)=>"insufficient_space",Some(libc::EINTR)=>"interrupted",_=>"io_error"};
    StepError {code,message:"보관 기록의 파일 작업을 완료하지 못했습니다. 기존 데이터를 보존했습니다.",detail:None}
}
fn hash(value:&str)->bool {value.len()==64 && value.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b))}
pub(crate) fn canonical_path(path:&str)->bool {
    !path.is_empty() && path.len()<=MAX_LINE && !path.contains('\0')
        && path.split('/').all(|part|!matches!(part,""|"."|".."))
}
pub(crate) fn compare_paths(a:&str,b:&str)->Ordering {a.split('/').map(str::as_bytes).cmp(b.split('/').map(str::as_bytes))}
impl Header {
    pub(crate) fn valid(&self)->bool {
        self.version==1 && canonical_generation(&self.backup_id) && self.source=="L"
            && chrono::DateTime::parse_from_rfc3339(&self.created_at).is_ok()
            && super::source::allowed_state(&self.control_state) && crate::db_upgrade::state::identifier(&self.system_identifier) && hash(&self.checkpoint_sha256) && self.root.mode<=0o7777
    }
}
impl Entry {
    fn valid(&self)->bool {
        canonical_path(&self.path) && self.mode<=0o7777 && match self.kind {
            Kind::File=>self.sha256.as_deref().is_some_and(hash),
            Kind::Dir=>self.bytes==0 && self.sha256.is_none(),
        }
    }
}
fn line<T:Serialize>(value:&T)->Result<Vec<u8>,StepError> {
    let mut bytes=serde_json::to_vec(value).map_err(|_|failed())?;bytes.push(b'\n');
    if bytes.len()>MAX_LINE {return Err(too_large());}Ok(bytes)
}
pub(crate) fn authority_space(entries:u64)->Result<u64,StepError> {
    if entries>MAX_ENTRIES {return Err(too_large());}entries.checked_mul(512).ok_or_else(too_large)
}
/// P0b/P2 call with the actual count and pre-encoded line sizes, before creating any output.
pub(crate) fn check_bound(entries:u64,preceding_bytes:u64)->Result<(),StepError> {
    authority_space(entries)?;
    let trailer=line(&Trailer {entries,sha256:"0".repeat(64)})?;
    if preceding_bytes.checked_add(trailer.len()as u64).is_none_or(|n|n>MAX_FILE) {return Err(too_large());}Ok(())
}
pub(crate) struct StreamWriter<W:Write> {output:W,digest:Sha256,entries:u64,bytes:u64,data_bytes:u64,previous:Option<String>}
impl<W:Write> StreamWriter<W> {
    pub fn new(mut output:W,header:&Header)->Result<Self,StepError> {
        if !header.valid(){return Err(failed());}let bytes=line(header)?;check_bound(0,bytes.len()as u64)?;
        output.write_all(&bytes).map_err(io_error)?;let mut digest=Sha256::new();digest.update(&bytes);
        Ok(Self {output,digest,entries:0,bytes:bytes.len()as u64,data_bytes:0,previous:None})
    }
    pub fn entry(&mut self,entry:&Entry)->Result<(),StepError> {
        if entry.path.len()>MAX_LINE {return Err(too_large());}
        if !entry.valid() || self.previous.as_ref().is_some_and(|p|compare_paths(p,&entry.path)!=Ordering::Less) {return Err(failed());}
        let bytes=line(entry)?;let count=self.entries.checked_add(1).ok_or_else(too_large)?;
        let total=self.bytes.checked_add(bytes.len()as u64).ok_or_else(too_large)?;check_bound(count,total)?;let data_bytes=self.data_bytes.checked_add(entry.bytes).ok_or_else(too_large)?;
        self.output.write_all(&bytes).map_err(io_error)?;self.digest.update(&bytes);
        self.entries=count;self.bytes=total;self.data_bytes=data_bytes;self.previous=Some(entry.path.clone());Ok(())
    }
    pub fn finish(mut self)->Result<(W,Summary),StepError> {
        let sha256=hex::encode(self.digest.finalize());let trailer=line(&Trailer {entries:self.entries,sha256:sha256.clone()})?;
        self.output.write_all(&trailer).map_err(io_error)?;
        Ok((self.output,Summary {entries:self.entries,sha256,bytes:self.bytes+trailer.len()as u64,data_bytes:self.data_bytes}))
    }
}
fn read_line<R:BufRead>(input:&mut R,total:&mut u64)->Result<Option<Vec<u8>>,StepError> {
    let mut bytes=Vec::with_capacity(256);
    let count=input.take((MAX_LINE+1)as u64).read_until(b'\n',&mut bytes).map_err(io_error)?;
    if count==0 {return Ok(None);}
    *total=total.checked_add(count as u64).ok_or_else(failed)?;
    if count>MAX_LINE || *total>MAX_FILE || bytes.last()!=Some(&b'\n') {return Err(failed());}Ok(Some(bytes))
}
fn pass<R:BufRead>(input:&mut R,expected:Option<&Header>,mut visit:impl FnMut(&Entry)->Result<(),StepError>)->Result<(Header,Summary),StepError> {
    let mut total=0;let first=read_line(input,&mut total)?.ok_or_else(failed)?;
    let header:Header=serde_json::from_slice(&first).map_err(|_|failed())?;
    if !header.valid() || expected.is_some_and(|e|e!=&header) {return Err(failed());}
    let mut digest=Sha256::new();digest.update(&first);let mut count=0u64;let mut data_bytes=0u64;let mut previous:Option<String>=None;
    loop {
        let bytes=read_line(input,&mut total)?.ok_or_else(failed)?;
        if let Ok(trailer)=serde_json::from_slice::<Trailer>(&bytes) {
            let actual=hex::encode(digest.finalize());
            if trailer.entries!=count || trailer.sha256!=actual || read_line(input,&mut total)?.is_some() {return Err(failed());}
            return Ok((header,Summary {entries:count,sha256:actual,bytes:total,data_bytes}));
        }
        let entry:Entry=serde_json::from_slice(&bytes).map_err(|_|failed())?;
        count=count.checked_add(1).ok_or_else(failed)?;
        if count>MAX_ENTRIES || !entry.valid() || previous.as_ref().is_some_and(|p|compare_paths(p,&entry.path)!=Ordering::Less) {return Err(failed());}
        data_bytes=data_bytes.checked_add(entry.bytes).ok_or_else(failed)?;digest.update(&bytes);visit(&entry)?;previous=Some(entry.path);
    }
}
#[derive(PartialEq,Eq)]
struct Stamp {dev:u64,ino:u64,bytes:u64,mode:u32,uid:u32,gid:u32,links:u64,mtime:(i64,i64),ctime:(i64,i64)}
fn stamp(file:&File)->Result<Stamp,StepError> {
    let m=file.metadata().map_err(io_error)?;
    if !m.is_file() || m.uid()!=0 || m.mode()&0o7777!=0o600 || m.nlink()!=1 || m.len()>MAX_FILE {return Err(failed());}
    Ok(Stamp {dev:m.dev(),ino:m.ino(),bytes:m.len(),mode:m.mode(),uid:m.uid(),gid:m.gid(),links:m.nlink(),mtime:(m.mtime(),m.mtime_nsec()),ctime:(m.ctime(),m.ctime_nsec())})
}
pub(crate) struct Reader {input:BufReader<File>,pub header:Header,pub summary:Summary,pub top_names:BTreeSet<String>,stamp:Stamp}
impl Reader {
    pub fn open(path:&Path)->Result<Self,StepError> {
        crate::db_upgrade::files::real_path(path).map_err(|_|failed())?;
        let parent=path.parent().ok_or_else(failed)?;let m=std::fs::symlink_metadata(parent).map_err(io_error)?;
        if !m.is_dir() || m.uid()!=0 || m.mode()&0o7777!=0o700 {return Err(failed());}
        let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK).open(path).map_err(io_error)?;
        Self::from_file(file)
    }
    /// File capability from a journal-owned slot; parent admission is the caller's responsibility.
    pub(crate) fn from_file(mut file:File)->Result<Self,StepError> {
        let before=stamp(&file)?;file.seek(SeekFrom::Start(0)).map_err(io_error)?;let mut input=BufReader::new(file);let mut top_names=BTreeSet::new();let (header,summary)=pass(&mut input,None,|entry|{top_names.insert(entry.path.split('/').next().ok_or_else(failed)?.to_owned());Ok(())})?;
        if stamp(input.get_ref())?!=before || summary.bytes!=before.bytes {return Err(failed());}
        Ok(Self {input,header,summary,top_names,stamp:before})
    }
    /// Pass 1 completed in open. Only now can callbacks trust entries from this immutable inode.
    pub fn for_each(&mut self,mut visit:impl FnMut(&Entry)->Result<(),StepError>)->Result<BTreeSet<String>,StepError> {
        if stamp(self.input.get_ref())?!=self.stamp {return Err(failed());}
        self.input.seek(SeekFrom::Start(0)).map_err(io_error)?;let mut names=BTreeSet::new();
        let (_,summary)=pass(&mut self.input,Some(&self.header),|entry| {
            names.insert(entry.path.split('/').next().ok_or_else(failed)?.to_owned());visit(entry)
        })?;
        if summary!=self.summary || stamp(self.input.get_ref())?!=self.stamp {return Err(failed());}Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor,os::unix::fs::{PermissionsExt,symlink}};
    fn header()->Header {Header {version:1,backup_id:"3d521ab4-34ad-44e8-b7eb-882c0feff693".into(),source:"L".into(),created_at:"2026-10-28T00:00:00Z".into(),source_location:SourceLocation::Root,control_state:"shut down".into(),system_identifier:"123456".into(),checkpoint_sha256:"1".repeat(64),root:Root {mode:0o700,uid:0,gid:0}}}
    fn entry(path:&str)->Entry {Entry {kind:Kind::File,path:path.into(),bytes:7,mode:0o600,uid:0,gid:0,sha256:Some("2".repeat(64))}}
    fn encoded()->Vec<u8> {let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();for path in ["a","a/b","a-c","z"] {let mut value=entry(path);if path=="a" {value.kind=Kind::Dir;value.bytes=0;value.sha256=None;}writer.entry(&value).unwrap();}writer.finish().unwrap().0}
    #[test]
    fn w1_backup_stream_uses_component_order_and_matches_the_raw_line_trailer() {
        assert_eq!(compare_paths("a/b","a-c"),Ordering::Less);let bytes=encoded();let mut cursor=Cursor::new(&bytes);
        let (_,summary)=pass(&mut cursor,None,|_|Ok(())).unwrap();assert_eq!(summary.entries,4);assert_eq!(summary.bytes,bytes.len()as u64);assert_eq!(summary.data_bytes,21);
        let lines=bytes.split_inclusive(|b|*b==b'\n').collect::<Vec<_>>();let mut digest=Sha256::new();for line in &lines[..lines.len()-1] {digest.update(line);}
        let trailer:Trailer=serde_json::from_slice(lines.last().unwrap()).unwrap();assert_eq!(trailer.sha256,hex::encode(digest.finalize()));
    }
    #[test]
    fn w1_backup_source_byte_overflow_is_refused_by_writer_and_reader_without_wrapping() {
        let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();let mut first=entry("a");first.bytes=u64::MAX;writer.entry(&first).unwrap();assert_eq!(writer.entry(&entry("b")).unwrap_err().code,"backup_manifest_too_large");
        let mut bytes=line(&header()).unwrap();bytes.extend(line(&first).unwrap());bytes.extend(line(&entry("b")).unwrap());let digest=hex::encode(Sha256::digest(&bytes));bytes.extend(line(&Trailer{entries:2,sha256:digest}).unwrap());assert_eq!(pass(&mut Cursor::new(bytes),None,|_|Ok(())).unwrap_err().code,"verification_failed");
    }
    #[test]
    fn w1_backup_stream_rejects_duplicates_aliases_and_whole_string_order() {
        let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();writer.entry(&entry("a-c")).unwrap();assert!(writer.entry(&entry("a/b")).is_err());
        let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();writer.entry(&entry("a")).unwrap();assert!(writer.entry(&entry("a")).is_err());
        for path in ["","/a","a/","a//b","a/./b","a/../b"] {let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();assert!(writer.entry(&entry(path)).is_err());}
    }
    #[test]
    fn w1_backup_stream_limits_have_distinct_write_and_read_errors() {
        assert_eq!(authority_space(MAX_ENTRIES).unwrap(),512_000_000);assert_eq!(authority_space(MAX_ENTRIES+1).unwrap_err().code,"backup_manifest_too_large");
        assert_eq!(check_bound(1,MAX_FILE).unwrap_err().code,"backup_manifest_too_large");
        let mut writer=StreamWriter::new(Vec::new(),&header()).unwrap();assert_eq!(writer.entry(&entry(&"x".repeat(MAX_LINE))).unwrap_err().code,"backup_manifest_too_large");
        let oversized=vec![b'x';MAX_LINE+1];assert_eq!(pass(&mut Cursor::new(oversized),None,|_|Ok(())).err().unwrap().code,"verification_failed");
    }
    #[test]
    fn w1_backup_stream_detects_header_count_hash_and_truncation_tampering_before_visit() {
        let good=encoded();
        for bad in [good[..good.len()-1].to_vec(),good[..good.len()/2].to_vec(),String::from_utf8(good.clone()).unwrap().replace("\"entries\":4","\"entries\":3").into_bytes(),String::from_utf8(good.clone()).unwrap().replace("\"version\":1","\"version\":2").into_bytes(),String::from_utf8(good.clone()).unwrap().replacen("\"bytes\":7","\"bytes\":8",1).into_bytes()] {
            assert_eq!(pass(&mut Cursor::new(bad),None,|_|Ok(())).err().unwrap().code,"verification_failed");
        }
    }
    #[test]
    fn w1_backup_stream_reader_trusts_only_after_first_pass_and_keeps_top_names() {
        let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("backup.jsonl");std::fs::write(&path,encoded()).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut reader=Reader::open(&path).unwrap();let mut count=0;let names=reader.for_each(|_| {count+=1;Ok(())}).unwrap();assert_eq!(count,4);assert_eq!(names,BTreeSet::from(["a".into(),"a-c".into(),"z".into()]));
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o644)).unwrap();assert!(Reader::open(&path).is_err());
        let alias=t.path().join("alias");symlink(&path,&alias).unwrap();assert!(Reader::open(&alias).is_err());
    }
    #[test]
    fn w1_backup_stream_exceeds_the_old_one_mib_limit_without_collecting_entries() {
        let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("large.jsonl");
        let file=OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path).unwrap();
        let mut writer=StreamWriter::new(file,&header()).unwrap();let mut directory=entry("base");directory.kind=Kind::Dir;directory.bytes=0;directory.sha256=None;writer.entry(&directory).unwrap();
        for n in 0..10_000 {writer.entry(&entry(&format!("base/{n:06}"))).unwrap();}
        let (file,summary)=writer.finish().unwrap();file.sync_all().unwrap();assert!(summary.bytes>1_048_576);assert_eq!(summary.entries,10_001);
        let mut reader=Reader::open(&path).unwrap();let mut count=0;let names=reader.for_each(|_| {count+=1;Ok(())}).unwrap();assert_eq!(count,10_001);assert_eq!(names,BTreeSet::from(["base".into()]));
    }
    #[test]
    fn w1_backup_stream_reader_rejects_disorder_even_with_a_valid_trailer_hash() {
        for paths in [["a-c","a/b"],["a","a"],["a","a/./b"]] {
            let mut raw=line(&header()).unwrap();for path in paths {raw.extend(line(&entry(path)).unwrap());}
            let digest=hex::encode(Sha256::digest(&raw));raw.extend(line(&Trailer {entries:2,sha256:digest}).unwrap());
            assert_eq!(pass(&mut Cursor::new(raw),None,|_|Ok(())).err().unwrap().code,"verification_failed");
        }
    }
    #[test]
    fn w1_backup_stream_read_rejects_sparse_file_above_the_frozen_size_bound() {
        let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("oversized.jsonl");
        let file=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();file.set_len(MAX_FILE+1).unwrap();
        assert_eq!(Reader::open(&path).err().unwrap().code,"verification_failed");
    }
    #[test]
    fn w1_backup_stream_reader_rewinds_the_journal_slot_after_the_writer_finished() {
        let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("slot");
        let mut file=OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path).unwrap();file.write_all(&encoded()).unwrap();file.sync_all().unwrap();
        assert!(file.stream_position().unwrap()>0);let mut reader=Reader::from_file(file).unwrap();let mut count=0;reader.for_each(|_| {count+=1;Ok(())}).unwrap();assert_eq!(count,4);
    }
    #[test]
    fn w1_backup_stream_second_pass_refuses_in_place_authority_changes() {
        let t=tempfile::tempdir().unwrap();std::fs::set_permissions(t.path(),std::fs::Permissions::from_mode(0o700)).unwrap();let path=t.path().join("backup.jsonl");std::fs::write(&path,encoded()).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut reader=Reader::open(&path).unwrap();std::fs::write(&path,b"changed\n").unwrap();let mut visited=false;
        assert!(reader.for_each(|_| {visited=true;Ok(())}).is_err());assert!(!visited);
    }
}
