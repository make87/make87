use crate::{
    tui::helper::{
        Align, ColSpec, RenderOpts, Table, bold, cyan, dim, green, pending_badge, red, role_badge,
        status_badge, terminal_width, yellow,
    },
    util::device_cache::try_get_name_from_long_id,
};
use m87_shared::{
    auth::DeviceAuthRequest,
    device::{AuditLog, DeviceStatus, PublicDevice},
};

pub fn print_devices_table(devices: &[PublicDevice], auth_requests: &[DeviceAuthRequest]) {
    if devices.is_empty() && auth_requests.is_empty() {
        println!("{}", dim("No devices found"));
        return;
    }

    let term_w = terminal_width().unwrap_or(96).max(60);
    let opts = RenderOpts::default();

    let t_devices = Table::new(
        term_w.saturating_sub(2),
        1,
        vec![
            ColSpec {
                title: "ID",
                min: 6,
                max: Some(8),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "NAME",
                min: 10,
                max: Some(18),
                weight: 2,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "STATUS",
                min: 6,
                max: Some(8),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "Role",
                min: 10,
                max: Some(16),
                weight: 4,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "ARCH",
                min: 4,
                max: Some(6),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "OS",
                min: 12,
                max: Some(26),
                weight: 3,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "IP",
                min: 10,
                max: Some(39),
                weight: 4,
                align: Align::Left,
                wrap: false,
            },
        ],
    );

    // Auth requests table (REQUEST column is important/copy-friendly)
    let t_auth = Table::new(
        term_w.saturating_sub(2),
        1,
        vec![
            ColSpec {
                title: "NAME",
                min: 10,
                max: Some(22),
                weight: 3,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "STATUS",
                min: 6,
                max: Some(8),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "ARCH",
                min: 4,
                max: Some(6),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "OS",
                min: 12,
                max: Some(26),
                weight: 3,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "IP",
                min: 10,
                max: Some(39),
                weight: 4,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "REQUEST",
                min: 8,
                max: None,
                weight: 2,
                align: Align::Left,
                wrap: false,
            },
        ],
    );

    let mut out = String::new();

    // ---- AUTH REQUESTS (only if > 0, shown first) ----
    if !auth_requests.is_empty() {
        out.push_str(&format!("{}\n", bold("Auth requests")));
        t_auth.header(&mut out, &opts);

        for req in auth_requests {
            let ip = req.device_info.public_ip_address.as_deref().unwrap_or("-");

            out.push_str("  ");
            t_auth.row(
                &mut out,
                &[
                    &req.device_info.hostname,
                    &pending_badge(true),
                    &req.device_info.architecture,
                    &req.device_info.operating_system,
                    ip,
                    &req.request_id,
                ],
                &opts,
            );
        }

        out.push('\n');
    }

    // ---- DEVICES ----
    if !devices.is_empty() {
        out.push_str(&format!("{}\n", bold("Devices")));
        t_devices.header(&mut out, &opts);

        for dev in devices {
            let os = dev.system_info.operating_system.as_str();
            let ip = dev.system_info.public_ip_address.as_deref().unwrap_or("-");

            t_devices.row(
                &mut out,
                &[
                    &dev.short_id,
                    &dev.name,
                    &status_badge(dev.online),
                    &role_badge(&dev.role),
                    &dev.system_info.architecture,
                    os,
                    ip,
                ],
                &opts,
            );
        }
    }

    print!("{out}");
}

pub fn print_device_status(name: &str, status: &DeviceStatus) {
    let term_w = terminal_width().unwrap_or(96).max(60);
    let opts = RenderOpts::default();

    println!("{} {}", "Device", bold(name));

    if status.observations.is_empty() && status.incidents.is_empty() {
        println!("  {}", dim("No observations or incidents"));
        return;
    }

    if !status.observations.is_empty() {
        println!("{}", bold("Observations"));

        // Keep it compact; let NAME grow, numbers stay small.
        let t = Table::new(
            term_w.saturating_sub(2),
            1,
            vec![
                ColSpec {
                    title: "NAME",
                    min: 10,
                    max: Some(18),
                    weight: 3,
                    align: Align::Left,
                    wrap: false,
                },
                ColSpec {
                    title: "LIVELINESS",
                    min: 9,
                    max: Some(12),
                    weight: 0,
                    align: Align::Left,
                    wrap: false,
                },
                ColSpec {
                    title: "HEALTH",
                    min: 7,
                    max: Some(10),
                    weight: 0,
                    align: Align::Left,
                    wrap: false,
                },
                ColSpec {
                    title: "CRASHES",
                    min: 7,
                    max: Some(8),
                    weight: 0,
                    align: Align::Right,
                    wrap: false,
                },
                ColSpec {
                    title: "UNHEALTHY_CHECKS",
                    min: 14,
                    max: Some(18),
                    weight: 0,
                    align: Align::Right,
                    wrap: false,
                },
            ],
        );

        let mut out = String::new();
        out.push_str("  ");
        t.header(&mut out, &opts);

        for obs in &status.observations {
            let life = if obs.alive {
                green("ALIVE")
            } else {
                red("DEAD")
            };
            let health = if obs.healthy {
                green("HEALTHY")
            } else {
                yellow("UNHEALTHY")
            };

            let crashes = if obs.crashes > 0 {
                red(&obs.crashes.to_string())
            } else {
                dim("0")
            };

            let checks = if obs.unhealthy_checks > 0 {
                yellow(&obs.unhealthy_checks.to_string())
            } else {
                dim("0")
            };

            out.push_str("  ");
            t.row(
                &mut out,
                &[&obs.name, &life, &health, &crashes, &checks],
                &opts,
            );
        }

        print!("{out}");
    }

    if !status.incidents.is_empty() {
        println!("{}", bold("Incidents"));

        let t = Table::new(
            term_w.saturating_sub(2),
            1,
            vec![
                ColSpec {
                    title: "ID",
                    min: 10,
                    max: Some(18),
                    weight: 0,
                    align: Align::Left,
                    wrap: false,
                },
                ColSpec {
                    title: "START",
                    min: 16,
                    max: Some(20),
                    weight: 0,
                    align: Align::Left,
                    wrap: false,
                },
                ColSpec {
                    title: "END",
                    min: 16,
                    max: None,
                    weight: 2,
                    align: Align::Left,
                    wrap: false,
                },
            ],
        );

        let mut out = String::new();
        out.push_str("  ");
        t.header(&mut out, &opts);

        for inc in &status.incidents {
            out.push_str("  ");
            t.row(
                &mut out,
                &[&red(&inc.id), &dim(&inc.start_time), &dim(&inc.end_time)],
                &opts,
            );
        }

        print!("{out}");
    } else {
        println!("{}", bold("No Incidents"));
    }
}

pub fn print_deployment_reports(reports: &[AuditLog], show_details: bool) {
    if reports.is_empty() {
        println!("{}", dim("No deployment reports found"));
        return;
    }

    let term_w = terminal_width().unwrap_or(96);
    let opts = RenderOpts::default();

    fn action_badge(action: &str) -> String {
        let a = action.to_ascii_lowercase();
        if a.contains("create") || a.contains("add") || a.contains("provision") {
            green(action)
        } else if a.contains("delete") || a.contains("remove") || a.contains("revoke") {
            red(action)
        } else if a.contains("update") || a.contains("edit") || a.contains("patch") {
            yellow(action)
        } else {
            cyan(action)
        }
    }

    println!("{}", bold("Deployment reports"));

    let t = Table::new(
        term_w.saturating_sub(2),
        1,
        vec![
            ColSpec {
                title: "TIME",
                min: 22,
                max: Some(25),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "DEVICE",
                min: 8,
                max: Some(16),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "USER",
                min: 8,
                max: Some(35),
                weight: 3,
                align: Align::Left,
                wrap: false,
            },
            ColSpec {
                title: "ACTION",
                min: 14,
                max: Some(200),
                weight: 0,
                align: Align::Left,
                wrap: false,
            },
        ],
    );

    let mut out = String::new();
    out.push_str("  ");
    t.header(&mut out, &opts);

    // Indent details by the visual width of column 0 (TIME), like your log-tail logic.
    // This assumes Table has the same helper you used: get_column_width_as_whitespace(col_idx).
    let details_ws = if show_details {
        format!("{}{}", t.get_column_width_as_whitespace(0), " ")
    } else {
        String::new()
    };

    for r in reports {
        let time = r.timestamp.clone();
        let user = format!("{} <{}>", r.user_name, r.user_email);
        let action = action_badge(&r.action);
        let device = match &r.device_id {
            Some(id) => dim(&try_get_name_from_long_id(id).unwrap_or(id.to_string())),
            None => dim("-").to_string(),
        };

        out.push_str("  ");

        t.row(&mut out, &[&time, &device, &user, &action], &opts);
        if show_details {
            let d = r.details.trim();
            if !d.is_empty() && d != "-" {
                // Print details below the row, dim, and align under TIME column width.
                // Multi-line details keep alignment (same trick as log tail).
                out.push_str(&format!(
                    "{}{}",
                    details_ws,
                    dim(&d.replace("\n", &format!("\n{}", details_ws)))
                ));
                out.push('\n');
            }
        }
    }

    print!("{out}");
}
