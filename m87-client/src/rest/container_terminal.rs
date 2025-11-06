use crate::rest::auth::validate_token_via_ws;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use pty_process::Size;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{error, info};

/// Per-connection interactive container shell (not shared on purpose)
pub async fn handle_container_terminal_ws(container_name: String, socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Authenticate first
    if let Err(e) = validate_token_via_ws(&mut ws_tx, &mut ws_rx, true).await {
        error!("auth failed: {}", e);
        return;
    }

    // Try different shells
    let candidate_cmds = ["/bin/bash", "/bin/sh", "bash", "sh"];
    let mut child_result = None;
    let mut final_pty = None;

    for shell in candidate_cmds {
        let (pty, pts) = match pty_process::open() {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to open PTY: {e}");
                continue;
            }
        };

        pty.resize(Size::new(24, 80)).ok();

        let mut cmd = pty_process::Command::new("docker");
        cmd = cmd.args(["exec", "-it", &container_name, shell]);

        match cmd.spawn(pts) {
            Ok(child) => {
                child_result = Some(child);
                final_pty = Some(pty);
                break;
            }
            Err(e) => {
                error!("Failed to start `{}` in `{}`: {e}", shell, container_name);
            }
        }
    }

    let (mut child, pty) = match (child_result, final_pty) {
        (Some(c), Some(p)) => (c, p),
        _ => {
            let msg = format!(
                "Could not start a shell in `{}`.\nTried: {:?}\nHint: install /bin/sh or /bin/bash",
                container_name, candidate_cmds
            );
            let _ = ws_tx.send(Message::Text(Utf8Bytes::from(msg))).await;
            return;
        }
    };

    let (mut pty_reader, mut pty_writer) = pty.into_split();

    // Forward PTY → WebSocket
    tokio::spawn(async move {
        let mut buf = [0u8; 128];
        loop {
            match pty_reader.read(&mut buf).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if ws_tx
                        .send(Message::Text(Utf8Bytes::from(text)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    error!("PTY read error: {:?}", e);
                    break;
                }
            }
        }
    });

    // WebSocket → PTY
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Close(_) => break,
            Message::Text(text) => {
                if pty_writer.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
            Message::Binary(bin) => {
                if pty_writer.write_all(&bin).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }

    let _ = child.kill();
    info!("Closed PTY for `{}`", container_name);
}
