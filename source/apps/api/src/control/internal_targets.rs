//! Proxy and stream targets on this image's own components.
//!
//! Inside the container 127.0.0.1 is the container itself: a reverse proxy, stream or upstream
//! pointing at the web app's, API's or database's port reaches that component directly,
//! bypassing the management nginx — and a proxy to the API lets whoever can edit proxies forge
//! the forwarded headers the API trusts from loopback. Refused only when running in the image
//! (`NWUI_CONTAINER`); elsewhere a loopback port belongs to someone else's service. The
//! management UI itself stays a valid direct HTTP target, where rendering sanitizes headers.
use std::net::{IpAddr, Ipv6Addr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde_json::Value;

use super::resources::{Resource, field_error};
use super::self_proxy::ADMIN_UI_PORT;
use crate::config::RuntimeConfig;
use crate::error::ApiError;

pub(crate) fn internal_ports(config: &RuntimeConfig) -> Vec<(u16, &'static str)> {
    let database = url::Url::parse(&config.database_url)
        .ok()
        .and_then(|u| u.port())
        .unwrap_or(5432);
    let mut ports=vec![
        (config.web_port, "웹 앱"),
        (config.listen.port(), "API"),
        (database, "데이터베이스"),
    ];
    let dns=crate::dns_guard::status(config);
    if dns.available { ports.push((dns.endpoint.port(), "실행 중 이름 확인 서비스")); }
    ports
}

/// Upstream blocks the image itself defines (`00-runtime-upstreams.conf`, rewritten at boot) for
/// its own components. nginx routes `http://NAME` to an upstream block of that name before any
/// address lookup, so a proxy to one of these reaches this container whatever port it names.
/// The admin server and the ACME snippet use them; users' proxies must not.
pub(crate) const RUNTIME_UPSTREAMS: [(&str, &str); 2] = [("nwui_api", "API"), ("nwui_web", "웹 앱")];

fn check_runtime_upstream(config: &RuntimeConfig, host: &str) -> Result<(), String> {
    if !config.in_container {
        return Ok(());
    }
    match RUNTIME_UPSTREAMS.iter().find(|(name, _)| host.eq_ignore_ascii_case(name)) {
        Some((name, owner)) => Err(format!(
            "{name} 은(는) 이 컨테이너의 {owner}(으)로 가는 내부 upstream 이름이라 대상으로 쓸 수 없습니다. 관리 화면을 연결하려면 http://127.0.0.1:{ADMIN_UI_PORT} 를 쓰세요."
        )),
        None => Ok(()),
    }
}

/// A variable in the host or port (`http://$arg_u`, `http://127.0.0.1:$arg_p`) is resolved per
/// request: nginx routes a value such as `nwui_api` to that upstream block and a loopback port
/// straight to this container, so no check at save time can hold. Variables after the authority
/// (`http://10.0.0.5:8080/$1`, `…?q=$arg_q`) keep the destination fixed and stay allowed.
/// Deliberately strict: `http://backend$request_uri` is refused too — its value is not checked
/// statically — and `http://backend` forwards the request URI anyway.
fn check_fixed_authority(config: &RuntimeConfig, target: &str) -> Result<(), String> {
    if !config.in_container {
        return Ok(());
    }
    let authority = target
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?']).next().unwrap_or(""))
        .unwrap_or("");
    if authority.contains('$') {
        return Err(format!(
            "대상의 호스트·포트 자리에는 변수($…)를 쓸 수 없습니다: 요청마다 이 컨테이너의 내부 upstream 이나 포트로 바뀔 수 있습니다. 변수는 호스트 뒤 `/` 다음(경로·쿼리)에 쓰세요. `http://backend$request_uri` 대신 `http://backend` 라고 쓰면 nginx 가 요청 경로를 그대로 넘깁니다."
        ));
    }
    Ok(())
}

fn this_container(ip: IpAddr) -> bool {
    let v4 = match ip {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(v6) if v6.is_loopback() || v6.is_unspecified() => return true,
        // IPv4-mapped and -compatible forms reach the same IPv4 address.
        IpAddr::V6(v6) => match v6.to_ipv4() {
            Some(v4) => v4,
            None => return false,
        },
    };
    v4.is_loopback() || v4.is_unspecified()
}

fn literal_ip(host: &str) -> Option<IpAddr> {
    let host = host.trim_matches(['[', ']']);
    host.parse().ok().or_else(|| match url::Host::parse(host).ok()? {
        url::Host::Ipv4(ip) => Some(ip.into()),
        url::Host::Ipv6(ip) => Some(ip.into()),
        url::Host::Domain(_) => None,
    })
}

fn metadata_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_link_local() || ip.octets() == [100, 100, 100, 200],
        IpAddr::V6(ip) => ip.is_unicast_link_local()
            || ip == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)
            || ip.to_ipv4().is_some_and(|ip| metadata_address(ip.into())),
    }
}

fn checked_addresses(host: &str, port: u16, resolve: Resolve) -> Result<Vec<IpAddr>, String> {
    let addresses = literal_ip(host).map(|ip| vec![ip]).unwrap_or_else(|| resolve(host, port));
    if addresses.iter().copied().any(metadata_address) {
        return Err("링크 로컬·클라우드 메타데이터 주소는 프록시 대상으로 쓸 수 없습니다.".into());
    }
    Ok(addresses)
}

/// Only a row's own ports need interface classification; other private peers stay usable.
pub(crate) type LocalAddr<'a> = &'a dyn Fn(IpAddr) -> bool;

pub(crate) fn is_local_addr(ip: IpAddr) -> bool {
    if this_container(ip) { return true; }
    let ip = match ip { IpAddr::V6(ip) => ip.to_ipv4().map(IpAddr::V4).unwrap_or(ip.into()), _ => ip };
    this_container(ip) || UdpSocket::bind((ip, 0)).is_ok()
}

pub(crate) fn loops_to_own_listener(host: &str, port: u16, own: &[u16], resolve: Resolve) -> bool {
    loops_to_own_listener_with(host, port, own, resolve, &is_local_addr)
}
pub(crate) fn loops_to_own_listener_with(host: &str, port: u16, own: &[u16], resolve: Resolve, is_local: LocalAddr) -> bool {
    if !own.contains(&port) { return false; }
    if host.trim_end_matches('.').eq_ignore_ascii_case("localhost") { return true; }
    match literal_ip(host) {
        Some(ip) => is_local(ip),
        None => resolve(host, port).into_iter().any(is_local),
    }
}

pub(crate) fn target_authority(target: &str) -> Option<&str> {
    if super::internal_socket::is_unix_target(target){return None;}
    let (scheme, rest) = target.split_once("://")?;
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then(|| rest.split(['/', '?', '#']).next().unwrap_or(""))
}

pub(crate) fn referenced_upstream<'a>(upstreams: &'a [Value], id: Option<&str>, target: Option<&str>) -> Option<&'a Value> {
    upstreams.iter().find(|row| match id {
        Some(id) => row["id"].as_str() == Some(id),
        None => target.and_then(target_authority).is_some_and(|host| row["name"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(host))),
    })
}

pub(crate) fn check_listener_endpoint(host: &str, port: u16, own: &[u16], resolve: Resolve) -> Result<(), String> {
    check_listener_endpoint_with(host, port, own, resolve, &is_local_addr)
}
fn check_listener_endpoint_with(host: &str, port: u16, own: &[u16], resolve: Resolve, is_local: LocalAddr) -> Result<(), String> {
    if !own.contains(&port) { return Ok(()); }
    let addresses = checked_addresses(host, port, resolve)?;
    if loops_to_own_listener_with(host, port, own, &|_, _| addresses.clone(), is_local) {
        Err(format!("이 서버 자신의 포트 {port}로 보내면 요청이 자기 자신에게 무한히 되돌아옵니다."))
    } else { Ok(()) }
}

pub(crate) fn check_listener_host_port(entry: &str, own: &[u16], resolve: Resolve) -> Result<(), String> {
    check_listener_host_port_with(entry, own, resolve, &is_local_addr)
}
fn check_listener_host_port_with(entry: &str, own: &[u16], resolve: Resolve, is_local: LocalAddr) -> Result<(), String> {
    if super::internal_socket::is_unix_target(entry){return Ok(());}
    let address = entry.split_whitespace().next().unwrap_or("");
    if let Some((host, port)) = address.rsplit_once(':').and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h, p))) {
        check_listener_endpoint_with(host, port, own, resolve, is_local)?;
    }
    Ok(())
}

pub(crate) fn check_listener_targets(config: &RuntimeConfig, kind: Resource, built: &Value, upstreams: &[Value]) -> Result<(), String> {
    check_listener_targets_with(config, kind, built, upstreams, &system_resolve, &is_local_addr)
}
pub(crate) fn check_listener_targets_with(config: &RuntimeConfig, kind: Resource, built: &Value, upstreams: &[Value], resolve: Resolve, is_local: LocalAddr) -> Result<(), String> {
    if !config.in_container || kind == Resource::Upstream { return Ok(()); }
    let mut own: Vec<_> = built["listen"].as_u64().and_then(|p| u16::try_from(p).ok()).into_iter().collect();
    if built["certificateId"].as_str().is_some() {
        own.push(built["config"]["tlsPort"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(443));
    }
    let empty = |_: &str, _: u16| Vec::new();
    if kind == Resource::Stream {
        if let Some(target) = built["proxyPass"].as_str() {
            check_listener_host_port_with(target, &own, if built["resolveAtRuntime"] == true { &empty } else { resolve }, is_local)?;
        }
        return Ok(());
    }
    let check = |id: Option<&str>, target: Option<&str>, dynamic: bool| -> Result<(), String> {
        if let Some(upstream) = referenced_upstream(upstreams, id, target) {
            let resolve: Resolve = if upstream["resolveAtRuntime"] == true { &empty } else { resolve };
            for member in upstream["servers"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                check_listener_host_port_with(member, &own, resolve, is_local).map_err(|e| format!("upstream {}: {e}", upstream["name"].as_str().unwrap_or("?")))?;
            }
        } else if id.is_none() {
            if let Some(url) = target.filter(|s|!super::internal_socket::is_unix_target(s)).and_then(|s| url::Url::parse(s).ok()) {
                if let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) {
                    check_listener_endpoint_with(host, port, &own, if dynamic { &empty } else { resolve }, is_local)?;
                }
            }
        }
        Ok(())
    };
    check(built["upstreamId"].as_str(), built["config"]["proxyPass"].as_str(), built["config"]["resolveAtRuntime"] == true)?;
    for (index, location) in built["config"]["locations"].as_array().into_iter().flatten().enumerate() {
        check(location["upstreamId"].as_str(), location["proxyPass"].as_str(), location["resolveAtRuntime"] == true)
            .map_err(|e| format!("location {} ({}): {e}", index + 1, location["path"].as_str().unwrap_or("?")))?;
    }
    Ok(())
}

// Cross-server cycles are report-only.
// shortcut: runtime answers to own interfaces remain unclassified; revisit with connect-time checks.

/// A name lookup is blocking and may wait on an unreachable DNS server; callers hold the apply
/// lock or a DB transaction, so it runs on its own thread for at most this long. A name that
/// does not resolve in time is not refused here: nginx cannot resolve it either when it loads
/// the configuration, so the apply fails instead of reaching this container.
const LOOKUP_LIMIT: Duration = Duration::from_secs(2);

/// Addresses a name resolves to. Production uses [`system_resolve`]; tests pass their own
/// answers to model a name whose DNS answer changes.
pub type Resolve<'a> = &'a dyn Fn(&str, u16) -> Vec<IpAddr>;

/// nginx resolves a target with the system resolver when it loads the configuration, so the
/// same resolver decides here: an IPv4-mapped address, the short form `127.1`, `/etc/hosts`
/// names such as `ip6-localhost`, and DNS names that answer 127.0.0.1 all count — comparing
/// strings let those through. Empty when the lookup fails or takes longer than the limit.
pub fn system_resolve(host: &str, port: u16) -> Vec<IpAddr> {
    let (tx, rx) = std::sync::mpsc::channel();
    let name = host.to_owned();
    std::thread::spawn(move || {
        let addresses = (name.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|a| a.ip()).collect())
            .unwrap_or_default();
        let _ = tx.send(addresses);
    });
    rx.recv_timeout(LOOKUP_LIMIT).unwrap_or_default()
}

/// Shared async boundary for target checks that may enter the blocking system resolver.
pub async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ApiError::internal())
}

/// Only called for a target on an internal port, so ordinary targets are never looked up. A
/// name resolves again on every call, including the post-validation/post-reload and restart
/// scans. A clean answer is evidence at that boundary, not proof against DNS rebinding.
pub(crate) fn reaches_this_container(host: &str, port: u16, resolve: Resolve) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(ip) = literal_ip(host) {
        return this_container(ip);
    }
    resolve(host, port).into_iter().any(this_container)
}

fn check(config: &RuntimeConfig, host: &str, port: u16, resolve: Resolve) -> Result<(), String> {
    if literal_ip(host).is_some_and(metadata_address) {
        return Err("링크 로컬·클라우드 메타데이터 주소는 프록시 대상으로 쓸 수 없습니다.".into());
    }
    if !config.in_container {
        return Ok(());
    }
    let Some((port, owner)) = internal_ports(config).into_iter().find(|(p, _)| *p == port) else {
        return Ok(());
    };
    let addresses = checked_addresses(host, port, resolve)?;
    if !reaches_this_container(host, port, &|_, _| addresses.clone()) {
        return Ok(());
    }
    Err(format!(
        "포트 {port} 는 이 컨테이너의 {owner} 포트라 대상으로 쓸 수 없습니다. 관리 화면을 연결하려면 http://127.0.0.1:{ADMIN_UI_PORT} 를 쓰세요."
    ))
}

/// A proxy_pass URL (`http://host[:port]/…`). Whether it is a valid URL is checked elsewhere.
pub fn check_url(config: &RuntimeConfig, target: &str) -> Result<(), String> {
    check_url_with(config, target, &system_resolve)
}
pub fn check_url_with(config: &RuntimeConfig, target: &str, resolve: Resolve) -> Result<(), String> {
    check_fixed_authority(config, target)?;
    if super::internal_socket::is_unix_target(target){return Ok(());}
    let Ok(url) = url::Url::parse(target) else { return Ok(()) };
    if let Some(host) = url.host_str() {
        check_runtime_upstream(config, host)?;
    }
    match (url.host_str(), url.port_or_known_default()) {
        (Some(host), Some(port)) => check(config, host, port, resolve),
        _ => Ok(()),
    }
}

/// `host:port`, optionally followed by parameters (stream proxy_pass, upstream server entries).
pub fn check_host_port(config: &RuntimeConfig, entry: &str) -> Result<(), String> {
    check_host_port_with(config, entry, &system_resolve)
}
pub fn check_host_port_with(config: &RuntimeConfig, entry: &str, resolve: Resolve) -> Result<(), String> {
    if super::internal_socket::is_unix_target(entry){return Ok(());}
    let address = entry.split_whitespace().next().unwrap_or("");
    let Some((host, port)) = address.rsplit_once(':') else { return Ok(()) };
    match port.parse::<u16>() {
        Ok(port) => {
            check(config, host, port, resolve)?;
            if config.in_container && port == ADMIN_UI_PORT {
                checked_addresses(host, port, resolve)?;
            }
            Ok(())
        },
        Err(_) => Ok(()),
    }
}

/// Runtime DNS names are checked by the filtered listener, not NSS at save/start time.
/// Literals still pass the existing local-component check; W2 rejects literals for direct mode.
pub fn check_dynamic_url(config:&RuntimeConfig,target:&str)->Result<(),String> {
    if super::internal_socket::is_unix_target(target){return Err("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다.".into());}
    check_url_with(config,target,&|_,_|Vec::new())
}
pub fn check_dynamic_host_port(config:&RuntimeConfig,entry:&str)->Result<(),String> {
    if super::internal_socket::is_unix_target(entry){return Err("unix 소켓 대상에는 실행 중 이름 확인을 쓸 수 없습니다.".into());}
    let address=entry.split_whitespace().next().unwrap_or("");
    if let Some((host,_))=address.rsplit_once(':') {check_runtime_upstream(config,host.trim_matches(['[',']']))?;}
    check_host_port_with(config,entry,&|_,_|Vec::new())
}
/// Distinguish real DNS names from literal/ambiguous numeric authorities without a lookup.
pub fn is_dns_endpoint(entry:&str)->bool {
    if super::internal_socket::is_unix_target(entry){return false;}
    let address=entry.split_whitespace().next().unwrap_or("");
    let Some((host,port))=address.rsplit_once(':') else{return false;};
    if !port.parse::<u16>().is_ok_and(|p|p>0)||host.contains('$'){return false;}
    matches!(url::Host::parse(host),Ok(url::Host::Domain(_)))
        &&hickory_resolver::proto::rr::Name::from_ascii(host).is_ok()
}

/// Resolve against the transaction's upstream rows before the ordinary target checks.
pub fn check_built_with_upstreams(config:&RuntimeConfig,kind:Resource,built:&Value,upstreams:&[Value])->Result<(),ApiError>{
    check_listener_targets(config,kind,built,upstreams).map_err(|m|field_error(if kind==Resource::Stream {"proxyPass"}else{"locations"},m))?;
    let mut resolved=built.clone();
    if kind==Resource::Server{resolved["config"]=super::location_refs::resolve_locations(&built["config"],&super::location_refs::upstream_names(upstreams)).map_err(|e|field_error("locations",e.to_string()))?;}
    check_built_targets(config,kind,&resolved,upstreams)
}

/// Save-time check of a parsed row, as a field error on the offending field.
pub fn check_built(config: &RuntimeConfig, kind: Resource, built: &Value) -> Result<(), ApiError> {
    check_listener_targets(config,kind,built,&[]).map_err(|m|field_error(if kind==Resource::Server && built["config"]["locations"].is_array(){"locations"}else{"proxyPass"},m))?;
    check_built_targets(config,kind,built,&[])
}

fn check_built_targets(config: &RuntimeConfig, kind: Resource, built: &Value, upstreams: &[Value]) -> Result<(), ApiError> {
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    match kind {
        Resource::Upstream => {
            for server in strings(&built["servers"]) {
                let check=if built["resolveAtRuntime"]==true {check_dynamic_host_port}else{check_host_port};
                check(config, &server).map_err(|m| field_error("servers", m))?;
            }
        }
        Resource::Stream => {
            if let Some(target) = built["proxyPass"].as_str() {
                let check=if built["resolveAtRuntime"]==true {check_dynamic_host_port}else{check_host_port};
                check(config, target).map_err(|m| field_error("proxyPass", m))?;
            }
        }
        Resource::Server => {
            if let Some(target) = built["config"]["proxyPass"].as_str().filter(|target| referenced_upstream(upstreams,None,Some(target)).is_none()) {
                let check=if built["config"]["resolveAtRuntime"]==true {check_dynamic_url}else{check_url};
                check(config, target).map_err(|m| field_error("proxyPass", m))?;
            }
            for (i, location) in built["config"]["locations"].as_array().into_iter().flatten().enumerate() {
                if location["upstreamId"].as_str().is_some(){continue;}
                if let Some(target) = location["proxyPass"].as_str().filter(|target| referenced_upstream(upstreams,None,Some(target)).is_none()) {
                    // Named like the other location errors (1.17.2): `location N (/path): …`.
                    let path = location["path"].as_str().unwrap_or("?");
                    let check=if location["resolveAtRuntime"]==true {check_dynamic_url}else{check_url};
                    check(config, target)
                        .map_err(|m| field_error("locations", format!("location {} ({path}): {m}", i + 1)))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod dns_tests {
    use super::*;
    use crate::dns_guard::Descriptor;
    use serde_json::json;
    fn configured() -> (RuntimeConfig,tempfile::TempDir) {
        let dir=tempfile::tempdir().unwrap();
        let mut config=RuntimeConfig::read(|key|match key {
            "DATABASE_URL"=>Some("postgres://test:test@127.0.0.1/test".into()),
            "AUTH_SECRET"=>Some("test-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None,
        }).unwrap();
        config.target_dns.descriptor_path=dir.path().join("dns.json");
        std::fs::write(&config.target_dns.descriptor_path,serde_json::to_vec(&Descriptor::available(&config.target_dns)).unwrap()).unwrap();
        (config,dir)
    }
    #[test]
    fn opted_in_dns_names_defer_lookup_but_static_names_still_use_static_checks() {
        let (config,_dir)=configured();
        let dynamic=json!({"config":{"proxyPass":"http://localhost:3001","resolveAtRuntime":true}});
        assert!(check_built(&config,Resource::Server,&dynamic).is_ok(),"future DNS answers are checked by the guard, never NSS at save time");
        let literal=json!({"config":{"proxyPass":"http://127.0.0.1:3001","resolveAtRuntime":true}});
        assert!(check_built(&config,Resource::Server,&literal).is_err());
        let static_name=json!({"config":{"proxyPass":"http://localhost:3001"}});
        assert!(check_built(&config,Resource::Server,&static_name).is_err());
    }
    #[test]
    fn guard_port_is_an_internal_target_only_when_it_is_available() {
        let (config,_dir)=configured();
        assert!(check_url(&config,"http://127.0.0.1:5354").is_err());
        assert!(check_url(&config,"http://127.0.0.1:8080").is_ok());
        std::fs::write(&config.target_dns.descriptor_path,serde_json::to_vec(&Descriptor::unavailable(&config.target_dns,"existing listener")).unwrap()).unwrap();
        assert!(check_url(&config,"http://127.0.0.1:5354").is_ok(),"first-upgrade conflict must not reinterpret existing static targets");
    }
}

#[cfg(test)]
mod scheduler_tests {
    #[tokio::test(flavor="current_thread")]
    async fn audit_e6_i16_target_resolver_does_not_park_the_async_executor() {
        use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
        let progressed=Arc::new(AtomicBool::new(false));
        let (started,ready)=tokio::sync::oneshot::channel();
        let observer=progressed.clone();
        let task=tokio::spawn(async move {let _=ready.await;observer.store(true,Ordering::SeqCst);});
        let responsive=super::blocking(move || {
            let dir=tempfile::tempdir().unwrap();
            let mut config=crate::config::RuntimeConfig::read(|key|match key {
                "DATABASE_URL"=>Some("postgres://test:test@127.0.0.1/test".into()),
                "AUTH_SECRET"=>Some("test-only".into()),
                "NWUI_CONTAINER"=>Some("1".into()),
                "DATA_KEY_FILE"=>Some(dir.path().join("key").display().to_string()),
                _=>None,
            }).unwrap();
            config.target_dns.descriptor_path=dir.path().join("target-dns.json");
            let started=std::sync::Mutex::new(Some(started));
            let checked=super::check_url_with(&config,"http://slow.example:3001",&|_,_| {
                if let Some(tx)=started.lock().unwrap().take(){let _=tx.send(());}
                std::thread::sleep(std::time::Duration::from_millis(100));
                vec!["127.0.0.1".parse().unwrap()]
            });
            assert!(checked.is_err(),"target security validation must still reject internal addresses");
            progressed.load(Ordering::SeqCst)
        }).await.unwrap();
        task.await.unwrap();
        assert!(responsive,"a stalled target resolver must not block other async work");
    }
}

#[cfg(test)]
mod fxb_security_tests {
    use super::*;
    use serde_json::json;
    fn config() -> RuntimeConfig {
        RuntimeConfig::read(|key| match key {
            "DATABASE_URL" => Some("postgres://test@127.0.0.1/db".into()),
            "AUTH_SECRET" => Some("test-only".into()),
            "NWUI_CONTAINER" => Some("1".into()),
            _ => None,
        }).unwrap()
    }
    #[test]
    fn fxb_w3_direct_and_stream_self_edges_are_refused() {
        let cfg=config();
        for host in ["127.0.0.1","127.0.0.2","127.1","0.0.0.0","localhost","localhost.","[::1]","[::ffff:127.0.0.1]"] {
            for port in [18086,18443] {
                let row=json!({"listen":18086,"certificateId":"cert","config":{"tlsPort":18443,"proxyPass":format!("http://{host}:{port}")}});
                assert!(check_built(&cfg,Resource::Server,&row).is_err(),"server {host}:{port} loops");
            }
            let row=json!({"listen":19000,"proxyPass":format!("{host}:19000")});
            assert!(check_built(&cfg,Resource::Stream,&row).is_err(),"stream {host} loops");
        }
        assert!(check_built(&cfg,Resource::Server,&json!({"listen":18086,"config":{"proxyPass":"http://10.0.0.5:18086"}})).is_ok());
    }
    #[test]
    fn fxb_w3_named_member_is_checked_in_its_consuming_server() {
        let cfg=config();let upstreams=vec![json!({"id":"up","name":"backend","servers":["127.0.0.1:18086"]})];
        let row=json!({"listen":18086,"config":{"locations":[{"path":"/","upstreamId":"up"}]}});
        assert!(check_built_with_upstreams(&cfg,Resource::Server,&row,&upstreams).is_err());
        let other=json!({"listen":18087,"config":{"locations":[{"path":"/","upstreamId":"up"}]}});
        assert!(check_built_with_upstreams(&cfg,Resource::Server,&other,&upstreams).is_ok());
    }
    #[test]
    fn fxb3_legacy_admin_members_and_streams_remain_accepted() {
        let cfg=config();
        for entry in ["127.0.0.1:8080","localhost:8080"] {
            assert!(check_built(&cfg,Resource::Upstream,&json!({"servers":[entry]})).is_ok(),"legacy member {entry}");
            assert!(check_built(&cfg,Resource::Stream,&json!({"listen":19000,"proxyPass":entry})).is_ok(),"legacy stream {entry}");
        }
        assert!(check_url(&cfg,"http://127.0.0.1:8080").is_ok(),"protected direct UI publishing stays valid");
        assert!(check_host_port(&cfg,"10.0.0.5:8080").is_ok(),"another service's8080 stays valid");
    }
    #[test]
    fn fxb_b2_checks_existing_dns_answers_without_adding_dynamic_lookups() {
        let cfg=config(); let metadata=|_:&str,_| vec!["100.100.100.200".parse().unwrap()];
        assert!(check_url_with(&cfg,"http://metadata.test:3001",&metadata).is_err());
        assert!(check_host_port_with(&cfg,"metadata.test:8080",&metadata).is_err());
        assert!(check_listener_endpoint("metadata.test",18086,&[18086],&metadata).is_err());
        let never=|_:&str,_| panic!("runtime name must not enter NSS");
        assert!(check_listener_endpoint("runtime.test",18086,&[18086],&|_,_|Vec::new()).is_ok());
        assert!(!loops_to_own_listener("external.test",18087,&[18086],&never));
        assert!(check_dynamic_host_port(&cfg,"runtime.test:8080").is_ok());
    }
    #[test]
    fn fxb_b2_link_local_literals_and_mapped_forms_are_refused() {
        let cfg=config();let empty=|_:&str,_|Vec::new();
        for host in ["169.254.169.254","169.254.1.2","2852039166","0xa9fea9fe","[fe80::1]","[febf::1]","[::ffff:169.254.169.254]","100.100.100.200","[fd00:ec2::254]","[::ffff:100.100.100.200]"] {
            assert!(check_url_with(&cfg,&format!("http://{host}/metadata"),&empty).is_err(),"url {host}");
            assert!(check_host_port_with(&cfg,&format!("{host}:80"),&empty).is_err(),"endpoint {host}");
        }
        for host in ["10.0.0.5","192.168.0.2","[fd00::2]","[fec0::1]","100.100.100.201","100.64.0.1"] {
            assert!(check_url_with(&cfg,&format!("http://{host}:9000"),&empty).is_ok(),"private {host}");
        }
    }
}

#[cfg(test)]
mod q23_tests {
    use super::*;
    #[test]
    fn q23_unix_targets_never_enter_dns_or_tcp_listener_checks() {
        let config=RuntimeConfig::read(|key|match key {"DATABASE_URL"=>Some("postgres://fixture@127.0.0.1/db".into()),"AUTH_SECRET"=>Some("fixture-only".into()),"NWUI_CONTAINER"=>Some("1".into()),_=>None}).unwrap();
        let no_lookup=|_:&str,_:u16|panic!("Unix target reached DNS");
        for target in ["http://unix:/tmp/x.sock","http://unix:/var/run/app.sock:/api/","http://unix:3001"] {assert!(check_url_with(&config,target,&no_lookup).is_ok());}
        for target in ["unix:3001","unix:8080 weight=2"] {assert!(check_host_port_with(&config,target,&no_lookup).is_ok());assert!(check_listener_host_port(target,&[3001,8080],&no_lookup).is_ok());}
    }
}
