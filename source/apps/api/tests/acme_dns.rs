use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig},
    proto::{
        op::{Message, ResponseCode},
        rr::{RData, Record, rdata::TXT},
    },
};
use nwui_api::certificates::protocol::{DnsTxtStatus, DnsVerifier};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn txt_chunks_are_joined_and_propagation_queries_do_not_reuse_cached_answers() {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let task = tokio::spawn(async move {
        let mut buf = [0; 4096];
        loop {
            let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
            let query = Message::from_vec(&buf[..len]).unwrap();
            let mut answer = Message::response(query.id, query.op_code);
            answer.metadata.recursion_available = true;
            answer.add_queries(query.queries.clone());
            let n = seen.fetch_add(1, Ordering::SeqCst);
            let chunks = if n == 0 {
                vec!["old".into()]
            } else {
                vec!["expected".into(), "value".into()]
            };
            answer.add_answer(Record::from_rdata(
                query.queries[0].name().clone(),
                600,
                RData::TXT(TXT::new(chunks)),
            ));
            socket
                .send_to(&answer.to_vec().unwrap(), peer)
                .await
                .unwrap();
        }
    });
    let mut ns = NameServerConfig::udp(addr.ip());
    ns.connections[0].port = addr.port();
    let verifier = DnsVerifier::with_config(
        ResolverConfig::from_name_servers(vec![ns]),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        verifier
            .verify("_acme-challenge.example.com", "expectedvalue")
            .await,
        DnsTxtStatus::Missing
    );
    assert_eq!(
        verifier
            .verify("_acme-challenge.example.com", "expectedvalue")
            .await,
        DnsTxtStatus::Ok
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn nxdomain_is_missing_but_dns_timeout_is_inconclusive() {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut buf = [0; 4096];
        loop {
            let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
            let query = Message::from_vec(&buf[..len]).unwrap();
            if query.queries[0].name().to_string().starts_with("timeout.") {
                continue;
            }
            let mut answer = Message::response(query.id, query.op_code);
            answer.metadata.recursion_available = true;
            answer.metadata.response_code = ResponseCode::NXDomain;
            answer.add_queries(query.queries.clone());
            socket
                .send_to(&answer.to_vec().unwrap(), peer)
                .await
                .unwrap();
        }
    });
    let mut ns = NameServerConfig::udp(addr.ip());
    ns.connections[0].port = addr.port();
    let verifier = DnsVerifier::with_config(
        ResolverConfig::from_name_servers(vec![ns]),
        Duration::from_millis(100),
    )
    .unwrap();
    assert_eq!(
        verifier.verify("missing.example.com", "value").await,
        DnsTxtStatus::Missing
    );
    assert_eq!(
        verifier.verify("timeout.example.com", "value").await,
        DnsTxtStatus::Inconclusive
    );
    task.abort();
}
