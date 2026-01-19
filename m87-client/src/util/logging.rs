use std::sync::{Once, OnceLock};

use tokio::sync::broadcast;

use tracing_subscriber::fmt as tracing_fmt;
use tracing_subscriber::{EnvFilter, prelude::*};

static LOG_TX: OnceLock<broadcast::Sender<String>> = OnceLock::new();

pub fn init_tracing_with_log_layer(default_level: &str) -> broadcast::Sender<String> {
    let (tx, _rx) = broadcast::channel(1024);
    LOG_TX.set(tx.clone()).ok();

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_fmt::layer())
        .init();

    tx
}

static INIT: Once = Once::new();

pub fn init_logging(log_level: &str) {
    INIT.call_once(|| {
        let _ = init_tracing_with_log_layer(log_level);
        // let _rx = tx.subscribe();
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

pub fn get_log_rx() -> Option<broadcast::Receiver<String>> {
    LOG_TX.get().map(|tx| tx.subscribe())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_time_zero() {
        assert_eq!(human_time(0), "00:00:00");
    }

    #[test]
    fn test_human_time_full_day() {
        // 23:59:59 = 23*3600 + 59*60 + 59 = 86399
        assert_eq!(human_time(86399), "23:59:59");
    }

    #[test]
    fn test_human_time_wraps_at_day() {
        // 86400 seconds = 1 day, should wrap to 00:00:00
        assert_eq!(human_time(86400), "00:00:00");
    }

    #[test]
    fn test_human_time_hours_minutes_seconds() {
        // 1 hour + 1 minute + 1 second = 3661
        assert_eq!(human_time(3661), "01:01:01");
    }

    #[test]
    fn test_human_time_only_seconds() {
        assert_eq!(human_time(45), "00:00:45");
    }

    #[test]
    fn test_human_time_only_minutes() {
        assert_eq!(human_time(300), "00:05:00");
    }

    #[test]
    fn test_timestamp_hms_format() {
        let ts = timestamp_hms();
        // Should match HH:MM:SS format
        assert_eq!(ts.len(), 8);
        assert_eq!(&ts[2..3], ":");
        assert_eq!(&ts[5..6], ":");
    }
}
