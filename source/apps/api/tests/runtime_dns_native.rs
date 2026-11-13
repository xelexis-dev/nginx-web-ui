//! Emits real renderer output for the bundled-nginx static/dynamic equivalence gate.
use nwui_api::control::render::{render_reverse_proxy, render_reverse_proxy_with_dns};
use serde_json::{Value, json};

#[test]
fn native_adapter_matrix_fixtures_use_the_real_renderer() {
    let header = |name: &str, value: &str| json!({"name":"proxy_set_header","args":[name,value]});
    let redirect = |args: Value| json!({"name":"proxy_redirect","args":args});
    let mut cases = vec![];
    for (name, target) in [
        ("no-uri", "http://fixture.test:19100"),
        ("slash", "http://fixture.test:19100/"),
        ("prefix", "http://fixture.test:19100/app/"),
        ("escaped", "http://fixture.test:19100/a%2Fb/"),
        ("query", "http://fixture.test:19100/app/?seed=x%2Fy"),
        ("default-port", "http://fixture.test:80/app/"),
        ("spelled-port", "http://fixture.test:019100/app/"),
        ("https-default", "https://fixture.test:443/app/"),
        ("https-sni", "https://fixture.test:19101/app/"),
        ("https-verify", "https://fixture.test:19101/app/"),
    ] {
        cases.push(json!({"name":name,"target":target}));
    }
    for (name, target) in [
        ("default-port-proxy-host", "http://fixture.test:80/app/"),
        (
            "https-default-port-proxy-host",
            "https://fixture.test:443/app/",
        ),
        ("spelled-port-proxy-host", "http://fixture.test:019100/app/"),
        ("case-sensitive-authority", "http://FiXtUrE.TeSt:19100/app/"),
        (
            "query-without-slash",
            "http://fixture.test:19100?seed=x%2Fy",
        ),
    ] {
        cases.push(json!({"name":name,"target":target,"directives":[header("Host","$proxy_host"),header("X-Proxy","${proxy_host}:$proxy_port")]}));
    }
    for (name, dirs) in [
        ("default-host", json!([])),
        (
            "empty-host",
            json!([header("Host",""),{"name":"proxy_http_version","args":["1.0"]}]),
        ),
        ("custom-host", json!([header("Host", "custom.example")])),
        (
            "proxy-vars",
            json!([
                header("Host", "${proxy_host}"),
                header("X-Proxy", "$proxy_host:${proxy_port}"),
                header("X-Escaped", r"\$proxy_host"),
                header("X-Double", r"\\$proxy_host"),
                header("X-Literal", "proxy_host")
            ]),
        ),
        ("redirect-default", json!([redirect(json!(["default"]))])),
        ("redirect-off", json!([redirect(json!(["off"]))])),
        (
            "redirect-custom",
            json!([
                redirect(json!(["http://unmatched/", "/never/"])),
                redirect(json!(["http://fixture.test:19100/app/", "/custom/"]))
            ]),
        ),
        (
            "redirect-vars",
            json!([redirect(json!([
                "http://$proxy_host/app/",
                "/custom/$proxy_port/"
            ]))]),
        ),
        (
            "rewrite",
            json!([{"name":"rewrite","args":["^/nested/(.*)$","/rewritten/$1","break"]},header("X-Proxy","$proxy_host")]),
        ),
        (
            "rewrite-vars",
            json!([{"name":"rewrite","args":["^/nested/(.*)$","/rewritten/$proxy_port/$1","break"]}]),
        ),
        (
            "return-vars",
            json!([{"name":"return","args":["200","return:$proxy_host:$proxy_port"]}]),
        ),
        (
            "return-response-vars",
            json!([{"name":"return","args":["200","return:$proxy_host:$proxy_port"]},{"name":"add_header","args":["X-Original","$proxy_host:${proxy_port}","always"]}]),
        ),
        ("expires", json!([{"name":"expires","args":["1h"]}])),
        (
            "response-vars",
            json!([{"name":"add_header","args":["X-Original","$host:$request_uri","always"]}]),
        ),
        (
            "response-proxy-vars",
            json!([{"name":"add_header","args":["X-Original","$proxy_host:${proxy_port}","always"]}]),
        ),
        (
            "expires-proxy-vars",
            json!([{"name":"expires","args":["$proxy_port"]}]),
        ),
        ("cache", json!([header("Host", "$proxy_host")])),
    ] {
        cases
            .push(json!({"name":name,"target":"http://fixture.test:19100/app/","directives":dirs}));
    }
    cases.push(json!({"name":"inherited-response-vars","target":"http://fixture.test:19100/app/","serverDirectives":[{"name":"add_header","args":["X-Original","$host:$request_uri","always"]}]}));
    for (name, cache, own, denied) in [
        ("hsts-user", false, false, false),
        ("hsts-cache", true, false, false),
        ("hsts-explicit-location", true, true, false),
        ("hsts-only", false, false, false),
        ("hsts-denied", true, false, true),
        ("denied-response-vars", false, true, true),
    ] {
        let mut case = json!({"name":name,"target":"http://fixture.test:19100/app/","frontTls":true,"cache":cache,"denied":denied});
        if name != "hsts-only" {
            case["serverDirectives"] =
                json!([{"name":"add_header","args":["X-Extra","value","always"]}]);
        }
        if own {
            case["directives"] =
                json!([{"name":"add_header","args":["X-Own","$host:$request_uri","always"]}]);
        }
        cases.push(case);
    }
    for name in [
        "denied-response-proxy-vars",
        "denied-expires-proxy-vars",
        "denied-inherited-response-proxy-vars",
    ] {
        let directive = if name == "denied-expires-proxy-vars" {
            json!({"name":"expires","args":["$proxy_port"]})
        } else {
            json!({"name":"add_header","args":["X-Original","$proxy_host:$proxy_port","always"]})
        };
        let mut case = json!({"name":name,"target":"http://fixture.test:19100/app/","denied":true});
        if name == "denied-inherited-response-proxy-vars" {
            case["serverDirectives"] = json!([directive]);
        } else {
            case["directives"] = json!([directive]);
        }
        cases.push(case);
    }
    let mut fixtures = vec![];
    for (i, case) in cases.into_iter().enumerate() {
        let mut pair = vec![];
        for dynamic in [false, true] {
            let index = 2 * i + usize::from(dynamic);
            let mut location =
                json!({"path":"/nested/","proxyPass":case["target"],"resolveAtRuntime":dynamic});
            if let Some(d) = case.get("directives") {
                location["directives"] = d.clone();
            }
            if case["denied"] == true {
                location["ipAccess"] = json!({"mode":"blacklist","entries":["127.0.0.1"]});
            }
            let mut cfg = json!({"serverId":format!("cc7b4664-6833-4d98-9625-{:012x}",index),"serverName":"fixture.test","listen":19200+index,"locations":[location]});
            if let Some(d) = case.get("serverDirectives") {
                cfg["serverDirectives"] = d.clone();
            }
            if case["frontTls"] == true {
                cfg["tls"] = json!({"certPath":"/tmp/cert.pem","keyPath":"/tmp/key.pem","tlsPort":19200+index,"redirectHttp":false,"tuning":{"hsts":{"enabled":true,"maxAge":31536000,"includeSubDomains":true,"preload":true}}});
            }
            if case["name"] == "cache" || case["cache"] == true {
                cfg["cache"] = json!({"enabled":true,"statusHeader":true});
            }
            let rendered = if dynamic {
                render_reverse_proxy_with_dns(&cfg, Some("127.0.0.1:15354".parse().unwrap()))
            } else {
                render_reverse_proxy(&cfg)
            };
            let rendered = match rendered {
                Ok(value) => value,
                Err(error)
                    if dynamic
                        && matches!(
                            case["name"].as_str(),
                            Some(
                                "return-response-vars"
                                    | "response-proxy-vars"
                                    | "expires-proxy-vars"
                                    | "denied-response-proxy-vars"
                                    | "denied-expires-proxy-vars"
                                    | "denied-inherited-response-proxy-vars"
                            )
                        ) =>
                {
                    assert!(error.to_string().contains("add_header·expires"));
                    pair.push(Value::Null);
                    continue;
                }
                Err(error) => panic!("{}: {error}", case["name"]),
            };
            assert!(rendered.contains("location /nested/"));
            pair.push(json!({"port":19200+index,"config":rendered,"scheme":if case["frontTls"]==true {"https"}else{"http"}}));
        }
        fixtures.push(json!({"case":case,"static":pair[0],"dynamic":pair[1]}));
    }
    if let Ok(path) = std::env::var("NWUI_DNS_MATRIX_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&fixtures).unwrap()).unwrap();
    }
}
