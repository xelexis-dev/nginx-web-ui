use super::*;
use hickory_resolver::proto::{
    op::{MessageType, Query},
    rr::{
        RData, Record, RecordType,
        rdata::{A, AAAA, CNAME},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    task::JoinHandle,
};

struct Fixture {
    mode: Arc<AtomicU8>,
    count: Arc<AtomicUsize>,
    endpoint: SocketAddr,
    tasks: Vec<JoinHandle<()>>,
    children: Arc<Mutex<Vec<JoinHandle<()>>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        for t in self.children.lock().unwrap().drain(..) {
            t.abort();
        }
    }
}
impl Fixture {
    async fn new(mode: u8) -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = tcp.local_addr().unwrap();
        let udp = Arc::new(UdpSocket::bind(endpoint).await.unwrap());
        let mode = Arc::new(AtomicU8::new(mode));
        let count = Arc::new(AtomicUsize::new(0));
        let children = Arc::new(Mutex::new(Vec::new()));
        let (m, c, child) = (mode.clone(), count.clone(), children.clone());
        let u = tokio::spawn(async move {
            let mut bytes = [0; 4096];
            while let Ok((n, peer)) = udp.recv_from(&mut bytes).await {
                c.fetch_add(1, Ordering::SeqCst);
                let mode = m.load(Ordering::SeqCst);
                if mode == 9 {
                    continue;
                }
                let q = Message::from_vec(&bytes[..n]).unwrap();
                if mode == 15 && q.queries[0].query_type == RecordType::AAAA {
                    continue;
                }
                let socket = udp.clone();
                child.lock().unwrap().push(tokio::spawn(async move {
                    let typ = q.queries[0].query_type;
                    if mode == 16 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    if mode == 11 && typ == RecordType::AAAA {
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                    }
                    if (mode == 12 && typ == RecordType::A)
                        || (mode == 13 && typ == RecordType::AAAA)
                    {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    let mut r = if mode == 12 || mode == 13 || mode == 14 || mode == 15 {
                        let failing = (mode == 13 && typ == RecordType::A)
                            || (mode != 13 && typ == RecordType::AAAA);
                        if failing {
                            response(&q, 4)
                        } else if mode == 13 {
                            response(&q, 2)
                        } else if mode == 12 {
                            response(&q, 1)
                        } else {
                            let mut r = response(&q, 0);
                            for record in &mut r.answers {
                                record.ttl = 30;
                            }
                            r
                        }
                    } else {
                        response(&q, mode)
                    };
                    if mode == 8 {
                        r.metadata.truncation = true;
                        r.answers.clear();
                    }
                    let _ = socket.send_to(&r.to_vec().unwrap(), peer).await;
                }));
            }
        });
        let (m, c) = (mode.clone(), count.clone());
        let t = tokio::spawn(async move {
            while let Ok((mut client, _)) = tcp.accept().await {
                c.fetch_add(1, Ordering::SeqCst);
                let Ok(size) = client.read_u16().await else {
                    continue;
                };
                let mut bytes = vec![0; usize::from(size)];
                if client.read_exact(&mut bytes).await.is_err() {
                    continue;
                }
                let q = Message::from_vec(&bytes).unwrap();
                let r = response(
                    &q,
                    if m.load(Ordering::SeqCst) == 8 {
                        0
                    } else {
                        m.load(Ordering::SeqCst)
                    },
                )
                .to_vec()
                .unwrap();
                let _ = client.write_u16(r.len() as u16).await;
                let _ = client.write_all(&r).await;
            }
        });
        Self {
            mode,
            count,
            endpoint,
            tasks: vec![u, t],
            children,
        }
    }
    fn guard(&self, ipv6: bool) -> Guard {
        Guard::new(vec![self.endpoint], ipv6)
    }
    fn mode(&self, value: u8) {
        self.mode.store(value, Ordering::SeqCst);
    }
}
fn query(kind: RecordType) -> Vec<u8> {
    let mut q = Message::new(412, MessageType::Query, OpCode::Query);
    q.metadata.recursion_desired = true;
    q.queries.push(Query::query(
        Name::from_ascii("peer.example.").unwrap(),
        kind,
    ));
    q.to_vec().unwrap()
}
fn response(q: &Message, mode: u8) -> Message {
    let mut r = Message::response(q.id, q.op_code);
    r.queries = q.queries.clone();
    let name = q.queries[0].name.clone();
    let typ = q.queries[0].query_type;
    if mode == 4 {
        r.metadata.response_code = ResponseCode::ServFail;
        return r;
    }
    if mode == 5 {
        r.metadata.response_code = ResponseCode::NXDomain;
        return r;
    }
    if mode == 10 {
        r.answers.push(Record::from_rdata(
            name.clone(),
            0,
            RData::CNAME(CNAME(name)),
        ));
        return r;
    }
    if mode == 3 {
        let alias = Name::from_ascii("unsafe.example.").unwrap();
        r.answers.push(Record::from_rdata(
            name,
            0,
            RData::CNAME(CNAME(alias.clone())),
        ));
        r.answers.push(Record::from_rdata(
            alias,
            0,
            RData::A(A("127.0.0.1".parse().unwrap())),
        ));
        return r;
    }
    let ttl = if mode == 6 || mode == 11 { 1 } else { 0 };
    if typ == RecordType::A {
        let ip = if mode == 1 { "127.0.0.1" } else { "172.18.0.2" };
        let count = if mode == 7 { 64 } else { 1 };
        for index in 0..count {
            let ip = if mode == 7 {
                std::net::Ipv4Addr::new(172, 18, 0, 2 + index)
            } else {
                ip.parse().unwrap()
            };
            r.answers
                .push(Record::from_rdata(name.clone(), ttl, RData::A(A(ip))));
        }
    } else if typ == RecordType::AAAA && mode != 7 {
        let ip = if mode == 2 {
            "::ffff:127.0.0.1"
        } else {
            "fd00::2"
        };
        r.answers.push(Record::from_rdata(
            name.clone(),
            ttl,
            RData::AAAA(AAAA(ip.parse().unwrap())),
        ));
    }
    // An unchecked additional record must never be copied to the client.
    r.additionals.push(Record::from_rdata(
        name,
        99,
        RData::A(A("127.0.0.1".parse().unwrap())),
    ));
    r
}
async fn ask(g: &Guard, kind: RecordType, tcp: bool) -> Message {
    Message::from_vec(&g.handle(&query(kind), tcp).await).unwrap()
}
#[tokio::test]
async fn safe_peer_answers_are_flattened_without_unchecked_additionals() {
    let f = Fixture::new(0).await;
    let g = f.guard(true);
    let r = ask(&g, RecordType::A, false).await;
    assert_eq!(r.response_code, ResponseCode::NoError);
    assert_eq!(r.answers.len(), 1);
    assert!(matches!(&r.answers[0].data,RData::A(A(ip)) if ip.to_string()=="172.18.0.2"));
    assert!(r.additionals.is_empty());
    assert_eq!(r.id, 412);
    assert_eq!(ask(&g, RecordType::AAAA, true).await.answers.len(), 1);
}
#[tokio::test]
async fn rebinding_mapped_mixed_families_and_unsafe_cname_return_nxdomain() {
    let f = Fixture::new(0).await;
    let g = f.guard(true);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
    for mode in [1, 2, 3] {
        f.mode(mode);
        let r = ask(&g, RecordType::A, false).await;
        assert_eq!(
            r.response_code,
            ResponseCode::NXDomain,
            "unsafe mode {mode}"
        );
        assert!(r.answers.is_empty());
        assert!(!r.authorities.is_empty());
        assert!(r.authorities.iter().all(|r| r.ttl <= 1));
    }
    f.mode(0);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
}
#[tokio::test]
async fn expired_cache_never_falls_back_and_cache_hits_are_filtered() {
    let f = Fixture::new(6).await;
    let g = f.guard(true);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
    let n = f.count.load(Ordering::SeqCst);
    f.mode(1);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
    assert_eq!(n, f.count.load(Ordering::SeqCst));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NXDomain
    );
    g.cache.lock().unwrap().insert(
        "peer.example.".into(),
        CachedAnswers {
            addresses: vec!["127.0.0.1".parse().unwrap()],
            deadline: Instant::now() + Duration::from_secs(1),
        },
    );
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NXDomain
    );
}
#[tokio::test]
async fn truncation_retries_upstream_tcp_and_truncates_only_udp_client_output() {
    let f = Fixture::new(8).await;
    let g = f.guard(false);
    assert_eq!(ask(&g, RecordType::A, false).await.answers.len(), 1);
    assert!(f.count.load(Ordering::SeqCst) >= 2);
    f.mode(7);
    let r = ask(&g, RecordType::A, false).await;
    assert!(r.truncation);
    assert!(r.answers.is_empty());
    assert_eq!(ask(&g, RecordType::A, true).await.answers.len(), 64);
}
#[tokio::test]
async fn ipv4_only_dns_ignores_target_aaaa_without_disabling_nameserver_families() {
    let f = Fixture::new(2).await;
    let g = f.guard(false);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
    let r = ask(&g, RecordType::AAAA, false).await;
    assert_eq!(r.response_code, ResponseCode::NoError);
    assert!(r.answers.is_empty());
}
#[tokio::test]
async fn failures_are_servfail_and_never_an_expired_safe_answer() {
    let f = Fixture::new(4).await;
    let mut g = f.guard(true);
    g.timeout = Duration::from_millis(60);
    assert_eq!(
        ask(&g, RecordType::A, false).await.response_code,
        ResponseCode::ServFail
    );
    f.mode(9);
    let begin = Instant::now();
    let r = ask(&g, RecordType::A, false).await;
    assert_eq!(r.response_code, ResponseCode::ServFail);
    assert!(r.answers.is_empty());
    assert!(begin.elapsed() < Duration::from_secs(1));
    f.mode(10);
    assert!(ask(&g, RecordType::A, false).await.answers.is_empty());
}
#[tokio::test]
async fn malformed_and_non_address_queries_never_reach_upstream() {
    let f = Fixture::new(0).await;
    let g = f.guard(true);
    let mut malformed = query(RecordType::A);
    malformed.truncate(14);
    malformed[12] = 0xc0;
    malformed[13] = 0x0c;
    let r = Message::from_vec(&g.handle(&malformed, false).await).unwrap();
    assert_eq!(r.response_code, ResponseCode::FormErr);
    let r = ask(&g, RecordType::TXT, false).await;
    assert_eq!(r.response_code, ResponseCode::NotImp);
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_slow_sibling_cannot_extend_the_other_familys_ttl() {
    let f = Fixture::new(11).await;
    let g = f.guard(true);
    let r = ask(&g, RecordType::A, false).await;
    assert_eq!(
        r.response_code,
        ResponseCode::ServFail,
        "do not return an address whose TTL expired during the sibling lookup"
    );
    assert!(r.answers.is_empty());
}
#[tokio::test]
async fn real_tcp_frames_are_bounded_and_a_bad_connection_does_not_stop_udp() {
    let f = Fixture::new(0).await;
    let g = Arc::new(f.guard(false));
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = tcp.local_addr().unwrap();
    let udp = UdpSocket::bind(endpoint).await.unwrap();
    assert!(tcp.local_addr().unwrap().ip().is_loopback());
    assert!(udp.local_addr().unwrap().ip().is_loopback());
    let (stop, done) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(serve_loop(udp, tcp, g, async {
        let _ = done.await;
    }));
    let mut invalid = tokio::net::TcpStream::connect(endpoint).await.unwrap();
    invalid.write_u16(u16::MAX).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), invalid.read_u8())
            .await
            .unwrap()
            .is_err()
    );
    let mut valid = tokio::net::TcpStream::connect(endpoint).await.unwrap();
    let q = query(RecordType::A);
    valid.write_u16(q.len() as u16).await.unwrap();
    valid.write_all(&q).await.unwrap();
    let len = valid.read_u16().await.unwrap();
    let mut bytes = vec![0; usize::from(len)];
    valid.read_exact(&mut bytes).await.unwrap();
    assert_eq!(Message::from_vec(&bytes).unwrap().answers.len(), 1);
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&q, endpoint).await.unwrap();
    let mut bytes = [0; 4096];
    let (n, _) = tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Message::from_vec(&bytes[..n]).unwrap().answers.len(), 1);
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
}
#[test]
fn answer_cache_is_bounded() {
    let g = Guard::new(vec![], false);
    for n in 0..(MAX_CACHE + 20) {
        g.insert_cache(
            format!("peer{n}."),
            CachedAnswers {
                addresses: vec!["172.18.0.2".parse().unwrap()],
                deadline: Instant::now() + Duration::from_secs(2),
            },
        );
    }
    assert_eq!(g.cache.lock().unwrap().len(), MAX_CACHE);
}

#[tokio::test]
async fn configured_secondary_is_used_within_the_total_deadline() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(0).await;
    let mut guard = fixture.guard(false);
    guard.servers.insert(0, silent.local_addr().unwrap());
    guard.timeout = Duration::from_millis(200);
    let started = Instant::now();
    assert_eq!(
        ask(&guard, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
    assert!(started.elapsed() < Duration::from_millis(200));
}
#[tokio::test]
async fn upstream_servfail_tries_the_next_configured_server() {
    let broken = Fixture::new(4).await;
    let fixture = Fixture::new(0).await;
    let mut guard = fixture.guard(false);
    guard.servers.insert(0, broken.endpoint);
    assert_eq!(
        ask(&guard, RecordType::A, false).await.response_code,
        ResponseCode::NoError
    );
}

#[tokio::test]
async fn fast_family_failure_never_hides_a_delayed_unsafe_sibling() {
    for mode in [12, 13] {
        let f = Fixture::new(mode).await;
        let g = f.guard(true);
        assert_eq!(
            ask(&g, RecordType::A, false).await.response_code,
            ResponseCode::NXDomain,
            "mode {mode}: fast failure must wait for sibling policy check"
        );
    }
}
#[tokio::test]
async fn verified_requested_family_survives_sibling_failure_without_cache() {
    let f = Fixture::new(14).await;
    let g = f.guard(true);
    let answer = ask(&g, RecordType::A, false).await;
    assert_eq!(answer.response_code, ResponseCode::NoError);
    assert_eq!(answer.answers.len(), 1);
    assert!(answer.answers[0].ttl <= 1);
    assert!(g.cache.lock().unwrap().is_empty());
    assert_eq!(
        ask(&g, RecordType::AAAA, false).await.response_code,
        ResponseCode::ServFail
    );
}

#[tokio::test]
async fn tcp_connection_pressure_cannot_starve_udp() {
    let f = Fixture::new(0).await;
    let guard = Arc::new(f.guard(false));
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = tcp.local_addr().unwrap();
    let udp = UdpSocket::bind(endpoint).await.unwrap();
    let (stop, done) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(serve_loop(udp, tcp, guard, async {
        let _ = done.await;
    }));
    let mut connecting = tokio::task::JoinSet::new();
    for _ in 0..128 {
        connecting.spawn(TcpStream::connect(endpoint));
    }
    let mut clients = Vec::new();
    while let Some(value) = connecting.join_next().await {
        clients.push(value.unwrap().unwrap());
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(&query(RecordType::A), endpoint)
        .await
        .unwrap();
    let mut bytes = [0; 4096];
    let response =
        tokio::time::timeout(Duration::from_millis(800), socket.recv_from(&mut bytes)).await;
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    let (n, _) = response
        .expect("128 trickling TCP clients must not take UDP's processing budget")
        .unwrap();
    assert_eq!(
        Message::from_vec(&bytes[..n]).unwrap().response_code,
        ResponseCode::NoError
    );
    drop(clients);
}

#[tokio::test]
async fn transient_accept_failure_retries_while_udp_continues_answering() {
    let f = Fixture::new(0).await;
    let guard = Arc::new(f.guard(false));
    let tcp = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
    let endpoint = tcp.local_addr().unwrap();
    let udp = UdpSocket::bind(endpoint).await.unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let (stop, done) = tokio::sync::oneshot::channel::<()>();
    let tries = count.clone();
    let serving = tokio::spawn(serve_with_accept(
        udp,
        guard,
        async {
            let _ = done.await;
        },
        move || {
            let listener = tcp.clone();
            let n = tries.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted))
                } else {
                    listener.accept().await
                }
            }
        },
    ));
    while count.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(&query(RecordType::A), endpoint)
        .await
        .unwrap();
    let mut bytes = [0; 4096];
    let (n, _) = tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        Message::from_vec(&bytes[..n]).unwrap().response_code,
        ResponseCode::NoError
    );
    let mut stream = TcpStream::connect(endpoint).await.unwrap();
    let q = query(RecordType::A);
    stream.write_u16(q.len() as u16).await.unwrap();
    stream.write_all(&q).await.unwrap();
    let len = tokio::time::timeout(Duration::from_secs(1), stream.read_u16())
        .await
        .unwrap()
        .unwrap();
    let mut bytes = vec![0; usize::from(len)];
    stream.read_exact(&mut bytes).await.unwrap();
    assert_eq!(Message::from_vec(&bytes).unwrap().answers.len(), 1);
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    assert!(count.load(Ordering::SeqCst) >= 2);
}
#[tokio::test]
async fn active_tcp_connection_has_a_total_lifetime_bound() {
    let f = Fixture::new(0).await;
    let guard = Arc::new(f.guard(false));
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = tcp.local_addr().unwrap();
    let udp = UdpSocket::bind(endpoint).await.unwrap();
    let (stop, done) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(serve_loop(udp, tcp, guard, async {
        let _ = done.await;
    }));
    let mut stream = TcpStream::connect(endpoint).await.unwrap();
    let mut closed = false;
    for _ in 0..8 {
        let request = async {
            let q = query(RecordType::A);
            stream.write_u16(q.len() as u16).await?;
            stream.write_all(&q).await?;
            let len = stream.read_u16().await?;
            let mut bytes = vec![0; usize::from(len)];
            stream.read_exact(&mut bytes).await?;
            Ok::<_, std::io::Error>(())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(2), request).await,
            Ok(Ok(()))
        ) {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(650)).await;
    }
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    assert!(
        closed,
        "active client must not renew its connection lifetime indefinitely"
    );
}

#[tokio::test]
async fn verified_requested_family_survives_sibling_timeout_without_cache() {
    let f = Fixture::new(15).await;
    let mut guard = f.guard(true);
    guard.timeout = Duration::from_millis(80);
    let started = Instant::now();
    let answer = ask(&guard, RecordType::A, false).await;
    assert_eq!(answer.response_code, ResponseCode::NoError);
    assert_eq!(answer.answers.len(), 1);
    assert!(answer.answers[0].ttl <= 1);
    assert!(guard.cache.lock().unwrap().is_empty());
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn socket_error_bursts_cannot_hide_rejection_notes() {
    let guard = Guard::new(vec![], true);
    for n in 0..100 {
        guard.note("socket", &format!("socket-{n}"), "temporary receive error");
    }
    assert!(
        guard
            .logs
            .lock()
            .unwrap()
            .line("rejected", "unsafe.test", "unsafe address", Instant::now())
            .is_some()
    );
}

#[tokio::test]
async fn default_partial_answer_arrives_before_nginx_resolver_deadline() {
    let f = Fixture::new(15).await;
    let guard = f.guard(true);
    let answer = tokio::time::timeout(
        Duration::from_millis(1800),
        ask(&guard, RecordType::A, false),
    )
    .await
    .expect("verified family must beat nginx's 2 second resolver timeout");
    assert_eq!(answer.response_code, ResponseCode::NoError);
    assert_eq!(answer.answers.len(), 1);
    assert!(answer.answers[0].ttl <= 1);
    assert!(guard.cache.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_clients_share_each_upstream_family_without_changing_answers() {
    let f = Fixture::new(16).await;
    let g = Arc::new(f.guard(true));
    let barrier = Arc::new(tokio::sync::Barrier::new(17));
    let mut clients = JoinSet::new();
    for index in 0..16 {
        let (g, barrier) = (g.clone(), barrier.clone());
        clients.spawn(async move {
            let kind = if index % 2 == 0 {
                RecordType::A
            } else {
                RecordType::AAAA
            };
            barrier.wait().await;
            let mut request = Message::from_vec(&query(kind)).unwrap();
            request.metadata.id = index as u16;
            let r = Message::from_vec(&g.handle(&request.to_vec().unwrap(), false).await).unwrap();
            assert_eq!(r.response_code, ResponseCode::NoError);
            assert_eq!(r.answers.len(), 1);
            assert_eq!(r.answers[0].record_type(), kind);
            assert_eq!(r.answers[0].ttl, 0);
            assert_eq!(r.id, index as u16);
            assert!(r.additionals.is_empty());
        });
    }
    barrier.wait().await;
    while let Some(result) = clients.join_next().await {
        result.unwrap();
    }
    assert_eq!(
        f.count.load(Ordering::SeqCst),
        2,
        "one A and one AAAA upstream query"
    );
}

#[tokio::test]
async fn shared_upstream_timeout_does_not_poison_the_next_lookup() {
    let f = Fixture::new(9).await;
    let mut g = f.guard(true);
    g.timeout = Duration::from_millis(100);
    let g = Arc::new(g);
    let barrier = Arc::new(tokio::sync::Barrier::new(17));
    let mut clients = JoinSet::new();
    for _ in 0..16 {
        let (g, barrier) = (g.clone(), barrier.clone());
        clients.spawn(async move {
            barrier.wait().await;
            g.forward(Query::query(
                Name::from_ascii("peer.example.").unwrap(),
                RecordType::A,
            ))
            .await
        });
    }
    barrier.wait().await;
    while let Some(result) = clients.join_next().await {
        assert!(matches!(result.unwrap(), Err(Failure::Unavailable)));
    }
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
    f.mode(0);
    let r = g
        .forward(Query::query(
            Name::from_ascii("peer.example.").unwrap(),
            RecordType::A,
        ))
        .await
        .unwrap();
    assert_eq!(r.response_code, ResponseCode::NoError);
    assert_eq!(f.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancelled_upstream_leader_wakes_waiters_and_allows_retry() {
    let f = Fixture::new(9).await;
    let g = Arc::new(f.guard(true));
    let q = Query::query(Name::from_ascii("peer.example.").unwrap(), RecordType::A);
    let (owner, question) = (g.clone(), q.clone());
    let leader = tokio::spawn(async move { owner.forward(question).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.count.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let waiter = g.forward(q.clone());
    tokio::pin!(waiter);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut waiter)
            .await
            .is_err()
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap(),
        Err(Failure::Unavailable)
    ));
    f.mode(0);
    assert!(g.forward(q).await.is_ok());
    assert_eq!(f.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_upstream_lookup_does_not_poison_the_next_lookup() {
    let f = Fixture::new(4).await;
    let g = f.guard(true);
    let q = Query::query(Name::from_ascii("peer.example.").unwrap(), RecordType::A);
    assert!(matches!(
        g.forward(q.clone()).await,
        Err(Failure::Unavailable)
    ));
    f.mode(0);
    assert!(g.forward(q).await.is_ok());
    assert_eq!(f.count.load(Ordering::SeqCst), 2);
}
