use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, Mutex};

/// A single multiplexed Assist WebSocket message.
#[derive(Debug, Clone)]
pub enum AssistWire {
    Text(String),
    Binary(Vec<u8>),
}

struct AssistChannel {
    agent: Option<mpsc::Sender<AssistWire>>,
    viewers: broadcast::Sender<AssistWire>,
    /// Latest full/keyframe, so a late viewer gets a first image immediately.
    last_keyframe: Option<AssistWire>,
}

/// In-memory broker for one Assist session: one Agent socket and N browser sockets.
#[derive(Default)]
pub struct AssistHub {
    inner: Mutex<HashMap<String, AssistChannel>>,
}

impl AssistHub {
    async fn ensure(&self, id: &str) -> () {
        let mut map = self.inner.lock().await;
        map.entry(id.to_string()).or_insert_with(|| AssistChannel {
            agent: None,
            viewers: broadcast::channel(32).0,
            last_keyframe: None,
        });
    }

    pub async fn subscribe(
        &self,
        id: &str,
    ) -> (broadcast::Receiver<AssistWire>, Option<AssistWire>) {
        self.ensure(id).await;
        let map = self.inner.lock().await;
        match map.get(id) {
            Some(c) => (c.viewers.subscribe(), c.last_keyframe.clone()),
            None => (broadcast::channel(32).0.subscribe(), None),
        }
    }

    pub async fn set_agent(&self, id: &str, tx: mpsc::Sender<AssistWire>) {
        self.ensure(id).await;
        let mut map = self.inner.lock().await;
        if let Some(channel) = map.get_mut(id) {
            channel.agent = Some(tx);
        }
    }

    pub async fn clear_agent(&self, id: &str) {
        let mut map = self.inner.lock().await;
        if let Some(channel) = map.get_mut(id) {
            channel.agent = None;
        }
    }

    pub async fn send_agent(&self, id: &str, msg: AssistWire) -> bool {
        let tx = {
            let map = self.inner.lock().await;
            map.get(id).and_then(|c| c.agent.clone())
        };
        match tx {
            Some(tx) => tx.try_send(msg).is_ok(),
            None => false,
        }
    }

    pub async fn broadcast(&self, id: &str, msg: AssistWire) {
        let tx = {
            let mut map = self.inner.lock().await;
            match map.get_mut(id) {
                Some(c) => {
                    if let AssistWire::Binary(bytes) = &msg
                        && is_full_frame(bytes)
                    {
                        c.last_keyframe = Some(msg.clone());
                    }
                    Some(c.viewers.clone())
                }
                None => None,
            }
        };
        if let Some(tx) = tx {
            let _ = tx.send(msg);
        }
    }
}

/// A v1 frame is full; a v2 frame is full when its rect covers the screen.
fn is_full_frame(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || &bytes[0..4] != b"IDVF" {
        return false;
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version < 2 || bytes.len() < 32 {
        return true;
    }
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    if flags & 1 != 0 {
        return true;
    }
    let sw = u16::from_le_bytes([bytes[20], bytes[21]]);
    let sh = u16::from_le_bytes([bytes[22], bytes[23]]);
    let x = u16::from_le_bytes([bytes[24], bytes[25]]);
    let y = u16::from_le_bytes([bytes[26], bytes[27]]);
    let w = u16::from_le_bytes([bytes[28], bytes[29]]);
    let h = u16::from_le_bytes([bytes[30], bytes[31]]);
    x == 0 && y == 0 && w == sw && h == sh
}

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub images_dir: PathBuf,
    pub software_dir: PathBuf,
    /// Directory holding uploaded `idv-agent` packages (`agent-<version>.exe`).
    pub agent_dir: PathBuf,
    /// Temp files for one-to-one remote file transfer.
    pub assist_fs_dir: PathBuf,
    /// Latest JPEG screen frame per assist session (in memory, bounded by overwrite).
    pub assist_frames: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Wakes helper long-polls when an input/clipboard/file command is queued.
    pub assist_notify: Arc<tokio::sync::Notify>,
    /// Multiplexed Assist WebSocket broker.
    pub assist_hub: Arc<AssistHub>,
    pub max_concurrent_deploy_targets: i64,
    pub max_pending_sync_per_edge: i64,
    pub recommended_deploy_batch: i64,
    /// Wakes agent long-poll waits when a new command is queued.
    pub agent_notify: Arc<tokio::sync::Notify>,
}
