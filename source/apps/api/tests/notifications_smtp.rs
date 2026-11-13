mod outbound_support;
use nwui_api::notifications::smtp::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
async fn fake() -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind((outbound_support::peer_ip(),0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = seen.clone();
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            let mut lines = BufReader::new(read).lines();
            write.write_all(b"220 local.test ESMTP\r\n").await.unwrap();
            let mut data = false;
            while let Ok(Some(line)) = lines.next_line().await {
                records.lock().unwrap().push(line.clone());
                let reply = if data {
                    if line == "." {
                        data = false;
                        Some("250 accepted\r\n")
                    } else {
                        None
                    }
                } else if line.starts_with("EHLO") {
                    Some("250-local.test\r\n250-8BITMIME\r\n250 AUTH PLAIN LOGIN\r\n")
                } else if line == "DATA" {
                    data = true;
                    Some("354 continue\r\n")
                } else if line == "QUIT" {
                    Some("221 bye\r\n")
                } else {
                    Some("250 OK\r\n")
                };
                if let Some(reply) = reply {
                    if write.write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
                if line == "QUIT" {
                    break;
                }
            }
        }
    });
    (port, seen, task)
}
fn config(port: u16) -> MailerConfig {
    MailerConfig {
        host: outbound_support::peer_host(),
        port,
        secure: false,
        username: None,
        password: None,
        from_email: "from@example.test".into(),
        from_name: Some("알림 서버".into()),
        reject_unauthorized: true,
    }
}
fn input() -> MailInput {
    MailInput {
        to: "to@example.test".into(),
        subject: "인증서 갱신 필요".into(),
        text: "plain\n.dot\nfinish".into(),
        html: Some("<p>html</p>".into()),
        message_id: Some("<stable@example.test>".into()),
    }
}
#[tokio::test]
async fn smtp_rfc_headers_alternatives_verify_and_stable_message_id() {
    let (port, seen, task) = fake().await;
    let config = config(port);
    assert!(verify_config(&config,&outbound_support::runtime_config()).await.ok);
    assert!(!seen.lock().unwrap().iter().any(|s| s == "DATA"));
    let sent = send_with_config(&config, &input(),&outbound_support::runtime_config()).await;
    assert!(sent.ok, "{:?}", sent.error);
    let log = seen.lock().unwrap().join("\n");
    assert!(log.contains("MAIL FROM:<from@example.test>"));
    assert!(log.contains("RCPT TO:<to@example.test>"));
    assert!(log.contains("multipart/alternative"));
    assert!(log.contains("text/plain"));
    assert!(log.contains("text/html"));
    assert!(log.contains("Message-ID: <stable@example.test>"));
    assert!(log.contains("=?utf-8?"));
    task.abort();
}
#[tokio::test]
async fn credentials_never_cross_plaintext_and_invalid_header_is_rejected() {
    let (port, seen, task) = fake().await;
    let mut cfg = config(port);
    cfg.username = Some("synthetic-user".into());
    cfg.password = Some("synthetic-password".into());
    let outcome = send_with_config(&cfg, &input(),&outbound_support::runtime_config()).await;
    assert!(!outcome.ok);
    let log = seen.lock().unwrap().join("\n");
    assert!(!log.contains("AUTH PLAIN"));
    assert!(!log.contains("AUTH LOGIN"));
    assert!(!outcome.error.unwrap().contains("synthetic"));
    cfg.username = None;
    let mut input = input();
    input.subject = "subject\r\nBcc: attack@example.test".into();
    assert!(!send_with_config(&cfg, &input,&outbound_support::runtime_config()).await.ok);
    task.abort();
}
