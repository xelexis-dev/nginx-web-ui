//! Field messages the reverse proxy, stream and upstream forms show. They match
//! 1.17.2 field by field (`packages/node-api/src/dto/{server,stream,upstream}.ts`,
//! `apps/web/app/(app)/reverse-proxy/{form,service}.ts`): the Korean prompt where
//! 1.17.2 had one, the validator's own text where 1.17.2 passed that through.
use nwui_api::{
    contracts::FormInput,
    control::{
        reserved_ports::ReservedPorts,
        resources::{self, Resource},
    },
};
use serde_json::{Value, json};

fn none() -> ReservedPorts {
    ReservedPorts::default()
}
fn field(result: Result<Value, nwui_api::error::ApiError>, name: &str) -> String {
    let errors = result.expect_err("input is rejected").field_errors.expect("field errors");
    errors.get(name).cloned().unwrap_or_else(|| panic!("no {name} error in {errors:?}"))
}
fn server(extra: Value) -> Value {
    let mut raw = json!({"serverName":"a.test","listen":80,"proxyPass":"http://app:8080"});
    for (k, v) in extra.as_object().unwrap() {
        raw[k] = v.clone();
    }
    raw
}

#[test]
fn server_name_and_ports_use_the_korean_prompts() {
    for name in ["", "   "] {
        assert_eq!(
            field(resources::parse_server(&server(json!({"serverName":name})), &none()), "serverName"),
            "server_name을 입력하세요."
        );
    }
    // A malformed name still names the rejected token, as 1.17.2 did.
    assert_eq!(
        field(resources::parse_server(&server(json!({"serverName":"a;b"})), &none()), "serverName"),
        "Invalid server_name: \"a;b\""
    );
    for listen in [json!(0), json!(-1), json!(65536), json!(1.5), json!("abc"), json!(null)] {
        assert_eq!(
            field(resources::parse_server(&server(json!({"listen":listen})), &none()), "listen"),
            "1-65535 사이의 포트 번호를 입력하세요.",
            "listen {listen}"
        );
    }
    let tls = server(json!({"certificateId":"cert","tls":{"tlsPort":0}}));
    assert_eq!(
        field(resources::parse_server(&tls, &none()), "tlsPort"),
        "1-65535 사이의 HTTPS 포트를 입력하세요."
    );
}

#[test]
fn a_reserved_port_keeps_its_explanation() {
    let reserved = ReservedPorts { process: vec![3000], ..Default::default() };
    let message = field(resources::parse_server(&server(json!({"listen":3000})), &reserved), "listen");
    assert!(message.contains("3000"), "{message}");
    assert_ne!(message, "1-65535 사이의 포트 번호를 입력하세요.");
    let stream = json!({"listen":3000,"protocol":"tcp","proxyPass":"db:5432"});
    let message = field(resources::parse_stream(&stream, &reserved), "listen");
    assert!(message.contains("3000"), "{message}");
}

#[test]
fn stream_fields_use_the_korean_prompts() {
    let stream = |listen: Value, target: &str| json!({"listen":listen,"protocol":"tcp","proxyPass":target});
    assert_eq!(
        field(resources::parse_stream(&stream(json!(0), "db:5432"), &none()), "listen"),
        "1-65535 사이의 포트 번호를 입력하세요."
    );
    assert_eq!(
        field(resources::parse_stream(&stream(json!(15432), " "), &none()), "proxyPass"),
        "proxy_pass 대상을 입력하세요. (host:port)"
    );
    for target in ["db:5432;", "db", "db:0", "http://db:5432"] {
        assert_eq!(
            field(resources::parse_stream(&stream(json!(15432), target), &none()), "proxyPass"),
            "host:port 형식으로 입력하세요. (예: 10.0.0.5:5432)",
            "target {target}"
        );
    }
}

#[test]
fn upstream_name_prompt_only_when_empty() {
    let upstream = |name: &str| json!({"name":name,"servers":["app:8080"]});
    assert_eq!(field(resources::parse_upstream(&upstream(" ")), "name"), "이름을 입력하세요.");
    assert_eq!(
        field(resources::parse_upstream(&upstream("a-b")), "name"),
        "Invalid upstream name (letters, digits, underscore only): \"a-b\""
    );
}

#[test]
fn location_errors_name_the_location_and_its_path_like_1_17_2() {
    let locations = |list: Value| server(json!({"targetMode":"locations","locations":list,"proxyPass":null}));
    // An invalid path is reported without the path it could not accept.
    assert_eq!(
        field(resources::parse_server(&locations(json!([{"path":"api","mode":"direct","proxyPass":"http://app"}])), &none()), "locations"),
        "location 1: Invalid location path (must start with \"/\"): \"api\""
    );
    assert_eq!(
        field(resources::parse_server(&locations(json!([{"path":"/api","mode":"direct","proxyPass":"http://a;b"}])), &none()), "locations"),
        "location 1 (/api): Invalid proxy_pass target: \"http://a;b\""
    );
    assert_eq!(
        field(resources::parse_server(&locations(json!([{"path":"/","mode":"direct","proxyPass":"http://app"},{"path":"/b","mode":"upstream"}])), &none()), "locations"),
        "location 2 (/b): upstream을 선택하세요."
    );
    // 1.17.2 checked the target before the directives.
    let both = json!([{"path":"/x","mode":"direct","proxyPass":"","directives":[{"name":"include","args":["/tmp/evil"]}]}]);
    assert_eq!(
        field(resources::parse_server(&locations(both), &none()), "locations"),
        "location 1 (/x): proxy_pass target must not be empty"
    );
    let directive = json!([{"path":"/x","mode":"direct","proxyPass":"http://app","directives":[{"name":"include","args":["/tmp/evil"]}]}]);
    let message = field(resources::parse_server(&locations(directive), &none()), "locations");
    assert!(message.starts_with("location 1 (/x): "), "{message}");
}

#[test]
fn server_form_names_the_setting_and_location_that_failed() {
    let form = |pairs: &[(&str, &str)]| {
        let mut form = FormInput::default();
        for (k, v) in [("serverName", "a.test"), ("listen", "80"), ("mode", "direct"), ("proxyPass", "http://app")]
            .iter()
            .chain(pairs)
        {
            form.fields.insert((*k).into(), vec![(*v).into()]);
        }
        form
    };
    assert_eq!(
        field(resources::form_body(Resource::Server, &form(&[("cache", "{")])), "cache"),
        "캐시 설정 형식이 올바르지 않습니다."
    );
    assert_eq!(
        field(resources::form_body(Resource::Server, &form(&[("gzip", "{")])), "gzip"),
        "gzip 설정 형식이 올바르지 않습니다."
    );
    let located = form(&[
        ("targetMode", "locations"),
        ("locations", r#"[{"path":"/x","mode":"direct","proxyPass":"http://app","directives":{"advanced":"include /tmp/evil;"}}]"#),
    ]);
    let message = field(resources::form_body(Resource::Server, &located), "locations");
    assert!(message.starts_with("location 1 (/x): "), "{message}");
}
