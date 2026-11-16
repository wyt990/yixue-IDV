use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub images_dir: PathBuf,
    pub software_dir: PathBuf,
    pub max_concurrent_deploy_targets: i64,
    pub max_pending_sync_per_edge: i64,
    pub recommended_deploy_batch: i64,
    /// Wakes agent long-poll waits when a new command is queued.
    pub agent_notify: Arc<tokio::sync::Notify>,
}
