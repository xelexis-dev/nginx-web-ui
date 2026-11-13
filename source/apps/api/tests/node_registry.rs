use nwui_api::nodes::views;
use std::collections::BTreeMap;

#[test]
fn static_reads_choose_paths_roles_and_never_accept_caller_api_paths() {
    let plan = views::remote_plan(&["reverse-proxy", "sid", "edit"], &BTreeMap::new()).unwrap();
    assert_eq!(
        plan.requests
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        [
            "/api/v1/servers/sid",
            "/api/v1/upstreams",
            "/api/v1/certificates"
        ]
    );
    assert!(views::remote_plan(&["api", "v1", "secrets"], &BTreeMap::new()).is_none());
    assert!(views::remote_plan(&["stream", "x", "delete"], &BTreeMap::new()).is_none());
    let plan = views::remote_plan(
        &["audit"],
        &BTreeMap::from([
            ("page".into(), "2&path=/api/v1/pair".into()),
            ("path".into(), "/api/v1/pair".into()),
        ]),
    )
    .unwrap();
    assert!(plan.requests[0].path.starts_with("/api/v1/audit?page="));
    assert!(!plan.requests[0].path.contains("&path="));
}
