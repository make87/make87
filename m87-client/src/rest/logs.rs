use crate::rest::auth::validate_token_via_ws;
use crate::util::logging::get_log_rx;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use tracing::error;

pub async fn handle_logs_ws(socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    if let Err(e) = validate_token_via_ws(&mut ws_tx, &mut ws_rx, true).await {
        error!("auth failed: {}", e);
        return;
    }

    let mut rx = match get_log_rx() {
        Some(r) => r,
        None => {
            let _ = ws_tx
                .send(Message::Text("logging not initialized".into()))
                .await;
            return;
        }
    };

    let forward = tokio::spawn(async move {
        while let Ok(line) = rx.recv().await {
            if ws_tx
                .send(Message::Text(Utf8Bytes::from(line)))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Close(_) => break, // <- new syntax
            _ => {}
        }
    }

    forward.abort();
}
