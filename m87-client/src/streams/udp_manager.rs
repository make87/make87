use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, time::Instant};
use tokio::sync::{Mutex, mpsc};

use crate::streams::stream_type::UdpTarget;

#[derive(Clone)]
pub struct UdpChannel {
    pub target: UdpTarget,
    pub sender: mpsc::Sender<Bytes>, // sends payload to UDP to QUIC worker
    pub last_used: Instant,
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
        let manager = Self {
            inner: Arc::new(Mutex::new(UdpChannelState {
                next_id: 1,
                channels: HashMap::new(),
            })),
        };

        tokio::spawn({
            let mgr = manager.inner.clone();
            async move {
                let timeout = Duration::from_secs(30);
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;

                    let mut state = mgr.lock().await;
                    let now = Instant::now();

                    state
                        .channels
                        .retain(|_, ch| now.duration_since(ch.last_used) < timeout);
                }
            }
        });
        manager
    }

    pub async fn alloc(&self, target: UdpTarget) -> (u32, mpsc::Receiver<Bytes>) {
        let mut g = self.inner.lock().await;

        let id = g.next_id;
        g.next_id += 1;

        let (tx, rx) = mpsc::channel::<Bytes>(1024);

        g.channels.insert(
            id,
            UdpChannel {
                target,
                sender: tx,
                last_used: Instant::now(),
            },
        );

        (id, rx)
    }

    pub async fn get(&self, id: u32) -> Option<UdpChannel> {
        let g = self.inner.lock().await;
        let mut ch = g.channels.get(&id).cloned();

        if let Some(ch_some) = &mut ch {
            ch_some.last_used = Instant::now();
        }

        ch
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
