use super::resources::{self, Mutation, Resource};
use crate::{
    auth::{
        principal::{TwoFactorGate, db_unavailable, require_gateway, require_user},
        rbac::Action,
    },
    contracts::FormResult,
    error::ApiError,
    http_policy::read_form,
    request_context::RequestContext,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

pub const FORM_IDS: &[&str] = &[
    "upstreams.createUpstream",
    "upstreams.updateUpstream",
    "upstreams.deleteUpstream",
    "reverse-proxy.createServer",
    "reverse-proxy.updateServer",
    "reverse-proxy.deleteServer",
    "reverse-proxy.toggleServer",
    "stream.createStream",
    "stream.updateStream",
    "stream.deleteStream",
    "stream.toggleStream",
];
pub fn router() -> Router<AppState> {
    let mut router = Router::new();
    for kind in [Resource::Upstream, Resource::Server, Resource::Stream] {
        let base = format!("/api/v1/{}", kind.plural());
        router = router.route(&base, get(node).post(node)).route(
            &format!("{base}/{{id}}"),
            get(node).patch(node).delete(node),
        );
        if kind != Resource::Upstream {
            router = router.route(&format!("{base}/{{id}}/toggle"), post(node));
        }
    }
    for id in FORM_IDS {
        router = router.route(&format!("/api/ui/v1/forms/{id}"), post(form));
    }
    router
}
async fn json_body(request: Request) -> Result<Value, ApiError> {
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !content_type.eq_ignore_ascii_case("application/json") {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "JSON 본문이 필요합니다.",
        ));
    }
    let bytes = axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "요청 크기가 너무 큽니다.",
            )
        })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "JSON 본문을 읽을 수 없습니다.",
        )
    })
}
async fn node(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Response, ApiError> {
    let method = request.method().clone();
    // Explicitly authenticate before reading JSON, including version gate before Bearer lookup.
    let gateway = require_gateway(
        &state,
        request.headers(),
        &ctx,
        &[if method == Method::GET || method == Method::HEAD {
            Action::ConfigRead
        } else {
            Action::ConfigWrite
        }],
    )
    .await?;
    let path = request.uri().path().to_owned();
    let parts: Vec<&str> = path.trim_start_matches("/api/v1/").split('/').collect();
    let kind = match parts[0] {
        "upstreams" => Resource::Upstream,
        "servers" => Resource::Server,
        "streams" => Resource::Stream,
        _ => return Err(ApiError::not_found()),
    };
    let decoded = parts
        .get(1)
        .map(|id| {
            percent_encoding::percent_decode_str(id)
                .decode_utf8()
                .map(|s| s.into_owned())
        })
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "잘못된 경로입니다."))?;
    let id = decoded.as_deref();
    if method == Method::GET || method == Method::HEAD {
        let client = state.db.get().await.map_err(|_| db_unavailable())?;
        let upstreams=if kind==Resource::Server{resources::list(&**client,Resource::Upstream).await?}else{vec![]};
        let names=super::location_refs::upstream_names(&upstreams);
        let mut body = if let Some(id) = id {
            let mut row = resources::get(&**client, kind, id).await?;
            if kind==Resource::Server{row=super::location_refs::project_for_read(&row,&names);}
            json!({kind.singular():row})
        } else {
            let rows=resources::list(&**client,kind).await?;
            json!({kind.plural():if kind==Resource::Server{rows.iter().map(|row|super::location_refs::project_for_read(row,&names)).collect::<Vec<_>>()}else{rows}})
        };
        let certificates = if id.is_some() && kind != Resource::Upstream {
            Some(super::views::preview_certificates(&**client,kind,&body[kind.singular()]).await?)
        } else { None };
        drop(client);
        if let Some(certificates) = certificates {
            body["preview"] = super::views::preview_with_upstreams(&state,kind,&body[kind.singular()],&upstreams,&certificates).await?;
        }
        return Ok(Json(body).into_response());
    }
    let operation = if parts.get(2) == Some(&"toggle") {
        Mutation::Toggle
    } else if method == Method::DELETE {
        Mutation::Delete
    } else if method == Method::PATCH {
        Mutation::Update
    } else {
        Mutation::Create
    };
    let raw = if matches!(operation, Mutation::Create | Mutation::Update) {
        json_body(request).await?
    } else {
        Value::Null
    };
    let value = resources::mutate(&state, kind, operation, id, raw, &gateway.actor(&ctx)).await?;
    let body = if matches!(operation, Mutation::Delete | Mutation::Toggle) {
        value
    } else {
        json!({kind.singular():value})
    };
    Ok((
        if operation == Mutation::Create {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(body),
    )
        .into_response())
}
async fn form(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    request: Request,
) -> Result<Json<FormResult>, ApiError> {
    let session = require_user(
        &state,
        request.headers(),
        &ctx,
        Some(Action::ConfigWrite),
        TwoFactorGate::Enforce, crate::auth::principal::Use::SessionOrToken)
    .await?;
    session.check_origin(request.headers(), &ctx)?;
    let command = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let (group, name) = command.split_once('.').ok_or_else(ApiError::not_found)?;
    let kind = match group {
        "upstreams" => Resource::Upstream,
        "reverse-proxy" => Resource::Server,
        "stream" => Resource::Stream,
        _ => return Err(ApiError::not_found()),
    };
    let operation = if name.starts_with("create") {
        Mutation::Create
    } else if name.starts_with("update") {
        Mutation::Update
    } else if name.starts_with("delete") {
        Mutation::Delete
    } else {
        Mutation::Toggle
    };
    let input = read_form(request).await?;
    if !input.args.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "잘못된 폼 인자입니다.",
        ));
    }
    let raw = if matches!(operation, Mutation::Create | Mutation::Update) {
        resources::form_body(kind, &input)
    } else {
        Ok(Value::Null)
    };
    let outcome = match raw {
        Ok(raw) => {
            resources::mutate(
                &state,
                kind,
                operation,
                input.get("id").map(str::trim),
                raw,
                &session.actor(&ctx),
            )
            .await
        }
        Err(e) => Err(e),
    };
    match outcome {
        Ok(value) => {
            let redirect = match operation {
                Mutation::Create | Mutation::Update => Some(format!(
                    "{}/{}",
                    kind.base(),
                    value["id"].as_str().unwrap_or("")
                )),
                Mutation::Delete => Some(kind.base().into()),
                Mutation::Toggle => None,
            };
            Ok(Json(FormResult {
                result: Some(json!({"ok":true})),
                redirect_to: redirect,
                refresh: Some(true),
            }))
        }
        Err(e)
            if matches!(
                e.status,
                StatusCode::BAD_REQUEST | StatusCode::CONFLICT | StatusCode::NOT_FOUND
            ) =>
        {
            let mut result = json!({"ok":false,"error":e.message});
            if let Some(fields) = e.field_errors {
                result["fieldErrors"] = json!(fields);
            }
            Ok(Json(FormResult {
                result: Some(result),
                ..Default::default()
            }))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod gateway_projection_tests {
    use super::*;
    #[test]
    fn modern_projection_preserves_upstream_ids_and_direct_targets() {
        let names = std::collections::BTreeMap::from([("u".into(), "backend".into())]);
        let row = json!({"config":{"locations":[{"path":"/","upstreamId":"u"},{"path":"/direct","proxyPass":"http://app.test:9000"}]}});
        let modern = super::super::location_refs::project_for_read(&row,&names);
        assert_eq!(modern["config"]["locations"][0],json!({"path":"/","upstreamId":"u","upstreamName":"backend"}));
        assert_eq!(modern["config"]["locations"][1],row["config"]["locations"][1]);
        assert_eq!(row["config"]["locations"][0]["upstreamId"],"u");
    }
}
