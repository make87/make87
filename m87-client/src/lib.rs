// === Core modules ===
pub mod app;
pub mod auth;
pub mod config;
pub mod device;
pub mod devices;

// Agent-specific modules (Linux-only via build.rs)
#[cfg(feature = "agent")]
pub mod rest;

pub mod server;
pub mod stack;
pub mod update;
pub mod util;

pub mod tui;
// === CLI entrypoint ===
pub mod cli;

use tokio_util::sync::CancellationToken;

/// Entrypoint used by `main.rs` and tests to run the full CLI.
pub async fn run_cli(cancel: CancellationToken) -> anyhow::Result<()> {
    cli::cli(cancel).await
}
