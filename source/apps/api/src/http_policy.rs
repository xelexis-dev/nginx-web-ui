use crate::{contracts::FormInput, error::ApiError, request_context::RequestContext};
use axum::http::{HeaderMap, StatusCode};

/// Requiring Origin on cookie-authenticated writes and denying cross-site Fetch Metadata are
/// application CSRF policies.
///
/// With AUTH_URL configured the RFC 6454 origin tuple must match exactly. Without it the
/// origin is derived from the request, and a TLS-terminating reverse proxy in front of the
/// published port makes that http:// while the browser's Origin says https:// — so only the
/// host and any explicit port are compared. A derived HTTPS origin also requires HTTPS.
pub fn check_browser_origin(headers: &HeaderMap, ctx: &RequestContext) -> Result<(), ApiError> {
    let external_origin = ctx.external_origin.as_str();
    let denied = || {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "invalid_origin",
            "요청 출처를 확인할 수 없습니다.",
        )
    };
    if headers.get_all("origin").iter().count() != 1 {
        return Err(denied());
    }
    let raw = headers
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(denied)?;
    let origin = url::Url::parse(raw).map_err(|_| denied())?;
    let expected = url::Url::parse(external_origin).map_err(|_| denied())?;
    let authority = raw.split_once("://").map(|(_, a)| a).ok_or_else(denied)?;
    if !matches!(origin.scheme(), "http" | "https")
        || !origin.username().is_empty()
        || origin.password().is_some()
        || authority.contains(['/', '\\', '?', '#'])
        || !same_site(&origin, &expected, ctx.origin_configured)
    {
        return Err(denied());
    }
    if let Some(site) = headers.get("sec-fetch-site")
        && !matches!(site.to_str(), Ok("same-origin" | "none")) {
        return Err(denied());
    }
    Ok(())
}

fn same_site(origin: &url::Url, expected: &url::Url, exact: bool) -> bool {
    if exact {
        return origin.origin() == expected.origin();
    }
    // `port()` is None for the scheme's default port, i.e. the port as the browser wrote it.
    origin.host_str() == expected.host_str() && origin.port() == expected.port()
        && (expected.scheme() != "https" || origin.scheme() == "https")
}

fn bad_form() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "bad_request",
        "잘못된 폼 요청입니다.",
    )
}

pub fn parse_urlencoded_form(body: &[u8]) -> Result<FormInput, ApiError> {
    // Reject invalid encoding rather than silently replacing malformed UTF-8.
    for (i, byte) in body.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= body.len()
                || !body[i + 1].is_ascii_hexdigit()
                || !body[i + 2].is_ascii_hexdigit())
        {
            return Err(bad_form());
        }
    }
    percent_encoding::percent_decode(body)
        .decode_utf8()
        .map_err(|_| bad_form())?;
    let mut input = FormInput::default();
    for (count, (key, value)) in url::form_urlencoded::parse(body).enumerate() {
        if count >= 512 {
            return Err(bad_form());
        }
        input
            .fields
            .entry(key.into_owned())
            .or_default()
            .push(value.into_owned());
        if input.fields.len() > 256 {
            return Err(bad_form());
        }
    }
    extract_form_args(&mut input)?;
    Ok(input)
}

pub fn extract_form_args(input: &mut FormInput) -> Result<(), ApiError> {
    if let Some(values) = input.fields.remove("_apiArgs") {
        if values.len() != 1 {
            return Err(bad_form());
        }
        input.args = serde_json::from_str::<Vec<String>>(&values[0]).map_err(|_| bad_form())?;
        if input.args.len() > 8
            || input
                .args
                .iter()
                .any(|s| s.len() > 2048 || s.contains('\0'))
        {
            return Err(bad_form());
        }
    }
    Ok(())
}

pub async fn read_form(request: axum::extract::Request) -> Result<FormInput, ApiError> {
    use axum::extract::{FromRequest, Multipart};
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match content_type.as_str() {
        "application/x-www-form-urlencoded" => {
            let bytes = axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024)
                .await
                .map_err(|_| {
                    ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "payload_too_large",
                        "요청 크기가 너무 큽니다.",
                    )
                })?;
            parse_urlencoded_form(&bytes)
        }
        "multipart/form-data" => {
            let mut multipart = Multipart::from_request(request, &())
                .await
                .map_err(|_| bad_form())?;
            let mut input = FormInput::default();
            let mut count = 0;
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| ApiError::new(e.status(), "bad_request", "폼을 읽지 못했습니다."))?
            {
                count += 1;
                if count > 512 || field.file_name().is_some() {
                    return Err(bad_form());
                }
                let name = field.name().ok_or_else(bad_form)?.to_owned();
                let bytes = field.bytes().await.map_err(|e| {
                    ApiError::new(e.status(), "bad_request", "폼을 읽지 못했습니다.")
                })?;
                let value = String::from_utf8(bytes.to_vec()).map_err(|_| bad_form())?;
                input.fields.entry(name).or_default().push(value);
                if input.fields.len() > 256 {
                    return Err(bad_form());
                }
            }
            extract_form_args(&mut input)?;
            Ok(input)
        }
        _ => Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "지원하지 않는 요청 형식입니다.",
        )),
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    #[test]
    fn derived_https_origin_refuses_http_without_fetch_metadata_but_http_allows_tls_termination() {
        for (expected, supplied, accepted) in [
            ("https://admin.test:4443", "http://admin.test:4443", false),
            ("https://admin.test:4443", "https://admin.test:4443", true),
            ("http://admin.test", "https://admin.test", true),
            ("https://admin.test:4443", "https://admin.test", false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("host", "admin.test".parse().unwrap());
            let mut ctx = RequestContext::resolve(&headers, "127.0.0.1:45000".parse().unwrap(), None).unwrap();
            ctx.external_origin = expected.into();
            assert!(!ctx.origin_configured);
            headers.insert("origin", supplied.parse().unwrap());
            assert!(!headers.contains_key("sec-fetch-site"));
            assert_eq!(check_browser_origin(&headers, &ctx).is_ok(), accepted, "{expected} <- {supplied}");
        }
    }
}
