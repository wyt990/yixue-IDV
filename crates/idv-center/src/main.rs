mod api;
mod auth;
mod db;
mod enroll;
mod lab_naming;
mod mac;
mod models;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::Router;
use axum::routing::get;
use clap::{Parser, Subcommand};
use idv_common::{Error, Result, init_tracing, load_toml_file};
use serde::{Deserialize, Serialize};
use state::AppState;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Debug, Parser)]
#[command(name = "idv-center", about = "易学 IDV 中心管理服务", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the HTTP management API
    Serve {
        /// Path to center.toml
        #[arg(short, long, default_value = "deploy/config/center.toml")]
        config: PathBuf,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct CenterConfig {
    listen: String,
    #[serde(default = "default_log")]
    log: String,
    #[serde(default = "default_database_url")]
    database_url: String,
    #[serde(default = "default_web_dir")]
    web_dir: String,
    #[serde(default = "default_images_dir")]
    images_dir: String,
    /// Cap on pending+running deploy targets (500+ protection).
    #[serde(default = "default_max_deploy")]
    max_concurrent_deploy_targets: i64,
    /// Max queued sync commands per edge before reject.
    #[serde(default = "default_max_sync_queue")]
    max_pending_sync_per_edge: i64,
    /// Suggested batch size for UI / ops docs (not a hard reject alone).
    #[serde(default = "default_recommended_batch")]
    recommended_deploy_batch: i64,
}

fn default_log() -> String {
    "info,idv_center=debug".into()
}

fn default_database_url() -> String {
    "sqlite://data/idv-center.db?mode=rwc".into()
}

fn default_web_dir() -> String {
    "web/dist".into()
}

fn default_images_dir() -> String {
    "data/images".into()
}
fn default_max_deploy() -> i64 {
    80
}
fn default_max_sync_queue() -> i64 {
    16
}
fn default_recommended_batch() -> i64 {
    40
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    service: &'static str,
    status: &'static str,
    product: &'static str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { config } => serve(config).await?,
    }
    Ok(())
}

async fn serve(config_path: PathBuf) -> Result<()> {
    let cfg: CenterConfig = load_toml_file(&config_path)?;
    init_tracing(&cfg.log);

    let pool = db::connect(&cfg.database_url).await.map_err(Error::Other)?;
    tokio::fs::create_dir_all(&cfg.images_dir)
        .await
        .map_err(Error::Io)?;
    let images_dir = PathBuf::from(&cfg.images_dir);
    let software_dir = images_dir
        .parent()
        .map(|p| p.join("software"))
        .unwrap_or_else(|| PathBuf::from("data/software"));
    tokio::fs::create_dir_all(&software_dir)
        .await
        .map_err(Error::Io)?;
    let state = AppState {
        pool: pool.clone(),
        images_dir,
        software_dir,
        max_concurrent_deploy_targets: cfg.max_concurrent_deploy_targets.max(1),
        max_pending_sync_per_edge: cfg.max_pending_sync_per_edge.max(1),
        recommended_deploy_batch: cfg.recommended_deploy_batch.max(1),
    };
    // Phase-3: tick power schedules every 30s
    {
        let tick_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let _ = crate::api::ops::tick_power_schedules(&tick_state).await;
            }
        });
    }
    // Fail stale maintenance targets (interval 60s; threshold from ops_settings)
    {
        let sweep_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                match crate::api::tasks::sweep_stale_tasks(&sweep_state).await {
                    Ok(n) if n > 0 => tracing::info!(count = n, "swept stale maintenance targets"),
                    Err(err) => tracing::warn!(error = %err, "stale task sweep failed"),
                    _ => {}
                }
            }
        });
    }

    let addr: SocketAddr = cfg
        .listen
        .parse()
        .map_err(|err| Error::Config(format!("invalid listen address '{}': {err}", cfg.listen)))?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_router = api::router().with_state(state.clone());

    let mut app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/health", get(health))
        .merge(api_router)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let web_dir = PathBuf::from(&cfg.web_dir);
    if web_dir.exists() {
        let index = web_dir.join("index.html");
        let static_files = ServeDir::new(&web_dir).not_found_service(ServeFile::new(index));
        app = app.fallback_service(static_files);
        info!(path = %web_dir.display(), "serving management UI");
    } else {
        info!(
            path = %web_dir.display(),
            "web_dir missing — API only (build web/ with npm run build)"
        );
    }

    info!(%addr, db = %cfg.database_url, "idv-center listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "idv-center",
        status: "ok",
        product: "易学 IDV 机房管理系统",
    })
}
