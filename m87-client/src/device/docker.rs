use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, UNIX_EPOCH};
use tokio::task::JoinHandle;
use tracing::{info, warn};

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// Docker proxy with automatic cleanup via Drop trait
struct DockerProxy {
    socket_path: PathBuf,
    proxy_handle: JoinHandle<Result<()>>,
}

impl DockerProxy {
    async fn new(device_name: &str) -> Result<Self> {
        // Generate unique socket path
        let socket_path = Self::generate_socket_path(device_name);

        // Start proxy in background
        let socket_clone = socket_path.clone();
        let device = device_name.to_string();
        let proxy_handle = tokio::spawn(async move {
            start_docker_proxy(&device, &socket_clone).await
        });

        // Wait for socket to be ready (up to 2 seconds)
        Self::wait_for_socket(&socket_path).await?;

        Ok(Self {
            socket_path,
            proxy_handle,
        })
    }

    fn generate_socket_path(device: &str) -> PathBuf {
        // Unique: PID + device + timestamp (for parallel execution)
        let unique_id = format!(
            "{}-{}-{}",
            std::process::id(),
            device,
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        #[cfg(unix)]
        {
            let mut path = std::env::temp_dir();
            path.push(format!("m87-docker-{}.sock", unique_id));
            path
        }

        #[cfg(windows)]
        {
            // Named pipes on Windows don't use filesystem paths
            PathBuf::from(format!("\\\\.\\pipe\\m87_docker_{}", unique_id))
        }
    }

    async fn wait_for_socket(path: &Path) -> Result<()> {
        for _ in 0..20 {
            if path.exists() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(anyhow!("Socket failed to initialize within 2 seconds"))
    }

    fn socket_uri(&self) -> String {
        #[cfg(unix)]
        {
            format!("unix://{}", self.socket_path.display())
        }

        #[cfg(windows)]
        {
            format!("npipe://{}", self.socket_path.display())
        }
    }
}

impl Drop for DockerProxy {
    fn drop(&mut self) {
        // Abort proxy task
        self.proxy_handle.abort();

        // Remove socket file (ignore errors - best effort cleanup)
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Execute docker command on remote device
pub async fn run_docker_command(device_name: &str, args: Vec<String>) -> Result<()> {
    // Check if docker CLI exists
    check_docker_cli()?;

    // Create proxy (automatic cleanup via Drop)
    let proxy = DockerProxy::new(device_name).await?;

    info!("Docker proxy ready at {}", proxy.socket_uri());

    // Execute docker command with DOCKER_HOST pointing to our socket
    let status = std::process::Command::new("docker")
        .args(&args)
        .env("DOCKER_HOST", proxy.socket_uri())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("Failed to execute docker command")?;

    // proxy.drop() called here automatically - socket cleaned up!

    std::process::exit(status.code().unwrap_or(1));
}

fn check_docker_cli() -> Result<()> {
    std::process::Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("Docker CLI not found. Please install Docker.")?;
    Ok(())
}

/// Start Docker socket proxy server
async fn start_docker_proxy(device_name: &str, socket_path: &Path) -> Result<()> {
    // Remove stale socket if exists
    let _ = std::fs::remove_file(socket_path);

    info!("Starting Docker proxy on {}", socket_path.display());

    // Create listener (platform-specific)
    #[cfg(unix)]
    let listener = UnixListener::bind(socket_path)
        .context("Failed to create Unix socket")?;

    #[cfg(windows)]
    let mut listener = ServerOptions::new()
        .first_pipe_instance(true)
        .create(socket_path)
        .context("Failed to create named pipe")?;

    // Accept connections loop
    loop {
        #[cfg(unix)]
        let (stream, _) = listener.accept().await?;

        #[cfg(windows)]
        let stream = {
            listener.connect().await?;
            let client = listener;
            listener = ServerOptions::new().create(socket_path)?;
            client
        };

        let device = device_name.to_string();
        tokio::spawn(async move {
            if let Err(e) = handle_docker_connection(stream, &device).await {
                warn!("Docker proxy connection error: {}", e);
            }
        });
    }
}

/// Handle single Docker API connection
#[cfg(unix)]
async fn handle_docker_connection(mut local: UnixStream, device_name: &str) -> Result<()> {
    use crate::config::Config;
    use crate::devices;
    use crate::util::tls;

    // 1. Get device info
    let dev = devices::list_devices()
        .await?
        .into_iter()
        .find(|d| d.name == device_name)
        .ok_or_else(|| anyhow!("Device '{}' not found", device_name))?;

    // 2. Get config for server hostname
    let config = Config::load()?;
    let host_name = format!("docker-{}.{}", dev.short_id, config.get_server_hostname());

    // 3. Connect to server via TLS (server will route to device's Docker socket)
    let mut remote = tls::get_tls_connection(host_name, config.trust_invalid_server_cert)
        .await
        .context("Failed to establish TLS connection to server")?;

    // 4. Bidirectional copy between local socket and remote connection
    tokio::io::copy_bidirectional(&mut local, &mut remote).await?;

    Ok(())
}

#[cfg(windows)]
async fn handle_docker_connection(mut local: NamedPipeServer, device_name: &str) -> Result<()> {
    use crate::config::Config;
    use crate::devices;
    use crate::util::tls;

    // 1. Get device info
    let dev = devices::list_devices()
        .await?
        .into_iter()
        .find(|d| d.name == device_name)
        .ok_or_else(|| anyhow!("Device '{}' not found", device_name))?;

    // 2. Get config for server hostname
    let config = Config::load()?;
    let host_name = format!("docker-{}.{}", dev.short_id, config.get_server_hostname());

    // 3. Connect to server via TLS (server will route to device's Docker socket)
    let mut remote = tls::get_tls_connection(host_name, config.trust_invalid_server_cert)
        .await
        .context("Failed to establish TLS connection to server")?;

    // 4. Bidirectional copy between local socket and remote connection
    tokio::io::copy_bidirectional(&mut local, &mut remote).await?;

    Ok(())
}
