//! A staged main keeps nginx's original conf_prefix and compiled prefix.
use crate::{config::RuntimeConfig, control::guard_targets::{self, End}};
use anyhow::{Context, Result};
use std::{ffi::CString, os::unix::ffi::OsStrExt, path::{Component, Path, PathBuf}};
use std::{fs::File, io::{Read, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::fs::MetadataExt}};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields,rename_all="camelCase")]
struct MainRecord {
    version:u8, generation:String, path:PathBuf, parent_dev:u64, parent_ino:u64,
    file_dev:Option<u64>, file_ino:Option<u64>,
}

pub(super) enum PreparedMain {
    Staged(StagedMain),
    LiveFallback(&'static str),
}

pub(super) struct StagedMain {
    pub path:PathBuf,
    parent:File,
    record:MainRecord,
    record_path:PathBuf,
    armed:bool,
}
impl StagedMain {
    pub fn remove(mut self) -> Result<()> {
        cleanup_record(&self.parent,&self.record_path,&self.record)?;self.armed=false;Ok(())
    }
}
impl Drop for StagedMain {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error)=cleanup_record(&self.parent,&self.record_path,&self.record) {
                eprintln!("[publisher] staged main cleanup remains journaled: {error:#}");
            }
        }
    }
}

fn cleanup_record(parent:&File,record_path:&Path,record:&MainRecord) -> Result<()> {
    super::store::uuid(&record.generation)?;
    let expected=format!(".nwui-staged-{}",record.generation);
    anyhow::ensure!(record.version==1 && record.path.is_absolute() && record.path.file_name()==Some(std::ffi::OsStr::new(&expected)),"Invalid staged-main cleanup record");
    let meta=parent.metadata()?;
    anyhow::ensure!(meta.dev()==record.parent_dev && meta.ino()==record.parent_ino,"Staged-main directory identity changed");
    let name=CString::new(expected)?;let mut leaf=std::mem::MaybeUninit::<libc::stat>::uninit();
    let status=unsafe {libc::fstatat(parent.as_raw_fd(),name.as_ptr(),leaf.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)};
    if status==0 {
        let leaf=unsafe {leaf.assume_init()};
        anyhow::ensure!(leaf.st_mode & libc::S_IFMT==libc::S_IFREG && leaf.st_uid==0 && leaf.st_mode & 0o7777==0o600 && leaf.st_nlink==1,"Unsafe staged-main cleanup target");
        if let Some(dev)=record.file_dev {anyhow::ensure!(leaf.st_dev as u64==dev,"Staged-main inode changed");}
        if let Some(ino)=record.file_ino {anyhow::ensure!(leaf.st_ino as u64==ino,"Staged-main inode changed");}
        // The intent records an absent pre-image in this API-unwritable directory before create.
        if unsafe {libc::unlinkat(parent.as_raw_fd(),name.as_ptr(),0)}!=0 {return Err(std::io::Error::last_os_error().into());}
        parent.sync_all()?;
    } else if std::io::Error::last_os_error().kind()!=std::io::ErrorKind::NotFound {
        return Err(std::io::Error::last_os_error().into());
    }
    match std::fs::remove_file(record_path) {
        Ok(())=>File::open(record_path.parent().context("Missing record parent")?)?.sync_all()?,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
        Err(error)=>return Err(error.into()),
    }
    Ok(())
}

pub(super) fn cleanup_generation(root:&Path) -> Result<()> {
    let path=root.join(".test/main-record.json");
    match std::fs::symlink_metadata(&path) {
        Ok(_)=>{},
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(()),
        Err(error)=>return Err(error.into()),
    }
    let record:MainRecord=crate::db_upgrade::files::read_json(&path)?;
    anyhow::ensure!(root.file_name()==Some(std::ffi::OsStr::new(&record.generation)),"Staged-main generation mismatch");
    let parent=crate::db_upgrade::key_file::validate_config_parent(&record.path)?;
    cleanup_record(&parent,&path,&record)
}

pub(super) fn capacity_writes(main:&Path,config:&RuntimeConfig,generation:&Path,id:&str)->Result<Vec<super::capacity::Write>> {
    let bytes=match read_rebased(main,config,generation)?{Rebased::Ready(bytes)=>bytes,Rebased::Skip(_)=>return Ok(Vec::new())};
    let path=main.parent().context("Missing effective main directory")?.join(format!(".nwui-staged-{id}"));
    let parent=crate::db_upgrade::key_file::validate_config_parent(&path).map_err(|error|error.trusted_input_failure(&path))?;
    let meta=parent.metadata()?;
    // The future inode is unknown; its existing u64 representation bounds the same record serializer.
    let record=MainRecord{version:1,generation:id.into(),path:path.clone(),parent_dev:meta.dev(),parent_ino:meta.ino(),file_dev:Some(meta.dev()),file_ino:Some(u64::MAX)};
    Ok(vec![super::capacity::Write{path,bytes:bytes.len() as u64,inodes:1},super::capacity::Write{path:generation.join(".test/main-record.json"),bytes:serde_json::to_vec(&record)?.len() as u64,inodes:2}])
}
fn read_rebased(main:&Path,config:&RuntimeConfig,generation:&Path)->Result<Rebased> {
    use crate::db_upgrade::key_file::{self,TrustedInput};
    let mut source=key_file::validate_trusted_input(main,TrustedInput::Config).map_err(|error|error.trusted_input_failure(main))?;
    let mut bytes=Vec::new();Read::by_ref(&mut source).take((super::trusted_includes::MAX_NGINX_T_OUTPUT as u64)+1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len()<=super::trusted_includes::MAX_NGINX_T_OUTPUT,"Main config safety limit exceeded");
    rebase_main(&bytes,main,config,&generation.join(".test"))
}

pub(super) fn prepare_main(main:&Path,config:&RuntimeConfig,generation:&Path,id:&str) -> Result<PreparedMain> {
    use crate::db_upgrade::{files,key_file};
    super::store::uuid(id)?;
    anyhow::ensure!(generation.file_name()==Some(std::ffi::OsStr::new(id)),"Staged-main generation mismatch");
    let test=generation.join(".test");
    let bytes=match read_rebased(main,config,generation)? {
        Rebased::Ready(bytes)=>bytes,
        Rebased::Skip(notice)=>return Ok(PreparedMain::LiveFallback(notice)),
    };
    files::private_dir(&test)?;
    let path=main.parent().context("Missing effective main directory")?.join(format!(".nwui-staged-{id}"));
    let parent=key_file::validate_config_parent(&path).map_err(|error|error.trusted_input_failure(&path))?;
    let meta=parent.metadata()?;let name=CString::new(path.file_name().unwrap().as_bytes())?;
    let mut previous=std::mem::MaybeUninit::<libc::stat>::uninit();
    let existing=unsafe {libc::fstatat(parent.as_raw_fd(),name.as_ptr(),previous.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)};
    anyhow::ensure!(existing!=0,"Staged-main filename already exists; preserved");
    anyhow::ensure!(std::io::Error::last_os_error().kind()==std::io::ErrorKind::NotFound,"Cannot establish staged-main pre-image");
    let record_path=test.join("main-record.json");
    let record=MainRecord {version:1,generation:id.into(),path:path.clone(),parent_dev:meta.dev(),parent_ino:meta.ino(),file_dev:None,file_ino:None};
    files::atomic_json(&record_path,&record)?;
    let fd=unsafe {libc::openat(parent.as_raw_fd(),name.as_ptr(),libc::O_WRONLY|libc::O_CREAT|libc::O_EXCL|libc::O_NOFOLLOW|libc::O_CLOEXEC,0o600)};
    if fd<0 {
        let error=std::io::Error::last_os_error();
        std::fs::remove_file(&record_path)?;File::open(&test)?.sync_all()?;
        if matches!(error.raw_os_error(),Some(libc::EROFS)|Some(libc::EACCES)) {
            return Ok(PreparedMain::LiveFallback("nginx 설정 폴더에 임시 파일을 쓸 수 없어 미리 검사를 생략하고 현재 설정의 include를 검사합니다."));
        }
        return Err(error.into());
    }
    let mut file=unsafe {File::from_raw_fd(fd)};
    let mut staged=StagedMain {path,parent,record,record_path,armed:true};
    if unsafe {libc::fchown(file.as_raw_fd(),0,0)}!=0 {return Err(std::io::Error::last_os_error().into());}
    let meta=file.metadata()?;
    staged.record.file_dev=Some(meta.dev());staged.record.file_ino=Some(meta.ino());
    files::atomic_json(&staged.record_path,&staged.record)?;
    file.write_all(&bytes)?;file.sync_all()?;staged.parent.sync_all()?;
    Ok(PreparedMain::Staged(staged))
}

pub(super) enum Rebased {
    Ready(Vec<u8>),
    Skip(&'static str),
}

fn absolute(base:&Path, path:&Path) -> PathBuf {
    if path.is_absolute() {path.to_path_buf()} else {base.join(path)}
}

fn canonical_parent(path:&Path) -> Result<Option<PathBuf>> {
    let parent=path.parent().context("Include path has no parent")?;
    match std::fs::canonicalize(parent) {
        Ok(parent)=>Ok(Some(parent.join(path.file_name().context("Include path has no filename")?))),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound
            && !path.components().any(|component|component==Component::ParentDir)=>Ok(Some(path.to_path_buf())),
        Err(_)=>Ok(None),
    }
}

// Configured managed directories retain the missing-directory bootstrap behaviour.
fn managed_directory(path:PathBuf)->PathBuf {
    match std::fs::canonicalize(&path) {
        Ok(path)=>path,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{
            let mut out=PathBuf::new();
            for component in path.components() {
                match component {Component::CurDir=>{},Component::ParentDir=>{out.pop();},component=>out.push(component.as_os_str())}
            }
            out
        },
        Err(_)=>path,
    }
}

fn reaches_directory(target:&Path,directory:&Path) -> bool {
    if target.starts_with(directory) {return true;}
    let Some(parent)=target.parent() else {return false};
    let (Ok(pattern),Ok(directory))=(CString::new(parent.as_os_str().as_bytes()),CString::new(directory.as_os_str().as_bytes())) else {return true};
    unsafe {libc::fnmatch(pattern.as_ptr(),directory.as_ptr(),libc::FNM_PATHNAME|libc::FNM_PERIOD)==0}
}

// Preserve byte offsets for opaque non-UTF-8 comments/content; these bytes are never emitted.
fn lexer_view(bytes:&[u8]) -> String {
    let mut view=bytes.to_vec();let mut offset=0;
    while let Err(error)=std::str::from_utf8(&view[offset..]) {
        offset+=error.valid_up_to();
        let end=offset+error.error_len().unwrap_or(view.len()-offset);
        view[offset..end].fill(b'x');offset=end;
    }
    String::from_utf8(view).expect("invalid source bytes replaced in lexical view")
}

fn include(path:&Path) -> Result<Vec<u8>> {
    let bytes=path.as_os_str().as_bytes();
    anyhow::ensure!(!bytes.contains(&0),"Invalid staged include path");
    let mut out=b"include \"".to_vec();
    for &byte in bytes {
        match byte {
            b'"'|b'\\'=>{out.push(b'\\');out.push(byte);},
            b'\n'=>out.extend_from_slice(b"\\n"),
            b'\r'=>out.extend_from_slice(b"\\r"),
            b'\t'=>out.extend_from_slice(b"\\t"),
            byte=>out.push(byte),
        }
    }
    out.extend_from_slice(b"\";");Ok(out)
}

pub(super) fn rebase_main(bytes:&[u8],main:&Path,config:&RuntimeConfig,test:&Path) -> Result<Rebased> {
    anyhow::ensure!(main.is_absolute() && test.is_absolute(),"Staged main paths must be absolute");
    let base=main.parent().context("Effective main config has no parent")?;
    let conf=managed_directory(absolute(base,&config.nginx_conf_dir));
    let stream=managed_directory(absolute(base,&config.nginx_stream_dir));
    const UNRESOLVED:&str="include 경로를 실제 파일시스템에서 확인할 수 없어 미리 검사를 생략하고 현재 설정의 include를 검사합니다.";
    let mut targets=Vec::new();
    for (path,replacement) in [
        (conf.join("*.conf"),test.join("conf/*.conf")),
        (stream.join("*.conf"),test.join("stream/*.conf")),
        (conf.join(".nwui-generation"),test.join("marker")),
    ] {
        let Some(target)=canonical_parent(&path)? else {return Ok(Rebased::Skip(UNRESOLVED));};
        targets.push((target,replacement));
    }
    let roots=[conf.clone(),stream.clone(),std::fs::canonicalize(&conf).unwrap_or(conf.clone()),std::fs::canonicalize(&stream).unwrap_or(stream.clone())];
    let view=lexer_view(bytes);let mut changes=Vec::new();
    for statement in guard_targets::statements(&view) {
        if statement.end!=End::Semicolon || statement.words.first().map(String::as_str)!=Some("include") {continue;}
        if statement.words.len()!=2 {return Ok(Rebased::Skip("include 형식을 스테이징으로 치환할 수 없어 미리 검사를 생략합니다."));}
        let range=statement.range();
        if std::str::from_utf8(&bytes[range.clone()]).is_err() {
            return Ok(Rebased::Skip("include의 원래 바이트를 보존하기 위해 스테이징 미리 검사를 생략합니다."));
        }
        let Some(target)=canonical_parent(&absolute(base,Path::new(&statement.words[1])))? else {return Ok(Rebased::Skip(UNRESOLVED));};
        if let Some((_,replacement))=targets.iter().find(|(known,_)|*known==target) {
            changes.push((range,include(replacement)?));
        } else if roots.iter().any(|root|reaches_directory(&target,root)) {
            return Ok(Rebased::Skip("관리 설정 include를 그대로 치환할 수 없어 스테이징 미리 검사를 생략합니다."));
        }
    }
    let mut out=Vec::with_capacity(bytes.len());let mut cursor=0;
    for (range,replacement) in changes {
        out.extend_from_slice(&bytes[cursor..range.start]);out.extend_from_slice(&replacement);cursor=range.end;
    }
    out.extend_from_slice(&bytes[cursor..]);Ok(Rebased::Ready(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config()->RuntimeConfig {
        RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/fixture".into()),
            "AUTH_SECRET"=>Some("staged-main-fixture".into()),_=>None,
        }).unwrap()
    }
    #[test]
    fn relative_managed_statements_change_and_every_outside_byte_stays_identical() {
        let source=b"# opaque \xff comment\ninclude mime.types;\nhttp {\n include  conf.d/*.conf; # keep\n include # removed with managed statement\n conf.d/.nwui-generation;\n include ../shared/*.conf;\n}\nstream { include \"stream.d/*.conf\"; }\n";
        let (temp,config,main,_generation,_id)=filesystem_fixture();
        std::fs::create_dir(temp.path().join("shared")).unwrap();
        let Rebased::Ready(actual)=rebase_main(source,&main,&config,Path::new("/payload/.test")).unwrap() else {panic!("supported managed forms")};
        let mut expected=source.to_vec();
        for (old,new) in [
            (b"include  conf.d/*.conf;".as_slice(),b"include \"/payload/.test/conf/*.conf\";".as_slice()),
            (b"include # removed with managed statement\n conf.d/.nwui-generation;".as_slice(),b"include \"/payload/.test/marker\";".as_slice()),
            (b"include \"stream.d/*.conf\";".as_slice(),b"include \"/payload/.test/stream/*.conf\";".as_slice()),
        ] {
            let at=expected.windows(old.len()).position(|part|part==old).unwrap();expected.splice(at..at+old.len(),new.iter().copied());
        }
        assert_eq!(actual,expected);
    }
    #[test]
    fn partial_globs_direct_managed_files_and_wildcard_parents_skip_instead_of_testing_live_files() {
        for arg in ["conf.d/server-*.conf","conf.d/server-existing.conf","*/server-*.conf","stream.d/stream-old.conf"] {
            let source=format!("http {{ include {arg}; }}");
            assert!(matches!(rebase_main(source.as_bytes(),Path::new("/etc/nginx/nginx.conf"),&config(),Path::new("/payload/.test")).unwrap(),Rebased::Skip(_)),"{arg}");
        }
    }
    #[test]
    fn unsupported_managed_forms_return_notice_without_creating_a_validation_view() {
        for argument in ["conf.d/server-*.conf","conf.d/server-existing.conf","stream.d/stream-existing.conf"] {
            let (_temp,config,main,generation,id)=filesystem_fixture();
            std::fs::create_dir_all(&config.nginx_conf_dir).unwrap();
            std::fs::create_dir_all(&config.nginx_stream_dir).unwrap();
            let live=config.nginx_conf_dir.join("server-existing.conf");
            std::fs::write(&live,b"live managed bytes").unwrap();
            let source=format!("events {{}} http {{ include {argument}; }}\n");
            std::fs::write(&main,&source).unwrap();
            let PreparedMain::LiveFallback(notice)=prepare_main(&main,&config,&generation,&id).unwrap() else {panic!("unsupported managed include must skip staged validation: {argument}")};
            assert!(!notice.is_empty());assert!(!generation.join(".test").exists());
            assert!(!main.parent().unwrap().join(format!(".nwui-staged-{id}")).exists());
            assert_eq!(std::fs::read(&main).unwrap(),source.as_bytes());
            assert_eq!(std::fs::read(&live).unwrap(),b"live managed bytes");
        }
    }
    fn filesystem_fixture() -> (tempfile::TempDir,RuntimeConfig,PathBuf,PathBuf,String) {
        use std::os::unix::fs::PermissionsExt;
        let temp=tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(),std::fs::Permissions::from_mode(0o700)).unwrap();
        let dir=temp.path().join("nginx");std::fs::create_dir(&dir).unwrap();
        let mut config=config();config.nginx_conf_dir=dir.join("conf.d");config.nginx_stream_dir=dir.join("stream.d");
        let main=dir.join("nginx.conf");
        std::fs::write(&main,b"events {}\nhttp { include mime.types; include conf.d/*.conf; }\n").unwrap();
        let id=uuid::Uuid::new_v4().to_string();let generation=temp.path().join(&id);
        crate::db_upgrade::files::private_dir(&generation).unwrap();
        (temp,config,main,generation,id)
    }
    #[test]
    fn symlink_then_parent_uses_os_resolution_before_managed_include_matching() {
        let (temp,mut config,main,_generation,_id)=filesystem_fixture();
        let actual=temp.path().join("actual");
        std::fs::create_dir_all(actual.join("inner")).unwrap();
        std::fs::create_dir(actual.join("conf.d")).unwrap();
        std::os::unix::fs::symlink(actual.join("inner"),main.parent().unwrap().join("alias")).unwrap();
        config.nginx_conf_dir=actual.join("conf.d");
        let test=temp.path().join("payload/.test");
        let Rebased::Ready(bytes)=rebase_main(b"http { include alias/../conf.d/*.conf; }",&main,&config,&test).unwrap() else {panic!("OS-resolved managed include must be rebased")};
        let mut expected=b"http { ".to_vec();
        expected.extend(include(&test.join("conf/*.conf")).unwrap());
        expected.extend_from_slice(b" }");
        assert_eq!(bytes,expected);
        assert!(matches!(rebase_main(b"http { include alias/../conf.d/server-*.conf; }",&main,&config,&test).unwrap(),Rebased::Skip(_)));
    }
    #[test]
    fn missing_parent_before_dotdot_skips_staged_validation_instead_of_guessing() {
        let (_temp,config,main,generation,id)=filesystem_fixture();
        let source=b"events {} http { include missing/../conf.d/*.conf; }";
        std::fs::write(&main,source).unwrap();
        assert!(matches!(rebase_main(source,&main,&config,&generation.join(".test")).unwrap(),Rebased::Skip(_)));
        assert!(matches!(prepare_main(&main,&config,&generation,&id).unwrap(),PreparedMain::LiveFallback(_)));
        assert!(!main.parent().unwrap().join(format!(".nwui-staged-{id}")).exists());
    }
    #[test]
    fn non_directory_include_component_skips_instead_of_refusing_the_upgrade() {
        let (_temp,config,main,generation,id)=filesystem_fixture();
        std::fs::write(main.parent().unwrap().join("not-a-directory"),b"ordinary file").unwrap();
        let source=b"events {} http { include not-a-directory/conf.d/*.conf; }";
        std::fs::write(&main,source).unwrap();
        assert!(matches!(rebase_main(source,&main,&config,&generation.join(".test")).unwrap(),Rebased::Skip(_)));
        assert!(matches!(prepare_main(&main,&config,&generation,&id).unwrap(),PreparedMain::LiveFallback(_)));
    }
    #[test]
    fn staged_main_keeps_conf_prefix_and_cleanup_preserves_the_original() {
        let (_temp,config,main,generation,id)=filesystem_fixture();
        let original=std::fs::read(&main).unwrap();
        let PreparedMain::Staged(staged)=prepare_main(&main,&config,&generation,&id).unwrap() else {panic!("writable fixture")};
        assert_eq!(staged.path.parent(),main.parent());
        assert_eq!(staged.path.file_name().unwrap(),std::ffi::OsStr::new(&format!(".nwui-staged-{id}")));
        let meta=std::fs::metadata(&staged.path).unwrap();
        assert_eq!((meta.uid(),meta.gid(),meta.mode() & 0o7777),(0,0,0o600));
        let text=std::fs::read(&staged.path).unwrap();
        assert!(text.windows(b"include mime.types;".len()).any(|part|part==b"include mime.types;"));
        assert_eq!(std::fs::read(&main).unwrap(),original);
        let path=staged.path.clone();staged.remove().unwrap();
        assert!(!path.exists());assert!(!generation.join(".test/main-record.json").exists());
        assert_eq!(std::fs::read(&main).unwrap(),original);
    }
    #[test]
    fn generation_gc_removes_its_journaled_crash_leftover() {
        let (_temp,config,main,generation,id)=filesystem_fixture();
        let PreparedMain::Staged(mut staged)=prepare_main(&main,&config,&generation,&id).unwrap() else {panic!("writable fixture")};
        let path=staged.path.clone();staged.armed=false;drop(staged);
        assert!(path.exists());cleanup_generation(&generation).unwrap();
        assert!(!path.exists());assert!(main.exists());
    }
    #[test]
    fn preexisting_staged_name_is_never_adopted_or_removed() {
        let (_temp,config,main,generation,id)=filesystem_fixture();
        let path=main.parent().unwrap().join(format!(".nwui-staged-{id}"));
        std::fs::write(&path,b"unrelated existing file").unwrap();
        assert!(prepare_main(&main,&config,&generation,&id).is_err());
        assert_eq!(std::fs::read(path).unwrap(),b"unrelated existing file");
        assert!(!generation.join(".test/main-record.json").exists());
    }
    #[test]
    fn sibling_staged_name_cannot_match_the_live_config_globs() {
        let name=CString::new(".nwui-staged-11111111-1111-4111-8111-111111111111").unwrap();
        for pattern in ["*","*.conf"] {
            let pattern=CString::new(pattern).unwrap();
            assert_ne!(unsafe {libc::fnmatch(pattern.as_ptr(),name.as_ptr(),libc::FNM_PERIOD)},0);
        }
    }
}
