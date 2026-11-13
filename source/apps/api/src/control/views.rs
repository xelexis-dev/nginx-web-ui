//! Browser view data; only presentation fields, never certificate material.
use super::resources::{self, Resource};
use crate::{
    auth::{
        principal::{SessionPrincipal, db_unavailable},
        rbac::{Action, can},
    },
    error::ApiError,
    state::AppState,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub async fn view(
    state: &AppState,
    session: &SessionPrincipal,
    path: &str,
    _query: &BTreeMap<String, String>,
) -> Result<Option<Value>, ApiError> {
    if path == "/reverse-proxy/templates" {
        if !can(session.role, Action::ConfigRead) {
            return Err(ApiError::forbidden());
        }
        let config = state.config.clone();
        let previews = super::internal_targets::blocking(move || template_previews(&config)).await?;
        return Ok(Some(json!({"canWrite":can(session.role,Action::ConfigWrite),"previews":previews})));
    }
    let (kind, tail) = if path == "/reverse-proxy/upstreams" {
        (Resource::Upstream, "")
    } else if let Some(t) = path.strip_prefix("/reverse-proxy/upstreams/") {
        (Resource::Upstream, t)
    } else if path == "/reverse-proxy" {
        (Resource::Server, "")
    } else if let Some(t) = path.strip_prefix("/reverse-proxy/") {
        if t == "templates" || t.starts_with("templates/") {
            return Ok(None);
        }
        (Resource::Server, t)
    } else if path == "/stream" {
        (Resource::Stream, "")
    } else if let Some(t) = path.strip_prefix("/stream/") {
        (Resource::Stream, t)
    } else {
        return Ok(None);
    };
    let editing = tail == "new" || tail.ends_with("/edit");
    if !can(
        session.role,
        if editing {
            Action::ConfigWrite
        } else {
            Action::ConfigRead
        },
    ) {
        return Err(ApiError::forbidden());
    }
    let parts: Vec<&str> = tail.split('/').collect();
    if parts.len() > 2 || parts.len() == 2 && parts[1] != "edit" {
        return Ok(None);
    }
    let client = state.db.get().await.map_err(|_| db_unavailable())?;
    let mut self_proxy_excluded = false;
    let mut out = json!({"canWrite":can(session.role,Action::ConfigWrite),"canApply":can(session.role,Action::NginxReload),"runtimeDns":runtime_dns_status(&state.config)});
    if tail.is_empty() {
        out[kind.plural()] = json!(resources::list(&**client, kind).await?);
    } else if tail != "new" {
        out[kind.singular()] = resources::get(&**client, kind, parts[0]).await?;
    }
    if kind == Resource::Server {
        out["upstreams"] = json!(resources::list(&**client, Resource::Upstream).await?);
        let names=super::location_refs::upstream_names(out["upstreams"].as_array().unwrap());
        if let Some(row)=out.get_mut("server"){*row=super::location_refs::project_for_read(row,&names);}
        if let Some(rows)=out.get_mut("servers").and_then(Value::as_array_mut){for row in rows{*row=super::location_refs::project_for_read(row,&names);}}

        if !tail.is_empty() {
            let certs=client.query("SELECT jsonb_build_object('id',id,'domains',domains,'alias',alias,'staging',staging,'status',status::text,'tlsReady',status::text='issued' AND coalesce(cert_pem,'')<>'' AND coalesce(key_pem,'')<>'') FROM certificates ORDER BY created_at DESC,id",&[]).await.map_err(|_|db_unavailable())?;
            out["certificates"] = json!(
                certs
                    .into_iter()
                    .map(|r| r.get::<_, Value>(0))
                    .collect::<Vec<_>>()
            );
            if out.get("server").is_some() {
                let server = &out["server"];
                let excluded = client
                    .query_opt(
                        "SELECT metrics_excluded_hosts FROM app_settings LIMIT 1",
                        &[],
                    )
                    .await
                    .map_err(|_| db_unavailable())?
                    .and_then(|r| r.get::<_, Option<String>>(0))
                    .unwrap_or_default();
                self_proxy_excluded = excluded.lines().any(|h| {
                    h.trim()
                        .eq_ignore_ascii_case(server["serverName"].as_str().unwrap_or("").trim())
                });
            }
        }
    }
    drop(client);
    if let Some(server) = out.get("server") {
        let cfg = server["config"].clone();
        let (is_self, unchecked) = super::internal_targets::blocking(move || {
            self_proxy_notices(&cfg, &super::internal_targets::system_resolve)
        }).await?;
        out["selfProxyNotice"] = json!(is_self && !self_proxy_excluded);
        if !unchecked.is_empty() {
            out["selfProxyUncheckedPaths"] = json!(unchecked);
        }
    }
    if !editing && !tail.is_empty() && kind != Resource::Upstream {
        out["preview"] = preview_with_upstreams(state,kind,&out[kind.singular()],
            out["upstreams"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            out["certificates"].as_array().map(Vec::as_slice).unwrap_or(&[])).await?;
    }
    Ok(Some(out))
}

/// Render with the target instance's paths without reading or exposing PEM material.
pub async fn preview(
    state: &AppState,
    client: &impl tokio_postgres::GenericClient,
    kind: Resource,
    row: &Value,
) -> Result<Value, ApiError> {
    let upstreams = if kind == Resource::Server {
        resources::list(client, Resource::Upstream).await?
    } else {
        vec![]
    };
    let certificates = preview_certificates(client,kind,row).await?;
    preview_with_upstreams(state,kind,row,&upstreams,&certificates).await
}

pub(crate) async fn preview_certificates(
    client: &impl tokio_postgres::GenericClient, kind: Resource, row: &Value,
) -> Result<Vec<Value>, ApiError> {
    Ok(if kind == Resource::Server {
        client.query("SELECT jsonb_build_object('id',id,'tlsReady',status::text='issued' AND coalesce(cert_pem,'')<>'' AND coalesce(key_pem,'')<>'') FROM certificates WHERE id=$1", &[&row["certificateId"].as_str()]).await.map_err(|_|db_unavailable())?.into_iter().map(|r|r.get(0)).collect()
    } else { vec![] })
}

pub(crate) async fn preview_with_upstreams(
    state: &AppState, kind: Resource, row: &Value, upstreams: &[Value], certificates: &[Value],
) -> Result<Value, ApiError> {
    let config = state.config.clone();
    let row = row.clone();let upstreams=upstreams.to_vec();let certificates=certificates.to_vec();
    super::internal_targets::blocking(move || {
        render_preview(&config, kind, &row, &upstreams, &certificates)
    }).await
}

pub fn render_preview(
    config: &crate::config::RuntimeConfig,
    kind: Resource,
    row: &Value,
    upstreams: &[Value],
    certificates: &[Value],
) -> Value {
    let mut target = row.clone();
    target["enabled"] = json!(true);
    let mut snapshot = super::model::ConfigSnapshotData::default();
    if kind == Resource::Server {
        let cfg = &row["config"];
        snapshot.upstreams = upstreams
            .iter()
            .filter(|upstream| {
                row["upstreamId"]
                    .as_str()
                    .is_some_and(|id| upstream["id"] == id)
                    || cfg["locations"].as_array().into_iter().flatten().any(|location|location["upstreamId"].as_str().is_some_and(|id|upstream["id"]==id))
                    || std::iter::once(cfg["proxyPass"].as_str())
                        .chain(
                            cfg["locations"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|l| l["proxyPass"].as_str()),
                        )
                        .flatten()
                        .any(|target| {
                            upstream["name"]
                                .as_str()
                                .is_some_and(|name| target == format!("http://{name}"))
                        })
            })
            .cloned()
            .collect();
        snapshot.servers.push(target);
    } else {
        snapshot.streams.push(target);
    }
    // Readiness metadata only; previews never fetch or materialize certificate secrets.
    let certificates: Vec<_> = certificates
        .iter()
        .filter(|c| c["tlsReady"] == true)
        .filter_map(|c| c["id"].as_str())
        .map(|id| super::model::CertificateRef {
            id: id.into(), status: "issued".into(), usable: true,
        })
        .collect();
    // The preview snapshot holds only this row and its upstreams, so any skip concerns it.
    let dns = crate::dns_guard::status(config);
    let mut outcome = match super::model::render_preview_with_context(config,&snapshot,&certificates,&dns) {
        Ok(outcome)=>outcome,
        Err(error)=>return json!({"config":null,"error":error.to_string()}),
    };
    let unresolved_paths = outcome.unresolved_paths.remove(row["id"].as_str().unwrap_or("")).unwrap_or_default();
    let result = match outcome.skipped.first() {
        None => Ok(outcome.rendered),
        Some(skipped) => Err(&skipped.reason),
    };
    let mut preview = match result {
        Ok(rendered) => {
            let id = row["id"].as_str().unwrap_or("");
            let bytes = if kind == Resource::Server {
                let mut parts = vec![];
                for key in ["upstreams.conf".to_owned(), format!("server-{id}.conf")] {
                    if let Some(value) = rendered.conf.get(&key).filter(|v| !v.is_empty()) {
                        parts.push(String::from_utf8_lossy(value).into_owned());
                    }
                }
                parts.join("\n")
            } else {
                rendered
                    .stream
                    .get(&format!("stream-{id}.conf"))
                    .map(|v| String::from_utf8_lossy(v).into_owned())
                    .unwrap_or_default()
            };
            json!({"config":bytes,"error":null})
        }
        Err(error) => json!({"config":null,"error":error.to_string()}),
    };
    if !unresolved_paths.is_empty() { preview["unresolvedPaths"] = json!(unresolved_paths); }
    preview
}

/// Fixed gallery presets. These are the same inputs used when the browser prefills a new server.
pub fn template_previews(config: &crate::config::RuntimeConfig) -> Value {
    fn directives(gitlab: bool, http11: bool) -> Vec<Value> {
        let mut dirs = vec![];
        for (name, value) in [
            ("Host", if gitlab { "$http_host" } else { "$host" }),
            ("X-Real-IP", "$remote_addr"),
            ("X-Forwarded-For", "$proxy_add_x_forwarded_for"),
            ("X-Forwarded-Proto", "$scheme"),
        ] {
            dirs.push(json!({"name":"proxy_set_header","args":[name,value]}));
        }
        for name in ["proxy_read_timeout", "proxy_send_timeout"] {
            dirs.push(json!({"name":name,"args":[if gitlab {"3600s"} else {"300s"}]}));
        }
        if http11 {
            dirs.push(json!({"name":"proxy_http_version","args":["1.1"]}));
        }
        dirs
    }
    let mut result = json!({});
    for (id, domain, port, body, gitlab, http11) in [
        ("nexus", "nexus.example.com", 8081, "1g", false, false),
        ("nexus-docker", "docker.example.com", 8082, "0", false, true),
        (
            "nexus-combined",
            "nexus.example.com",
            8081,
            "0",
            false,
            false,
        ),
        ("gitlab", "gitlab.example.com", 8181, "0", true, false),
        (
            "gitlab-registry",
            "registry.gitlab.example.com",
            5050,
            "0",
            true,
            true,
        ),
    ] {
        let mut cfg = json!({"proxyPass":format!("http://127.0.0.1:{port}"),"serverDirectives":[{"name":"client_max_body_size","args":[body]}],"locationDirectives":directives(gitlab,http11)});
        if id == "nexus-combined" {
            cfg["locations"] = json!([{"path":"/","proxyPass":"http://127.0.0.1:8081","directives":directives(false,false)},{"path":"/v2/","proxyPass":"http://127.0.0.1:8082","directives":directives(false,true)}]);
        }
        result[id] = render_preview(
            config,
            Resource::Server,
            &json!({"id":id,"serverName":domain,"listen":80,"config":cfg}),
            &[],
            &[],
        );
    }
    result
}

/// Public availability only; descriptor endpoint and filesystem path stay private.
pub(crate) fn runtime_dns_status(config: &crate::config::RuntimeConfig) -> Value {
    let status = crate::dns_guard::status(config);
    json!({"available":status.available,"reason":status.reason})
}

const MAX_SELF_PROXY_NOTICE_LOOKUPS: usize = 8;

fn self_proxy_notices(cfg: &Value, resolve: super::internal_targets::Resolve) -> (bool, Vec<String>) {
    use std::cell::Cell;
    let calls = Cell::new(0);
    let mut is_self = false;
    let mut unchecked = Vec::new();
    for location in super::render::effective_locations(cfg) {
        let skipped = Cell::new(false);
        let already_self = is_self;
        let found = super::self_proxy::points_at_admin_ui_for_location(&location, &|host, port| {
            if already_self || calls.get() >= MAX_SELF_PROXY_NOTICE_LOOKUPS {
                skipped.set(true);
                Vec::new()
            } else {
                calls.set(calls.get() + 1);
                resolve(host, port)
            }
        }).unwrap_or(false);
        is_self |= found;
        if skipped.get() {
            unchecked.push(location["path"].as_str().unwrap_or("/").to_owned());
        }
    }
    (is_self, unchecked)
}

#[cfg(test)]
mod notice_budget_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn w13_notice_budget_counts_resolver_calls_and_reports_unchecked_paths() {
        let calls = Cell::new(0);
        let mut locations: Vec<_> = (0..MAX_SELF_PROXY_NOTICE_LOOKUPS + 3)
            .map(|i| json!({"path":format!("/{i}"),"proxyPass":format!("http://host-{i}.test:8080")})).collect();
        locations.extend([
            json!({"path":"/runtime","proxyPass":"http://runtime.test:8080","resolveAtRuntime":true}),
            json!({"path":"/literal","proxyPass":"http://127.0.0.1:8080"}),
            json!({"path":"/other-port","proxyPass":"http://other.test:19000"}),
        ]);
        let cfg = json!({"locations":locations});
        let before = cfg.clone();
        let (is_self, unchecked) = self_proxy_notices(&cfg, &|_, _| {
            calls.set(calls.get() + 1);
            vec!["192.0.2.1".parse().unwrap()]
        });
        assert_eq!(calls.get(), MAX_SELF_PROXY_NOTICE_LOOKUPS);
        assert!(is_self, "literal classification must complete after the DNS budget");
        assert_eq!(unchecked, vec!["/8", "/9", "/10"]);
        assert_eq!(cfg, before);
    }

    #[test]
    fn w13_known_self_notice_does_not_start_unnecessary_name_lookups() {
        let cfg = json!({"locations":[{"path":"/","proxyPass":"http://127.0.0.1:8080"},{"path":"/name","proxyPass":"http://external.test:8080"}]});
        let (is_self, unchecked) = self_proxy_notices(&cfg, &|_, _| panic!("the server notice is already known"));
        assert!(is_self);
        assert_eq!(unchecked, vec!["/name"]);
    }
}

#[cfg(test)]
mod location_reference_tests {
    use super::*;
    fn preview_fixture() -> (tempfile::TempDir,crate::config::RuntimeConfig) {
        let root=tempfile::tempdir().unwrap();
        let mut config=crate::config::RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://test@localhost/test".into()),"AUTH_SECRET"=>Some("test-only".into()),_=>None}).unwrap();
        config.in_container=true;config.target_dns.descriptor_path=root.path().join("dns.json");
        (root,config)
    }
    #[test]
    fn preview_adapter_reports_unresolved_location_paths() {
        let (_root,config)=preview_fixture();
        let row=json!({"id":"server","serverName":"preview.test","listen":18086,"enabled":true,"config":{"locations":[
            {"path":"/named","proxyPass":"http://private-admin.invalid:8080"},
            {"path":"/literal","proxyPass":"http://127.0.0.1:8080"}
        ]}});
        let before=row.clone();let preview=render_preview(&config,Resource::Server,&row,&[],&[]);
        assert_eq!(preview["unresolvedPaths"],json!(["/named"]));
        assert!(preview["error"].is_null(),"{preview}");
        assert!(preview["config"].is_string());assert_eq!(row,before);
        let pooled=json!({"id":"resource-id","serverName":"preview.test","listen":18086,"enabled":true,"config":{"locations":[{"path":"/pooled","upstreamId":"pool-id"}]}});
        let pool=json!({"id":"pool-id","name":"pool","servers":["private-pool.invalid:8080"]});
        let preview=render_preview(&config,Resource::Server,&pooled,&[pool],&[]);
        assert_eq!(preview["unresolvedPaths"],json!(["/pooled"]));
        let stream=json!({"id":"stream","listen":18087,"protocol":"tcp","enabled":true,"proxyPass":"private-admin.invalid:8080"});
        let preview=render_preview(&config,Resource::Stream,&stream,&[],&[]);
        assert_eq!(preview["unresolvedPaths"],json!(["stream"]));
    }
    #[test]
    fn literal_and_bulk_previews_keep_the_optional_field_absent() {
        let (_root,config)=preview_fixture();
        let row=json!({"id":"server","serverName":"preview.test","listen":18086,"enabled":true,"config":{"locations":[{"path":"/","proxyPass":"http://127.0.0.1:8080"}]}});
        let preview=render_preview(&config,Resource::Server,&row,&[],&[]);
        assert!(preview["error"].is_null(),"{preview}");assert!(preview.get("unresolvedPaths").is_none());
        for (name,preview) in template_previews(&config).as_object().unwrap() {
            assert!(preview["error"].is_null(),"{name}: {preview}");
            assert!(preview.get("unresolvedPaths").is_none(),"{name}: {preview}");
        }
    }

    #[test]
    fn pf_a1_preview_resolves_location_id_and_rename_without_mutating_config() {
        let root=tempfile::tempdir().unwrap();
        let mut config=crate::config::RuntimeConfig::read(|key|match key{"DATABASE_URL"=>Some("postgres://test@localhost/test".into()),"AUTH_SECRET"=>Some("test-only".into()),_=>None}).unwrap();
        config.target_dns.descriptor_path=root.path().join("dns.json");
        let row=json!({"id":"server","serverName":"example.test","listen":8081,"enabled":true,"config":{"locations":[{"path":"/","upstreamId":"used"}]}});let before=row.clone();
        for name in ["backend","renamed"] {
            let preview=render_preview(&config,Resource::Server,&row,&[json!({"id":"used","name":name,"servers":["127.0.0.1:9000"]}),json!({"id":"unused","name":"unused","servers":["bad;"]})],&[]);
            assert!(preview["error"].is_null(),"{preview}");let text=preview["config"].as_str().unwrap();assert!(text.contains(&format!("proxy_pass http://{name};")));assert!(text.contains(&format!("upstream {name}")));assert!(!text.contains("unused"));
        }
        let missing=render_preview(&config,Resource::Server,&row,&[],&[]);assert!(missing["error"].as_str().unwrap().contains("upstream을 찾을 수 없습니다"));
        let snapshot=super::super::model::ConfigSnapshotData{servers:vec![row.clone()],upstreams:vec![json!({"id":"used","name":"backend","servers":["bad;"]})],streams:vec![]};let skipped=crate::publisher::test_support::render_for_apply(&config,&snapshot,&[]);assert!(skipped.skipped.iter().any(|s|s.kind==super::super::model::SkippedKind::Server&&s.reason.contains("적용에서 제외")));assert_eq!(row,before);
    }
}
