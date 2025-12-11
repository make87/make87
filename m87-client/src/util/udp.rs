use bytes::{BufMut, BytesMut};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// Layout: [family: u8][port: u16][ip bytes]
/// family = 4 => IPv4 (4 bytes), 6 => IPv6 (16 bytes)
pub fn encode_socket_addr(buf: &mut BytesMut, addr: SocketAddr) {
    match addr {
        SocketAddr::V4(v4) => {
            buf.put_u8(4);
            buf.put_u16(v4.port());
            buf.extend_from_slice(&v4.ip().octets());
        }
        SocketAddr::V6(v6) => {
            buf.put_u8(6);
            buf.put_u16(v6.port());
            buf.extend_from_slice(&v6.ip().octets());
        }
    }
}

/// Returns (addr, header_len) on success
pub fn decode_socket_addr(buf: &[u8]) -> Option<(SocketAddr, usize)> {
    if buf.len() < 1 + 2 {
        return None;
    }

    let fam = buf[0];
    let port = u16::from_be_bytes([buf[1], buf[2]]);

    match fam {
        4 => {
            if buf.len() < 1 + 2 + 4 {
                return None;
            }
            let ip = Ipv4Addr::new(buf[3], buf[4], buf[5], buf[6]);
            let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
            Some((addr, 1 + 2 + 4))
        }
        6 => {
            if buf.len() < 1 + 2 + 16 {
                return None;
            }
            let ip = Ipv6Addr::new(
                u16::from_be_bytes([buf[3], buf[4]]),
                u16::from_be_bytes([buf[5], buf[6]]),
                u16::from_be_bytes([buf[7], buf[8]]),
                u16::from_be_bytes([buf[9], buf[10]]),
                u16::from_be_bytes([buf[11], buf[12]]),
                u16::from_be_bytes([buf[13], buf[14]]),
                u16::from_be_bytes([buf[15], buf[16]]),
                u16::from_be_bytes([buf[17], buf[18]]),
            );
            let addr = SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0));
            Some((addr, 1 + 2 + 16))
        }
        _ => None,
    }
}
