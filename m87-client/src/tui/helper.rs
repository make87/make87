use chrono::{DateTime, Local, TimeZone, Utc};
use ratatui::crossterm;

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Right,
}

pub struct RenderOpts {
    pub use_color: bool,
    pub separator_char: char,
    pub separator_pad: usize,
    pub show_logs_inline: bool,
    pub max_log_hint: usize,
    pub wrap: bool,
    pub time_only: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            use_color: true,
            separator_char: '_',
            separator_pad: 0,
            show_logs_inline: true,
            max_log_hint: 72,
            wrap: true,
            time_only: false,
        }
    }
}

pub fn terminal_width() -> Option<usize> {
    match crossterm::terminal::size() {
        Ok((w, _h)) if w > 0 => Some(w as usize),
        _ => None,
    }
}

pub fn separator_line(w: usize, opts: &RenderOpts) -> String {
    let pad = opts.separator_pad;
    let inner = w.saturating_sub(pad * 2);
    format!(
        "{}{}{}",
        " ".repeat(pad),
        opts.separator_char.to_string().repeat(inner),
        " ".repeat(pad)
    )
}

pub fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn log_hint(s: &str, max: usize) -> String {
    let one = single_line(s);
    if visible_width(&one) <= max {
        one
    } else {
        let mut t = truncate_visible(&one, max.saturating_sub(1));
        while t.ends_with(' ') {
            t.pop();
        }
        t.push('…');
        t
    }
}

pub fn visible_width(s: &str) -> usize {
    let mut w = 0usize;
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        if ch == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                while let Some(c) = it.next() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        w += 1;
    }
    w
}

pub fn truncate_visible(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    let mut it = s.chars().peekable();

    while let Some(ch) = it.next() {
        if ch == '\x1b' {
            out.push(ch);
            if it.peek() == Some(&'[') {
                out.push(it.next().unwrap());
                while let Some(c) = it.next() {
                    out.push(c);
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }

        if w + 1 > max_w {
            break;
        }
        out.push(ch);
        w += 1;
    }

    out
}

fn pad(s: &str, width: usize, align: Align) -> String {
    let t = truncate_visible(s, width);
    let vw = visible_width(&t);
    if vw >= width {
        return t;
    }
    let pad = " ".repeat(width - vw);
    match align {
        Align::Left => format!("{t}{pad}"),
        Align::Right => format!("{pad}{t}"),
    }
}

fn wrap_visible(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let s = s.trim_end_matches('\n');

    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    for word in s.split_whitespace() {
        let ww = visible_width(word);

        if cur_w == 0 {
            if ww <= width {
                cur.push_str(word);
                cur_w = ww;
            } else {
                let mut rest = word.to_string();
                while !rest.is_empty() {
                    let chunk = truncate_visible(&rest, width);
                    let cw = visible_width(&chunk);
                    lines.push(chunk);
                    rest = rest.chars().skip(cw).collect();
                }
            }
            continue;
        }

        if cur_w + 1 + ww <= width {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        } else {
            lines.push(cur);
            cur = String::new();
            cur_w = 0;

            if ww <= width {
                cur.push_str(word);
                cur_w = ww;
            } else {
                let mut rest = word.to_string();
                while !rest.is_empty() {
                    let chunk = truncate_visible(&rest, width);
                    let cw = visible_width(&chunk);
                    lines.push(chunk);
                    rest = rest.chars().skip(cw).collect();
                }
            }
        }
    }

    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[derive(Clone)]
pub struct ColSpec {
    pub title: &'static str,
    pub min: usize,
    pub max: Option<usize>,
    pub weight: usize,
    pub align: Align,
    pub wrap: bool,
}

pub struct Table {
    w: usize,
    gutter: usize,
    cols: Vec<ColSpec>,
    widths: Vec<usize>,
}

impl Table {
    pub fn get_column_width(&self, index: usize) -> Option<usize> {
        self.widths.get(index).copied()
    }

    pub fn get_column_width_as_whitespace(&self, index: usize) -> String {
        self.widths
            .get(index)
            .map(|&w| " ".repeat(w))
            .unwrap_or_default()
    }

    pub fn new(w: usize, gutter: usize, cols: Vec<ColSpec>) -> Self {
        let w = w.max(60);
        let widths = compute_widths(w, gutter, &cols);
        Self {
            w,
            gutter,
            cols,
            widths,
        }
    }

    pub fn header(&self, out: &mut String, opts: &RenderOpts) {
        let cells: Vec<&str> = self.cols.iter().map(|c| c.title).collect();
        self.row(out, &cells, opts);
    }

    pub fn row(&self, out: &mut String, cells: &[&str], opts: &RenderOpts) {
        let mut per_col_lines: Vec<Vec<String>> = Vec::with_capacity(self.cols.len());

        for (i, col) in self.cols.iter().enumerate() {
            let s = cells.get(i).copied().unwrap_or("");
            let w = self.widths[i];
            let lines = if opts.wrap && col.wrap {
                wrap_visible(s, w)
            } else {
                vec![truncate_visible(s, w)]
            };
            per_col_lines.push(lines);
        }

        let rows = per_col_lines.iter().map(|v| v.len()).max().unwrap_or(1);

        for r in 0..rows {
            let mut line = String::new();
            for i in 0..self.cols.len() {
                if i > 0 {
                    line.push_str(&" ".repeat(self.gutter));
                }
                let col = &self.cols[i];
                let w = self.widths[i];

                let cell = per_col_lines[i].get(r).map(String::as_str).unwrap_or("");
                let cell = if r == 0 {
                    cell
                } else if opts.wrap && col.wrap {
                    cell
                } else {
                    ""
                };

                line.push_str(&pad(cell, w, col.align));
            }

            let vw = visible_width(&line);
            if vw < self.w {
                line.push_str(&" ".repeat(self.w - vw));
            } else if vw > self.w {
                line = truncate_visible(&line, self.w);
            }

            out.push_str(&line);
            out.push('\n');
        }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn widths(&self) -> &[usize] {
        &self.widths
    }
}

fn compute_widths(w: usize, gutter: usize, cols: &[ColSpec]) -> Vec<usize> {
    let gutters_total = gutter * cols.len().saturating_sub(1);
    let mut widths: Vec<usize> = cols.iter().map(|c| c.min).collect();

    let used_min: usize = widths.iter().sum::<usize>() + gutters_total;
    let mut remaining = w.saturating_sub(used_min);

    loop {
        let mut progressed = false;
        let total_weight: usize = cols
            .iter()
            .enumerate()
            .filter(|(i, _)| cols[*i].max.map(|m| widths[*i] < m).unwrap_or(true))
            .map(|(_, c)| c.weight.max(1))
            .sum();

        if remaining == 0 || total_weight == 0 {
            break;
        }

        for (i, c) in cols.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            let capped = c.max.map(|m| widths[i] >= m).unwrap_or(false);
            if capped {
                continue;
            }
            let share = ((remaining as u128 * (c.weight.max(1) as u128)) / (total_weight as u128))
                .max(1) as usize;
            let add = share.min(remaining);

            let new_w = widths[i] + add;
            let final_w = match c.max {
                Some(m) => new_w.min(m),
                None => new_w,
            };
            let actual_add = final_w - widths[i];
            if actual_add > 0 {
                widths[i] = final_w;
                remaining -= actual_add;
                progressed = true;
            }
        }

        if !progressed {
            break;
        }
    }

    widths
}

// Labeled, one-line header that doesn't waste the whole table as a single huge first column.
pub fn kv_line(w: usize, label: &str, value: &str, opts: &RenderOpts) -> String {
    let label = format!("{label}:");
    let lw = (visible_width(&label)).min(12).max(8);
    let vw = w.saturating_sub(lw + 2);
    let mut line = String::new();
    line.push_str(&pad(&label, lw, Align::Left));
    line.push_str("  ");
    let v = if opts.wrap { value } else { value };
    line.push_str(&pad(v, vw, Align::Left));
    line
}

#[derive(Clone, Copy)]
pub enum AnsiColor {
    Red,
    Green,
    Yellow,
    Cyan,
    Dim,
    None,
}

pub fn colorize(enabled: bool, s: &str, c: AnsiColor) -> String {
    if !enabled || matches!(c, AnsiColor::None) {
        return s.to_string();
    }
    let code = match c {
        AnsiColor::Red => "31",
        AnsiColor::Green => "32",
        AnsiColor::Yellow => "33",
        AnsiColor::Cyan => "36",
        AnsiColor::Dim => "2",
        AnsiColor::None => "",
    };
    format!("\x1b[{}m{}\x1b[0m", code, s)
}

pub fn format_time(ts: u64, time_only: bool) -> String {
    if ts == 0 {
        return "".into();
    }

    let (secs, nsec_opt) = if ts >= 1_000_000_000_000_000_000 {
        (ts / 1_000_000_000, (ts % 1_000_000_000) as u32)
    } else if ts >= 1_000_000_000_000_000 {
        (ts / 1_000_000, ((ts % 1_000_000) * 1_000) as u32)
    } else if ts >= 1_000_000_000_000 {
        (ts / 1_000, ((ts % 1_000) * 1_000_000) as u32)
    } else if ts >= 1_000_000_000 {
        (ts, 0u32)
    } else {
        return format!("+{}s", ts);
    };

    if secs < 946684800 || secs > 4102444800 {
        return ts.to_string();
    }

    let dt: DateTime<Local> = match Local.timestamp_opt(secs as i64, nsec_opt).single() {
        Some(dt) => dt,
        None => return "invalid timestamp".to_string(),
    };

    if time_only {
        dt.format("%H:%M:%S").to_string()
    } else {
        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

/// Format an ISO timestamp as relative time (e.g., "2 min ago", "3 days ago")
pub fn format_relative_time(iso_time: &str) -> String {
    let Ok(time) = iso_time.parse::<DateTime<Utc>>() else {
        return iso_time.to_string();
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(time);

    let secs = duration.num_seconds();
    if secs < 0 {
        return "just now".to_string();
    }
    if secs < 60 {
        return format!("{} sec ago", secs);
    }

    let mins = duration.num_minutes();
    if mins < 60 {
        return format!("{} min ago", mins);
    }

    let hours = duration.num_hours();
    if hours < 24 {
        return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
    }

    let days = duration.num_days();
    if days < 30 {
        return format!("{} day{} ago", days, if days == 1 { "" } else { "s" });
    }

    let months = days / 30;
    if months < 12 {
        return format!("{} month{} ago", months, if months == 1 { "" } else { "s" });
    }

    let years = days / 365;
    format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

pub fn dim(s: &str) -> String {
    format!("{DIM}{s}{RESET}")
}
pub fn green(s: &str) -> String {
    format!("{GREEN}{s}{RESET}")
}
pub fn red(s: &str) -> String {
    format!("{RED}{s}{RESET}")
}
pub fn yellow(s: &str) -> String {
    format!("{YELLOW}{s}{RESET}")
}
pub fn cyan(s: &str) -> String {
    format!("{CYAN}{s}{RESET}")
}
pub fn bold(s: &str) -> String {
    format!("{BOLD}{s}{RESET}")
}
pub fn gray(s: &str) -> String {
    format!("{DIM}{s}{RESET}")
}

pub fn status_badge(online: bool) -> String {
    if online {
        green("online").to_string()
    } else {
        red("offline").to_string()
    }
}

pub fn pending_badge(pending: bool) -> String {
    if pending {
        yellow("pending").to_string()
    } else {
        dim("-").to_string()
    }
}
