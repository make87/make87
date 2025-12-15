use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::UNIX_EPOCH;
use tokio::time::Duration;

use crate::device::tunnel::open_local_tunnel;
use crate::util::subprocess::SubprocessBuilder;

/// docker -H <socket> … → forwarded via QUIC
pub async fn run_docker_command(device: &str, args: Vec<String>) -> Result<()> {
    check_docker_cli()?;

    let endpoint = generate_local_socket_path(device);

    spawn_socket_tunnel(device, &endpoint).await?;
    wait_for_socket_ready(&endpoint).await?;
    tracing::info!("[done] Connected");

    SubprocessBuilder::new("docker")
        .args(args)
        .env("DOCKER_HOST", docker_host_uri(&endpoint))
        .exec()
        .await?;

    cleanup(&endpoint);

    Ok(())
}

//
// ─────────────────────────────────────────────────────────────
//  Socket endpoint generation
// ─────────────────────────────────────────────────────────────
//

fn generate_local_socket_path(device: &str) -> PathBuf {
    let id = format!(
        "{}_{}_{}",
        std::process::id(),
        device,
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    #[cfg(unix)]
    {
        let mut p = std::env::temp_dir();
        p.push(format!("m87-docker-{id}.sock"));
        p
    }

    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\m87_docker_{id}"))
    }
}

fn docker_host_uri(p: &PathBuf) -> String {
    #[cfg(unix)]
    {
        format!("unix://{}", p.display())
    }
    #[cfg(windows)]
    {
        // Docker on Windows uses npipe://
        format!("npipe://{}", p.display())
    }
}

//
// ─────────────────────────────────────────────────────────────
//  Spawning the QUIC socket tunnel
// ─────────────────────────────────────────────────────────────
//

async fn spawn_socket_tunnel(device: &str, endpoint: &PathBuf) -> Result<()> {
    let local = endpoint.display().to_string();
    let remote = "/var/run/docker.sock".to_string(); // robot docker sock

    let spec = format!("{local}:{remote}");

    let device = device.to_string();
    tokio::spawn(async move {
        if let Err(e) = open_local_tunnel(&device, vec![spec]).await {
            eprintln!("Docker socket tunnel exited with error: {e}");
        }
    });

    Ok(())
}

//
// ─────────────────────────────────────────────────────────────
//  Waiting for readiness
// ─────────────────────────────────────────────────────────────
//

async fn wait_for_socket_ready(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        for _ in 0..50 {
            if path.exists() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(anyhow!("Unix socket did not appear"))
    }

    #[cfg(windows)]
    {
        // Named pipes are not "files", so existence must be probed by trying to connect
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING,
        };

        let pipe = path.display().to_string();
        let wide: Vec<u16> = pipe.encode_utf16().chain(std::iter::once(0)).collect();

        for _ in 0..50 {
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    0,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };
            if h != INVALID_HANDLE_VALUE {
                unsafe { CloseHandle(h) };
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(anyhow!("Named pipe did not become available"))
    }
}

//
// ─────────────────────────────────────────────────────────────
//  Cleanup
// ─────────────────────────────────────────────────────────────
//

fn cleanup(p: &PathBuf) {
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(p);
    }
    #[cfg(windows)]
    {
        // nothing to delete
    }
}

//
// ─────────────────────────────────────────────────────────────
//  Docker CLI detection
// ─────────────────────────────────────────────────────────────
//

fn check_docker_cli() -> Result<()> {
    std::process::Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("Docker CLI not installed")?;
    Ok(())
}
