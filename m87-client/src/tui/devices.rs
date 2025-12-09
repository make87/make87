use chrono::{DateTime, Utc};

use m87_shared::{auth::DeviceAuthRequest, device::PublicDevice};

pub fn print_devices_table(devices: &[PublicDevice], auth_requests: &[DeviceAuthRequest]) {
    if devices.is_empty() && auth_requests.is_empty() {
        println!("No devices found");
        return;
    }

    // Fixed widths for all *except* Request ID
    const WIDTH_ID: usize = 11;
    const WIDTH_NAME: usize = 15;
    const WIDTH_STATUS: usize = 8;
    const WIDTH_ARCH: usize = 7;
    const WIDTH_OS: usize = 32;
    const WIDTH_IP: usize = 15;
    const WIDTH_LAST: usize = 20;
    const WIDTH_PENDING: usize = 8;

    // Header — REQUEST ID is not width-limited
    println!(
        "{:<WIDTH_ID$} {:<WIDTH_NAME$} {:<WIDTH_STATUS$} {:<WIDTH_ARCH$} {:<WIDTH_OS$} {:<WIDTH_IP$} {:<WIDTH_LAST$} {:<WIDTH_PENDING$} {}",
        "DEVICE ID", "NAME", "STATUS", "ARCH", "OS", "IP", "LAST SEEN", "PENDING", "REQUEST ID",
    );

    // Devices
    for dev in devices {
        let status = if dev.online { "online" } else { "offline" };
        let os = truncate_str(&dev.system_info.operating_system, WIDTH_OS - 1);
        let ip = dev.system_info.public_ip_address.as_deref().unwrap_or("-");
        let last_seen = format_relative_time(&dev.last_connection);

        println!(
            "{:<WIDTH_ID$} {:<WIDTH_NAME$} {:<WIDTH_STATUS$} {:<WIDTH_ARCH$} {:<WIDTH_OS$} {:<WIDTH_IP$} {:<WIDTH_LAST$} {:<WIDTH_PENDING$} {}",
            dev.short_id,
            dev.name,
            status,
            dev.system_info.architecture,
            os,
            ip,
            last_seen,
            "",
            "" // no request ID
        );
    }

    // Pending auth requests
    for req in auth_requests {
        let name = truncate_str(&req.device_info.hostname, WIDTH_NAME - 1);
        let os = truncate_str(&req.device_info.operating_system, WIDTH_OS - 1);
        let ip = req.device_info.public_ip_address.as_deref().unwrap_or("-");

        // REQUEST ID prints FULL
        println!(
            "{:<WIDTH_ID$} {:<WIDTH_NAME$} {:<WIDTH_STATUS$} {:<WIDTH_ARCH$} {:<WIDTH_OS$} {:<WIDTH_IP$} {:<WIDTH_LAST$} {:<WIDTH_PENDING$} {}",
            "",
            name,
            "pending",
            req.device_info.architecture,
            os,
            ip,
            "",
            "yes",
            req.request_id, // <- full, untruncated, copy-friendly
        );
    }
}

/// Truncate a string to max length, adding "..." if truncated
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}...", s.chars().take(max - 3).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Format an ISO timestamp as relative time (e.g., "2 min ago", "3 days ago")
fn format_relative_time(iso_time: &str) -> String {
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
