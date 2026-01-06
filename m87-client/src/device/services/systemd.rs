use anyhow::Result;
use std::time::Duration;
use tracing::warn;

use crate::device::services::service_info::{ServiceInfo, ServiceKind};
use crate::util::command::{binary_exists, safe_run_command};

pub async fn collect_systemd_services() -> Result<Vec<ServiceInfo>> {
    if !binary_exists("systemctl") {
        return Ok(Vec::new());
    }

    let mut cmd = tokio::process::Command::new("systemctl");
    cmd.args(&["list-units", "--type=service", "--no-pager", "--no-legend"]);

    let output = safe_run_command(cmd, Duration::from_secs(3)).await?;

    if !output.status.success() {
        warn!("`systemctl list-units` exited with {:?}", output.status);
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_systemd_services(stdout.as_ref()))
}

fn parse_systemd_services(output: &str) -> Vec<ServiceInfo> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                Some(ServiceInfo {
                    name: parts[0].to_string(),
                    kind: ServiceKind::Systemd,
                    status: parts[3].to_string(),
                    uptime_secs: 0, // Not provided by systemctl
                    restart_count: 0,
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_systemd_services_empty() {
        let result = parse_systemd_services("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_systemd_services_single_running() {
        let output = "ssh.service loaded active running OpenSSH server daemon";
        let result = parse_systemd_services(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ssh.service");
        assert_eq!(result[0].status, "running");
        assert_eq!(result[0].uptime_secs, 0);
        assert_eq!(result[0].restart_count, 0);
        assert!(matches!(result[0].kind, ServiceKind::Systemd));
    }

    #[test]
    fn test_parse_systemd_services_single_exited() {
        let output = "docker.service loaded active exited Docker Application Container Engine";
        let result = parse_systemd_services(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docker.service");
        assert_eq!(result[0].status, "exited");
    }

    #[test]
    fn test_parse_systemd_services_multiple() {
        let output = "ssh.service loaded active running OpenSSH server daemon
nginx.service loaded active running A high performance web server
cron.service loaded active running Regular background program processing daemon";
        let result = parse_systemd_services(output);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "ssh.service");
        assert_eq!(result[1].name, "nginx.service");
        assert_eq!(result[2].name, "cron.service");
    }

    #[test]
    fn test_parse_systemd_services_short_line_skipped() {
        let output = "too short
ssh.service loaded active running OpenSSH server daemon";
        let result = parse_systemd_services(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "ssh.service");
    }

    #[test]
    fn test_parse_systemd_services_mixed_states() {
        let output = "active.service loaded active running Running service
failed.service loaded failed failed Failed service
inactive.service loaded inactive dead Inactive service";
        let result = parse_systemd_services(output);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].status, "running");
        assert_eq!(result[1].status, "failed");
        assert_eq!(result[2].status, "dead");
    }
}
