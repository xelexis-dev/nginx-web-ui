use axum::{
    body::Body,
    response::{IntoResponse, Sse},
};
use http_body_util::BodyExt;
use nwui_api::observability::logs::tail;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
async fn event(body: &mut Body) -> String {
    let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("bounded event wait")
        .unwrap()
        .unwrap();
    String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
}
async fn append(path: &std::path::Path, text: &str) {
    let mut f = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .unwrap();
    f.write_all(text.as_bytes()).await.unwrap();
    f.flush().await.unwrap();
}

#[tokio::test]
async fn sse_tails_only_complete_lines_and_detects_same_size_inode_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, "first\r\npartial").await.unwrap();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let mut body = Sse::new(tail(
        path.clone(),
        Duration::from_millis(5),
        shutdown.subscribe(),
    ))
    .into_response()
    .into_body();
    assert_eq!(event(&mut body).await, "data: first\n\n");
    assert!(
        tokio::time::timeout(Duration::from_millis(30), body.frame())
            .await
            .is_err()
    );
    append(&path, " end\n").await;
    assert_eq!(event(&mut body).await, "data: partial end\n\n");
    tokio::fs::rename(&path, dir.path().join("old.log"))
        .await
        .unwrap();
    tokio::fs::write(&path, "replacement data\n").await.unwrap();
    assert_eq!(event(&mut body).await, "data: replacement data\n\n");
    shutdown.send(true).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn initial_tail_is_bounded_and_missing_file_starts_at_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    let data = (0..350).map(|n| format!("line{n}\n")).collect::<String>();
    tokio::fs::write(&path, data).await.unwrap();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let mut body = Sse::new(tail(
        path.clone(),
        Duration::from_millis(5),
        shutdown.subscribe(),
    ))
    .into_response()
    .into_body();
    for n in 150..350 {
        assert_eq!(event(&mut body).await, format!("data: line{n}\n\n"));
    }
    drop(body);
    let missing = dir.path().join("error.log");
    let mut body = Sse::new(tail(
        missing.clone(),
        Duration::from_millis(5),
        shutdown.subscribe(),
    ))
    .into_response()
    .into_body();
    assert_eq!(event(&mut body).await, ": waiting for log file\n\n");
    tokio::fs::write(&missing, "historical\n").await.unwrap();
    assert!(event(&mut body).await.contains("log file found"));
    append(&missing, "new\n").await;
    assert_eq!(event(&mut body).await, "data: new\n\n");
}

#[tokio::test]
async fn oversized_live_line_is_discarded_without_losing_following_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, "ready\n").await.unwrap();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let mut body = Sse::new(tail(
        path.clone(),
        Duration::from_millis(5),
        shutdown.subscribe(),
    ))
    .into_response()
    .into_body();
    assert!(event(&mut body).await.contains("ready"));
    append(&path, &format!("{}\nnext\n", "x".repeat(256 * 1024))).await;
    assert_eq!(event(&mut body).await, "data: next\n\n");
}

#[tokio::test]
async fn initial_window_ending_exactly_on_oversized_line_boundary_keeps_next_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, format!("{}\n", "x".repeat(100 * 1024)))
        .await
        .unwrap();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let mut body = Sse::new(tail(
        path.clone(),
        Duration::from_millis(5),
        shutdown.subscribe(),
    ))
    .into_response()
    .into_body();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), body.frame())
            .await
            .is_err()
    );
    append(&path, "next\n").await;
    assert_eq!(event(&mut body).await, "data: next\n\n");
}

/// A busy log grows faster than one bounded read per poll. Every poll catches up
/// with everything written before it, like 1.17.2, in bounded chunks.
#[tokio::test]
async fn one_poll_catches_up_with_a_burst_larger_than_one_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.log");
    tokio::fs::write(&path, "ready\n").await.unwrap();
    let (shutdown, _) = tokio::sync::watch::channel(false);
    let poll = Duration::from_millis(300);
    let mut body = Sse::new(tail(path.clone(), poll, shutdown.subscribe()))
        .into_response()
        .into_body();
    assert!(event(&mut body).await.contains("ready"));
    // Ten bounded reads' worth of complete lines, written at once.
    let line = format!("{}\n", "x".repeat(99));
    let count = 10 * nwui_api::observability::logs::TAIL_BYTES / line.len();
    append(&path, &format!("{}last\n", line.repeat(count))).await;
    let started = std::time::Instant::now();
    let mut seen = 0;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("bounded wait")
            .unwrap()
            .unwrap();
        let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
        seen += text.matches("data: ").count();
        if text.contains("data: last\n") {
            break;
        }
    }
    assert_eq!(seen, count + 1);
    assert!(
        started.elapsed() < poll * 3,
        "caught up in {:?}; one bounded read per poll needs about {:?}",
        started.elapsed(),
        poll * 10
    );
}
