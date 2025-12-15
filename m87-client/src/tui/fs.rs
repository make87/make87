use russh_sftp::{client::fs::DirEntry, protocol::FileType};

fn mode_string(perm: u32, ty: FileType) -> String {
    let file_type = match ty {
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        _ => '-',
    };

    let r = |b| if perm & b != 0 { 'r' } else { '-' };
    let w = |b| if perm & b != 0 { 'w' } else { '-' };
    let x = |b| if perm & b != 0 { 'x' } else { '-' };

    format!(
        "{}{}{}{}{}{}{}{}{}{}",
        file_type,
        r(0o400),
        w(0o200),
        x(0o100),
        r(0o040),
        w(0o020),
        x(0o010),
        r(0o004),
        w(0o002),
        x(0o001)
    )
}

fn human_size(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if size < 1024 {
        format!("{}B", size)
    } else if size < (1024 * 1024) {
        format!("{:.1}K", size as f64 / KB)
    } else if size < (1024 * 1024 * 1024) {
        format!("{:.1}M", size as f64 / MB)
    } else {
        format!("{:.1}G", size as f64 / GB)
    }
}

fn color_name(name: &str, ty: FileType) -> String {
    if matches!(ty, FileType::Dir) {
        format!("\x1b[34m{}\x1b[0m", name)
    } else {
        name.to_string()
    }
}

pub fn print_dir_entries(entries: &[DirEntry]) {
    if entries.is_empty() {
        println!("(empty)");
        return;
    }

    // compute width for size alignment
    let max_size = entries
        .iter()
        .map(|e| e.metadata().size.unwrap_or(0))
        .max()
        .unwrap_or(1);

    let size_width = human_size(max_size).len().max(6);

    for e in entries {
        print_direntry_unix(e, size_width);
    }
}

// Single-line Unix-style print
fn print_direntry_unix(e: &DirEntry, size_width: usize) {
    let attrs = e.metadata();

    let ftype = attrs.file_type();
    let perms = mode_string(attrs.permissions.unwrap_or(0), ftype);
    let user = attrs.user.clone().unwrap_or_else(|| "-".into());
    let group = attrs.group.clone().unwrap_or_else(|| "-".into());

    let size = attrs.size.unwrap_or(0);
    let size_s = format!("{:>width$}", human_size(size), width = size_width);

    let mtime = attrs.mtime.unwrap_or(0) as u64;
    let date = crate::util::logging::human_time(mtime);

    let name = color_name(&e.file_name(), ftype);

    println!(
        "{:} {:>8} {:>8} {} {} {}",
        perms, user, group, size_s, date, name
    );
}
