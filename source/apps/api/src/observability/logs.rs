//! Bounded SSE tail. The response owns its task, including the initial file read.
use super::parser::MAX_LINE_BYTES;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
use chrono::{Datelike, NaiveDateTime, Timelike};
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{mpsc, watch},
    task::JoinHandle,
};

pub const TAIL_BYTES: usize = 64 * 1024;
pub const TAIL_LINES: usize = 200;
pub(crate) const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const KEEPALIVE_TEXT: &str = "keepalive";
pub struct LogStream {
    receiver: mpsc::Receiver<Result<Event, Infallible>>,
    task: JoinHandle<()>,
}
impl Stream for LogStream {
    type Item = Result<Event, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
impl Drop for LogStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn response(
    path: PathBuf,
    shutdown: watch::Receiver<bool>,
    error: bool,
) -> impl axum::response::IntoResponse {
    Sse::new(tail(path, Duration::from_secs(1), shutdown, error)).keep_alive(
        KeepAlive::new()
            .interval(KEEPALIVE_INTERVAL)
            .text(KEEPALIVE_TEXT),
    )
}
pub fn tail(path: PathBuf, poll: Duration, mut shutdown: watch::Receiver<bool>, error: bool) -> LogStream {
    let (tx, receiver) = mpsc::channel(32);
    let task = tokio::spawn(async move {
        tokio::select! {
            _=async {if !*shutdown.borrow() {let _=shutdown.wait_for(|v|*v).await;}} => {},
            _=run(&path,poll,&tx,error) => {},
            _=tx.closed()=>{},
        }
    });
    LogStream { receiver, task }
}

pub(crate) fn identity(meta: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!("{}", meta.ino())
    }
    #[cfg(not(unix))]
    {
        format!("{:?}", meta.created().ok())
    }
}
async fn send(tx: &mpsc::Sender<Result<Event, Infallible>>, line: &[u8]) -> bool {
    // Event::data applies SSE framing to every embedded newline; a log can never inject fields.
    tx.send(Ok(
        Event::default().data(String::from_utf8_lossy(line).trim_end_matches('\r'))
    ))
    .await
    .is_ok()
}
// Use the writer's libc rules, including :POSIX TZ forms. Round trips reject DST gaps.
fn writer_offset(wall: NaiveDateTime) -> Option<i32> {
    let mut earliest = None;
    for isdst in [-1, 0, 1] {
        // SAFETY: both tm values are owned, initialized buffers; libc retains neither pointer.
        let candidate = unsafe {
            let mut input: libc::tm = std::mem::zeroed();
            input.tm_year = wall.year() - 1900;
            input.tm_mon = wall.month0() as i32;
            input.tm_mday = wall.day() as i32;
            input.tm_hour = wall.hour() as i32;
            input.tm_min = wall.minute() as i32;
            input.tm_sec = wall.second() as i32;
            input.tm_isdst = isdst;
            let instant = libc::mktime(&mut input);
            let mut local: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&instant, &mut local).is_null()
                || local.tm_year != wall.year() - 1900 || local.tm_mon != wall.month0() as i32
                || local.tm_mday != wall.day() as i32 || local.tm_hour != wall.hour() as i32
                || local.tm_min != wall.minute() as i32 || local.tm_sec != wall.second() as i32 {
                continue;
            }
            let offset = i32::try_from(local.tm_gmtoff).ok()?;
            if !(-86400..=86400).contains(&offset) { continue; }
            (instant, offset)
        };
        if earliest.is_none_or(|(instant, _)| candidate.0 < instant) { earliest = Some(candidate); }
    }
    earliest.map(|(_, offset)| offset)
}

struct Offsets {
    resolve: Option<fn(NaiveDateTime) -> Option<i32>>,
    last: Option<i32>,
}
impl Offsets {
    async fn send(&mut self, tx: &mpsc::Sender<Result<Event, Infallible>>, line: &[u8]) -> bool {
        if let Some(resolve) = self.resolve
            && line.get(19..21) == Some(b" [")
            && let Some(wall) = std::str::from_utf8(&line[..19]).ok()
                .and_then(|value| NaiveDateTime::parse_from_str(value, "%Y/%m/%d %H:%M:%S").ok())
            && let Some(offset) = resolve(wall)
            && self.last != Some(offset) {
                if tx.send(Ok(Event::default().event("offset").data(offset.to_string()))).await.is_err() { return false; }
                self.last = Some(offset);
        }
        send(tx, line).await
    }
}

async fn run(path: &Path, poll: Duration, tx: &mpsc::Sender<Result<Event, Infallible>>, error: bool) {
    if tx.send(Ok(Event::default().comment(KEEPALIVE_TEXT))).await.is_err() {
        return;
    }
    let mut offsets = Offsets { resolve: error.then_some(writer_offset), last: None };
    let mut offset = 0_u64;
    let mut ino = None;
    let mut pending = Vec::new();
    let mut discard = false;
    if let Ok(mut file) = File::open(path).await {
        if let Ok(meta) = file.metadata().await {
            ino = Some(identity(&meta));
            let start = meta.len().saturating_sub(TAIL_BYTES as u64);
            let _ = file.seek(std::io::SeekFrom::Start(start)).await;
            let mut bytes = vec![0; TAIL_BYTES];
            if let Ok(n) = file.read(&mut bytes).await {
                bytes.truncate(n);
                offset = start + n as u64;
                let first = if start > 0 {
                    bytes
                        .iter()
                        .position(|b| *b == b'\n')
                        .map(|i| i + 1)
                        .unwrap_or(n)
                } else {
                    0
                };
                let end = bytes
                    .iter()
                    .rposition(|b| *b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(first);
                let lines: Vec<_> = bytes[first..end]
                    .split(|b| *b == b'\n')
                    .filter(|b| !b.is_empty())
                    .collect();
                for line in lines.iter().skip(lines.len().saturating_sub(TAIL_LINES)) {
                    if !offsets.send(tx, line).await {
                        return;
                    }
                }
                pending.extend_from_slice(&bytes[end..]);
                // A tail window entirely within one huge line must not turn its suffix into a line.
                if start > 0 && !bytes.contains(&b'\n') {
                    pending.clear();
                    discard = true;
                }
            }
        }
    } else if tx
        .send(Ok(Event::default().comment("waiting for log file")))
        .await
        .is_err()
    {
        return;
    }
    let mut interval = tokio::time::interval(poll);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let Ok(mut file) = File::open(path).await else {
            continue;
        };
        let Ok(meta) = file.metadata().await else {
            continue;
        };
        let next_ino = identity(&meta);
        if ino.is_none() {
            ino = Some(next_ino);
            offset = meta.len();
            pending.clear();
            discard = false;
            if !offsets.send(tx, b"[log-viewer] log file found, tailing new lines").await {
                return;
            }
            continue;
        }
        if ino.as_ref() != Some(&next_ino) || meta.len() < offset {
            offset = 0;
            pending.clear();
            discard = false;
            ino = Some(next_ino);
        }
        if meta.len() <= offset {
            continue;
        }
        if file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
            continue;
        }
        // Catch up with everything written before this poll, one bounded read at a time.
        let end = meta.len();
        let mut bytes = vec![0; TAIL_BYTES];
        while offset < end {
            let Ok(n) = file.read(&mut bytes).await else {
                break;
            };
            if n == 0 {
                break;
            }
            offset += n as u64;
            for part in bytes[..n].split_inclusive(|b| *b == b'\n') {
                let complete = part.ends_with(b"\n");
                if !discard {
                    if pending.len() + part.len() > MAX_LINE_BYTES {
                        pending.clear();
                        discard = true;
                    } else {
                        pending.extend_from_slice(part);
                    }
                }
                if complete {
                    if !discard
                        && pending.len() > 1
                        && !offsets.send(tx, &pending[..pending.len() - 1]).await
                    {
                        return;
                    }
                    pending.clear();
                    discard = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dropping_during_initialization_aborts_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (shutdown, _) = watch::channel(false);
        for _ in 0..100 {
            let stream = tail(
                dir.path().join("absent"),
                Duration::from_millis(1),
                shutdown.subscribe(), false,
            );
            let abort = stream.task.abort_handle();
            drop(stream);
            tokio::task::yield_now().await;
            assert!(abort.is_finished());
        }
    }
    #[test]
    fn writer_offset_child() {
        let Ok(wall) = std::env::var("NWUI_TZ_WALL") else { return; };
        let expected = std::env::var("NWUI_TZ_EXPECTED").unwrap();
        let wall = NaiveDateTime::parse_from_str(&wall, "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(writer_offset(wall), expected.parse::<i32>().ok());
    }

    #[test]
    fn writer_offsets_use_libc_in_isolated_children() {
        for (zone, wall, expected) in [
            ("Asia/Seoul", "2026-11-17 18:40:14", "32400"),
            ("KST-9", "2026-11-17 18:40:14", "32400"),
            (":KST-9", "2026-11-17 18:40:14", "32400"),
            ("UTC", "2026-11-17 18:40:14", "0"),
            ("", "2026-11-17 18:40:14", "0"),
            ("Invalid/Zone", "2026-11-17 18:40:14", "0"),
            ("America/New_York", "2026-07-01 12:00:00", "-14400"),
            ("America/New_York", "2026-12-01 12:00:00", "-18000"),
            ("America/New_York", "2026-11-01 01:30:00", "-14400"),
            ("America/New_York", "2026-03-08 02:30:00", "none"),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "observability::logs::tests::writer_offset_child", "--nocapture"])
                .env_clear().env("TZ", zone).env("NWUI_TZ_WALL", wall)
                .env("NWUI_TZ_EXPECTED", expected).output().unwrap();
            assert!(output.status.success(), "TZ={zone}: {}", String::from_utf8_lossy(&output.stdout));
        }
    }

    #[tokio::test]
    async fn offsets_precede_only_error_lines_and_preserve_data() {
        use axum::{body::Body, response::IntoResponse};
        use http_body_util::BodyExt;
        let (tx, receiver) = mpsc::channel(16);
        let mut offsets = Offsets { last: None, resolve: Some(|wall| match wall.hour() { 1 => Some(-14400), 2 => None, _ => Some(-18000) }) };
        let lines = ["unparsed", "2026/11/01 01:30:00 [error] first", "2026/11/01 01:31:00 [warn] same", "2026/11/01 02:30:00 [error] gap", "2026/11/01 03:30:00 [error] next"];
        for line in lines { assert!(offsets.send(&tx, line.as_bytes()).await); }
        drop(tx);
        let task = tokio::spawn(async {});
        let body: Body = Sse::new(LogStream { receiver, task }).into_response().into_body();
        let all = body.collect().await.unwrap().to_bytes();
        let expected = format!("data: {}\n\nevent: offset\ndata: -14400\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\nevent: offset\ndata: -18000\n\ndata: {}\n\n", lines[0], lines[1], lines[2], lines[3], lines[4]);
        assert_eq!(all.as_ref(), expected.as_bytes());
        let (tx, mut rx) = mpsc::channel(2);
        let mut access = Offsets { last: None, resolve: None };
        assert!(access.send(&tx, lines[1].as_bytes()).await);
        drop(tx);
        assert!(rx.recv().await.is_some()); assert!(rx.recv().await.is_none());
    }

}
