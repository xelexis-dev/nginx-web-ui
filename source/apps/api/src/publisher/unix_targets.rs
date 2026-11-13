//! Root-authoritative Unix socket isolation shares W-11's existing consuming units.
use crate::control::{guard_targets::{self,End,TargetScan},internal_socket};
use super::containment::{Action,Unit};
use std::{collections::{BTreeMap,BTreeSet},path::Path};

fn refusal(text:&str,roots:&[&Path])->Option<String> {
    for statement in guard_targets::statements(text) {
        if statement.end!=End::Semicolon || !statement.words.first().is_some_and(|name|matches!(name.as_str(),"proxy_pass"|"server")) {continue;}
        let Some(path)=statement.words.get(1).and_then(|target|internal_socket::http_socket_path(target)) else {continue;};
        if path.as_os_str().as_encoded_bytes().contains(&b'$') {
            return Some("unix 소켓 경로에 변수가 있어 안전한 대상을 입증할 수 없어 제외했습니다.".into());
        }
        match internal_socket::matches_resolved(path,roots) {
            Ok(Some(prefix))=>return Some(format!("이 컨테이너 내부 unix 소켓 경로 {}을(를) 가리켜 제외했습니다.",prefix.display())),
            Err(error)=>return Some(format!("unix 소켓 경로를 입증할 수 없어 제외했습니다. ({:?})",error.kind())),
            Ok(None)=>{},
        }
    }
    None
}

/// Called inside the existing blocking scan, before any selected unit can be reloaded.
pub(super) fn extend_scan(conf:&BTreeMap<String,Vec<u8>>,stream:&BTreeMap<String,Vec<u8>>,scan:&mut TargetScan)->Vec<Action> {
    let roots=[Path::new(crate::db_upgrade::paths::APPROOT),Path::new(crate::db_upgrade::paths::PGROOT)];
    let mut actions=Vec::new();let mut groups=BTreeMap::new();
    if let Some(original)=conf.get(guard_targets::UPSTREAMS_FILE) {
        let bytes=scan.upstreams.as_ref().unwrap_or(original);let text=String::from_utf8_lossy(bytes);let mut kept=text.to_string();
        for block in guard_targets::upstream_blocks(&text).iter().rev() {
            if let Some(reason)=refusal(&text[block.range.clone()],&roots) {
                kept.replace_range(block.range.clone(),"");groups.insert(block.name.to_ascii_lowercase(),reason.clone());
                scan.removed_upstreams.push(block.name.clone());actions.push(Action{unit:Unit::UpstreamBlock(block.name.clone()),kept_previous:false,reason});
            }
        }
        if !groups.is_empty() {
            scan.conf.insert(guard_targets::UPSTREAMS_FILE.into());scan.upstreams=Some(kept.into_bytes());
            let unique:BTreeSet<_>=scan.removed_upstreams.drain(..).collect();scan.removed_upstreams.extend(unique);
        }
    }
    for (is_stream,files) in [(false,conf),(true,stream)] {
        for (name,bytes) in files {
            if !is_stream&&name==guard_targets::UPSTREAMS_FILE {continue;}
            let text=String::from_utf8_lossy(bytes);
            let reason=refusal(&text,&roots).or_else(||guard_targets::statements(&text).into_iter().find_map(|statement| {
                if statement.end!=End::Semicolon||statement.words.first().map(String::as_str)!=Some("proxy_pass") {return None;}
                let target=statement.words.get(1)?;
                let name=crate::control::internal_targets::target_authority(target).unwrap_or(target);
                groups.get(&name.to_ascii_lowercase()).cloned()
            }));
            if let Some(reason)=reason {
                if is_stream {scan.stream.insert(name.clone());}else{scan.conf.insert(name.clone());}
                actions.push(Action{unit:if is_stream{Unit::StreamFile(name.clone())}else{Unit::ConfFile(name.clone())},kept_previous:false,reason});
            }
        }
    }
    actions
}

pub(super) fn apply_actions(data:&crate::control::model::ConfigSnapshotData,outcome:&mut crate::control::model::RenderOutcome,actions:&[Action]) {
    for action in actions {
        match &action.unit {
            Unit::ConfFile(name)=>{outcome.rendered.conf.remove(name);},
            Unit::StreamFile(name)=>{outcome.rendered.stream.remove(name);},
            Unit::UpstreamBlock(name)=>{
                if let Some(bytes)=outcome.rendered.conf.get_mut(guard_targets::UPSTREAMS_FILE) {
                    let mut text=String::from_utf8_lossy(bytes).into_owned();
                    if let Some(block)=guard_targets::upstream_blocks(&text).into_iter().find(|block|block.name==*name) {text.replace_range(block.range,"");}
                    *bytes=text.into_bytes();
                }
            },
        }
    }
    super::apply::extend_containment(outcome,data,actions);
}

pub(super) fn filter_outcome(data:&crate::control::model::ConfigSnapshotData,outcome:&mut crate::control::model::RenderOutcome)->Vec<Action> {
    let mut scan=TargetScan::default();let actions=extend_scan(&outcome.rendered.conf,&outcome.rendered.stream,&mut scan);
    apply_actions(data,outcome,&actions);actions
}

pub(super) fn notices(actions:&[Action])->Vec<String> {
    actions.iter().map(|action| {
        let name=match &action.unit {Unit::ConfFile(name)|Unit::StreamFile(name)|Unit::UpstreamBlock(name)=>name};
        format!("{name}: {}",action.reason)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_path_variables_are_unprovable_but_uri_variables_are_not_socket_paths() {
        assert!(refusal("server { location / { proxy_pass http://unix:/tmp/$socket:/api/; } }",&[]).unwrap().contains("변수"));
        assert!(refusal("server { location / { proxy_pass http://unix:/tmp/public.sock:$request_uri; } }",&[]).is_none());
        assert!(refusal("server { location / { add_header X-Text http://unix:/run/nwui-locks/fdholder.sock; } }",&[]).is_none());
    }
    #[test]
    fn existing_leaf_and_parent_symlinks_use_the_shared_resolved_matcher() {
        use std::os::unix::fs::symlink;
        let root=tempfile::tempdir().unwrap();let internal=root.path().join("internal");std::fs::create_dir(&internal).unwrap();
        let socket=internal.join("control.sock");std::fs::write(&socket,b"synthetic socket path metadata").unwrap();
        let leaf=root.path().join("leaf.sock");let parent=root.path().join("mounted");symlink(&socket,&leaf).unwrap();symlink(&internal,&parent).unwrap();
        for path in [leaf,parent.join("control.sock")] {
            assert!(refusal(&format!("server {{ location / {{ proxy_pass http://unix:{}:/api/; }} }}",path.display()),&[&internal]).is_some());
        }
    }
    #[test]
    fn a_path_resolution_io_error_isolates_the_unit_as_unprovable() {
        use std::os::unix::fs::symlink;
        let root=tempfile::tempdir().unwrap();let link=root.path().join("loop");symlink("loop",&link).unwrap();
        let reason=refusal(&format!("server {{ location / {{ proxy_pass http://unix:{}; }} }}",link.display()),&[]).unwrap();
        assert!(reason.contains("입증할 수 없어"));
    }
    #[test]
    fn only_the_consuming_file_is_added_and_no_unix_target_reaches_dns() {
        let mut config=crate::config::RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/test".into()),"AUTH_SECRET"=>Some("unix-isolation-fixture".into()),_=>None}).unwrap();config.in_container=true;
        let conf=BTreeMap::from([
            ("server-internal.conf".into(),b"server { listen 18081; location / { proxy_pass http://unix:/run/nwui-locks/fdholder.sock; } }".to_vec()),
            ("server-public.conf".into(),b"server { listen 18082; location / { proxy_pass http://unix:/tmp/public.sock:$request_uri; } }".to_vec()),
        ]);
        let stream=BTreeMap::from([("stream-relative.conf".into(),b"server { listen 18083; proxy_pass unix:8080; }".to_vec())]);
        let mut scan=guard_targets::scan(&config,&conf,&stream,&|_,_|panic!("Unix targets must not enter DNS/TCP checks"));
        let actions=extend_scan(&conf,&stream,&mut scan);
        assert_eq!(scan.conf,BTreeSet::from(["server-internal.conf".into()]));assert!(scan.stream.is_empty());assert_eq!(actions.len(),1);
        assert!(notices(&actions)[0].contains("server-internal.conf"));
    }
}
