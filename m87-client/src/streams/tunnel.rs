use std::sync::Arc;

use bytes::Bytes;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpStream, UdpSocket, UnixStream},
};
use tracing::{info, warn};

use crate::streams::{
    quic::QuicIo,
    stream_type::{SocketTarget, TcpTarget, TunnelTarget, UdpTarget},
    udp_manager::UdpChannelManager,
};

pub async fn handle_port_forward_io(
    tunnel_target: TunnelTarget,
    mut io: QuicIo,
    manager: UdpChannelManager,
    datagram_tx: tokio::sync::mpsc::Sender<(u32, Bytes)>,
) {
    match tunnel_target {
        TunnelTarget::Tcp(target) => tcp_forward(target, &mut io).await,

        TunnelTarget::Udp(target) => udp_unicast_forward(target, io, manager, datagram_tx).await,
        TunnelTarget::Socket(target) => socket_forward(target, io).await,
        TunnelTarget::Vpn(_target) => {}
    }
}

pub async fn tcp_forward(target: TcpTarget, io: &mut QuicIo) {
    match TcpStream::connect((target.remote_host.as_str(), target.remote_port)).await {
        Ok(mut local) => match tokio::io::copy_bidirectional(io, &mut local).await {
            Ok((a, b)) => tracing::info!("tcp forward closed (rx={a}, tx={b})"),
            Err(e) => tracing::error!("tcp forward error: {e}"),
        },
        Err(e) => {
            let _ = io
                .send
                .write_all(format!("TCP connect failed: {e}\n").as_bytes())
                .await;
        }
    }
}

pub async fn udp_unicast_forward(
    target: UdpTarget,
    mut io: QuicIo,
    manager: UdpChannelManager,
    datagram_tx: tokio::sync::mpsc::Sender<(u32, Bytes)>,
) {
    let (id, rx) = manager.alloc(target.clone()).await;
    // return ID to CLI/server over the stream
    let id_buf = id.to_be_bytes();
    if let Err(e) = io.send.write_all(&id_buf).await {
        tracing::error!("Failed to send ID: {e}");
        return;
    }
    if let Err(e) = io.send.finish() {
        tracing::error!("Failed to finish sending ID: {e}");
        return;
    }
    // spawn actual UDP forward worker
    tokio::spawn(start_udp_forward(id, target, rx, datagram_tx, manager));
}

async fn start_udp_forward(
    id: u32,
    target: UdpTarget,
    mut rx: tokio::sync::mpsc::Receiver<Bytes>,
    datagram_tx: tokio::sync::mpsc::Sender<(u32, Bytes)>,
    manager: UdpChannelManager,
) {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await.unwrap());
    sock.connect((target.remote_host.as_str(), target.remote_port))
        .await
        .unwrap();

    let sock_recv = sock.clone();
    let chan_id = id;
    info!(
        "Started UDP forward worker for channel {} on port {}",
        chan_id,
        sock.local_addr().unwrap().port()
    );

    // === UDP -> QUIC ===
    let dt = datagram_tx.clone();
    let mgr = manager.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 65535];
        loop {
            let n = match sock_recv.recv(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    warn!("UDP recv error on channel {}: {:?}", chan_id, e);
                    mgr.remove(chan_id).await;
                    break;
                }
            };

            let payload = Bytes::copy_from_slice(&buf[..n]);
            if dt.send((chan_id, payload)).await.is_err() {
                // QUIC side shut down
                break;
            }
        }
    });

    // === QUIC -> UDP ===
    let sock_tx = sock.clone();
    tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            if let Err(e) = sock_tx.send(&payload).await {
                warn!("UDP send error on channel {}: {:?}", chan_id, e);
                // remove channel => closes rx on next loop tick
                manager.remove(chan_id).await;
                break;
            }
        }
    });
}

pub async fn socket_forward(target: SocketTarget, mut io: QuicIo) {
    let path = target.remote_path;

    let mut stream = match UnixStream::connect(&path).await {
        Ok(s) => s,
        Err(e) => {
            let _ = io
                .write_all(format!("UNIX connect failed ({}): {}\n", path, e).as_bytes())
                .await;
            return;
        }
    };

    tracing::info!("unix forward opened to {}", path);

    match tokio::io::copy_bidirectional(&mut io, &mut stream).await {
        Ok((rx, tx)) => {
            tracing::info!("unix forward {} closed cleanly (rx={rx}, tx={tx})", path);
        }
        Err(e) => {
            // “sending stopped by peer: error 0” is quinn signalling that the
            // remote side stopped the stream – it’s often a normal shutdown.
            let msg = e.to_string();
            let is_expected = msg.contains("sending stopped by peer")
                || matches!(
                    e.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::UnexpectedEof
                );

            if is_expected {
                tracing::info!("unix forward {} closed (expected close: {})", path, msg);
            } else {
                tracing::error!("unix forward {} closed with error: {}", path, msg);
            }
        }
    }
}
