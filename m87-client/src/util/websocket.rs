use std::{
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;
use tracing::warn;

// ---------------------------------------------------------------
// Shared low-level worker task
// ---------------------------------------------------------------

async fn ws_pump_axum(
    mut ws: axum::extract::ws::WebSocket,
    tx_to_app: mpsc::Sender<Vec<u8>>,
    mut rx_from_app: mpsc::Receiver<Vec<u8>>,
) {
    loop {
        tokio::select! {
            // Incoming WebSocket frame → application
            msg = ws.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                        let data = b.to_vec();
                        let _ = tx_to_app.send(data).await;
                    }
                    Some(Ok(axum::extract::ws::Message::Text(t))) => {
                        let data = t.as_bytes();
                        let _ = tx_to_app.send(data.to_vec()).await;
                    }
                    Some(Ok(axum::extract::ws::Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }

            // Application → outgoing WebSocket frame
            Some(data) = rx_from_app.recv() => {
                if ws.send(axum::extract::ws::Message::binary(data)).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn ws_pump_tungstenite<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    tx_to_app: mpsc::Sender<Vec<u8>>,
    mut rx_from_app: mpsc::Receiver<Vec<u8>>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    loop {
        tokio::select! {
            msg = ws.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Binary(b))) => {
                        let data = b.to_vec();
                        let _ = tx_to_app.send(data).await;
                    }
                    Some(Ok(tungstenite::Message::Text(t))) => {
                        let data = t.as_bytes();
                        let _ = tx_to_app.send(data.to_vec()).await;
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }

            Some(out) = rx_from_app.recv() => {
                if ws.send(tungstenite::Message::binary(out)).await.is_err() {
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------
// Base struct implementing AsyncRead + AsyncWrite
// ---------------------------------------------------------------

pub struct ByteWebSocketBase {
    rx_from_net: mpsc::Receiver<Vec<u8>>, // WS → app
    tx_to_net: mpsc::Sender<Vec<u8>>,     // app → WS
    _task: JoinHandle<()>,                // background pump

    // pending chunk from last recv we haven't fully consumed yet
    pending: Option<Vec<u8>>,
    pending_pos: usize,
}

impl AsyncRead for ByteWebSocketBase {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            // --------------------------------------------------------
            // 1. If we have pending data, use it first
            // --------------------------------------------------------
            if let Some(mut pending) = self.pending.take() {
                let pos = self.pending_pos;
                let remaining_in_chunk = pending.len() - pos;

                if remaining_in_chunk == 0 {
                    // nothing left, reset and continue
                    self.pending_pos = 0;
                    continue;
                }

                let space = buf.remaining();
                if space == 0 {
                    // caller cannot accept more
                    // restore pending
                    self.pending = Some(pending);
                    return Poll::Ready(Ok(()));
                }

                let to_copy = remaining_in_chunk.min(space);
                buf.put_slice(&pending[pos..pos + to_copy]);

                self.pending_pos = pos + to_copy;

                if self.pending_pos < pending.len() {
                    // still leftover → store back
                    self.pending = Some(pending);
                } else {
                    // chunk fully consumed
                    self.pending_pos = 0;
                }

                return Poll::Ready(Ok(()));
            }

            // --------------------------------------------------------
            // 2. No pending chunk → poll channel
            // --------------------------------------------------------
            match Pin::new(&mut self.rx_from_net).poll_recv(cx) {
                Poll::Ready(Some(bytes)) => {
                    if bytes.is_empty() {
                        // skip empty
                        continue;
                    }
                    // put into pending and loop to copy from it
                    self.pending = Some(bytes);
                    self.pending_pos = 0;
                    continue;
                }
                Poll::Ready(None) => {
                    // EOF
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

impl AsyncWrite for ByteWebSocketBase {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.tx_to_net.try_send(data.to_vec()).is_ok() {
            Poll::Ready(Ok(data.len()))
        } else {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "WebSocket closed",
            )))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

// ---------------------------------------------------------------
// Server-side constructor (Axum)
// ---------------------------------------------------------------

pub struct ServerByteWebSocket(ByteWebSocketBase);

impl ServerByteWebSocket {
    pub fn new(ws: axum::extract::ws::WebSocket) -> Self {
        let (tx_to_net, rx_from_app) = mpsc::channel(32);
        let (tx_to_app, rx_from_net) = mpsc::channel(32);

        let task = tokio::spawn(ws_pump_axum(ws, tx_to_app, rx_from_app));

        Self(ByteWebSocketBase {
            rx_from_net,
            tx_to_net,
            _task: task,
            pending: None,
            pending_pos: 0,
        })
    }
}

impl AsyncRead for ServerByteWebSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for ServerByteWebSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        d: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, d)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------
// Client-side constructor (tokio-tungstenite)
// ---------------------------------------------------------------

pub struct ClientByteWebSocket(ByteWebSocketBase);

impl ClientByteWebSocket {
    pub fn new<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (tx_to_net, rx_from_app) = mpsc::channel(32);
        let (tx_to_app, rx_from_net) = mpsc::channel(32);

        let task = tokio::spawn(async move {
            ws_pump_tungstenite(ws, tx_to_app, rx_from_app).await;
            warn!("ClientByteWebSocket task completed");
        });

        Self(ByteWebSocketBase {
            rx_from_net,
            tx_to_net,
            _task: task,
            pending: None,
            pending_pos: 0,
        })
    }
}

impl AsyncRead for ClientByteWebSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for ClientByteWebSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        d: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, d)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}
