//! Admission covers what root executes and what each binary's compiled config loads.
use crate::{config::RuntimeConfig, db_upgrade::key_file::{self, KeyFileError, TrustedInput}, upgrade::StepError};
use std::{collections::BTreeSet, ffi::{CString, OsStr}, fs, io::Read, os::unix::{ffi::OsStrExt, fs::MetadataExt}, path::{Path, PathBuf}, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

fn unsafe_input(path: &Path) -> StepError { KeyFileError::Unsafe.trusted_input_failure(path) }
// Message selection happens only after the shared validator has refused the input.
fn input_failure(error: KeyFileError, path: &Path) -> StepError {
    if fs::symlink_metadata(path).is_ok_and(|meta|meta.file_type().is_symlink()) {
        return StepError{code:"trusted_input_unsafe",message:"심볼릭 링크 대신 실제 파일 경로를 사용하세요.",detail:Some(path.display().to_string())};
    }
    if error==KeyFileError::Missing {
        return StepError{code:"trusted_input_unsafe",message:"파일을 찾을 수 없습니다.",detail:Some(format!("file not found: {}",path.display()))};
    }
    error.trusted_input_failure(path)
}
fn absolute_lookup(path: &Path) -> StepError {
    StepError {code:"trusted_input_unsafe", message:"PATH 항목과 경로를 지정한 NGINX_BIN에는 비어 있지 않은 절대 경로를 사용하세요.", detail:Some(path.display().to_string())}
}
fn safe_directory(path: &Path) -> Result<(),StepError> {
    let mut selected = path;
    loop {
        match key_file::validate_config_parent(&selected.join(".nwui-admission")) {
            Ok(_) => return Ok(()),
            Err(KeyFileError::Missing) => selected = selected.parent().ok_or_else(||unsafe_input(path))?,
            Err(error) => return Err(input_failure(error,path)),
        }
    }
}
fn resolve(binary: &Path, search: &OsStr) -> Result<PathBuf,StepError> {
    if binary.is_absolute() {
        key_file::validate_trusted_input(binary,TrustedInput::Executable).map_err(|error|input_failure(error,binary))?;
        return Ok(binary.into());
    }
    if binary.as_os_str().as_bytes().contains(&b'/') || binary.as_os_str().is_empty() {return Err(absolute_lookup(binary));}
    for directory in std::env::split_paths(search) {
        if !directory.is_absolute() {return Err(absolute_lookup(&directory));}
        safe_directory(&directory)?;
        let candidate = directory.join(binary);
        let name = CString::new(candidate.as_os_str().as_bytes()).map_err(|_|unsafe_input(&candidate))?;
        if fs::metadata(&candidate).is_ok_and(|meta|meta.is_file()) && unsafe{libc::access(name.as_ptr(),libc::X_OK)}==0 {
            key_file::validate_trusted_input(&candidate,TrustedInput::Executable).map_err(|error|input_failure(error,&candidate))?;
            return Ok(candidate);
        }
    }
    Err(unsafe_input(binary))
}

#[derive(Debug,PartialEq,Eq)]
struct Compiled {prefix:PathBuf, main:PathBuf}
fn compiled(output: &str) -> Option<Compiled> {
    if !output.lines().any(|line|line.strip_prefix("nginx version: ").is_some_and(|version|!version.trim().is_empty())) {return None;}
    let arguments = output.lines().find_map(|line|line.strip_prefix("configure arguments:"))?;
    // Configure metadata uses shell quoting, never expansion or command execution.
    let mut words = Vec::new(); let mut word = String::new(); let mut quote = None; let mut escape = false;
    for c in arguments.chars() {
        if escape {word.push(c); escape=false;continue;}
        if c=='\\' && quote!=Some('\'') {escape=true;continue;}
        if let Some(q)=quote {if c==q {quote=None;}else{word.push(c);}continue;}
        match c {
            '\''|'"'=>quote=Some(c),
            ' '| '\t' | '\r' | '\n'=>if !word.is_empty(){words.push(std::mem::take(&mut word));},
            _=>word.push(c),
        }
    }
    if quote.is_some() || escape {return None;}
    if !word.is_empty(){words.push(word);}
    let mut prefix = PathBuf::from("/usr/local/nginx"); let mut conf = PathBuf::from("conf/nginx.conf");
    for word in words {
        if !word.starts_with("--") {return None;}
        if let Some(value)=word.strip_prefix("--prefix=") {prefix=value.into();}
        if let Some(value)=word.strip_prefix("--conf-path=") {conf=value.into();}
    }
    if !prefix.is_absolute() || conf.as_os_str().is_empty() {return None;}
    let main=if conf.is_absolute(){conf}else{prefix.join(conf)};
    Some(Compiled {prefix,main})
}
async fn version(binary: &Path, timeout: Duration) -> Result<Compiled,StepError> {
    let mut child = Command::new(binary).arg("-V").env_clear().stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).process_group(0)
        .spawn().map_err(|_|unsafe_input(binary))?;
    let mut stdout=child.stdout.take().unwrap(); let mut stderr=child.stderr.take().unwrap();
    let operation=async {
        let mut output=Vec::new();let(mut out_open,mut err_open)=(true,true);let(mut out,mut err)=([0;4096],[0;4096]);
        while out_open || err_open {
            let bytes=tokio::select! {
                n=stdout.read(&mut out),if out_open=>{let n=n.map_err(|_|unsafe_input(binary))?;out_open=n!=0;&out[..n]},
                n=stderr.read(&mut err),if err_open=>{let n=n.map_err(|_|unsafe_input(binary))?;err_open=n!=0;&err[..n]},
            };
            if output.len()+bytes.len()>65536 {return Err(unsafe_input(binary));}
            output.extend_from_slice(bytes);
        }
        if !child.wait().await.map_err(|_|unsafe_input(binary))?.success(){return Err(unsafe_input(binary));}
        compiled(std::str::from_utf8(&output).map_err(|_|unsafe_input(binary))?).ok_or_else(||unsafe_input(binary))
    };
    let result=match tokio::time::timeout(timeout,operation).await {Ok(result)=>result,Err(_)=>Err(unsafe_input(binary))};
    if result.is_err() {
        if let Some(pid)=child.id(){unsafe{libc::kill(-(pid as i32),libc::SIGKILL);}}
        let _=child.kill().await;let _=child.wait().await;
    }
    result
}

// Bounds apply before glob expansion/read allocation; normal nginx configs are far smaller.
const MAX_ENTRIES: usize=4096;
const MAX_FILES: usize=256;
const MAX_BYTES: usize=4*1024*1024;
fn includes(pattern: &Path) -> Result<Vec<PathBuf>,StepError> {
    if !pattern.as_os_str().as_bytes().iter().any(|c|matches!(c,b'*'|b'?'|b'[')) {
        return Ok(vec![pattern.into()]);
    }
    let mut paths=vec![PathBuf::from("/")];let mut entries=0;let mut expanded=false;
    for component in pattern.components().skip(1) {
        let raw=component.as_os_str();
        if !raw.as_bytes().iter().any(|c|matches!(c,b'*'|b'?'|b'[')) {
            use std::os::unix::ffi::OsStringExt;
            let mut literal=Vec::new();let mut bytes=raw.as_bytes().iter();
            while let Some(&c)=bytes.next() {
                if c==b'\\' {let Some(&escaped)=bytes.next() else{return Ok(Vec::new());};literal.push(escaped);}else{literal.push(c);}
            }
            let literal=std::ffi::OsString::from_vec(literal);let mut next=Vec::new();
            for parent in paths {
                if expanded {safe_directory(&parent)?;}
                let path=parent.join(&literal);
                if !expanded || fs::symlink_metadata(&path).is_ok(){next.push(path);}
            }
            paths=next;continue;
        }
        expanded=true;
        let name=CString::new(raw.as_bytes()).map_err(|_|unsafe_input(pattern))?;
        let mut next=Vec::new();
        for parent in paths {
            if fs::metadata(&parent).is_ok_and(|meta|!meta.is_dir()) {continue;}
            safe_directory(&parent)?;
            let dir=match fs::read_dir(&parent){Ok(dir)=>dir,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>continue,Err(_)=>return Err(unsafe_input(&parent))};
            for entry in dir {
                entries+=1;if entries>MAX_ENTRIES{return Err(unsafe_input(pattern));}
                let entry=entry.map_err(|_|unsafe_input(&parent))?;
                let candidate=CString::new(entry.file_name().as_bytes()).map_err(|_|unsafe_input(&parent))?;
                if unsafe{libc::fnmatch(name.as_ptr(),candidate.as_ptr(),libc::FNM_PERIOD)}==0 {next.push(entry.path());}
            }
        }
        next.sort(); paths=next;
    }
    Ok(paths)
}
fn configuration(layout: &Compiled) -> Result<(),StepError> {
    use crate::control::guard_targets::{End, statements};
    fn visit(path:&Path,layout:&Compiled,seen:&mut BTreeSet<PathBuf>,bytes:&mut usize,depth:usize)->Result<(),StepError> {
        if depth>32 || seen.len()>=MAX_FILES {return Err(unsafe_input(path));}
        if !seen.insert(path.into()){return Ok(());}
        let file=key_file::validate_trusted_input(path,TrustedInput::Config).map_err(|error|input_failure(error,path))?;
        let mut text=String::new(); file.take((MAX_BYTES-*bytes+1) as u64).read_to_string(&mut text).map_err(|_|unsafe_input(path))?;
        *bytes+=text.len();if *bytes>MAX_BYTES{return Err(unsafe_input(path));}
        let mut context=0usize;
        for statement in statements(&text) {
            match statement.end {
                End::Open=>context+=1,
                End::Close=>context=context.saturating_sub(1),
                End::Semicolon if context==0 && statement.words.len()==2=> {
                    let input=Path::new(&statement.words[1]);
                    match statement.words[0].as_str() {
                        "include"=> {
                            let pattern=if input.is_absolute(){input.into()}else{layout.main.parent().unwrap().join(input)};
                            for included in includes(&pattern)? {visit(&included,layout,seen,bytes,depth+1)?;}
                        },
                        "load_module"=> {
                            let module=if input.is_absolute(){input.into()}else{layout.prefix.join(input)};
                            key_file::validate_trusted_input(&module,TrustedInput::Module).map_err(|error|input_failure(error,&module))?;
                        },
                        _=>{},
                    }
                },
                _=>{},
            }
        }
        Ok(())
    }
    visit(&layout.main,layout,&mut BTreeSet::new(),&mut 0,0)
}

#[derive(Clone,Debug)]
pub(crate) struct Admitted {
    pub master_bin:PathBuf,
    pub control_bin:PathBuf,
    pub conf_path:PathBuf, pub conf_mismatch:bool,
}

pub(crate) async fn admit_nginx(config:&RuntimeConfig) -> Result<Admitted,StepError> {
    static ADMITTED:tokio::sync::OnceCell<Admitted>=tokio::sync::OnceCell::const_new();
    ADMITTED.get_or_try_init(||async {
        let search=std::env::var_os("PATH").or_else(||super::startup_value("PATH").map(Into::into)).ok_or_else(||absolute_lookup(Path::new("PATH")))?;
        admit_with_search(config,search).await
    }).await.cloned()
}

/// The binary crate needs only success/failure, never the crate-private admitted paths.
pub async fn admit_guard(config:&RuntimeConfig) -> Result<(),StepError> {
    admit_nginx(config).await.map(|_|())
}

async fn admit_with_search(config:&RuntimeConfig,search:std::ffi::OsString) -> Result<Admitted,StepError> {
    let master=crate::control::internal_targets::blocking({let search=search.clone();move||resolve(Path::new("nginx"),OsStr::new(&search))}).await.map_err(|_|unsafe_input(Path::new("nginx")))??;
    let control=crate::control::internal_targets::blocking({let search=search.clone();let bin=config.nginx_bin.clone();move||resolve(&bin,OsStr::new(&search))}).await.map_err(|_|unsafe_input(&config.nginx_bin))??;
    let layout=version(&master,Duration::from_secs(5)).await?;
    if fs::metadata(&master).ok().zip(fs::metadata(&control).ok()).is_none_or(|(a,b)|a.dev()!=b.dev() || a.ino()!=b.ino()) {
        let control_layout=version(&control,Duration::from_secs(5)).await?;
        if layout!=control_layout {
            return Err(StepError{code:"trusted_input_unsafe",message:"nginx와 NGINX_BIN의 prefix 및 conf-path가 같아야 합니다.",detail:Some(format!("{}; {}",master.display(),control.display()))});
        }
    }
    let admitted=Admitted{master_bin:master.clone(),control_bin:control,conf_path:layout.main.clone(),conf_mismatch:config.nginx_main_conf!=layout.main};
    crate::control::internal_targets::blocking(move||configuration(&layout)).await.map_err(|_|unsafe_input(&master))??;
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn fixture() -> tempfile::TempDir {
        assert_eq!(unsafe{libc::geteuid()},0,"trusted-input checks require the root test container");
        let dir=tempfile::tempdir().unwrap(); fs::set_permissions(dir.path(),fs::Permissions::from_mode(0o700)).unwrap();dir
    }
    fn file(path:&Path,text:&str,mode:u32) {
        fs::write(path,text).unwrap();fs::set_permissions(path,fs::Permissions::from_mode(mode)).unwrap();
    }
    #[tokio::test]
    #[ignore = "bundled nginx image: NWUI_N2_NATIVE_NGINX=1"]
    async fn bundled_nginx_admission_checks_the_actual_binaries_and_main_config() {
        if std::env::var_os("NWUI_N2_NATIVE_NGINX").as_deref()!=Some(OsStr::new("1")) {
            eprintln!("native admission requires NWUI_N2_NATIVE_NGINX=1 in the bundled nginx image");
            return;
        }
        let config=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://postgres@127.0.0.1/postgres".into()),
            "AUTH_SECRET"=>Some("fixture-only".into()),_=>None,
        }).unwrap();
        // The builder PATH contains writable Cargo tools absent from the product image.
        admit_with_search(&config,"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()).await.unwrap();
    }
    fn fake_version(path:&Path,prefix:&Path,main:&Path) {
        file(path,&format!("#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = -V ] || exit 91\nprintf '%s\\n' 'nginx version: nginx/1.30.6' 'configure arguments: --prefix={} --conf-path={}' >&2\n",prefix.display(),main.display()),0o700);
    }
    fn admission_fixture() -> (tempfile::TempDir,RuntimeConfig,PathBuf,PathBuf) {
        let root=fixture();let main=root.path().join("main.conf");file(&main,"events {}",0o600);
        let master=root.path().join("nginx");let control=root.path().join("control");
        fake_version(&master,root.path(),&main);fake_version(&control,root.path(),&main);
        let mut config=RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://fixture@localhost/fixture".into()),"AUTH_SECRET"=>Some("fixture-only".into()),_=>None}).unwrap();
        config.nginx_bin=control.clone();config.nginx_main_conf=main.clone();
        (root,config,master,main)
    }
    #[tokio::test]
    async fn different_binaries_return_their_agreed_compiled_paths() {
        let (root,config,master,main)=admission_fixture();
        let got=admit_with_search(&config,root.path().as_os_str().into()).await.unwrap();
        assert_eq!(got.master_bin,master);assert_eq!(got.control_bin,config.nginx_bin);
        assert_eq!(got.conf_path,main);assert!(!got.conf_mismatch);
    }
    #[tokio::test]
    async fn different_binaries_must_agree_on_both_prefix_and_conf_path() {
        let (root,config,master,main)=admission_fixture();
        let other_prefix=root.path().join("other-prefix");fs::create_dir(&other_prefix).unwrap();
        let other_main=root.path().join("other.conf");file(&other_main,"events {}",0o600);
        for (prefix,conf) in [(other_prefix.as_path(),main.as_path()),(root.path(),other_main.as_path())] {
            fake_version(&config.nginx_bin,prefix,conf);
            let error=admit_with_search(&config,root.path().as_os_str().into()).await.unwrap_err();
            assert_eq!(error.code,"trusted_input_unsafe");
            let detail=error.detail.unwrap();assert!(detail.contains(master.to_str().unwrap()));assert!(detail.contains(config.nginx_bin.to_str().unwrap()));
        }
    }
    #[tokio::test]
    async fn configured_conf_difference_is_a_flag_and_never_selects_that_file() {
        let (root,mut config,_master,main)=admission_fixture();config.nginx_main_conf=root.path().join("not-the-effective-config");
        let got=admit_with_search(&config,root.path().as_os_str().into()).await.unwrap();
        assert_eq!(got.conf_path,main);assert!(got.conf_mismatch);assert!(!config.nginx_main_conf.exists());
    }

    #[test]
    fn low_symlink_refusal_points_to_the_real_file_without_repairing_it() {
        let root=fixture();let target=root.path().join("real.conf");file(&target,"events {}",0o600);
        let link=root.path().join("linked.conf");std::os::unix::fs::symlink(&target,&link).unwrap();
        let error=configuration(&Compiled{prefix:root.path().into(),main:link.clone()}).unwrap_err();
        assert_eq!(error.code,"trusted_input_unsafe");
        assert!(error.message.contains("실제 파일 경로"),"{error:?}");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(target).unwrap(),"events {}");
        let binary=root.path().join("real-nginx");file(&binary,"#!/bin/sh\nexit 0\n",0o700);
        let link=root.path().join("linked-nginx");std::os::unix::fs::symlink(&binary,&link).unwrap();
        assert!(resolve(&link,OsStr::new("/usr/bin")).unwrap_err().message.contains("실제 파일 경로"));
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
    #[test]
    fn low_literal_include_and_module_report_the_missing_path() {
        let root=fixture();let main=root.path().join("main.conf");let missing=root.path().join("absent.conf");
        for directive in ["include","load_module"] {
            file(&main,&format!("{directive} absent.conf;"),0o600);
            let error=configuration(&Compiled{prefix:root.path().into(),main:main.clone()}).unwrap_err();
            assert_eq!(error.code,"trusted_input_unsafe");
            assert_eq!(error.detail.as_deref(),Some(format!("file not found: {}",missing.display()).as_str()));
            assert!(!missing.exists());
        }
    }

    #[test]
    fn compiled_paths_follow_prefix_and_shell_quoted_metadata() {
        assert_eq!(compiled("nginx version: nginx/1.30.6\nconfigure arguments:\n"),Some(Compiled{prefix:"/usr/local/nginx".into(),main:"/usr/local/nginx/conf/nginx.conf".into()}));
        assert_eq!(compiled("nginx version: nginx/1.30.6\nconfigure arguments: --prefix='/opt/with space' --conf-path=etc/nginx.conf\n"),Some(Compiled{prefix:"/opt/with space".into(),main:"/opt/with space/etc/nginx.conf".into()}));
        assert_eq!(compiled("nginx version: nginx/1.30.6\nconfigure arguments: --prefix=/opt/nginx --conf-path=/etc/nginx.conf"),Some(Compiled{prefix:"/opt/nginx".into(),main:"/etc/nginx.conf".into()}));
        for raw in ["", "nginx version: \nconfigure arguments:", "nginx version: nginx/1\nconfigure arguments: nonsense", "configure arguments: --prefix=/opt", "nginx version: nginx/1\nconfigure arguments: --prefix=relative", "nginx version: nginx/1\nconfigure arguments: --conf-path=", "nginx version: nginx/1\nconfigure arguments: --prefix='unfinished"] {assert!(compiled(raw).is_none(),"{raw}");}
    }
    #[test]
    fn path_lookup_refuses_shadowing_and_cwd_dependent_entries_only_before_match() {
        let root=fixture();let bin=root.path().join("nginx");file(&bin,"#!/bin/sh\nexit 0\n",0o700);
        assert_eq!(resolve(Path::new("nginx"),root.path().as_os_str()).unwrap(),bin);
        assert_eq!(resolve(Path::new("nginx"),OsStr::new(&format!("{}:relative",root.path().display()))).unwrap(),bin);
        for path in [format!(":{}",root.path().display()),format!("relative:{}",root.path().display())] {assert_eq!(resolve(Path::new("nginx"),OsStr::new(&path)).unwrap_err().code,"trusted_input_unsafe");}
        assert!(resolve(Path::new("bin/nginx"),root.path().as_os_str()).is_err());
        let shadow=root.path().join("shadow");fs::create_dir(&shadow).unwrap();fs::set_permissions(&shadow,fs::Permissions::from_mode(0o777)).unwrap();
        assert!(resolve(Path::new("nginx"),OsStr::new(&format!("{}:{}",shadow.display(),root.path().display()))).is_err());
        assert!(resolve(Path::new("nginx"),OsStr::new(&format!("/tmp/missing-nwui-{}:{}",uuid::Uuid::new_v4(),root.path().display()))).is_err());
        std::os::unix::fs::chown(&bin,Some(10001),None).unwrap();assert!(resolve(&bin,root.path().as_os_str()).is_err());
    }
    #[test]
    fn main_includes_use_main_directory_and_modules_use_binary_prefix() {
        let root=fixture();let prefix=root.path().join("prefix");let config=root.path().join("config");
        fs::create_dir(&prefix).unwrap();fs::create_dir(&config).unwrap();fs::create_dir(config.join("nested")).unwrap();
        let main=config.join("main.conf");file(&main,"include nested/first.conf;\nhttp {include ignored-http.conf;}\n",0o600);
        file(&config.join("nested/first.conf"),"include second.conf;",0o600);
        file(&config.join("second.conf"),r"load_module mod\nule.so;",0o600);
        let module=prefix.join("mod\nule.so");file(&module,"not loaded during static validation",0o600);
        let layout=Compiled{prefix,main}; configuration(&layout).unwrap();
        std::os::unix::fs::chown(&module,Some(10001),None).unwrap();
        let error=configuration(&layout).unwrap_err();assert_eq!(error.code,"trusted_input_unsafe");assert_eq!(error.detail.as_deref(),module.to_str());
    }
    #[test]
    fn unmatched_glob_still_refuses_a_writable_include_directory() {
        let root=fixture();let conf=root.path().join("extra");fs::create_dir(&conf).unwrap();
        let pattern=conf.join("*.conf");assert!(includes(&pattern).unwrap().is_empty());
        fs::set_permissions(&conf,fs::Permissions::from_mode(0o777)).unwrap();assert!(includes(&pattern).is_err());
    }
    #[test]
    fn glob_order_escaping_and_non_directory_matches_follow_nginx() {
        let root=fixture();let conf=root.path().join("extra");fs::create_dir(&conf).unwrap();
        file(&root.path().join("not-a-directory"),"",0o600);
        for name in ["b.conf","a.conf",".hidden.conf"] {file(&conf.join(name),"",0o600);}
        assert_eq!(includes(&root.path().join("*/*.conf")).unwrap(),vec![conf.join("a.conf"),conf.join("b.conf")]);
        file(&conf.join("a[1].conf"),"",0o600);
        assert_eq!(includes(&conf.join(r"a\[1\]*")).unwrap(),vec![conf.join("a[1].conf")]);
    }
    #[test]
    fn include_cycles_are_bounded_and_oversized_input_refuses() {
        let root=fixture();let main=root.path().join("main.conf");file(&main,"include main.conf;",0o600);
        let layout=Compiled{prefix:root.path().into(),main:main.clone()};configuration(&layout).unwrap();
        file(&main,&"#".repeat(MAX_BYTES+1),0o600);assert!(configuration(&layout).is_err());
    }
    #[tokio::test]
    async fn version_is_cleared_bounded_and_never_passes_a_config_argument() {
        let root=fixture();let bin=root.path().join("nginx");
        file(&bin,"#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = -V ] && [ -z \"${HOME+x}\" ] || exit 9\nprintf 'nginx version: nginx/1.30.6\\nconfigure arguments: --prefix=/etc/nginx\\n' >&2\n",0o700);
        assert_eq!(version(&bin,Duration::from_secs(2)).await.unwrap().main,Path::new("/etc/nginx/conf/nginx.conf"));
        file(&bin,"#!/bin/sh\nwhile :; do printf '0123456789abcdef0123456789abcdef'; done\n",0o700);
        assert!(version(&bin,Duration::from_secs(2)).await.is_err());
        let pid_file=root.path().join("pid");
        file(&bin,&format!("#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",pid_file.display()),0o700);
        let before=std::time::Instant::now();assert!(version(&bin,Duration::from_millis(100)).await.is_err());assert!(before.elapsed()<Duration::from_secs(2));
        if let Ok(pid)=fs::read_to_string(&pid_file) {
            assert_eq!(unsafe{libc::kill(pid.parse().unwrap(),0)},-1);
            assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(libc::ESRCH));
        }
        file(&bin,"#!/bin/sh\nexit 1\n",0o700);assert!(version(&bin,Duration::from_secs(2)).await.is_err());
    }
}
