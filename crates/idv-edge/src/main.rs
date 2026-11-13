mod dhcp;
mod p2p;
mod pe_api;
mod tftp;
mod worker;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::Router;
use axum::routing::{get, post, put};
use clap::{Parser, Subcommand};
use dhcp::{DhcpConfig, DhcpService};
use idv_common::{Error, Result, init_tracing, load_toml_file};
use idv_common::chunking::DEFAULT_CHUNK_SIZE;
use p2p::P2pState;
use pe_api::PeState;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use worker::EdgeRuntime;

#[derive(Debug, Parser)]
#[command(name = "idv-edge", about = "易学 IDV 边缘缓存 / PXE 节点", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run edge HTTP + sync worker (+ optional DHCP/TFTP)
    Serve {
        #[arg(short, long, default_value = "deploy/config/edge.toml")]
        config: PathBuf,
    },
    /// Send a DHCP Discover and print Offer summary (stage 3 lab/probe)
    DhcpProbe {
        #[arg(long, default_value = "127.0.0.1:6767")]
        server: String,
        #[arg(long, default_value = "02:11:22:33:44:55")]
        mac: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct EdgeConfig {
    listen: String,
    center_url: String,
    node_token: String,
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_cache_dir")]
    cache_dir: String,
    /// Local default for PE capture staging. Empty => `{cache_dir}/capture-tmp`.
    /// Center UI `capture_tmp_dir` overrides this when non-empty (via heartbeat).
    #[serde(default)]
    capture_tmp_dir: String,
    #[serde(default = "default_log")]
    log: String,
    #[serde(default = "default_heartbeat_secs")]
    heartbeat_secs: u64,
    #[serde(default = "default_poll_secs")]
    poll_secs: u64,
    /// Soft cache quota in bytes (0 = unlimited / no watermark).
    #[serde(default)]
    cache_quota_bytes: u64,
    #[serde(default = "default_cache_warn")]
    cache_warn_pct: f64,
    /// Sync download throttle (bytes/sec, 0 = unlimited).
    #[serde(default)]
    sync_bandwidth_bps: u64,
    #[serde(default = "default_max_sync")]
    max_concurrent_syncs: usize,
    #[serde(default = "default_chunk_size")]
    p2p_chunk_size: usize,
    /// Required for WiFi PE checkin / claim (empty rejects wifi source).
    #[serde(default = "default_pe_deploy_token")]
    pe_deploy_token: String,
    #[serde(default)]
    pxe: PxeConfig,
}

fn default_pe_deploy_token() -> String {
    "lab-pe-token".into()
}

#[derive(Debug, Clone, Deserialize)]
struct PxeConfig {
    #[serde(default)]
    enabled: bool,
    /// full | proxy | off
    #[serde(default = "default_pxe_mode")]
    mode: String,
    #[serde(default = "default_dhcp_bind")]
    dhcp_bind: String,
    #[serde(default = "default_tftp_bind")]
    tftp_bind: String,
    #[serde(default = "default_tftp_root")]
    tftp_root: String,
    #[serde(default = "default_boot_file")]
    boot_file: String,
    /// Edge IP visible to clients (for next-server / iPXE)
    #[serde(default = "default_next_server")]
    next_server: String,
    #[serde(default = "default_subnet")]
    subnet_mask: String,
    #[serde(default = "default_router")]
    router: String,
    #[serde(default = "default_offer_start")]
    offer_start: String,
    #[serde(default = "default_offer_end")]
    offer_end: String,
    #[serde(default = "default_lease")]
    lease_secs: u32,
}

impl Default for PxeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_pxe_mode(),
            dhcp_bind: default_dhcp_bind(),
            tftp_bind: default_tftp_bind(),
            tftp_root: default_tftp_root(),
            boot_file: default_boot_file(),
            next_server: default_next_server(),
            subnet_mask: default_subnet(),
            router: default_router(),
            offer_start: default_offer_start(),
            offer_end: default_offer_end(),
            lease_secs: default_lease(),
        }
    }
}

fn default_name() -> String {
    "edge-local".into()
}
fn default_cache_dir() -> String {
    "data/edge-cache".into()
}
fn default_log() -> String {
    "info,idv_edge=debug".into()
}
fn default_heartbeat_secs() -> u64 {
    10
}
fn default_poll_secs() -> u64 {
    2
}
fn default_cache_warn() -> f64 {
    85.0
}
fn default_max_sync() -> usize {
    2
}
fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}
fn default_pxe_mode() -> String {
    "full".into()
}
fn default_dhcp_bind() -> String {
    "0.0.0.0:67".into()
}
fn default_tftp_bind() -> String {
    "0.0.0.0:69".into()
}
fn default_tftp_root() -> String {
    "pe".into()
}
fn default_boot_file() -> String {
    "undionly.kpxe".into()
}
fn default_next_server() -> String {
    "10.10.0.1".into()
}
fn default_subnet() -> String {
    "255.255.255.0".into()
}
fn default_router() -> String {
    "10.10.0.1".into()
}
fn default_offer_start() -> String {
    "10.10.0.100".into()
}
fn default_offer_end() -> String {
    "10.10.0.200".into()
}
fn default_lease() -> u32 {
    3600
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    service: &'static str,
    status: &'static str,
    center_url: String,
    center_reachable: bool,
    name: String,
    cache_dir: String,
    pxe_enabled: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { config } => serve(config).await?,
        Commands::DhcpProbe { server, mac } => dhcp_probe(&server, &mac).await?,
    }
    Ok(())
}

async fn dhcp_probe(server: &str, mac: &str) -> Result<()> {
    use tokio::net::UdpSocket;
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return Err(Error::Config(format!("invalid mac: {mac}")));
    }
    let mut chaddr = [0u8; 6];
    for i in 0..6 {
        chaddr[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|err| Error::Config(err.to_string()))?;
    }

    let mut pkt = vec![0u8; 240];
    pkt[0] = 1; // request
    pkt[1] = 1; // ethernet
    pkt[2] = 6; // hw len
    pkt[4..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]); // xid
    pkt[28..34].copy_from_slice(&chaddr);
    pkt[236..240].copy_from_slice(&[99, 130, 83, 99]);
    pkt.extend_from_slice(&[53, 1, 1]); // Discover
    pkt.extend_from_slice(&[55, 4, 1, 3, 66, 67]); // param request
    pkt.push(255);

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(Error::Io)?;
    sock.send_to(&pkt, server)
        .await
        .map_err(Error::Io)?;
    let mut buf = vec![0u8; 1500];
    let (n, peer) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        sock.recv_from(&mut buf),
    )
    .await
    .map_err(|_| Error::Other(anyhow::anyhow!("DHCP probe timeout")))?
    .map_err(Error::Io)?;

    let msg = buf.get(240..).and_then(|opts| {
        let mut i = 0;
        while i + 1 < opts.len() {
            let code = opts[i];
            if code == 255 {
                break;
            }
            if code == 0 {
                i += 1;
                continue;
            }
            let len = opts[i + 1] as usize;
            if code == 53 && i + 2 < opts.len() {
                return Some(opts[i + 2]);
            }
            i += 2 + len;
        }
        None
    });
    let boot = {
        let end = buf[108..236]
            .iter()
            .position(|b| *b == 0)
            .map(|p| 108 + p)
            .unwrap_or(236);
        String::from_utf8_lossy(&buf[108..end]).to_string()
    };
    println!("dhcp_probe peer={peer} bytes={n} msg_type={msg:?} boot_file={boot}");
    if msg != Some(2) && msg != Some(5) {
        return Err(Error::Other(anyhow::anyhow!(
            "expected DHCP Offer/Ack, got {msg:?}"
        )));
    }
    if boot.is_empty() {
        return Err(Error::Other(anyhow::anyhow!("empty boot file in reply")));
    }
    Ok(())
}

async fn serve(config_path: PathBuf) -> Result<()> {
    let cfg: EdgeConfig = load_toml_file(&config_path)?;
    init_tracing(&cfg.log);

    if cfg.node_token.trim().is_empty()
        || cfg.node_token.contains("REPLACE_WITH_CENTER_EDGE_TOKEN")
    {
        return Err(Error::Config(
            "node_token is required in edge.toml (create edge in center first)".into(),
        ));
    }

    tokio::fs::create_dir_all(&cfg.cache_dir)
        .await
        .map_err(Error::Io)?;
    tokio::fs::create_dir_all(&cfg.pxe.tftp_root)
        .await
        .map_err(Error::Io)?;

    let capture_tmp_default = {
        let trimmed = cfg.capture_tmp_dir.trim();
        if trimmed.is_empty() {
            PathBuf::from(&cfg.cache_dir).join("capture-tmp")
        } else {
            PathBuf::from(trimmed)
        }
    };
    tokio::fs::create_dir_all(&capture_tmp_default)
        .await
        .map_err(Error::Io)?;

    let addr: SocketAddr = cfg
        .listen
        .parse()
        .map_err(|err| Error::Config(format!("invalid listen address '{}': {err}", cfg.listen)))?;

    let listen_url = format!("http://{addr}");
    let center_reachable = Arc::new(AtomicBool::new(false));
    let runtime = EdgeRuntime {
        center_url: cfg.center_url.trim_end_matches('/').to_string(),
        node_token: cfg.node_token.clone(),
        name: cfg.name.clone(),
        listen_url,
        cache_dir: PathBuf::from(&cfg.cache_dir),
        capture_tmp_default: capture_tmp_default.clone(),
        capture_tmp_dir: Arc::new(std::sync::RwLock::new(capture_tmp_default)),
        cache_quota_bytes: cfg.cache_quota_bytes,
        cache_warn_pct: cfg.cache_warn_pct,
        sync_bandwidth_bps: cfg.sync_bandwidth_bps,
        center_reachable: center_reachable.clone(),
        // System HTTP_PROXY must not hijack Edge→Center loopback/LAN calls.
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("reqwest client"),
        sync_slots: Arc::new(tokio::sync::Semaphore::new(
            cfg.max_concurrent_syncs.max(1),
        )),
    };

    if let Err(err) = runtime.register().await {
        warn!(error = %err, "initial register failed; will retry in background");
    } else {
        center_reachable.store(true, Ordering::Relaxed);
    }

    let worker_runtime = runtime.clone();
    let heartbeat_secs = cfg.heartbeat_secs.max(3);
    let poll_secs = cfg.poll_secs.max(1);
    tokio::spawn(async move {
        worker_runtime.run_loops(heartbeat_secs, poll_secs).await;
    });

    if cfg.pxe.enabled {
        let mode = if cfg.pxe.mode == "off" {
            "full".to_string()
        } else {
            cfg.pxe.mode.clone()
        };
        let dhcp_cfg = DhcpConfig {
            mode,
            bind: cfg.pxe.dhcp_bind.clone(),
            next_server: parse_v4(&cfg.pxe.next_server)?,
            boot_file: cfg.pxe.boot_file.clone(),
            subnet_mask: parse_v4(&cfg.pxe.subnet_mask)?,
            router: parse_v4(&cfg.pxe.router)?,
            offer_start: parse_v4(&cfg.pxe.offer_start)?,
            offer_end: parse_v4(&cfg.pxe.offer_end)?,
            lease_secs: cfg.pxe.lease_secs,
            dns: None,
        };
        let dhcp = DhcpService::new(dhcp_cfg);
        tokio::spawn(async move {
            if let Err(err) = dhcp.run().await {
                warn!(error = %err, "DHCP/PXE service stopped");
            }
        });
        let tftp_bind = cfg.pxe.tftp_bind.clone();
        let tftp_root = PathBuf::from(&cfg.pxe.tftp_root);
        tokio::spawn(async move {
            if let Err(err) = tftp::run(&tftp_bind, tftp_root).await {
                warn!(error = %err, "TFTP service stopped");
            }
        });
        info!(
            dhcp = %cfg.pxe.dhcp_bind,
            tftp = %cfg.pxe.tftp_bind,
            root = %cfg.pxe.tftp_root,
            "PXE stack enabled"
        );
    }

    let pe_state = PeState {
        sessions: Arc::new(RwLock::new(HashMap::new())),
        runtime: runtime.clone(),
        pe_deploy_token: cfg.pe_deploy_token.clone(),
    };
    let health_state = runtime.clone();
    let pxe_enabled = cfg.pxe.enabled;
    let pe_root = PathBuf::from(&cfg.pxe.tftp_root);

    let pe_routes = Router::new()
        .route(
            "/api/v1/pe/checkin",
            get(pe_api::checkin_get).post(pe_api::checkin_post),
        )
        .route("/api/v1/pe/checkin.ipxe", get(pe_api::checkin_ipxe))
        .route("/api/v1/pe/sessions", get(pe_api::list_sessions))
        .route("/api/v1/pe/task/claim", get(pe_api::claim))
        .route("/api/v1/pe/task/targets/{id}/report", post(pe_api::report))
        .route(
            "/api/v1/pe/capture/{target_id}",
            put(pe_api::capture_upload),
        )
        .route("/api/v1/pe/images/{id}", get(pe_api::image_meta))
        .route("/api/v1/pe/images/{id}/blob", get(pe_api::image_blob))
        .with_state(pe_state);

    let p2p_state = P2pState {
        runtime: runtime.clone(),
        swarm: Arc::new(RwLock::new(HashMap::new())),
        peers: Arc::new(RwLock::new(HashMap::new())),
        chunk_size: if cfg.p2p_chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            cfg.p2p_chunk_size
        },
    };
    let p2p_routes = Router::new()
        .route("/api/v1/p2p/peers", post(p2p::register_peer).get(p2p::list_peers))
        .route("/api/v1/p2p/manifest/{image_id}", get(p2p::manifest))
        .route(
            "/api/v1/p2p/chunk/{image_id}/{index}",
            get(p2p::get_chunk).put(p2p::put_chunk),
        )
        .route("/api/v1/p2p/swarm/{image_id}", get(p2p::swarm_stats))
        .route(
            "/api/v1/p2p/swarm/{image_id}/stop",
            post(p2p::stop_swarm),
        )
        .with_state(p2p_state);

    let app = Router::new()
        .route(
            "/health",
            get(move || {
                let health_state = health_state.clone();
                async move {
                    Json(HealthResponse {
                        service: "idv-edge",
                        status: "ok",
                        center_url: health_state.center_url.clone(),
                        center_reachable: health_state.center_reachable.load(Ordering::Relaxed),
                        name: health_state.name.clone(),
                        cache_dir: health_state.cache_dir.display().to_string(),
                        pxe_enabled,
                    })
                }
            }),
        )
        .merge(pe_routes)
        .merge(p2p_routes)
        .nest_service("/pxe", ServeDir::new(pe_root))
        .layer(TraceLayer::new_for_http());

    info!(
        %addr,
        center = %cfg.center_url,
        cache = %cfg.cache_dir,
        "idv-edge listening"
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    Ok(())
}

fn parse_v4(s: &str) -> Result<Ipv4Addr> {
    s.parse()
        .map_err(|err| Error::Config(format!("invalid IPv4 '{s}': {err}")))
}
