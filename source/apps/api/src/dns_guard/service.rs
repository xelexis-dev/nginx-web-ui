//! A bounded DNS stub, not a transparent forwarder: emit only verified final A/AAAA records.
use super::{TIMEOUT_SECONDS, VALID_SECONDS, policy::unsafe_address};
use hickory_resolver::proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{
        DNSClass, Name, RData, Record, RecordType,
        rdata::{A, AAAA, SOA},
    },
    serialize::binary::{BinDecodable, BinDecoder},
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::watch,
    task::JoinSet,
};

mod logs;

pub(super) const MAX_PACKET: usize = 4096;
const MAX_CNAME: usize = 8;
const MAX_ADDRESSES: usize = 64;
const MAX_RECORDS: usize = 128;
const MAX_CACHE: usize = 1024;
const MAX_UDP_CONCURRENT: usize = 128;
const MAX_TCP_CONCURRENT: usize = 32;

pub(super) struct CachedAnswers {
    pub addresses: Vec<IpAddr>,
    pub deadline: Instant,
}
pub(super) struct Guard {
    pub servers: Vec<SocketAddr>,
    pub ipv6: bool,
    pub timeout: Duration,
    pub cache: Mutex<HashMap<String, CachedAnswers>>,
    logs: Mutex<logs::Budget>,
    socket_logs: Mutex<logs::Budget>,
    in_flight: Mutex<HashMap<(Name, RecordType), SharedLookup>>,
}
#[derive(Debug, Clone)]
enum Failure {
    Rejected(&'static str),
    Unavailable,
}
impl<E: std::error::Error> From<E> for Failure {
    fn from(_: E) -> Self {
        Self::Unavailable
    }
}
type SharedLookup = watch::Receiver<Option<Result<Message, Failure>>>;

struct PendingLookup<'a> {
    guard: &'a Guard,
    key: (Name, RecordType),
}
impl Drop for PendingLookup<'_> {
    fn drop(&mut self) {
        // Cancellation must allow a new attempt and close the channel for existing waiters.
        self.guard.in_flight.lock().unwrap().remove(&self.key);
    }
}
struct Family {
    addresses: Vec<IpAddr>,
    ttl: u32,
    nxdomain: bool,
}
struct Answer {
    addresses: Vec<IpAddr>,
    ttl: u32,
    code: ResponseCode,
}

fn parse(bytes: &[u8]) -> Result<Message, Failure> {
    if !(12..=MAX_PACKET).contains(&bytes.len()) {
        return Err(Failure::Unavailable);
    }
    // Bound declared counts before the protocol decoder can allocate from them.
    let count = |offset| usize::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));
    if count(4) != 1 || count(6) + count(8) + count(10) > MAX_RECORDS {
        return Err(Failure::Unavailable);
    }
    let mut decoder = BinDecoder::new(bytes);
    let value = Message::read(&mut decoder)?;
    if decoder.index() != bytes.len() {
        return Err(Failure::Unavailable);
    }
    Ok(value)
}
fn base(query: &Message, code: ResponseCode) -> Message {
    let mut value = Message::response(query.id, OpCode::Query);
    value.metadata.recursion_desired = query.recursion_desired;
    value.metadata.recursion_available = true;
    value.metadata.response_code = code;
    value.queries = query.queries.clone();
    // Do not copy client/upstream EDNS options, additional records, signatures or AD claims.
    value
}
fn encoded(mut value: Message, tcp: bool) -> Vec<u8> {
    let bytes = value.to_vec().unwrap_or_default();
    if bytes.len() > if tcp { MAX_PACKET } else { 512 } {
        value = value.truncate();
        return value.to_vec().unwrap_or_default();
    }
    bytes
}
fn negative(query: &Message, code: ResponseCode, tcp: bool) -> Vec<u8> {
    let mut value = base(query, code);
    if code == ResponseCode::NXDomain {
        if let Some(q) = query.queries.first() {
            value.authorities.push(Record::from_rdata(
                q.name.clone(),
                1,
                RData::SOA(SOA::new(Name::root(), Name::root(), 0, 1, 1, 1, 1)),
            ));
        }
    }
    encoded(value, tcp)
}
impl Guard {
    pub fn new(servers: Vec<SocketAddr>, ipv6: bool) -> Self {
        Self {
            servers,
            ipv6,
            // Leave time to deliver a verified partial answer before nginx times out.
            timeout: Duration::from_millis(1500),
            cache: Mutex::new(HashMap::new()),
            logs: Mutex::new(logs::Budget::new()),
            socket_logs: Mutex::new(logs::Budget::new()),
            in_flight: Mutex::new(HashMap::new()),
        }
    }
    fn note(&self, category: &str, subject: &str, reason: &str) {
        let budget = if category == "socket" {
            &self.socket_logs
        } else {
            &self.logs
        };
        let line = match budget.lock() {
            Ok(mut budget) => budget.line(category, subject, reason, Instant::now()),
            Err(_) => None,
        };
        if let Some(line) = line {
            super::startup_note(format_args!("{line}\n"));
        }
    }
    fn insert_cache(&self, key: String, answer: CachedAnswers) {
        if answer.deadline <= Instant::now() {
            return;
        }
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= MAX_CACHE {
            cache.retain(|_, v| v.deadline > Instant::now());
            if cache.len() >= MAX_CACHE {
                if let Some(key) = cache.keys().next().cloned() {
                    cache.remove(&key);
                }
            }
        }
        cache.insert(key, answer);
    }
    async fn lookup(&self, name: &Name, queried: RecordType) -> Result<Answer, Failure> {
        let key = name.to_ascii().to_ascii_lowercase();
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(answer) = cache.get(&key) {
                if answer.addresses.iter().any(|ip| unsafe_address(*ip)) {
                    cache.remove(&key);
                    return Err(Failure::Rejected("unsafe cached address"));
                }
                if answer.deadline > Instant::now() {
                    return Ok(Answer {
                        addresses: answer.addresses.clone(),
                        ttl: answer
                            .deadline
                            .saturating_duration_since(Instant::now())
                            .as_secs() as u32,
                        code: ResponseCode::NoError,
                    });
                }
            }
            cache.remove(&key);
        }
        let started = Instant::now();
        let (a, aaaa, cacheable) = self.resolve_families(name, queried).await?;
        let mut addresses = a.addresses;
        let mut ttl = a.ttl;
        let mut nx = a.nxdomain;
        if let Some(v6) = aaaa {
            addresses.extend(v6.addresses);
            ttl = ttl.min(v6.ttl);
            nx &= v6.nxdomain;
        }
        if addresses.len() > MAX_ADDRESSES {
            return Err(Failure::Unavailable);
        }
        if addresses.iter().any(|ip| unsafe_address(*ip)) {
            return Err(Failure::Rejected("unsafe final address"));
        }
        ttl = ttl.min(VALID_SECONDS);
        if !addresses.is_empty() && ttl > 0 {
            // Anchor before both lookups: waiting for a sibling or CNAME can only shorten
            // a received TTL. It must not extend the first family's lease.
            let deadline = started + Duration::from_secs(u64::from(ttl));
            if deadline <= Instant::now() {
                return Err(Failure::Unavailable);
            }
            if cacheable {
                self.insert_cache(
                    key,
                    CachedAnswers {
                        addresses: addresses.clone(),
                        deadline,
                    },
                );
            }
            ttl = deadline.saturating_duration_since(Instant::now()).as_secs() as u32;
        }
        if !cacheable {
            ttl = ttl.min(1);
        }
        Ok(Answer {
            code: if addresses.is_empty() && nx {
                ResponseCode::NXDomain
            } else {
                ResponseCode::NoError
            },
            addresses,
            ttl,
        })
    }
    async fn resolve_families(
        &self,
        name: &Name,
        queried: RecordType,
    ) -> Result<(Family, Option<Family>, bool), Failure> {
        let a = self.family(name, RecordType::A);
        let b = async {
            if self.ipv6 {
                self.family(name, RecordType::AAAA).await
            } else {
                Ok(Family {
                    addresses: vec![],
                    ttl: VALID_SECONDS,
                    nxdomain: true,
                })
            }
        };
        let deadline = tokio::time::sleep(self.timeout);
        tokio::pin!(a, b, deadline);
        let (mut left, mut right) = (None, None);
        // A transport failure never cancels a sibling policy check. Rejections
        // win immediately; a verified queried family may survive the sibling's
        // failure/deadline only with a short TTL and no cache entry.
        while left.is_none() || right.is_none() {
            tokio::select! {
                biased;
                value=&mut a, if left.is_none()=>match value {
                    Err(Failure::Rejected(reason))=>return Err(Failure::Rejected(reason)),
                    value=>left=Some(value),
                },
                value=&mut b, if right.is_none()=>match value {
                    Err(Failure::Rejected(reason))=>return Err(Failure::Rejected(reason)),
                    value=>right=Some(value),
                },
                _=&mut deadline=>break,
            }
        }
        match (
            left.unwrap_or(Err(Failure::Unavailable)),
            right.unwrap_or(Err(Failure::Unavailable)),
        ) {
            (Ok(a), Ok(b)) => Ok((a, Some(b), true)),
            (Ok(a), Err(Failure::Unavailable)) if queried == RecordType::A => Ok((a, None, false)),
            (Err(Failure::Unavailable), Ok(b)) if queried == RecordType::AAAA => {
                Ok((b, None, false))
            }
            _ => Err(Failure::Unavailable),
        }
    }
    async fn family(&self, name: &Name, kind: RecordType) -> Result<Family, Failure> {
        let mut current = name.clone();
        let mut seen = HashSet::from([current.clone()]);
        let mut hops = 0;
        let mut ttl = VALID_SECONDS;
        loop {
            let query = Query::query(current.clone(), kind);
            let response = self.forward(query).await?;
            if response.answers.len() > MAX_RECORDS {
                return Err(Failure::Unavailable);
            }
            match response.response_code {
                ResponseCode::NXDomain => {
                    return Ok(Family {
                        addresses: vec![],
                        ttl: 1,
                        nxdomain: true,
                    });
                }
                ResponseCode::NoError => {}
                _ => return Err(Failure::Unavailable),
            }
            // Inspect every address in the answer section, including intermediate CNAME data.
            // Additional records are ignored entirely and cannot poison a subsequent lookup.
            for record in &response.answers {
                let ip = match &record.data {
                    RData::A(A(ip)) => Some(IpAddr::V4(*ip)),
                    RData::AAAA(AAAA(ip)) => Some(IpAddr::V6(*ip)),
                    _ => None,
                };
                if ip.is_some_and(unsafe_address) {
                    return Err(Failure::Rejected("unsafe DNS answer or CNAME destination"));
                }
            }
            let original = current.clone();
            loop {
                let mut addresses = Vec::new();
                let mut aliases = Vec::new();
                for record in response
                    .answers
                    .iter()
                    .filter(|r| r.name == current && r.dns_class == DNSClass::IN)
                {
                    match &record.data {
                        RData::A(A(ip)) if kind == RecordType::A => {
                            addresses.push(IpAddr::V4(*ip));
                            ttl = ttl.min(record.ttl);
                        }
                        RData::AAAA(AAAA(ip)) if kind == RecordType::AAAA => {
                            addresses.push(IpAddr::V6(*ip));
                            ttl = ttl.min(record.ttl);
                        }
                        RData::CNAME(v) => {
                            aliases.push(v.0.clone());
                            ttl = ttl.min(record.ttl);
                        }
                        _ => {}
                    }
                }
                if !addresses.is_empty() {
                    if !aliases.is_empty() || addresses.len() > MAX_ADDRESSES {
                        return Err(Failure::Unavailable);
                    }
                    return Ok(Family {
                        addresses,
                        ttl,
                        nxdomain: false,
                    });
                }
                if aliases.is_empty() {
                    if current == original {
                        return Ok(Family {
                            addresses: vec![],
                            ttl,
                            nxdomain: false,
                        });
                    }
                    break; // CNAME target was not included: query its exact FQDN, never NSS/hosts.
                }
                if aliases.len() != 1 || hops >= MAX_CNAME {
                    return Err(Failure::Unavailable);
                }
                current = aliases.remove(0);
                hops += 1;
                if !seen.insert(current.clone()) {
                    return Err(Failure::Unavailable);
                }
            }
        }
    }
    async fn forward(&self, query: Query) -> Result<Message, Failure> {
        let key = (query.name.clone(), query.query_type);
        let (mut receiver, sender) = {
            let mut pending = self.in_flight.lock().unwrap();
            if let Some(receiver) = pending.get(&key) {
                (receiver.clone(), None)
            } else {
                let (sender, receiver) = watch::channel(None);
                pending.insert(key.clone(), receiver.clone());
                (receiver, Some(sender))
            }
        };
        if let Some(sender) = sender {
            let _pending = PendingLookup { guard: self, key };
            let result = self.forward_upstream(query).await;
            sender.send_replace(Some(result.clone()));
            return result;
        }
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver.changed().await.map_err(|_| Failure::Unavailable)?;
        }
    }
    async fn forward_upstream(&self, query: Query) -> Result<Message, Failure> {
        // Share the end-to-end deadline across configured resolvers. A silent
        // first server must not consume every attempt at the remaining servers.
        let budget = self.timeout / (self.servers.len().max(1) as u32);
        for server in &self.servers {
            match tokio::time::timeout(budget, self.exchange(*server, &query)).await {
                Ok(Ok(value))
                    if matches!(
                        value.response_code,
                        ResponseCode::NoError | ResponseCode::NXDomain
                    ) =>
                {
                    return Ok(value);
                }
                _ => continue,
            }
        }
        Err(Failure::Unavailable)
    }
    async fn exchange(&self, server: SocketAddr, query: &Query) -> Result<Message, Failure> {
        let mut random = [0; 2];
        getrandom::fill(&mut random)?;
        let id = u16::from_be_bytes(random);
        let mut request = Message::new(id, MessageType::Query, OpCode::Query);
        request.metadata.recursion_desired = true;
        request.queries.push(query.clone());
        let wire = request.to_vec()?;
        let bind = if server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).await?;
        socket.connect(server).await?;
        socket.send(&wire).await?;
        let mut buf = [0; MAX_PACKET + 1];
        let count = socket.recv(&mut buf).await?;
        if count > MAX_PACKET || count < 12 || buf[..2] != random {
            return Err(Failure::Unavailable);
        }
        let bytes = if buf[2] & 0x02 != 0 {
            let mut tcp = TcpStream::connect(server).await?;
            tcp.write_u16(wire.len() as u16).await?;
            tcp.write_all(&wire).await?;
            let len = usize::from(tcp.read_u16().await?);
            if !(12..=MAX_PACKET).contains(&len) {
                return Err(Failure::Unavailable);
            }
            let mut buf = vec![0; len];
            tcp.read_exact(&mut buf).await?;
            buf
        } else {
            buf[..count].to_vec()
        };
        let response = parse(&bytes)?;
        if response.id != id
            || response.message_type != MessageType::Response
            || response.op_code != OpCode::Query
            || response.truncation
            || response.queries != [query.clone()]
        {
            return Err(Failure::Unavailable);
        }
        Ok(response)
    }
    pub async fn handle(&self, bytes: &[u8], tcp: bool) -> Vec<u8> {
        let query = match parse(bytes) {
            Ok(value) => value,
            Err(_) => {
                let id = bytes
                    .get(..2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .unwrap_or(0);
                return encoded(
                    Message::error_msg(id, OpCode::Query, ResponseCode::FormErr),
                    tcp,
                );
            }
        };
        if query.message_type != MessageType::Query
            || query.queries.len() != 1
            || !query.answers.is_empty()
            || !query.authorities.is_empty()
            || !query.additionals.is_empty()
            || query.signature.is_some()
        {
            return negative(&query, ResponseCode::FormErr, tcp);
        }
        let q = &query.queries[0];
        if query.op_code != OpCode::Query
            || q.query_class != DNSClass::IN
            || !matches!(q.query_type, RecordType::A | RecordType::AAAA)
        {
            return negative(&query, ResponseCode::NotImp, tcp);
        }
        if q.query_type == RecordType::AAAA && !self.ipv6 {
            return negative(&query, ResponseCode::NoError, tcp);
        }
        let answer = match self.lookup(&q.name, q.query_type).await {
            Ok(value) => value,
            Err(Failure::Rejected(reason)) => {
                self.note("rejected", &q.name.to_ascii(), reason);
                return negative(&query, ResponseCode::NXDomain, tcp);
            }
            _ => return negative(&query, ResponseCode::ServFail, tcp),
        };
        if answer.code != ResponseCode::NoError {
            return negative(&query, answer.code, tcp);
        }
        let mut response = base(&query, ResponseCode::NoError);
        for ip in answer.addresses {
            let data = match ip {
                IpAddr::V4(ip) if q.query_type == RecordType::A => Some(RData::A(A(ip))),
                IpAddr::V6(ip) if q.query_type == RecordType::AAAA => Some(RData::AAAA(AAAA(ip))),
                _ => None,
            };
            if let Some(data) = data {
                response
                    .answers
                    .push(Record::from_rdata(q.name.clone(), answer.ttl, data));
            }
        }
        encoded(response, tcp)
    }
}

async fn udp_worker(udp: UdpSocket, guard: Arc<Guard>, mut stop: watch::Receiver<()>) {
    let udp = Arc::new(udp);
    let mut tasks = JoinSet::new();
    let mut bytes = [0; MAX_PACKET + 1];
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            packet=udp.recv_from(&mut bytes), if tasks.len()<MAX_UDP_CONCURRENT=>{
                let (count,peer)=match packet {Ok(value)=>value,Err(error)=>{
                    guard.note("socket","UDP receive",&error.to_string());
                    tokio::time::sleep(Duration::from_millis(50)).await;continue;
                }};
                let (guard,socket,data)=(guard.clone(),udp.clone(),bytes[..count].to_vec());
                tasks.spawn(async move {let answer=guard.handle(&data,false).await;let _=socket.send_to(&answer,peer).await;});
            },
            _=tasks.join_next(), if !tasks.is_empty()=>{},
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
async fn tcp_client(mut stream: TcpStream, guard: Arc<Guard>) {
    let connection = async {
        for _ in 0..16 {
            // A trickling client has one second to start/complete each frame,
            // and the outer deadline caps the whole connection at four seconds.
            let len = tokio::time::timeout(Duration::from_secs(1), stream.read_u16()).await??;
            let len = usize::from(len);
            if !(12..=MAX_PACKET).contains(&len) {
                guard.note("rejected", "malformed TCP frame", "invalid length");
                return Err(std::io::Error::other("invalid DNS TCP frame"));
            }
            let mut bytes = vec![0; len];
            tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut bytes)).await??;
            let reply = guard.handle(&bytes, true).await;
            tokio::time::timeout(Duration::from_secs(1), async {
                stream.write_u16(reply.len() as u16).await?;
                stream.write_all(&reply).await
            })
            .await??;
        }
        Ok::<_, std::io::Error>(())
    };
    let _ = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECONDS * 2), connection).await;
}
async fn tcp_worker<A, F>(mut accept: A, guard: Arc<Guard>, mut stop: watch::Receiver<()>)
where
    A: FnMut() -> F,
    F: Future<Output = std::io::Result<(TcpStream, SocketAddr)>>,
{
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            accepted=accept(), if tasks.len()<MAX_TCP_CONCURRENT=>{
                let (stream,_)=match accepted {Ok(value)=>value,Err(error)=>{
                    guard.note("socket","TCP accept",&error.to_string());
                    // Independent transport tasks keep this backoff from delaying UDP.
                    tokio::time::sleep(Duration::from_millis(50)).await;continue;
                }};
                tasks.spawn(tcp_client(stream,guard.clone()));
            },
            _=tasks.join_next(), if !tasks.is_empty()=>{},
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
async fn serve_with_accept<A, F>(
    udp: UdpSocket,
    guard: Arc<Guard>,
    stop: impl Future<Output = ()>,
    accept: A,
) -> anyhow::Result<()>
where
    A: FnMut() -> F + Send + 'static,
    F: Future<Output = std::io::Result<(TcpStream, SocketAddr)>> + Send + 'static,
{
    let (shutdown, receiver) = watch::channel(());
    let mut udp = tokio::spawn(udp_worker(udp, guard.clone(), receiver.clone()));
    let mut tcp = tokio::spawn(tcp_worker(accept, guard, receiver));
    tokio::pin!(stop);
    enum End {
        Stop,
        Udp(Result<(), tokio::task::JoinError>),
        Tcp(Result<(), tokio::task::JoinError>),
    }
    let end = tokio::select! {_=&mut stop=>End::Stop,result=&mut udp=>End::Udp(result),result=&mut tcp=>End::Tcp(result)};
    let _ = shutdown.send(());
    match end {
        End::Stop => {
            let (a, b) = tokio::join!(udp, tcp);
            a?;
            b?;
        }
        End::Udp(result) => {
            let other = tcp.await;
            result?;
            other?;
        }
        End::Tcp(result) => {
            let other = udp.await;
            result?;
            other?;
        }
    }
    Ok(())
}
pub(super) async fn serve_loop(
    udp: UdpSocket,
    tcp: TcpListener,
    guard: Arc<Guard>,
    stop: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    let tcp = Arc::new(tcp);
    serve_with_accept(udp, guard, stop, move || {
        let tcp = tcp.clone();
        async move { tcp.accept().await }
    })
    .await
}
#[cfg(test)]
mod tests;
