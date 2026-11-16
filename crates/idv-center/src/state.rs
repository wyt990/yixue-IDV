use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;

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
    pub assist_frames: Arc<tokio::sync::Mutex<HashMap<String, Vec<u8>>>>,
    /// Wakes helper long-polls when an input/clipboard/file command is queued.
    pub assist_notify: Arc<tokio::sync::Notify>,
    pub max_concurrent_deploy_targets: i64,
    pub max_pending_sync_per_edge: i64,
    pub recommended_deploy_batch: i64,
    /// Wakes agent long-poll waits when a new command is queued.
    pub agent_notify: Arc<tokio::sync::Notify>,
}
