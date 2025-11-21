use crate::rest::shared::acquire_process_task;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use tracing::error;

pub async fn handle_container_logs_ws(container_name: String, socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Spawn docker logs -f process
    let (task, mut rx) = acquire_process_task(
        &format!("container-logs:{container_name}"),
        "docker",
        &["logs", "-f", "--tail", "1000", &container_name],
    )
    .await;

    // Forward process output → WebSocket
    let forward = tokio::spawn(async move {
        while let Ok(line) = rx.recv().await {
            let msg = format!("{line}");
            if ws_tx
                .send(Message::Text(Utf8Bytes::from(msg)))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Wait for close or disconnect
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Close(_) => break,
            _ => {}
        }
    }

    forward.abort();
    task.dec_or_shutdown();
}
