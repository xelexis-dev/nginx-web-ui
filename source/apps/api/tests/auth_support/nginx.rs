//! Successful reload double for orthogonal tests; native G0/T14 prove nginx's protocol.
use super::AppState;
use std::{os::unix::fs::PermissionsExt, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Call after the fixture's last config/binary override, before cloning its state.
pub fn witness(state: &mut AppState) {
    let weak = Arc::downgrade(&state.runtime);
    let config =
        Arc::get_mut(&mut state.config).expect("configure fake witness before cloning state");
    let root = config.nginx_conf_dir.parent().unwrap().to_path_buf();
    assert!(
        config.nginx_bin.starts_with(&root),
        "fake nginx must be task-local"
    );
    std::fs::create_dir_all(&config.nginx_conf_dir).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let active = root.join(".fake-nginx-active-generation");
    let marker = config.nginx_conf_dir.join(".nwui-generation");
    let pid = root.join(".fake-nginx.pid");
    std::fs::write(&pid, std::process::id().to_string()).unwrap();
    let original = if config.nginx_main_conf.starts_with(&root) {
        std::fs::read_to_string(&config.nginx_main_conf).unwrap_or_default()
    } else {
        String::new()
    };
    let main = root.join(".fake-nginx-main.conf");
    std::fs::write(&main,format!("{original}\npid {}; events {{}} http {{ server {{ listen 127.0.0.1:{port}; location = /nginx_status {{ stub_status; allow 127.0.0.3; deny all; include {}; }} }} }}\n",pid.display(),marker.display())).unwrap();
    let bin = root.join(format!(".fake-nginx-with-witness-{}", uuid::Uuid::new_v4()));
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
'{}' "$@"
code=$?
if [ "$code" = 0 ] && [ "$1" = -s ]; then cp '{}' '{}'; fi
exit "$code"
"#,
            config.nginx_bin.display(),
            marker.display(),
            active.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    config.nginx_main_conf = main;
    config.nginx_bin = bin;
    config.nginx_status_url = format!("http://127.0.0.1:{port}/nginx_status");
    tokio::spawn(async move {
        loop {
            let socket = tokio::select! {
                socket=listener.accept()=>match socket{Ok((socket,_))=>socket,Err(_)=>break},
                _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{if weak.upgrade().is_none(){break}continue},
            };
            let active = active.clone();
            // The double serves one fresh connection at a time and never owns the fixture state.
            let _=tokio::time::timeout(std::time::Duration::from_secs(1),async move {
                let mut socket=socket;let mut request=Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len()<4096 {let mut b=[0];if socket.read(&mut b).await?==0{return Ok::<_,std::io::Error>(())}request.push(b[0]);}
                let value=tokio::fs::read_to_string(active).await.unwrap_or_default();let token=value.split('"').nth(1).unwrap_or("old");
                socket.write_all(format!("HTTP/1.1 200 OK\r\nX-NWUI-Config-Generation: {token}\r\nConnection: close\r\nContent-Length: 2\r\n\r\nOK").as_bytes()).await?;
                Ok(())
            }).await;
        }
    });
}
