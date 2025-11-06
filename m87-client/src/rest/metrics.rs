use crate::rest::shared::acquire_metrics_task;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};

pub async fn handle_system_metrics_ws(socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Start metrics collection task
    let (task, mut rx) = acquire_metrics_task("system-metrics").await;

    // Spawn a forwarder for metric updates
    let forward = tokio::spawn(async move {
        while let Ok(json) = rx.recv().await {
            if ws_tx
                .send(Message::Text(Utf8Bytes::from(json)))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Wait for client close or disconnect
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Close(_) => break,
            _ => {}
        }
    }

    forward.abort();
    task.dec_or_shutdown();
}
