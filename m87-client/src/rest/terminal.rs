use crate::rest::auth::validate_token_via_ws;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    process::Command,
    select,
    time::{timeout, Duration},
};
use tracing::error;

pub async fn handle_terminal_ws(socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // --- Authenticate client ---
    if let Err(e) = validate_token_via_ws(&mut ws_tx, &mut ws_rx, true).await {
        error!("auth failed: {}", e);
        return;
    }

    // --- Spawn SSH child process ---
    let mut child = match Command::new("ssh")
        .arg("localhost")
        .arg("-tt")
        .arg("-o")
        .arg("LogLevel=QUIET")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("RequestTTY=force")
        .env("TERM", "xterm-256color")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Text(Utf8Bytes::from(format!(
                    "Failed to start SSH: {e}"
                ))))
                .await;
            return;
        }
    };

    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => return,
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => return,
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => return,
    };

    let mut stdin_writer = BufWriter::new(stdin);
    let mut stdout_reader = tokio::io::BufReader::new(stdout);
    let mut stderr_reader = tokio::io::BufReader::new(stderr);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (ready_tx, mut ready_rx) = tokio::sync::mpsc::channel::<()>(1);

    // --- STDOUT reader task ---
    let tx1 = tx.clone();
    let ready1 = ready_tx.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 128];
        let mut sent = false;
        loop {
            match stdout_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if !sent {
                        let _ = ready1.send(()).await;
                        sent = true;
                    }
                    let _ = tx1.send(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                Err(_) => break,
            }
        }
        if !sent {
            let _ = ready1.send(()).await;
        }
    });

    // --- STDERR reader task ---
    let tx2 = tx.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 128];
        loop {
            match stderr_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx2.send(String::from_utf8_lossy(&buf[..n]).to_string());
                }
                Err(_) => break,
            }
        }
    });

    // --- Wait for child readiness ---
    if timeout(Duration::from_secs(2), ready_rx.recv())
        .await
        .is_err()
    {
        let _ = ws_tx
            .send(Message::Text(Utf8Bytes::from_static(
                "SSH shell failed to start within timeout\n",
            )))
            .await;
        let _ = child.kill().await;
        return;
    }

    let _ = ws_tx
        .send(Message::Text(Utf8Bytes::from_static(
            "Shell connected successfully\n\r",
        )))
        .await;

    // --- Main IO loop (WebSocket <-> SSH) ---
    loop {
        select! {
            // WebSocket → SSH
            Some(Ok(msg)) = ws_rx.next() => {
                match msg {
                    Message::Text(text) => {
                        if stdin_writer.write_all(text.as_bytes()).await.is_err() { break; }
                        if stdin_writer.flush().await.is_err() { break; }
                    }
                    Message::Binary(bin) => {
                        if stdin_writer.write_all(&bin).await.is_err() { break; }
                        if stdin_writer.flush().await.is_err() { break; }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // SSH → WebSocket
            Some(out) = rx.recv() => {
                if ws_tx.send(Message::Text(Utf8Bytes::from(out))).await.is_err() {
                    break;
                }
            }
            else => break,
        }
    }

    let _ = child.kill().await;
}
