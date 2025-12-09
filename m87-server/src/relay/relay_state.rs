use std::collections::HashMap;
use std::sync::Arc;

use quinn::Connection;
use tokio::sync::RwLock;
use tracing::info;

use crate::response::ServerResult;

#[derive(Clone)]
pub struct RelayState {
    pub tunnels: Arc<RwLock<HashMap<String, Connection>>>,
    lost: Arc<RwLock<HashMap<String, std::time::Instant>>>,
}

impl RelayState {
    pub fn new() -> ServerResult<Self> {
        Ok(Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            lost: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn register_tunnel(&self, device_id: &str, conn: Connection) {
        info!("Registering tunnel for device {}", device_id);
        {
            let mut t = self.tunnels.write().await;
            t.insert(device_id.to_string(), conn);
        }

        {
            let mut lost = self.lost.write().await;
            lost.remove(device_id);
        }
    }

    pub async fn remove_tunnel(&self, device_id: &str) {
        info!("Removing tunnel for device {}", device_id);
        {
            let mut t = self.tunnels.write().await;
            t.remove(device_id);
        }
        {
            let mut lost = self.lost.write().await;
            lost.remove(device_id);
        }
    }

    pub async fn mark_tunnel_lost(&self, device_id: &str) {
        let mut lost = self.lost.write().await;
        lost.insert(device_id.to_string(), std::time::Instant::now());
    }

    pub async fn is_still_lost(&self, device_id: &str) -> bool {
        let lost = self.lost.read().await;
        lost.contains_key(device_id)
    }

    pub async fn has_tunnel(&self, device_id: &str) -> bool {
        self.tunnels.read().await.contains_key(device_id)
    }

    pub async fn get_tunnel(&self, device_id: &str) -> Option<Connection> {
        info!("Getting tunnel for device {}", device_id);
        // print all keys
        println!("Tunnels: {:?}", self.tunnels.read().await.keys());
        let t = self.tunnels.read().await;
        t.get(device_id).cloned()
    }
}
