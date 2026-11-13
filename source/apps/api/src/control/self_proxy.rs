use super::internal_targets::{Resolve, reaches_this_container};
use super::internal_socket::is_unix_target;
pub const ADMIN_UI_PORT: u16 = 8080;

pub fn points_at_admin_ui_with(target: &str, resolve: Resolve) -> bool {
    if is_unix_target(target.trim_matches(super::whitespace)) {return false;}
    let Ok(url) = url::Url::parse(target.trim_matches(super::whitespace)) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") || url.port() != Some(ADMIN_UI_PORT) {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // Legacy UI hints use loopback; trusted source binding below also covers assigned interfaces.
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || reaches_this_container(host, ADMIN_UI_PORT, resolve)
}

pub(crate) fn points_at_admin_ui_for_location(
    location: &serde_json::Value,
    resolve: Resolve,
) -> super::Result<bool> {
    let target = location["proxyPass"].as_str().unwrap_or("");
    if super::validation::parse_runtime_dns_flag(location)? {
        // The runtime DNS guard rejects loopback answers; opted-in names never use NSS here.
        Ok(points_at_admin_ui_with(target, &|_, _| Vec::new()))
    } else {
        Ok(points_at_admin_ui_with(target, resolve))
    }
}

pub(crate) fn requires_admin_headers(
    location: &serde_json::Value,
    resolve: Resolve,
) -> super::Result<bool> {
    let runtime=super::validation::parse_runtime_dns_flag(location)?;
    if runtime {
        if is_unix_target(location["proxyPass"].as_str().unwrap_or("")) {return Ok(false);}
        // Runtime port 8080 is sanitized without NSS; these headers are safe for external upstreams too.
        let target = location["proxyPass"].as_str().unwrap_or("");
        return Ok(
            url::Url::parse(target.trim_matches(super::whitespace)).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https") && url.port() == Some(ADMIN_UI_PORT)
            }),
        );
    }
    points_at_admin_ui_for_location(location, resolve)
}

/// Only known admin endpoints can use the trusted source. Runtime names never enter NSS.
pub(crate) fn binds_admin_target_with(target: &str, runtime: bool, resolve: Resolve, is_local: super::internal_targets::LocalAddr) -> bool {
    if is_unix_target(target) {return false;}
    let Ok(url)=url::Url::parse(target) else{return false;};
    if !matches!(url.scheme(),"http"|"https") || url.port()!=Some(ADMIN_UI_PORT) {return false;}
    if runtime && matches!(url.host(),Some(url::Host::Domain(_))) {return false;}
    url.host_str().is_some_and(|host| super::internal_targets::loops_to_own_listener_with(host,ADMIN_UI_PORT,&[ADMIN_UI_PORT],resolve,is_local))
}

pub(crate) fn admin_member_with(entry: &str, runtime: bool, resolve: Resolve, is_local: super::internal_targets::LocalAddr) -> bool {
    let endpoint=entry.split_whitespace().next().unwrap_or("");
    binds_admin_target_with(&format!("http://{endpoint}"),runtime,resolve,is_local)
}

#[cfg(test)]
mod alias_tests {
    use super::*;
    #[test]
    fn unix_endpoints_never_resolve_or_bind_as_admin_even_with_the_relative_8080_name() {
        let resolve=|_:&str,_:u16|->Vec<std::net::IpAddr>{panic!("Unix is not a DNS target")};
        let local=|_:std::net::IpAddr|->bool{panic!("Unix is not a TCP listener")};
        for target in ["http://unix:8080","http://unix:/var/run/app.sock:/api/","https://unix:/tmp/x.sock"] {
            assert!(!points_at_admin_ui_with(target,&resolve));
            assert!(!binds_admin_target_with(target,false,&resolve,&local));
            for runtime in [false,true] {assert!(!requires_admin_headers(&serde_json::json!({"proxyPass":target,"resolveAtRuntime":runtime}),&resolve).unwrap());}
        }
        assert!(!admin_member_with("unix:8080 weight=2",false,&resolve,&local));
    }
    #[test]
    fn mapped_alias_uses_existing_resolution_and_external_answers_do_not_match() {
        let resolve = |host: &str, port| {
            assert_eq!((host, port), ("mapped.test", 8080));
            vec!["127.0.0.2".parse().unwrap()]
        };
        assert!(points_at_admin_ui_with("http://mapped.test:8080", &resolve));
        assert!(!points_at_admin_ui_with(
            "http://mapped.test:8080",
            &|_, _| vec!["192.0.2.1".parse().unwrap()]
        ));
        assert!(!points_at_admin_ui_with(
            "http://127.0.0.2:9000",
            &|_, _| panic!("non-admin port must never resolve")
        ));
    }
    #[test]
    fn conservative_runtime_headers_do_not_claim_a_known_admin_ui() {
        let location = serde_json::json!({"path":"/","proxyPass":"http://external.test:8080","resolveAtRuntime":true});
        let never =
            |_: &str, _| -> Vec<std::net::IpAddr> { panic!("runtime DNS must not call NSS") };
        assert!(!points_at_admin_ui_for_location(&location, &never).unwrap());
        let cfg = serde_json::json!({"locations":[location]});
        assert_eq!(
            super::super::validation::validate_server_config_with(&cfg, &never).unwrap(),
            vec![true]
        );
    }
}

#[cfg(test)]
mod fxb3_tests {
    #[test]
    fn fxb3_shipped_admin_and_status_trust_use_distinct_sources() {
        let conf=include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../docker/nginx.conf"));
        let parsed=super::super::guard_targets::statements(conf);
        let trusted:Vec<_>=parsed.iter().filter(|s|s.words.first().is_some_and(|w|w=="set_real_ip_from")).map(|s|s.words[1].as_str()).collect();assert_eq!(trusted,vec!["127.0.0.2"]);
        let allowed:Vec<_>=parsed.iter().filter(|s|s.words.first().is_some_and(|w|w=="allow")).map(|s|s.words[1].as_str()).collect();assert_eq!(allowed,vec!["127.0.0.3"]);
        assert!(conf.contains("127.0.0.2:https"));assert!(!conf.contains("\"127.0.0.1:https\""));
        let mut depth=0;for statement in &parsed {use super::super::guard_targets::End;match statement.end {End::Open=>depth+=1,End::Close=>depth-=1,End::Semicolon=>if statement.words.first().is_some_and(|name|name=="proxy_set_header"){assert!(depth>=3,"base http/server header inheritance needs an explicit preservation rule");}}}

    }
}

#[cfg(test)]
mod q23_tests {
    #[test]
    fn q23_unix_target_is_never_admin_bound_or_resolved() {
        let never=|_:&str,_:u16|panic!("Unix admin classification must not resolve");
        for target in ["http://unix:8080","http://unix:/var/run/app.sock:/api/","https://unix:/tmp/x.sock"] {
            assert!(!super::points_at_admin_ui_with(target,&never));
            assert!(!super::binds_admin_target_with(target,false,&never,&|_|panic!("Unix admin classification must not probe TCP")));
        }
    }
}
