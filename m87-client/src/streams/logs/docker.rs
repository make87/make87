use std::collections::HashMap;

use anyhow::{Result, anyhow};
use bytes::{Buf, BytesMut};
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::mpsc::{self, Receiver},
};

use crate::streams::logs::format;

const DOCKER_SOCK: &str = "/var/run/docker.sock";

#[derive(Debug, Deserialize)]
struct ContainerInfo {
    Id: String,
    Names: Vec<String>,
}

/* ---------------- HTTP helpers ---------------- */

async fn docker_http(path: &str, follow: bool) -> Result<(UnixStream, Vec<u8>)> {
    let mut stream = UnixStream::connect(DOCKER_SOCK).await?;

    let req = format!(
        "GET {} HTTP/1.1\r\n\
         Host: localhost\r\n\
         Accept: application/json\r\n\
         {}\r\n",
        path,
        if follow { "" } else { "Connection: close\r\n" }
    );

    stream.write_all(req.as_bytes()).await?;

    // Read headers
    let mut header_buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        header_buf.push(byte[0]);
        if header_buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8_lossy(&header_buf);
    let mut lines = header_str.lines();

    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(500);

    if status != 200 {
        return Err(anyhow!("Docker returned HTTP {}", status));
    }

    let chunked = header_str
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");

    if follow {
        return Ok((stream, Vec::new()));
    }

    let body = if chunked {
        read_chunked(&mut stream).await?
    } else {
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        buf
    };

    Ok((stream, body))
}

async fn read_chunked(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut body = Vec::new();

    loop {
        let mut len_buf = Vec::new();
        let mut b = [0u8; 1];

        loop {
            stream.read_exact(&mut b).await?;
            if b[0] == b'\n' {
                break;
            }
            len_buf.push(b[0]);
        }

        let len = usize::from_str_radix(std::str::from_utf8(&len_buf)?.trim(), 16)?;

        if len == 0 {
            break;
        }

        let mut chunk = vec![0u8; len];
        stream.read_exact(&mut chunk).await?;
        body.extend(chunk);

        // skip CRLF
        stream.read_exact(&mut b).await?;
        stream.read_exact(&mut b).await?;
    }

    Ok(body)
}

/* ---------------- Docker API ---------------- */

async fn list_containers() -> Result<Vec<ContainerInfo>> {
    let (_, body) = docker_http("/containers/json?all=1", false).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn stream_logs(id: String, name: String, tx: mpsc::Sender<String>) -> Result<()> {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let path = format!(
        "/containers/{}/logs?stdout=1&stderr=1&follow=1&since={}",
        id, since
    );
    let (mut stream, _) = docker_http(&path, true).await?;

    let mut buf = BytesMut::new();

    loop {
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        buf.extend_from_slice(&tmp[..n]);

        while buf.len() >= 8 {
            let stream_id = buf[0];
            if stream_id != 1 && stream_id != 2 {
                buf.advance(1);
                continue;
            }

            let len = u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;

            if buf.len() < 8 + len {
                break;
            }

            buf.advance(8);
            let payload = buf.split_to(len);

            // let stream_name = match stream_id {
            //     1 => "stdout",
            //     2 => "stderr",
            //     _ => continue,
            // };

            let msg = String::from_utf8_lossy(&payload).trim_end().to_string();
            let formatted_msg = format::format_log(&name, &msg, true);
            if tx.send(formatted_msg).await.is_err() {
                return Ok(());
            }
        }
    }

    Ok(())
}

/* ---------------- Public API ---------------- */

pub async fn get_docker_log_rx(wanted_names: Vec<String>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>(1024);

    tokio::spawn(async move {
        let containers = match list_containers().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to list containers: {}", e);
                return;
            }
        };

        let mut targets = HashMap::new();
        for c in containers {
            for n in c.Names {
                let name = n.trim_start_matches('/').to_string();
                if wanted_names.iter().any(|w| w == &name) {
                    targets.insert(name, c.Id.clone());
                }
            }
        }

        for (name, id) in targets {
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = stream_logs(id, name, tx).await {
                    tracing::error!("Docker log stream failed: {}", e);
                }
            });
        }
    });

    rx
}
