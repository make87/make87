use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default, Hash)]
pub struct ObservationConfig {
    pub docker_services: Vec<String>,
    pub systemd_services: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Hash)]
pub struct DeviceClientConfig {
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u32>,
    #[serde(default)]
    pub observe: ObservationConfig,
}

impl Default for DeviceClientConfig {
    fn default() -> Self {
        DeviceClientConfig {
            heartbeat_interval_secs: Some(30),
            observe: ObservationConfig::default(),
        }
    }
}

impl DeviceClientConfig {
    pub fn get_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}
