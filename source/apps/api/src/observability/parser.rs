//! The deployed nginx combined_host format, including old combined lines without a host.
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

pub const MAX_LINE_BYTES: usize = 192 * 1024;
pub const MINUTE_MS: i64 = 60_000;
pub const HOST_OTHER: &str = "__other__";

#[derive(Debug, PartialEq)]
pub struct AccessEntry {
    pub time_ms: i64,
    pub host: String,
    pub status: u32,
    pub bytes_sent: i64,
}

pub fn normalize_host(raw: &str) -> String {
    if raw.is_empty() {
        return "__empty_host__".into();
    }
    if raw == "_" {
        return "__default_vhost__".into();
    }
    if raw.len() > 259 {
        return "__invalid_host__".into();
    }
    let host = raw.to_ascii_lowercase();
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            let inner = &host[1..end];
            if !inner.is_empty()
                && inner
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() || c == b':' || c == b'.')
            {
                return host[..=end].into();
            }
        }
        return "__invalid_host__".into();
    }
    let host = host.split(':').next().unwrap_or("");
    if host.is_empty() {
        return "__empty_host__".into();
    }
    if host == "_" {
        return "__default_vhost__".into();
    }
    if host.len() > 253
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return "__invalid_host__".into();
    }
    host.into()
}

pub fn parse_access_line(line: &str) -> Option<AccessEntry> {
    if line.len() > MAX_LINE_BYTES {
        return None;
    }
    let left = line.find('[')?;
    let right = left + 1 + line[left + 1..].find(']')?;
    let stamp = &line[left + 1..right];
    if stamp.len() != 26 {
        return None;
    }
    let time_ms = chrono::DateTime::parse_from_str(stamp, "%d/%b/%Y:%H:%M:%S %z")
        .ok()?
        .timestamp_millis();
    let quotes: Vec<_> = line
        .bytes()
        .enumerate()
        .skip(right + 1)
        .filter_map(|(i, c)| (c == b'"').then_some(i))
        .take(9)
        .collect();
    if quotes.len() != 6 && quotes.len() != 8 {
        return None;
    }
    let mut fields = line[quotes[1] + 1..quotes[2]].split_whitespace();
    let status = fields.next()?.parse().ok()?;
    let bytes = fields.next()?;
    let bytes_sent = if bytes == "-" {
        0
    } else {
        bytes.parse::<i64>().ok()?
    };
    if bytes_sent < 0 || fields.next().is_some() {
        return None;
    }
    let host = if quotes.len() == 8 {
        normalize_host(&line[quotes[6] + 1..quotes[7]])
    } else {
        "__no_host_field__".into()
    };
    Some(AccessEntry {
        time_ms,
        host,
        status,
        bytes_sent,
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub requests: i64,
    pub bytes_sent: i64,
    pub s2xx: i64,
    pub s3xx: i64,
    pub s4xx: i64,
    pub s5xx: i64,
}
impl Counts {
    pub fn merge(&mut self, other: &Self) {
        self.requests += other.requests;
        self.bytes_sent += other.bytes_sent;
        self.s2xx += other.s2xx;
        self.s3xx += other.s3xx;
        self.s4xx += other.s4xx;
        self.s5xx += other.s5xx;
    }
}
#[derive(Default, Debug)]
pub struct Scan {
    pub consumed: usize,
    pub buckets: BTreeMap<(i64, String), Counts>,
    pub unparsed: i64,
}

/// Count exactly the checkpointed prefix. Stop at an immature line, even if later lines are old.
/// This avoids dropping an earlier bucket when out-of-order lines straddle the grace boundary.
pub fn scan_access_log(
    bytes: &[u8],
    now_ms: i64,
    grace_ms: i64,
    excluded: &HashSet<String>,
) -> Scan {
    let mut result = Scan::default();
    while let Some(end) = bytes[result.consumed..].iter().position(|c| *c == b'\n') {
        let start = result.consumed;
        let end = start + end;
        let raw = bytes[start..end]
            .strip_suffix(b"\r")
            .unwrap_or(&bytes[start..end]);
        if !raw.is_empty() {
            match std::str::from_utf8(raw).ok().and_then(parse_access_line) {
                Some(e) if !excluded.contains(&e.host) => {
                    let bucket = e.time_ms.div_euclid(MINUTE_MS) * MINUTE_MS;
                    if bucket.saturating_add(MINUTE_MS).saturating_add(grace_ms) > now_ms {
                        break;
                    }
                    let counts = result.buckets.entry((bucket, e.host)).or_default();
                    counts.requests += 1;
                    counts.bytes_sent += e.bytes_sent;
                    match e.status / 100 {
                        2 => counts.s2xx += 1,
                        3 => counts.s3xx += 1,
                        4 => counts.s4xx += 1,
                        5 => counts.s5xx += 1,
                        _ => {}
                    }
                }
                Some(_) => {}
                None => result.unparsed += 1,
            }
        }
        result.consumed = end + 1;
    }
    result
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct NginxMetrics {
    pub active: i32,
    pub accepts: i64,
    pub handled: i64,
    pub requests: i64,
    pub reading: i32,
    pub writing: i32,
    pub waiting: i32,
}
pub fn parse_stub_status(text: &str) -> Option<NginxMetrics> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?s)Active connections:\s*(\d+)\s+server accepts handled requests\s+(\d+)\s+(\d+)\s+(\d+)\s+Reading:\s*(\d+)\s+Writing:\s*(\d+)\s+Waiting:\s*(\d+)").unwrap()
    });
    let c = RE.captures(text)?;
    Some(NginxMetrics {
        active: c[1].parse().ok()?,
        accepts: c[2].parse().ok()?,
        handled: c[3].parse().ok()?,
        requests: c[4].parse().ok()?,
        reading: c[5].parse().ok()?,
        writing: c[6].parse().ok()?,
        waiting: c[7].parse().ok()?,
    })
}
