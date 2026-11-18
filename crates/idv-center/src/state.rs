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
        });
    }

    pub async fn subscribe(&self, id: &str) -> broadcast::Receiver<AssistWire> {
        self.ensure(id).await;
        let map = self.inner.lock().await;
        map.get(id)
            .map(|c| c.viewers.subscribe())
            .unwrap_or_else(|| broadcast::channel(32).0.subscribe())
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
            let map = self.inner.lock().await;
            map.get(id).map(|c| c.viewers.clone())
        };
        if let Some(tx) = tx {
            let _ = tx.send(msg);
        }
    }
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
