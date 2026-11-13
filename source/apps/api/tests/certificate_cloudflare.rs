use axum::{Json, Router, extract::State, http::Method};
use nwui_api::certificates::cloudflare::CloudflareClient;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[tokio::test]
async fn recovery_matches_exact_owner_name_and_value() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let copy = seen.clone();
    let app=Router::new().fallback(|State(s):State<Arc<Mutex<Vec<Value>>>>,method:Method,body:axum::body::Bytes|async move{
 if method==Method::POST {s.lock().unwrap().push(serde_json::from_slice(&body).unwrap());return Json(json!({"success":true,"result":{"id":"created"}}));}
 Json(json!({"success":true,"result":[{"id":"ours","type":"TXT","name":"_acme-challenge.example.com","content":"value","comment":"nwui:job:intent"},{"id":"other-owner","type":"TXT","name":"_acme-challenge.example.com","content":"value","comment":"nwui:other:intent"},{"id":"other-value","type":"TXT","name":"_acme-challenge.example.com","content":"other","comment":"nwui:job:intent"}],"result_info":{"total_pages":1}}))
 }).with_state(copy);
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", socket.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let cf = CloudflareClient::new(&base, "token", Duration::from_secs(2)).unwrap();
    cf.create_owned_txt(
        "zone",
        "_acme-challenge.example.com",
        "value",
        "nwui:job:intent",
    )
    .await
    .unwrap();
    assert_eq!(seen.lock().unwrap()[0]["comment"], "nwui:job:intent");
    let records = cf
        .find_owned_txt(
            "zone",
            "_acme-challenge.example.com",
            "value",
            "nwui:job:intent",
        )
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record_id, "ours");
    task.abort();
}
