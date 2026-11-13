//! Bounded SSE tail. The response owns its task, including the initial file read.
use super::parser::MAX_LINE_BYTES;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_core::Stream;
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
) -> impl axum::response::IntoResponse {
    Sse::new(tail(path, Duration::from_secs(1), shutdown)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}
pub fn tail(path: PathBuf, poll: Duration, mut shutdown: watch::Receiver<bool>) -> LogStream {
    let (tx, receiver) = mpsc::channel(32);
    let task = tokio::spawn(async move {
        tokio::select! {
            _=async {if !*shutdown.borrow() {let _=shutdown.wait_for(|v|*v).await;}} => {},
            _=run(&path,poll,&tx) => {},
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
async fn run(path: &Path, poll: Duration, tx: &mpsc::Sender<Result<Event, Infallible>>) {
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
                    if !send(tx, line).await {
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
            if !send(tx, b"[log-viewer] log file found, tailing new lines").await {
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
                        && !send(tx, &pending[..pending.len() - 1]).await
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
                shutdown.subscribe(),
            );
            let abort = stream.task.abort_handle();
            drop(stream);
            tokio::task::yield_now().await;
            assert!(abort.is_finished());
        }
    }
}
