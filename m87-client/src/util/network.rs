use anyhow::{anyhow, Context, Result};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use stun::message::Getter;
use tokio::net::UdpSocket;

/// List of public STUN servers to use for IP detection
/// These servers are tried in sequence until one succeeds
const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun.cloudflare.com:3478",
    "stun1.l.google.com:19302",
];

/// Timeout for STUN requests (per server)
const STUN_TIMEOUT: Duration = Duration::from_secs(3);

/// Get the public IP address of the current machine using STUN protocol
///
/// This function tries multiple public STUN servers in sequence until one succeeds.
/// STUN (Session Traversal Utilities for NAT) is a standardized protocol (RFC 5389)
/// commonly used by WebRTC for NAT traversal and public IP detection.
///
/// # Errors
/// Returns an error if all STUN servers fail or if no valid response is received
pub async fn get_public_ip() -> Result<String> {
    let mut last_error = None;

    // Try each STUN server in sequence
    for server in STUN_SERVERS {
        match query_stun_server(server).await {
            Ok(ip) => return Ok(ip.to_string()),
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("No STUN servers available")))
}

/// Query a single STUN server for the public IP address
async fn query_stun_server(server: &str) -> Result<IpAddr> {
    // Resolve the STUN server address
    let server_addr = tokio::net::lookup_host(server)
        .await
        .context(format!("Failed to resolve STUN server: {}", server))?
        .next()
        .ok_or_else(|| anyhow!("No addresses found for STUN server: {}", server))?;

    // Create UDP socket bound to any available port
    let local_addr: SocketAddr = if server_addr.is_ipv6() {
        "[::]:0".parse()?
    } else {
        "0.0.0.0:0".parse()?
    };

    let socket = UdpSocket::bind(local_addr)
        .await
        .context("Failed to bind UDP socket")?;

    // Create STUN binding request
    let mut message = stun::message::Message::new();
    message.build(&[
        Box::new(stun::message::BINDING_REQUEST),
        Box::new(stun::agent::TransactionId::new()),
    ])?;

    // Send STUN request
    socket
        .send_to(&message.raw, server_addr)
        .await
        .context("Failed to send STUN request")?;

    // Receive STUN response with timeout
    let mut buf = vec![0u8; 1500];
    let len = tokio::time::timeout(STUN_TIMEOUT, socket.recv(&mut buf))
        .await
        .context("STUN request timed out")??;

    // Parse STUN response
    let mut response = stun::message::Message::new();
    response.raw = buf[..len].to_vec();
    response.decode()?;

    // Extract XOR-MAPPED-ADDRESS (public IP) from response
    let mut xor_addr = stun::xoraddr::XorMappedAddress::default();
    xor_addr.get_from(&response)?;

    Ok(xor_addr.ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_public_ip() {
        let result = get_public_ip().await;
        assert!(result.is_ok(), "Failed to get public IP: {:?}", result.err());

        let ip = result.unwrap();
        println!("Detected public IP: {}", ip);

        // Verify it's a valid IP address
        assert!(!ip.is_empty());
        assert!(ip.parse::<IpAddr>().is_ok(), "Invalid IP address: {}", ip);
    }
}
