use std::fmt::{self, Write};
use std::sync::OnceLock;
use tokio::sync::broadcast;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{prelude::*, EnvFilter};

static LOG_TX: OnceLock<broadcast::Sender<String>> = OnceLock::new();

struct MsgVisitor {
    msg: String,
}

impl Visit for MsgVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.msg = format!("{value:?}");
        }
    }
}

pub struct LogBroadcastLayer {
    tx: broadcast::Sender<String>,
}

impl LogBroadcastLayer {
    pub fn new(tx: broadcast::Sender<String>) -> Self {
        Self { tx }
    }
}

impl<S> Layer<S> for LogBroadcastLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Extract message field
        let mut visitor = MsgVisitor { msg: String::new() };
        event.record(&mut visitor);

        // Metadata
        let meta = event.metadata();
        let level = meta.level();
        let target = meta.target();

        // Timestamp
        let ts = timestamp_hms();

        // Simple ANSI colors
        let lvl_colored = match *level {
            tracing::Level::ERROR => "\x1b[31mERROR\x1b[0m",
            tracing::Level::WARN => "\x1b[33mWARN\x1b[0m",
            tracing::Level::INFO => "\x1b[32mINFO\x1b[0m",
            tracing::Level::DEBUG => "\x1b[34mDEBUG\x1b[0m",
            tracing::Level::TRACE => "\x1b[90mTRACE\x1b[0m",
        };

        // Build final line
        let mut line = String::new();
        let _ = write!(
            line,
            "[{}] {} [{}] {}",
            ts, lvl_colored, target, visitor.msg
        );

        let _ = self.tx.send(line);
    }
}

pub fn init_tracing_with_log_layer(log_level: &str) -> broadcast::Sender<String> {
    let (tx, _rx) = broadcast::channel(1000);
    LOG_TX.set(tx.clone()).ok();

    let filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(LogBroadcastLayer::new(tx.clone()))
        .init();

    tx
}

pub fn get_log_rx() -> Option<broadcast::Receiver<String>> {
    LOG_TX.get().map(|tx| tx.subscribe())
}

fn timestamp_hms() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs() % 86_400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;

    format!("{:02}:{:02}:{:02}", h, m, s)
}
