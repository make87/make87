use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crate::streams::stream_type::UdpTarget;

#[derive(Clone)]
pub struct UdpChannel {
    pub target: UdpTarget,
    pub sender: mpsc::Sender<Bytes>, // sends payload to UDP->QUIC worker
}

#[derive(Clone)]
pub struct UdpChannelManager {
    inner: Arc<Mutex<UdpChannelState>>,
}

struct UdpChannelState {
    next_id: u32,
    channels: HashMap<u32, UdpChannel>,
}

impl UdpChannelManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(UdpChannelState {
                next_id: 1,
                channels: HashMap::new(),
            })),
        }
    }

    pub async fn alloc(&self, target: UdpTarget) -> (u32, mpsc::Receiver<Bytes>) {
        let mut g = self.inner.lock().await;

        let id = g.next_id;
        g.next_id += 1;

        let (tx, rx) = mpsc::channel::<Bytes>(1024);

        g.channels.insert(id, UdpChannel { target, sender: tx });

        (id, rx)
    }

    pub async fn get(&self, id: u32) -> Option<UdpChannel> {
        let g = self.inner.lock().await;
        g.channels.get(&id).cloned()
    }

    pub async fn remove(&self, id: u32) -> bool {
        let mut g = self.inner.lock().await;
        g.channels.remove(&id).is_some()
    }

    pub async fn remove_all(&self) {
        let mut g = self.inner.lock().await;
        g.channels.clear();
    }
}
