use std::collections::HashMap;
use std::sync::Arc;

use quinn::Connection;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Clone)]
pub struct RelayState {
    tunnels: Arc<RwLock<HashMap<String, Connection>>>,
    lost: Arc<RwLock<HashMap<String, ()>>>, // just a set, we don't need Instant
}

impl RelayState {
    pub fn new() -> Self {
        Self {
            tunnels: Arc::new(RwLock::new(HashMap::new())),
            lost: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert a new tunnel and close the old one if present.
    pub async fn replace_tunnel(&self, device_id: &str, conn: Connection) {
        info!("Replacing tunnel for device {}", device_id);

        // Replace old tunnel atomically.
        let old = {
            let mut t = self.tunnels.write().await;
            t.insert(device_id.to_string(), conn)
        };

        // Remove "lost" flag — the device is now online.
        {
            let mut lost = self.lost.write().await;
            lost.remove(device_id);
        }

        // Clean up old tunnel if there was one
        if let Some(old_conn) = old {
            warn!("Closing old tunnel for device {}", device_id);
            old_conn.close(0u32.into(), b"replaced-by-new-connection");
        }
    }

    /// Remove the tunnel ONLY if this connection is still the active one.
    pub async fn remove_if_match(&self, device_id: &str, conn_id: usize) {
        let mut tunnels = self.tunnels.write().await;

        if let Some(active) = tunnels.get(device_id) {
            // Connection ID must be compared to ensure we don't remove a newer tunnel
            if active.stable_id() == conn_id {
                info!("Removing tunnel for device {} (matched)", device_id);
                tunnels.remove(device_id);

                // Mark device lost
                let mut lost = self.lost.write().await;
                lost.insert(device_id.to_string(), ());
            } else {
                warn!(
                    "Skipping removal for device {} because connection ID does not match (stale close event)",
                    device_id
                );
            }
        }
    }

    /// Returns true only if device has an active and *not lost* tunnel.
    pub async fn has_tunnel(&self, device_id: &str) -> bool {
        let lost = self.lost.read().await;
        if lost.contains_key(device_id) {
            return false;
        }

        let tunnels = self.tunnels.read().await;
        tunnels.contains_key(device_id)
    }

    /// Return active (non-lost) tunnel
    pub async fn get_tunnel(&self, device_id: &str) -> Option<Connection> {
        let lost = self.lost.read().await;
        if lost.contains_key(device_id) {
            return None;
        }

        let tunnels = self.tunnels.read().await;
        tunnels.get(device_id).cloned()
    }
}
