//! Shared path-only Unix target classification; caller supplies the trusted data roots.
use std::{io,path::{Path,PathBuf}};

fn normalized(path:&Path)->Option<PathBuf> {
    use std::path::Component;
    if !path.is_absolute(){return None;}
    let mut out=PathBuf::from("/");
    for part in path.components() {match part {
        Component::Normal(name)=>out.push(name),
        Component::ParentDir=>{out.pop();},
        Component::RootDir|Component::CurDir=>{},
        Component::Prefix(_)=>return None,
    }}
    if let Ok(rest)=out.strip_prefix("/var/run") {return Some(Path::new("/run").join(rest));}
    Some(out)
}

/// Return the matched component-prefix, not a substring or a DNS/TCP authority.
pub fn matches_lexical(path:&Path,roots:&[&Path])->Option<PathBuf> {
    let path=normalized(path)?;
    if let Ok(rest)=path.strip_prefix("/run") {
        if let Some(first)=rest.components().next() {
            let name=first.as_os_str();let bytes=name.as_encoded_bytes();
            if bytes.starts_with(b"nwui-")||bytes.starts_with(b"s6-rc:")
                ||["nginx-web-ui","postgresql","service","s6","s6-rc","uncaught-logs"].iter().any(|fixed|name==std::ffi::OsStr::new(fixed)) {
                return Some(Path::new("/run").join(name));
            }
        }
    }
    roots.iter().filter_map(|root|normalized(root)).find(|root|path.starts_with(root))
}

/// Root callers propagate non-missing-path I/O errors rather than treating them as safe.
pub fn matches_resolved(path:&Path,roots:&[&Path])->io::Result<Option<PathBuf>> {
    use std::{collections::VecDeque,path::Component};
    let absolute=if path.is_absolute(){path.to_path_buf()}else{return Ok(None)};
    let mut resolved=PathBuf::from("/");
    let mut pending:VecDeque<_>=absolute.components().map(|part|part.as_os_str().to_owned()).collect();
    let mut hops=0;
    while let Some(part)=pending.pop_front() {
        match Path::new(&part).components().next() {
            Some(Component::RootDir)=>resolved=PathBuf::from("/"),
            Some(Component::CurDir)|None=>{},
            Some(Component::ParentDir)=>{resolved.pop();},
            Some(Component::Prefix(_))=>return Err(io::Error::new(io::ErrorKind::InvalidInput,"Unsupported socket path prefix")),
            Some(Component::Normal(name))=>{
                resolved.push(name);
                match std::fs::symlink_metadata(&resolved) {
                    Ok(meta) if meta.file_type().is_symlink()=>{
                        hops+=1;
                        if hops>40 {return Err(io::Error::from_raw_os_error(libc::ELOOP));}
                        let target=std::fs::read_link(&resolved)?;
                        resolved.pop();
                        let mut expanded:VecDeque<_>=target.components().map(|part|part.as_os_str().to_owned()).collect();
                        expanded.append(&mut pending);
                        pending=expanded;
                    }
                    Ok(_)=>{},
                    Err(error) if error.kind()==io::ErrorKind::NotFound=>{},
                    Err(error)=>return Err(error),
                }
            }
        }
    }
    Ok(matches_lexical(&resolved,roots))
}

/// Keep nginx's raw socket path; URL normalization would erase the unix separator.
pub fn http_socket_path(target:&str)->Option<&Path> {
    let (scheme,rest)=target.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http")&&!scheme.eq_ignore_ascii_case("https"){return None;}
    if !rest.get(..5)?.eq_ignore_ascii_case("unix:"){return None;}
    Some(Path::new(rest[5..].split(':').next().unwrap_or("")))
}

pub fn is_unix_target(target:&str)->bool {
    http_socket_path(target).is_some()||target.split_ascii_whitespace().next().and_then(|word|word.get(..5)).is_some_and(|prefix|prefix.eq_ignore_ascii_case("unix:"))
}

#[cfg(test)]
mod q23_tests {
    use super::*;
    #[test]
    fn q23_lexical_closed_prefixes_aliases_and_component_boundaries() {
        let roots=[Path::new("/custom/db"),Path::new("/custom/app")];
        for path in ["/run/nwui-publisher/control.sock","/run/nwui-locks/fdholder.sock","/run/nwui-api-private/socket","/run/nwui-pg-17/.s.PGSQL.5432","/run/nginx-web-ui/x","/run/postgresql/.s.PGSQL.5432","/run/service/api/supervise/control","/run/s6/x","/run/s6-rc/x","/run/s6-rc:servicedirs/x","/run/uncaught-logs/x","/var//run/./nwui-publisher/control.sock","/run/./postgresql/../postgresql/.s.PGSQL.5432","/custom/db/work/socket","/custom/app/socket"] {
            assert!(matches_lexical(Path::new(path),&roots).is_some(),"{path}");
        }
        for path in ["/var/run/app.sock","/tmp/x.sock","/run/nwui","/run/nginx-web-ui-other/x","/run/service-app/x","/run/s6-extra/x","/run/s6-rc2/x","/run/uncaught-logs-extra/x","/custom/database/x","/custom/application/x","8080"] {
            assert!(matches_lexical(Path::new(path),&roots).is_none(),"{path}");
        }
        assert_eq!(matches_lexical(Path::new("/run/./postgresql/../postgresql/x"),&roots).unwrap(),Path::new("/run/postgresql"));
    }
    #[test]
    fn q23_resolved_parent_alias_and_missing_parent() {
        use std::os::unix::fs::symlink;
        let temp=tempfile::tempdir().unwrap();let root=temp.path().join("private-db");std::fs::create_dir(&root).unwrap();
        let alias=temp.path().join("alias");symlink(&root,&alias).unwrap();let roots=[root.as_path()];
        assert_eq!(matches_resolved(&alias.join("socket"),&roots).unwrap(),Some(root.clone()));
        let socket=root.join("socket");std::fs::write(&socket,b"fixture").unwrap();
        let leaf=temp.path().join("alias.sock");symlink(&socket,&leaf).unwrap();
        assert_eq!(matches_resolved(&leaf,&roots).unwrap(),Some(root.clone()));
        let looped=temp.path().join("loop");symlink(&looped,&looped).unwrap();
        assert!(matches_resolved(&looped,&roots).is_err());
        assert!(matches_resolved(&root.join("missing/sub/socket"),&roots).unwrap().is_some());
        assert!(matches_resolved(&temp.path().join("external/missing/socket"),&roots).unwrap().is_none());
    }
    #[cfg(target_os="linux")]
    #[test]
    fn q23_resolved_directory_symlink_into_runtime_locks() {
        use std::os::unix::fs::symlink;
        let runtime=tempfile::Builder::new().prefix("nwui-locks-").tempdir_in("/run").unwrap();
        let host=tempfile::tempdir().unwrap();let alias=host.path().join("mounted");symlink(runtime.path(),&alias).unwrap();
        assert_eq!(matches_resolved(&alias.join("fdholder.sock"),&[]).unwrap(),Some(runtime.path().to_owned()));
    }
}

#[cfg(all(test,target_os="linux"))]
mod a27_1_tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn dangling_internal_leaf_is_matched_even_when_the_target_is_missing(){
        let temp=tempfile::tempdir().unwrap();let alias=temp.path().join("alias.sock");
        assert!(!Path::new("/run/nwui-locks/missing.sock").exists());
        symlink("/run/nwui-locks/missing.sock",&alias).unwrap();
        assert_eq!(matches_resolved(&alias,&[]).unwrap(),Some(PathBuf::from("/run/nwui-locks")));
    }
    #[test]
    fn dangling_internal_parent_is_matched_with_a_missing_suffix(){
        let temp=tempfile::tempdir().unwrap();let alias=temp.path().join("mounted");
        assert!(!Path::new("/run/nwui-x").exists());symlink("/run/nwui-x",&alias).unwrap();
        assert_eq!(matches_resolved(&alias.join("nested/socket"),&[]).unwrap(),Some(PathBuf::from("/run/nwui-x")));
    }
    #[test]
    fn dangling_external_link_is_allowed_and_connects_when_the_backend_starts(){
        use std::{io::{Read,Write},os::unix::net::{UnixListener,UnixStream}};
        let temp=tempfile::tempdir().unwrap();let alias=temp.path().join("alias.sock");symlink("/tmp/app.sock",&alias).unwrap();
        assert!(!alias.exists(),"fixture target must be dangling");assert!(matches_resolved(&alias,&[]).unwrap().is_none());std::fs::remove_file(&alias).unwrap();
        let socket=temp.path().join("app.sock");symlink("app.sock",&alias).unwrap();
        assert!(matches_resolved(&alias,&[]).unwrap().is_none());
        let listener=UnixListener::bind(&socket).unwrap();let mut client=UnixStream::connect(&alias).unwrap();let(mut server,_)=listener.accept().unwrap();server.write_all(b"ok").unwrap();let mut bytes=[0;2];client.read_exact(&mut bytes).unwrap();assert_eq!(&bytes,b"ok");
    }
    #[test]
    fn a_missing_component_does_not_hide_a_later_internal_symlink(){
        let temp=tempfile::tempdir().unwrap();let alias=temp.path().join("alias.sock");symlink("/run/nwui-locks/missing.sock",&alias).unwrap();
        let seed=temp.path().join("seed");assert!(!seed.exists());
        assert_eq!(matches_resolved(&seed.join("../alias.sock"),&[]).unwrap(),Some(PathBuf::from("/run/nwui-locks")));
    }
    #[test]
    fn relative_dangling_link_resolves_dotdot_after_missing_component(){
        let temp=tempfile::tempdir().unwrap();let alias=temp.path().join("alias");symlink("missing/../protected/socket",&alias).unwrap();
        let protected=temp.path().join("protected");assert_eq!(matches_resolved(&alias,&[&protected]).unwrap(),Some(protected));
    }
    #[test]
    fn symlink_loop_and_more_than_forty_hops_are_unprovable(){
        let temp=tempfile::tempdir().unwrap();let looped=temp.path().join("loop");symlink("loop",&looped).unwrap();
        assert_eq!(matches_resolved(&looped,&[]).unwrap_err().raw_os_error(),Some(libc::ELOOP));
        let mut target=PathBuf::from("/run/nwui-x/missing.sock");
        for i in 0..40 {let link=temp.path().join(format!("hop-{i}"));symlink(&target,&link).unwrap();target=link;}
        assert_eq!(matches_resolved(&target,&[]).unwrap(),Some(PathBuf::from("/run/nwui-x")));
        let extra=temp.path().join("extra");symlink(target,&extra).unwrap();assert_eq!(matches_resolved(&extra,&[]).unwrap_err().raw_os_error(),Some(libc::ELOOP));
        let file=temp.path().join("file");std::fs::write(&file,b"not a directory").unwrap();assert!(matches_resolved(&file.join("socket"),&[]).is_err());
    }
    #[test]
    fn relative_socket_does_not_use_the_publisher_service_cwd(){
        if std::env::var_os("NWUI_Q23_RELATIVE_CWD_CHILD").is_some() {
            let cwd=std::env::current_dir().unwrap();assert!(matches_lexical(&cwd,&[]).is_some());
            assert!(matches_resolved(Path::new("8080"),&[]).unwrap().is_none(),"relative nginx socket must not resolve against publisher cwd");
            std::fs::remove_dir(&cwd).unwrap();assert!(std::env::current_dir().is_err());
            assert!(matches_resolved(Path::new("8080"),&[]).unwrap().is_none(),"relative input must not read current_dir");return;
        }
        for prefix in ["nwui-q23-relative-","s6-rc:q23-relative-"] {
            let temp=tempfile::Builder::new().prefix(prefix).tempdir_in("/run").unwrap();
            let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","control::internal_socket::a27_1_tests::relative_socket_does_not_use_the_publisher_service_cwd","--nocapture","--test-threads=1"]).env("NWUI_Q23_RELATIVE_CWD_CHILD","1").current_dir(temp.path()).output().unwrap();
            assert!(output.status.success(),"{}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        }
    }

}
