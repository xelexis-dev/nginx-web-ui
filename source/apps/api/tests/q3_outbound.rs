mod outbound_support;
#[test]
fn webhook_literal_addresses_use_target_safety_policy() {
    for host in ["127.0.0.1", "2130706433", "0x7f000001", "[::1]"] {
        assert!(nwui_api::notify::parse_webhook_url(&format!("http://{host}:18081/hook")).is_ok(), "compatible local peer {host}");
    }
    for host in ["169.254.169.254", "0.0.0.0", "224.0.0.1", "255.255.255.255", "[fe80::1]", "[::ffff:127.0.0.1]"] {
        assert!(nwui_api::notify::parse_webhook_url(&format!("http://{host}:18081/hook")).is_err(), "unsafe {host}");
    }
    for host in ["10.10.0.2", "192.168.1.2", "[fd00::2]"] {
        assert!(nwui_api::notify::parse_webhook_url(&format!("http://{host}/hook")).is_ok(), "private peer {host}");
    }
}
#[tokio::test]
async fn smtp_internal_port_is_refused_before_connection() {
    use nwui_api::notifications::smtp::{MailerConfig,verify_config};
    let config=MailerConfig {host:"127.0.0.1".into(),port:3001,secure:false,username:None,password:None,from_email:"test@example.test".into(),from_name:None,reject_unauthorized:true};
    let result=verify_config(&config,&outbound_support::runtime_config()).await;
    assert!(!result.ok);
    assert_eq!(result.error.as_deref(),Some("SMTP 서버 설정이 올바르지 않습니다."));
}

#[tokio::test]
async fn node_transport_allows_a_noninternal_loopback_receiver() {
    use nwui_api::{nodes::client::{NodeHttp,NodeTarget},crypto::SecretBox};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move{axum::serve(listener,axum::Router::new().route("/api/v1/info",axum::routing::get(||async{"{}"}))).await.unwrap()});
    let result=NodeHttp::new("2.0.0",&outbound_support::runtime_config()).unwrap().call(&SecretBox::from_key([42;32]),&NodeTarget{node_version:None,base_url:url,token_enc:None,allow_insecure_tls:false},reqwest::Method::GET,"/api/v1/info",None,std::time::Duration::from_secs(1)).await;
    server.abort();assert!(result.is_ok(),"noninternal loopback receiver must stay compatible: {result:?}");
}
#[tokio::test]
async fn webhook_dns_allows_localhost_on_a_noninternal_port() {
    use nwui_api::notify::send::{client,send_webhook,WebhookRequest,SendOptions};
    let hits=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let seen=hits.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
    let server=tokio::spawn(async move{axum::serve(listener,axum::Router::new().route("/hook",axum::routing::post(move||{seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst);async{"ok"}}))).await.unwrap()});
    let result=send_webhook(&client(&outbound_support::runtime_config()),&WebhookRequest{url:format!("http://localhost:{port}/hook"),body:"{}".into(),content_type:"application/json".into(),headers:vec![]},&SendOptions{attempts:1,..Default::default()}).await;
    server.abort();assert!(result.ok,"{result:?}");assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst),1);
}

#[tokio::test]
async fn smtp_allows_a_noninternal_loopback_receiver() {
    use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};
    use nwui_api::notifications::smtp::{MailerConfig,verify_config};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
    let server=tokio::spawn(async move {
        let (stream,_)=listener.accept().await.unwrap();let (reader,mut writer)=stream.into_split();let mut reader=BufReader::new(reader);
        writer.write_all(b"220 test ready\r\n").await.unwrap();
        loop {let mut line=String::new();if reader.read_line(&mut line).await.unwrap()==0{break;}
            let quit=line.starts_with("QUIT");writer.write_all(if quit{b"221 bye\r\n"}else{b"250 ok\r\n"}).await.unwrap();if quit{break;}}
    });
    let config=MailerConfig{host:"127.0.0.1".into(),port,secure:false,username:None,password:None,from_email:"test@example.test".into(),from_name:None,reject_unauthorized:true};
    let result=verify_config(&config,&outbound_support::runtime_config()).await;server.abort();assert!(result.ok,"{result:?}");
}

#[tokio::test]
async fn node_transport_refuses_this_containers_internal_loopback_port() {
    use nwui_api::{nodes::client::{NodeHttp,NodeTarget},crypto::SecretBox};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:3001").await.unwrap();
    let hits=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let seen=hits.clone();
    let server=tokio::spawn(async move {axum::serve(listener,axum::Router::new().route("/api/v1/info",axum::routing::get(move||{seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst);async{"{}"}}))).await.unwrap()});
    let result=NodeHttp::new("2.0.0",&outbound_support::runtime_config()).unwrap().call(&SecretBox::from_key([42;32]),&NodeTarget{node_version:None,base_url:"http://127.0.0.1:3001".into(),token_enc:None,allow_insecure_tls:false},reqwest::Method::GET,"/api/v1/info",None,std::time::Duration::from_secs(1)).await;
    server.abort();assert!(!result.is_ok());assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst),0);
}

#[tokio::test]
async fn webhook_dns_refuses_localhost_on_an_internal_port() {
    use nwui_api::notify::send::{client,send_webhook,WebhookRequest,SendOptions};
    let hits=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let seen=hits.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();let port=listener.local_addr().unwrap().port();
    let server=tokio::spawn(async move{axum::serve(listener,axum::Router::new().route("/hook",axum::routing::post(move||{seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst);async{"ok"}}))).await.unwrap()});
    let result=send_webhook(&client(&outbound_support::runtime_config()),&WebhookRequest{url:format!("http://localhost:{port}/hook"),body:"{}".into(),content_type:"application/json".into(),headers:vec![]},&SendOptions{attempts:1,..Default::default()}).await;
    server.abort();assert!(!result.ok,"{result:?}");assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst),0);
}
