use anyhow::Result;
use serde::Deserialize;
use std::time::Duration;
use tracing::warn;

use crate::device::services::service_info::{ServiceInfo, ServiceKind};
use crate::util::command::{binary_exists, safe_run_command};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerJson {
    names: Option<Vec<String>>,
    state: Option<String>,
    running_for: Option<String>,
    restart_count: Option<u32>,
}

pub async fn collect_docker_services() -> Result<Vec<ServiceInfo>> {
    if !binary_exists("docker") {
        return Ok(Vec::new());
    }

    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(&["ps", "--all", "--format", "{{json .}}"]);

    let output = safe_run_command(cmd, Duration::from_secs(3)).await?;

    if !output.status.success() {
        warn!("`docker ps` exited with {:?}", output.status);
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_containers(stdout.as_ref(), ServiceKind::Docker))
}

fn parse_containers(output: &str, kind: ServiceKind) -> Vec<ServiceInfo> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<ContainerJson>(line).ok())
        .map(|c| ServiceInfo {
            name: c
                .names
                .as_ref()
                .and_then(|n| n.first())
                .cloned()
                .unwrap_or_default(),
            kind: kind.clone(),
            status: c.state.unwrap_or_else(|| "unknown".into()),
            uptime_secs: parse_running_for(&c.running_for),
            restart_count: c.restart_count.unwrap_or(0),
        })
        .collect()
}

fn parse_running_for(val: &Option<String>) -> u64 {
    if let Some(v) = val {
        let parts: Vec<_> = v.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(n) = parts[0].parse::<u64>() {
                return match parts[1] {
                    "seconds" | "second" => n,
                    "minutes" | "minute" => n * 60,
                    "hours" | "hour" => n * 3600,
                    "days" | "day" => n * 86400,
                    _ => 0,
                };
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_running_for_seconds() {
        assert_eq!(parse_running_for(&Some("30 seconds".to_string())), 30);
    }

    #[test]
    fn test_parse_running_for_second_singular() {
        assert_eq!(parse_running_for(&Some("1 second".to_string())), 1);
    }

    #[test]
    fn test_parse_running_for_minutes() {
        assert_eq!(parse_running_for(&Some("5 minutes".to_string())), 300);
    }

    #[test]
    fn test_parse_running_for_minute_singular() {
        assert_eq!(parse_running_for(&Some("1 minute".to_string())), 60);
    }

    #[test]
    fn test_parse_running_for_hours() {
        assert_eq!(parse_running_for(&Some("2 hours".to_string())), 7200);
    }

    #[test]
    fn test_parse_running_for_hour_singular() {
        assert_eq!(parse_running_for(&Some("1 hour".to_string())), 3600);
    }

    #[test]
    fn test_parse_running_for_days() {
        assert_eq!(parse_running_for(&Some("3 days".to_string())), 259200);
    }

    #[test]
    fn test_parse_running_for_day_singular() {
        assert_eq!(parse_running_for(&Some("1 day".to_string())), 86400);
    }

    #[test]
    fn test_parse_running_for_none() {
        assert_eq!(parse_running_for(&None), 0);
    }

    #[test]
    fn test_parse_running_for_invalid_unit() {
        assert_eq!(parse_running_for(&Some("5 weeks".to_string())), 0);
    }

    #[test]
    fn test_parse_running_for_invalid_number() {
        assert_eq!(parse_running_for(&Some("abc hours".to_string())), 0);
    }

    #[test]
    fn test_parse_running_for_empty_string() {
        assert_eq!(parse_running_for(&Some("".to_string())), 0);
    }

    #[test]
    fn test_parse_containers_empty() {
        let result = parse_containers("", ServiceKind::Docker);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_containers_single() {
        let json = r#"{"Names":["my-container"],"State":"running","RunningFor":"2 hours","RestartCount":0}"#;
        let result = parse_containers(json, ServiceKind::Docker);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "my-container");
        assert_eq!(result[0].status, "running");
        assert_eq!(result[0].uptime_secs, 7200);
        assert_eq!(result[0].restart_count, 0);
    }

    #[test]
    fn test_parse_containers_multiple() {
        let json = r#"{"Names":["container1"],"State":"running","RunningFor":"1 hour","RestartCount":0}
{"Names":["container2"],"State":"exited","RunningFor":"30 minutes","RestartCount":2}"#;
        let result = parse_containers(json, ServiceKind::Docker);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "container1");
        assert_eq!(result[1].name, "container2");
        assert_eq!(result[1].restart_count, 2);
    }

    #[test]
    fn test_parse_containers_missing_optional_fields() {
        let json = r#"{"Names":null,"State":null,"RunningFor":null,"RestartCount":null}"#;
        let result = parse_containers(json, ServiceKind::Docker);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "");
        assert_eq!(result[0].status, "unknown");
        assert_eq!(result[0].uptime_secs, 0);
        assert_eq!(result[0].restart_count, 0);
    }

    #[test]
    fn test_parse_containers_invalid_json_line_skipped() {
        let json = "not valid json\n{\"Names\":[\"valid\"],\"State\":\"running\"}";
        let result = parse_containers(json, ServiceKind::Docker);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "valid");
    }

    #[test]
    fn test_container_json_deserialization() {
        let json = r#"{"Names":["test"],"State":"running","RunningFor":"5 minutes","RestartCount":3}"#;
        let container: ContainerJson = serde_json::from_str(json).unwrap();
        assert_eq!(container.names, Some(vec!["test".to_string()]));
        assert_eq!(container.state, Some("running".to_string()));
        assert_eq!(container.running_for, Some("5 minutes".to_string()));
        assert_eq!(container.restart_count, Some(3));
    }
}
