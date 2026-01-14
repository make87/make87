use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Hash)]
pub struct DeviceClientConfig {
    #[serde(default)]
    pub heartbeat_interval_secs: Option<u32>,
}

impl Default for DeviceClientConfig {
    fn default() -> Self {
        DeviceClientConfig {
            heartbeat_interval_secs: Some(30),
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
