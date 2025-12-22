use anyhow::{Result, anyhow};
use tokio::io::AsyncWriteExt;
mod docker;
mod format;

use crate::{config::Config, streams::quic::QuicIo, util::logging::get_log_rx};

pub async fn handle_logs_io(io: &mut QuicIo) -> Result<()> {
    let mut app_rx = match get_log_rx() {
        Some(r) => r,
        None => {
            let _ = io.write_all(b"logging not initialized\n").await;
            return Err(anyhow!("logging not initialized"));
        }
    };

    let config = Config::load()?;
    let mut docker_rx = docker::get_docker_log_rx(config.observe.docker_services).await;
    // let mut systemd_rx = systemd_journal::get_log_rx().await;

    loop {
        tokio::select! {
            // Some(line) = systemd_rx.recv() => {
            //     if io.write_all(line.as_bytes()).await.is_err() {
            //         break;
            //     }
            //     if io.write_all(b"\n").await.is_err() {
            //         break;
            //     }
            // }
            Some(line) = docker_rx.recv() => {
                if io.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if io.write_all(b"\n").await.is_err() {
                    break;
                }
            }
            Ok(line) = app_rx.recv() => {
                let Some(line) = line.as_line() else {
                    continue;
                };

                let formatted_msg = format::format_log("m87", &line, true);
                if io.write_all(formatted_msg.as_bytes()).await.is_err() {
                    break;
                }
                if io.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}
