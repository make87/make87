use chrono::{SecondsFormat, Utc};

const RESET: &str = "\x1b[0m";
const GREY: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";
const WHITE: &str = "\x1b[37m";

pub fn format_log(source: &str, message: &str, ansi: bool) -> String {
    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let msg = message.trim_end_matches('\n');

    if !ansi {
        return format!("{ts} {source}: {msg}");
    }

    format!(
        "{grey}[{ts}]{reset} {cyan}{source}{reset}: {white}{msg}{reset}",
        grey = GREY,
        cyan = CYAN,
        white = WHITE,
        reset = RESET,
    )
}
