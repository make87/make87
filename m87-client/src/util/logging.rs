use std::fmt::{self};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Once, OnceLock};

use tokio::sync::broadcast;

use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt as tracing_fmt;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::util::log_renderer::UiEvent;

static LOG_TX: OnceLock<broadcast::Sender<UiEvent>> = OnceLock::new();
static CLI_MODE: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);

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
    tx: broadcast::Sender<UiEvent>,
}

impl LogBroadcastLayer {
    pub fn new(tx: broadcast::Sender<UiEvent>) -> Self {
        Self { tx }
    }

    fn is_loading(msg: &str) -> bool {
        msg.trim() == "[loading]"
    }

    fn is_done(msg: &str) -> Option<&str> {
        msg.trim().strip_prefix("[done]").map(str::trim)
    }
}

impl<S> Layer<S> for LogBroadcastLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MsgVisitor { msg: String::new() };
        event.record(&mut visitor);
        let msg = visitor.msg.trim_matches('"');

        // let meta = event.metadata();
        // let level = *meta.level();
        // let target = meta.target();

        let cli_mode = CLI_MODE.load(Ordering::Relaxed);
        // let verbose = VERBOSE.load(Ordering::Relaxed);
        let verbose = VERBOSE.load(Ordering::Relaxed);

        // Verbose = bypass renderer completely
        if verbose {
            //     let ts = timestamp_hms();
            //     let mut line = String::new();
            //     let _ = write!(line, "[{}] {:?} [{}] {}", ts, level, target, msg);
            let _ = self.tx.send(UiEvent::Line(msg.to_string()));
            return;
        }

        // ---- Loading start ----
        if cli_mode && Self::is_loading(msg) {
            let _ = self.tx.send(UiEvent::LoadingStart);
            return;
        }

        // ---- Done ----
        if cli_mode {
            if let Some(text) = Self::is_done(msg) {
                let _ = self.tx.send(UiEvent::Done {
                    ok: true,
                    text: text.to_string(),
                });
                return;
            }
        }

        // ---- Normal line ----
        let _ = self.tx.send(UiEvent::Line(msg.to_string()));
    }
}

pub fn init_tracing_with_log_layer(
    default_level: &str,
    cli_mode: bool,
    verbose: bool,
) -> broadcast::Sender<UiEvent> {
    let (tx, _rx) = broadcast::channel(1024);
    LOG_TX.set(tx.clone()).ok();

    CLI_MODE.store(cli_mode, Ordering::Relaxed);
    VERBOSE.store(verbose, Ordering::Relaxed);

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    match (cli_mode, verbose) {
        (_, true) => {
            // Agent / daemon normal mode
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_fmt::layer())
                .with(LogBroadcastLayer::new(tx.clone()))
                .init();
        }
        (true, false) => {
            // CLI UX
            tracing_subscriber::registry()
                .with(filter)
                .with(LogBroadcastLayer::new(tx.clone()))
                .init();
        }
        (false, false) => {
            // Agent / daemon normal mode
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_fmt::layer())
                .init();
        }
    }

    tx
}

static INIT: Once = Once::new();

pub fn init_logging(log_level: &str, cli_mode: bool, verbose: bool) {
    INIT.call_once(|| {
        let tx = init_tracing_with_log_layer(log_level, cli_mode, verbose);
        let rx = tx.subscribe();
        if cli_mode && !verbose {
            tokio::spawn(async move {
                crate::util::log_renderer::run_renderer(rx).await;
            });
        }
    });
}

pub fn timestamp_hms() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    human_time(now.as_secs())
}

pub fn human_time(ts: u64) -> String {
    let secs = ts % 86_400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn get_log_rx() -> Option<broadcast::Receiver<UiEvent>> {
    LOG_TX.get().map(|tx| tx.subscribe())
}
