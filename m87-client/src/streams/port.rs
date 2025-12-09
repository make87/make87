use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

use crate::streams::{quic::QuicIo, stream_type::Protocols};

pub async fn handle_port_forward_io(
    port: u16,
    host: Option<String>,
    protocol: Protocols,
    mut io: QuicIo,
) {
    let host = host.unwrap_or_else(|| "127.0.0.1".to_string());

    match protocol {
        Protocols::Tcp => tcp_forward(host, port, &mut io).await,

        Protocols::Udp => udp_unicast_forward(host, port, io).await,
    }
}

pub async fn tcp_forward(host: String, port: u16, io: &mut QuicIo) {
    match TcpStream::connect((host.as_str(), port)).await {
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

pub async fn udp_unicast_forward(host: String, port: u16, mut io: QuicIo) {
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            let _ = io
                .write_all(format!("UDP bind failed: {e}\n").as_bytes())
                .await;
            return;
        }
    };

    if let Err(e) = socket.connect((host.as_str(), port)).await {
        let _ = io
            .write_all(format!("UDP connect failed: {e}\n").as_bytes())
            .await;
        return;
    }

    let socket = Arc::new(socket);
    let (mut quic_r, mut quic_w) = tokio::io::split(io);
    let sock_tx = socket.clone();
    let sock_rx = socket.clone();

    // QUIC → UDP
    let to_udp = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let n = match quic_r.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("udp unicast read from quic failed: {e}");
                    break;
                }
            };
            if let Err(e) = sock_tx.send(&buf[..n]).await {
                tracing::error!("udp unicast send to socket failed: {e}");
                break;
            }
        }
    });

    // UDP → QUIC
    let to_quic = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let n = match sock_rx.recv(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("udp unicast recv from socket failed: {e}");
                    break;
                }
            };
            if let Err(e) = quic_w.write_all(&buf[..n]).await {
                tracing::error!("udp unicast write to quic failed: {e}");
                break;
            }
        }
    });

    let _ = tokio::join!(to_udp, to_quic);
    tracing::info!("udp unicast forward closed");
}
