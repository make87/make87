use serde::{Deserialize, Serialize};

use crate::config::DeviceClientConfig;
use crate::device::DeviceSystemInfo;
use crate::metrics::SystemMetrics;
use crate::services::ServiceInfo;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct HeartbeatRequest {
    pub last_instruction_hash: String,
    #[serde(default)]
    pub system_info: Option<DeviceSystemInfo>,
    #[serde(default)]
    pub client_version: Option<String>,
    #[serde(default)]
    pub metrics: Option<SystemMetrics>,
    #[serde(default)]
    pub services: Option<Vec<ServiceInfo>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HeartbeatResponse {
    pub up_to_date: bool,
    pub config: Option<DeviceClientConfig>,
    pub instruction_hash: String,
}
