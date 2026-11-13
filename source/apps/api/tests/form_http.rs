use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use nwui_api::http_policy::read_form;

#[tokio::test]
async fn browser_multipart_and_urlencoded_forms_have_the_same_values() {
    let multipart = "--fixture\r\nContent-Disposition: form-data; name=\"events\"\r\n\r\nissued\r\n--fixture\r\nContent-Disposition: form-data; name=\"events\"\r\n\r\nfailed\r\n--fixture\r\nContent-Disposition: form-data; name=\"_apiArgs\"\r\n\r\n[\"one\"]\r\n--fixture--\r\n";
    let request = |content_type: &str, body: &str| {
        Request::builder()
            .method("POST")
            .header("content-type", content_type)
            .body(Body::from(body.to_owned()))
            .unwrap()
    };
    let a = read_form(request("multipart/form-data; boundary=fixture", multipart))
        .await
        .unwrap();
    let b = read_form(request(
        "application/x-www-form-urlencoded",
        "events=issued&events=failed&_apiArgs=[%22one%22]",
    ))
    .await
    .unwrap();
    assert_eq!(a.args, ["one"]);
    assert_eq!(a.args, b.args);
    assert_eq!(a.fields, b.fields);
}

#[tokio::test]
async fn unsupported_media_and_oversized_forms_fail_before_dispatch() {
    let request = Request::builder()
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        read_form(request).await.err().unwrap().status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let request = Request::builder()
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(vec![b'a'; 2 * 1024 * 1024 + 1]))
        .unwrap();
    assert_eq!(
        read_form(request).await.err().unwrap().status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
