use axum::{Extension, Router, routing::get};
use nwui_api::{app::{finish_router, with_request_context}, request_context::RequestContext, server::serve};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}};

#[tokio::test]
async fn actual_listener_installs_peer_context_and_stops_accepting_on_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/api/v1/info", get(|Extension(ctx): Extension<RequestContext>| async move { ctx.ip.unwrap() }));
    let router = finish_router(with_request_context(router, None), "1.18.0");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(serve(listener, router, async { let _ = stopped.await; }));
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(b"GET /api/v1/info HTTP/1.1\r\nHost: ui.test\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("x-node-version: 1.18.0"));
    assert!(response.ends_with("127.0.0.1"));
    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(TcpStream::connect(address).await.is_err());
}

/// RFC 9112 §6.3: contradictory Content-Length values and a request whose final
/// transfer coding is not chunked must be rejected before application dispatch.
#[tokio::test]
async fn malformed_http11_framing_never_reaches_a_handler() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let router = Router::new().route("/probe", axum::routing::post(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        async { "unexpected handler dispatch" }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(serve(listener, router, async { let _ = stopped.await; }));
    for framing in ["Content-Length: 0\r\nContent-Length: 1", "Transfer-Encoding: gzip"] {
        let mut client = TcpStream::connect(address).await.unwrap();
        let wire = format!("POST /probe HTTP/1.1\r\nHost: ui.test\r\nConnection: close\r\n{framing}\r\n\r\n");
        client.write_all(wire.as_bytes()).await.unwrap();
        let mut response = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(2), client.read_to_string(&mut response))
            .await.unwrap().unwrap();
        assert!(response.starts_with("HTTP/1.1 400"), "{framing}: {response}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
}
