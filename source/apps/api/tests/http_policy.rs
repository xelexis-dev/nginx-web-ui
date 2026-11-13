use axum::http::HeaderMap;
use nwui_api::http_policy::{check_browser_origin, parse_urlencoded_form};
use nwui_api::request_context::RequestContext;

fn ctx(external_origin: &str, origin_configured: bool) -> RequestContext {
    RequestContext { ip: None, external_origin: external_origin.into(), secure: external_origin.starts_with("https://"), origin_configured }
}

#[test]
fn cookie_writes_reject_cross_origin_opaque_and_missing_origins() {
    // AUTH_URL pins the origin: the RFC 6454 tuple must match exactly.
    let pinned = ctx("https://ui.example.test", true);
    for origin in [
        None,
        Some("null"),
        Some("https://other.example.test"),
        Some("https://ui.example.test.attacker.test"),
        Some("http://ui.example.test"),
        Some("https://ui.example.test/path"),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(value) = origin {
            headers.insert("origin", value.parse().unwrap());
        }
        assert!(
            check_browser_origin(&headers, &pinned).is_err(),
            "accepted {origin:?}"
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert("origin", "https://ui.example.test:443".parse().unwrap());
    assert!(check_browser_origin(&headers, &pinned).is_ok());
    headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert!(check_browser_origin(&headers, &pinned).is_err());
}

#[test]
fn a_derived_origin_compares_host_and_explicit_port_but_not_the_scheme() {
    // Behind a TLS-terminating proxy this API derives http:// for an https:// page.
    let derived = ctx("http://ui.example.test", false);
    let check = |origin: &str| {
        let mut headers = HeaderMap::new();
        headers.insert("origin", origin.parse().unwrap());
        check_browser_origin(&headers, &derived)
    };
    assert!(check("https://ui.example.test").is_ok());
    assert!(check("http://ui.example.test").is_ok());
    for rejected in [
        "https://other.example.test",
        "https://ui.example.test.attacker.test",
        "https://ui.example.test:8443",
        "https://ui.example.test:80",
        "null",
        "https://ui.example.test/path",
    ] {
        assert!(check(rejected).is_err(), "accepted {rejected}");
    }
    let with_port = ctx("http://ui.example.test:8080", false);
    let mut headers = HeaderMap::new();
    headers.insert("origin", "https://ui.example.test:8080".parse().unwrap());
    assert!(check_browser_origin(&headers, &with_port).is_ok());
    headers.insert("origin", "https://ui.example.test".parse().unwrap());
    assert!(check_browser_origin(&headers, &with_port).is_err());
}

#[test]
fn form_transport_preserves_repeated_fields_without_dispatch_arguments_leaking() {
    let form = parse_urlencoded_form(
        b"events=issued&events=failed&alias=%ED%95%9C%EA%B8%80&_apiArgs=%5B%22id-1%22%5D",
    )
    .unwrap();
    assert_eq!(form.args, ["id-1"]);
    assert_eq!(form.all("events"), ["issued", "failed"]);
    assert_eq!(form.get("alias"), Some("한글"));
    assert!(form.get("_apiArgs").is_none());
}

#[test]
fn malformed_or_ambiguous_dispatch_arguments_are_rejected() {
    for body in [
        "_apiArgs=[]&_apiArgs=[]",
        "_apiArgs=42",
        "_apiArgs=[null]",
        "_apiArgs=[1]",
        "_apiArgs=[",
        "alias=%FF",
        "alias=%ZZ",
    ] {
        assert!(
            parse_urlencoded_form(body.as_bytes()).is_err(),
            "accepted {body}"
        );
    }
}
