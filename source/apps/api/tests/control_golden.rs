use nwui_api::control::{
    self, ControlError, performance as p, render as r, reserved_ports as ports, validation as v,
};
use serde_json::{Value, json};

#[test]
fn gzip_effective_state_respects_explicit_override_and_absence() {
    assert!(!p::is_gzip_effective(None, None));
    assert!(p::is_gzip_effective(None, Some(true)));
    let gzip = p::parse_gzip_config(&json!({"enabled":true}))
        .unwrap()
        .unwrap();
    assert!(p::is_gzip_effective(Some(&gzip), None));
    assert!(!p::is_gzip_effective(Some(&gzip), Some(false)));
}

#[test]
fn malformed_resource_shapes_return_errors_instead_of_panicking() {
    for cfg in [
        json!(true),
        json!([]),
        json!("invalid"),
        json!({"serverName":"example.com","listen":80,"locations":["invalid"]}),
    ] {
        assert!(r::render_reverse_proxy(&cfg).is_err());
    }
}

#[test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
fn integer_wire_values_render_without_decimal_points_or_negative_zero() {
    let root = std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("nginx/render.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(root).unwrap()).unwrap();
    let vector = fixture["vectors"].as_array().unwrap().iter()
        .find(|v| v["fn"] == "renderReverseProxy" && v["args"][0]["tls"].is_object()).unwrap();
    let mut cfg = decode(&vector["args"][0]);
    cfg["tls"]["tlsPort"] = json!(443.0);
    cfg["tls"]["httpPort"] = json!(80.0);
    cfg["tls"]["tuning"] = json!({"hsts":{"enabled":true,"maxAge":-0.0}});
    let rendered = r::render_reverse_proxy(&cfg).unwrap();
    assert!(rendered.contains("listen 443 ssl;"), "{rendered}");
    assert!(rendered.contains("max-age=0\""), "{rendered}");
    cfg["tls"]["tuning"]["hsts"]["maxAge"] = json!(1e21);
    assert!(r::render_reverse_proxy(&cfg).unwrap().contains("max-age=1000000000000000000000\""));
}

#[test]
#[ignore = "golden vectors: set NWUI_CONTROL_FIXTURES"]
fn control_golden_vectors() {
    let root = std::path::Path::new(&std::env::var("NWUI_CONTROL_FIXTURES").expect("set NWUI_CONTROL_FIXTURES")).join("nginx");
    let mut count = 0;
    for module in [
        "render",
        "validation",
        "performance",
        "reserved-ports",
        "self-proxy",
    ] {
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(root.join(format!("{module}.json"))).unwrap())
                .unwrap();
        for vector in fixture["vectors"].as_array().unwrap() {
            let actual = dispatch(vector["fn"].as_str().unwrap(), &vector["args"]);
            assert_eq!(
                actual,
                // Preserve TS captures while asserting the approved 2.0 security changes.
                vector.get("hardeningExpected").cloned().unwrap_or_else(|| {
                    vector.get("error")
                        .map(|v| serde_json::json!({"error":v}))
                        .unwrap_or_else(|| serde_json::json!({"result":vector["result"]}))
                }),
                "{}",
                vector["id"]
            );
            count += 1;
        }
    }
    assert_eq!(count, 523);
}

// Decode golden-only undefined markers outside product code. Object keys are
// omitted, preserving the absent-vs-empty legacy directive behavior. JSON cannot
// represent NaN; nested $number markers intentionally exercise the equivalent
// non-number rejection/default branch. The numeric port entrypoint uses number().
fn decode(value: &Value) -> Value {
    if value.get("$undefined") == Some(&json!(true)) {
        return Value::Null;
    }
    match value {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(_, v)| v.get("$undefined") != Some(&json!(true)))
                .map(|(k, v)| (k.clone(), decode(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(decode).collect()),
        _ => value.clone(),
    }
}
fn undefined() -> Value {
    json!({"$undefined":true})
}
fn optional<T: serde::Serialize>(value: Option<T>) -> Value {
    value
        .map(|v| serde_json::to_value(v).unwrap())
        .unwrap_or_else(undefined)
}
fn number(v: &Value) -> f64 {
    match v["$number"].as_str() {
        Some("NaN") => f64::NAN,
        Some("Infinity") => f64::INFINITY,
        Some("-Infinity") => f64::NEG_INFINITY,
        Some("-0") => -0.0,
        _ => v.as_f64().unwrap(),
    }
}
fn dispatch(name: &str, args: &Value) -> Value {
    let a = decode(args);
    let result: control::Result<Value> = (|| {
        let s = || a[0].as_str().unwrap();
        Ok(match name {
            "renderReverseProxy" => json!(r::render_reverse_proxy(&a[0])?),
            "renderUpstream" => json!(r::render_upstream(&a[0])?),
            "renderStream" => json!(r::render_stream(&a[0])?),
            "pointsAtAdminUi" => json!(control::self_proxy::points_at_admin_ui_with(s(), &|_,_|Vec::new())),
            "findReservedPortConflict" => {
                let reserved: ports::ReservedPorts = serde_json::from_value(a[2].clone()).unwrap();
                let context = serde_json::from_value(a[1].clone()).unwrap();
                ports::find_reserved_port_conflict(
                    u16::try_from(a[0].as_u64().unwrap()).unwrap(),
                    context,
                    &reserved,
                )
                .map(|e| json!({"port":e.port,"kind":e.kind,"name":"ReservedPortError"}))
                .unwrap_or(Value::Null)
            }
            "parseCacheConfig" => optional(p::parse_cache_config(&a[0])?),
            "parseGzipConfig" => optional(p::parse_gzip_config(&a[0])?),
            "parsePerformanceOverride" => optional(p::parse_performance_override(&a[0])?),
            "isCacheEffective" => json!(p::is_cache_effective(
                p::parse_cache_config(&a[0])?.as_ref(),
                p::parse_performance_override(&a[1])?
            )),
            "isGzipEffective" => json!(p::is_gzip_effective(
                p::parse_gzip_config(&a[0])?.as_ref(),
                p::parse_performance_override(&a[1])?
            )),
            "validatePerformanceConflicts" => {
                p::validate_performance_conflicts(&a[0])?;
                undefined()
            }
            "parseRealIpConfig" => optional(v::parse_real_ip_config(&a[0])?),
            "parseFreeformDirectives" => json!(v::parse_freeform_directives(
                s(),
                serde_json::from_value(a[1].clone()).unwrap()
            )?),
            "assertValidDirective" => {
                v::assert_valid_directive(
                    s(),
                    &serde_json::from_value::<Vec<String>>(a[1].clone()).unwrap(),
                    serde_json::from_value(a[2].clone()).unwrap(),
                )?;
                undefined()
            }
            "assertValidServerName" => {
                v::assert_valid_server_name(s())?;
                undefined()
            }
            "assertValidProxyTarget" => {
                v::assert_valid_proxy_target(s())?;
                undefined()
            }
            "assertValidStreamProxyPass" => {
                v::assert_valid_stream_proxy_pass(s())?;
                undefined()
            }
            "assertValidUpstreamServer" => {
                v::assert_valid_upstream_server(s())?;
                undefined()
            }
            "assertValidLocationPath" => {
                v::assert_valid_location_path(s())?;
                undefined()
            }
            "assertValidUpstreamName" => {
                v::assert_valid_upstream_name(s())?;
                undefined()
            }
            "assertValidIpOrCidr" => {
                v::assert_valid_ip_or_cidr(s())?;
                undefined()
            }
            "assertValidListenPort" => {
                let reserved = serde_json::from_value(a[1]["reserved"].clone()).unwrap();
                let context = serde_json::from_value(a[1]["context"].clone()).unwrap();
                v::assert_valid_listen_port(number(&a[0]), context, &reserved)?;
                undefined()
            }
            _ => panic!("uncovered function {name}"),
        })
    })();
    match result {
        Ok(value) => json!({"result":value}),
        Err(e) => {
            let name = match &e {
                ControlError::Reserved(_) => "ReservedPortError",
                _ => "Error",
            };
            let mut error = json!({"name":name,"message":e.to_string()});
            if let ControlError::Reserved(reserved) = &e {
                error["port"] = json!(reserved.port);
                error["kind"] = json!(reserved.kind);
            }
            json!({"error":error})
        }
    }
}
