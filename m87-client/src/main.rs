use m87_client::run_cli;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();

    // Background task: translate Ctrl+C into a cancellation request
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel_for_signal.cancel();
        }
    });

    // Delegate to the CLI implementation in lib.rs
    run_cli(cancel).await
}
