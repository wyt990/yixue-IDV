use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use idv_common::image_format::{
    ImageHeader, deploy_reader_to_disk, format_bytes_progress, hex_sha256,
};
use idv_common::ntfs_used::pack_disk_ntfs_used_to_writer_ex;
use idv_common::{Error, Result, init_tracing, load_toml_file};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Streams packed image bytes from the capture thread into the HTTP body.
struct CaptureChunkWriter {
    tx: SyncSender<std::result::Result<bytes::Bytes, String>>,
    written: Arc<AtomicU64>,
}

impl Write for CaptureChunkWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Cap chunk size so a large write_all doesn't stall the sync channel as one blob.
        const MAX: usize = 1024 * 1024;
        let n = buf.len().min(MAX);
        self.tx
            .send(Ok(bytes::Bytes::copy_from_slice(&buf[..n])))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))?;
        self.written.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "idv-pe-agent",
    about = "易学 IDV PE 维护环境代理",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print agent identity and configured edge endpoint
    Status {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
    },
    /// Report into Edge maintenance session
    Checkin {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
        #[arg(long)]
        mac: String,
        #[arg(long, default_value_t = 0)]
        wait_secs: u64,
    },
    /// Capture a disk (file or physical device) into yixue-image-v1 and upload via Edge
    Capture {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
        #[arg(long)]
        mac: String,
        /// Disk path: file, Linux `/dev/sda`, or Windows `\\.\PhysicalDrive0`
        #[arg(long, default_value = "")]
        disk_file: PathBuf,
        #[arg(long, default_value = "windows")]
        os_family: String,
        #[arg(long, default_value = "uefi")]
        boot_mode: String,
    },
    /// Deploy claimed image onto a (file) disk from Edge cache
    Deploy {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
        #[arg(long)]
        mac: String,
        #[arg(long, default_value = "")]
        disk_file: PathBuf,
        /// Prefer Edge-mediated P2P swarm chunks (stage 5 pilot)
        #[arg(long, default_value_t = false)]
        p2p: bool,
    },
    /// Checkin then run capture or deploy based on claimed task
    Run {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
        #[arg(long)]
        mac: String,
        #[arg(long, default_value = "")]
        disk_file: PathBuf,
        #[arg(long, default_value = "windows")]
        os_family: String,
        #[arg(long, default_value = "uefi")]
        boot_mode: String,
        #[arg(long, default_value_t = false)]
        p2p: bool,
    },
    /// List / validate PE WiFi driver pack manifest
    WifiDrivers {
        #[arg(long, default_value = "pe/wifi-drivers/manifest.json")]
        manifest: PathBuf,
    },
    /// PE network wizard: persist iface/WiFi/Edge URL, probe health, optional WiFi checkin
    NetworkWizard {
        #[arg(short, long, default_value = "deploy/config/pe-agent.toml")]
        config: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "ethernet")]
        iface: String,
        #[arg(long, default_value = "")]
        ssid: String,
        #[arg(long, default_value = "")]
        psk: String,
        #[arg(long)]
        edge_url: String,
        #[arg(long, default_value = "")]
        pe_token: String,
        #[arg(long, default_value = "")]
        mac: String,
        /// Attempt PE checkin after probe (source=wifi when iface=wifi)
        #[arg(long, default_value_t = false)]
        checkin: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct PeAgentConfig {
    #[serde(default = "default_edge_url")]
    edge_url: String,
    #[serde(default = "default_log")]
    log: String,
}

fn default_edge_url() -> String {
    "http://127.0.0.1:8081".into()
}

fn default_log() -> String {
    "info,idv_pe_agent=debug".into()
}

#[derive(Debug, Deserialize)]
struct CheckinResponse {
    session_id: String,
    mac: String,
    status: String,
    has_task: bool,
    message: String,
    action: String,
    #[serde(default)]
    task: Option<ClaimedTask>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaimedTask {
    target_id: String,
    task_id: String,
    kind: String,
    disk_plan: String,
    image_id: Option<String>,
    mac: String,
    os_family: String,
    task_name: String,
}

#[derive(Debug, Serialize)]
struct CheckinRequest<'a> {
    mac: &'a str,
    source: &'a str,
}

#[derive(Debug, Serialize)]
struct TargetReport<'a> {
    state: &'a str,
    progress: f64,
    error_message: String,
    detail: String,
    p2p_bytes: i64,
    edge_bytes: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status { config } => status(config).await?,
        Commands::Checkin {
            config,
            mac,
            wait_secs,
        } => checkin(config, mac, wait_secs).await?,
        Commands::Capture {
            config,
            mac,
            disk_file,
            os_family,
            boot_mode,
        } => {
            let (_cfg, task) = require_task(&config, &mac, "capture").await?;
            capture_with_task(&config, &task, &disk_file, &os_family, &boot_mode).await?;
        }
        Commands::Deploy {
            config,
            mac,
            disk_file,
            p2p,
        } => {
            let (_cfg, task) = require_task(&config, &mac, "deploy").await?;
            deploy_with_task(&config, &task, &disk_file, p2p).await?;
        }
        Commands::Run {
            config,
            mac,
            disk_file,
            os_family,
            boot_mode,
            p2p,
        } => run_auto(config, mac, disk_file, os_family, boot_mode, p2p).await?,
        Commands::WifiDrivers { manifest } => {
            wifi_drivers_list(manifest).await?;
        }
        Commands::NetworkWizard {
            config,
            out,
            iface,
            ssid,
            psk,
            edge_url,
            pe_token,
            mac,
            checkin,
        } => {
            network_wizard(
                config, out, iface, ssid, psk, edge_url, pe_token, mac, checkin,
            )
            .await?
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct WifiDriverManifest {
    format_version: u32,
    drivers: Vec<WifiDriverEntry>,
}

#[derive(Debug, Deserialize)]
struct WifiDriverEntry {
    id: String,
    vendor: String,
    product: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    inf_path: String,
}

async fn wifi_drivers_list(manifest: PathBuf) -> Result<()> {
    let bytes = tokio::fs::read(&manifest)
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!("read manifest: {e}")))?;
    let doc: WifiDriverManifest = serde_json::from_slice(&bytes)
        .map_err(|e| Error::Other(anyhow::anyhow!("parse manifest: {e}")))?;
    if doc.drivers.is_empty() {
        return Err(Error::Other(anyhow::anyhow!("wifi driver manifest empty")));
    }
    println!(
        "wifi_drivers_ok format={} count={} path={}",
        doc.format_version,
        doc.drivers.len(),
        manifest.display()
    );
    for d in &doc.drivers {
        println!(
            "driver id={} vendor={} product={} status={} inf={}",
            d.id, d.vendor, d.product, d.status, d.inf_path
        );
    }
    Ok(())
}

async fn load_cfg(config_path: &PathBuf) -> Result<PeAgentConfig> {
    let mut cfg = if config_path.exists() {
        load_toml_file(config_path)?
    } else {
        PeAgentConfig {
            edge_url: default_edge_url(),
            log: default_log(),
        }
    };
    if let Ok(url) = std::env::var("IDV_EDGE_URL") {
        if !url.trim().is_empty() {
            cfg.edge_url = url.trim().to_string();
        }
    }
    Ok(cfg)
}

async fn status(config_path: PathBuf) -> Result<()> {
    let cfg = load_cfg(&config_path).await?;
    init_tracing(&cfg.log);
    info!(edge_url = %cfg.edge_url, "idv-pe-agent ready");
    println!("service=idv-pe-agent status=ok edge_url={}", cfg.edge_url);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct NetworkSession {
    iface: String,
    ssid: String,
    /// never log raw psk in production; stored only for session file in lab
    psk_set: bool,
    edge_url: String,
    reachable: bool,
    source: String,
    message: String,
}

async fn network_wizard(
    config_path: PathBuf,
    out: PathBuf,
    iface: String,
    ssid: String,
    psk: String,
    edge_url: String,
    pe_token: String,
    mac: String,
    do_checkin: bool,
) -> Result<()> {
    let _cfg = load_cfg(&config_path).await?;
    init_tracing(&_cfg.log);
    let iface_l = iface.to_ascii_lowercase();
    if iface_l != "ethernet" && iface_l != "wifi" {
        return Err(Error::Other(anyhow::anyhow!(
            "iface must be ethernet or wifi"
        )));
    }
    if iface_l == "wifi" && ssid.trim().is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "wifi requires --ssid (driver missing would also fail here)"
        )));
    }
    let edge = edge_url.trim_end_matches('/').to_string();
    let client = http_client();
    let health_url = format!("{edge}/health");
    let reachable = match client.get(&health_url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    };
    if !reachable {
        return Err(Error::Other(anyhow::anyhow!(
            "edge not reachable at {edge}"
        )));
    }
    let source = if iface_l == "wifi" {
        "wifi".to_string()
    } else {
        "ethernet".to_string()
    };
    let session = NetworkSession {
        iface: iface_l.clone(),
        ssid: ssid.clone(),
        psk_set: !psk.is_empty(),
        edge_url: edge.clone(),
        reachable: true,
        source: source.clone(),
        message: "network session ready".into(),
    };
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    let bytes = serde_json::to_vec_pretty(&session).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    tokio::fs::write(&out, &bytes)
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    println!(
        "network_wizard_ok iface={} edge={} session={}",
        session.iface,
        session.edge_url,
        out.display()
    );

    if do_checkin {
        if mac.trim().is_empty() {
            return Err(Error::Other(anyhow::anyhow!(
                "--checkin requires --mac"
            )));
        }
        let url = format!("{edge}/api/v1/pe/checkin");
        let mut req = client.post(&url).json(&CheckinRequest {
            mac: &mac,
            source: &source,
        });
        if source == "wifi" && !pe_token.is_empty() {
            req = req.header("X-Idv-Pe-Token", pe_token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(Error::Other(anyhow::anyhow!(
                "wifi/ethernet checkin failed: {status} {body}"
            )));
        }
        println!("checkin_ok source={source} body={body}");
    }
    Ok(())
}

async fn checkin(config_path: PathBuf, mac: String, wait_secs: u64) -> Result<()> {
    let cfg = load_cfg(&config_path).await?;
    init_tracing(&cfg.log);
    let body = do_checkin(&cfg, &mac).await?;
    println!("=== 易学 IDV 维护环境 ===");
    println!("session={}", body.session_id);
    println!("mac={}", body.mac);
    println!("status={}", body.status);
    println!("has_task={}", body.has_task);
    println!("action={}", body.action);
    println!("{}", body.message);
    if let Some(ref t) = body.task {
        println!(
            "task kind={} id={} plan={} image={:?}",
            t.kind, t.task_id, t.disk_plan, t.image_id
        );
    }
    if !body.has_task {
        println!("[提示] 无任务：返回本地磁盘启动。");
    }
    if wait_secs > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    }
    Ok(())
}

async fn do_checkin(cfg: &PeAgentConfig, mac: &str) -> Result<CheckinResponse> {
    let edge = cfg.edge_url.trim_end_matches('/');
    let url = format!("{edge}/api/v1/pe/checkin");
    let client = http_client();
    let resp = client
        .post(&url)
        .json(&CheckinRequest {
            mac,
            source: "pe-agent",
        })
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "checkin failed: {status} {body}"
        )));
    }
    resp.json()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))
}

async fn require_task(
    config_path: &PathBuf,
    mac: &str,
    expect_kind: &str,
) -> Result<(PeAgentConfig, ClaimedTask)> {
    let cfg = load_cfg(config_path).await?;
    init_tracing(&cfg.log);
    let body = do_checkin(&cfg, mac).await?;
    let task = body.task.ok_or_else(|| {
        Error::Other(anyhow::anyhow!("no maintenance task for mac={mac}"))
    })?;
    if task.kind != expect_kind {
        return Err(Error::Other(anyhow::anyhow!(
            "expected {expect_kind} task, got {}",
            task.kind
        )));
    }
    Ok((cfg, task))
}

async fn run_auto(
    config_path: PathBuf,
    mac: String,
    disk_file: PathBuf,
    os_family: String,
    boot_mode: String,
    p2p: bool,
) -> Result<()> {
    use std::io::Write;
    eprintln!(
        "pe-agent run: mac={mac} disk={} config={}",
        disk_file.display(),
        config_path.display()
    );
    let _ = std::io::stderr().flush();
    let cfg = load_cfg(&config_path).await?;
    init_tracing(&cfg.log);
    eprintln!("pe-agent: edge={} checkin...", cfg.edge_url);
    let _ = std::io::stderr().flush();
    let body = do_checkin(&cfg, &mac).await?;
    eprintln!(
        "pe-agent: checkin ok has_task={} action={}",
        body.has_task, body.action
    );
    let _ = std::io::stderr().flush();
    let Some(task) = body.task else {
        // Cold-capture PE always expects a task (iPXE already selected capture).
        // Returning Ok would make initramfs treat this as "capture OK" and reboot.
        eprintln!(
            "pe-agent: no task after checkin ({}); refusing success exit",
            body.message
        );
        println!("action=exit_to_local_disk message={}", body.message);
        return Err(Error::Other(anyhow::anyhow!(
            "no maintenance task for {mac} (was it already claimed then lost?)"
        )));
    };
    match task.kind.as_str() {
        "capture" => capture_with_task(&config_path, &task, &disk_file, &os_family, &boot_mode).await,
        "deploy" => deploy_with_task(&config_path, &task, &disk_file, p2p).await,
        other => Err(Error::Other(anyhow::anyhow!("unknown task kind: {other}"))),
    }
}

fn default_disk_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"\\.\PhysicalDrive0")
    } else {
        PathBuf::from("/dev/sda")
    }
}

fn resolve_disk(disk_file: &PathBuf) -> PathBuf {
    if disk_file.as_os_str().is_empty() {
        default_disk_path()
    } else {
        disk_file.clone()
    }
}

async fn capture_with_task(
    config_path: &PathBuf,
    task: &ClaimedTask,
    disk_file: &PathBuf,
    os_family: &str,
    boot_mode: &str,
) -> Result<()> {
    use std::sync::mpsc::sync_channel;

    let cfg = load_cfg(config_path).await?;
    init_tracing(&cfg.log);
    let disk = resolve_disk(disk_file);
    eprintln!(
        "pe-agent: capture start target={} disk={}",
        task.target_id,
        disk.display()
    );
    let _ = std::io::stderr().flush();
    report_target(&cfg, &task.target_id, "running", 0.05, "", "capture-start", 0, 0).await?;

    let os = if task.os_family.trim().is_empty() {
        os_family.to_string()
    } else {
        task.os_family.clone()
    };
    let boot = boot_mode.to_string();
    report_target(
        &cfg,
        &task.target_id,
        "running",
        0.15,
        "",
        "packing+uploading (single pass, pending sha)",
        0,
        0,
    )
    .await?;

    let edge = cfg.edge_url.trim_end_matches('/');
    let url = format!(
        "{edge}/api/v1/pe/capture/{}?task_id={}&name={}",
        task.target_id,
        urlencoding_lite(&task.task_id),
        urlencoding_lite(&task.task_name)
    );
    eprintln!("pe-agent: single-pass NTFS-used pack+upload -> {url}");
    let _ = std::io::stderr().flush();

    // Pack on a blocking thread; stream chunks to HTTP body (pending payload SHA).
    // Larger queue reduces pack-thread stalls while HTTP/progress share the runtime.
    let (tx, rx) = sync_channel::<std::result::Result<bytes::Bytes, String>>(32);
    let disk_for_pack = disk.clone();
    let written = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let stop_progress = Arc::new(AtomicBool::new(false));
    let written_w = Arc::clone(&written);
    let total_w = Arc::clone(&total);
    let pack_handle = std::thread::spawn(move || -> anyhow::Result<ImageHeader> {
        let mut w = CaptureChunkWriter {
            tx: tx.clone(),
            written: written_w,
        };
        let mut on_total = |n: u64| {
            total_w.store(n, Ordering::Relaxed);
            eprintln!(
                "pe-agent: pack total {}",
                format_bytes_progress(0, n)
            );
            let _ = std::io::stderr().flush();
        };
        match pack_disk_ntfs_used_to_writer_ex(&disk_for_pack, &mut w, &os, &boot, &mut on_total)
        {
            Ok(header) => Ok(header),
            Err(err) => {
                let _ = tx.send(Err(err.to_string()));
                Err(err)
            }
        }
    });

    let progress_cfg = cfg.clone();
    let progress_target = task.target_id.clone();
    let written_p = Arc::clone(&written);
    let total_p = Arc::clone(&total);
    let stop_p = Arc::clone(&stop_progress);
    let progress_task = tokio::spawn(async move {
        let mut last_bytes = 0u64;
        let mut last_at = std::time::Instant::now();
        let mut announced_wait = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if stop_p.load(Ordering::Relaxed) {
                break;
            }
            let done = written_p.load(Ordering::Relaxed);
            let tot = total_p.load(Ordering::Relaxed);
            if done == 0 {
                continue;
            }
            // Bytes are fully handed to HTTP; Center/Edge may still be hashing ~tens of GB.
            if tot > 0 && done >= tot {
                if !announced_wait {
                    announced_wait = true;
                    eprintln!(
                        "pe-agent: upload bytes complete ({}); waiting for server ingest/hash (can take several minutes)...",
                        format_bytes_progress(done, tot)
                    );
                    let _ = std::io::stderr().flush();
                    let _ = report_target(
                        &progress_cfg,
                        &progress_target,
                        "running",
                        0.99,
                        "",
                        "upload done; server hashing",
                        0,
                        done as i64,
                    )
                    .await;
                    last_at = std::time::Instant::now();
                } else if last_at.elapsed() >= std::time::Duration::from_secs(30) {
                    last_at = std::time::Instant::now();
                    eprintln!("pe-agent: still waiting for server ingest/hash...");
                    let _ = std::io::stderr().flush();
                }
                continue;
            }
            let due_bytes = done.saturating_sub(last_bytes) >= 256 * 1024 * 1024;
            let due_time = last_at.elapsed() >= std::time::Duration::from_secs(5);
            if !due_bytes && !due_time {
                continue;
            }
            last_bytes = done;
            last_at = std::time::Instant::now();
            let detail = format_bytes_progress(done, tot);
            let ratio = if tot > 0 {
                (done as f64 / tot as f64).clamp(0.0, 0.99)
            } else {
                0.2
            };
            eprintln!("pe-agent: uploaded {detail}");
            let _ = std::io::stderr().flush();
            let _ = report_target(
                &progress_cfg,
                &progress_target,
                "running",
                ratio,
                "",
                &detail,
                0,
                done as i64,
            )
            .await;
        }
    });

    let stream = futures_util::stream::unfold(Some(rx), |state| async move {
        let Some(rx) = state else {
            return None;
        };
        match tokio::task::spawn_blocking(move || {
            let r = rx.recv();
            (r, rx)
        })
        .await
        {
            Ok((Ok(Ok(chunk)), rx)) => Some((Ok::<_, std::io::Error>(chunk), Some(rx))),
            Ok((Ok(Err(msg)), _)) => Some((Err(std::io::Error::other(msg)), None)),
            Ok((Err(_), _)) => None,
            Err(e) => Some((Err(std::io::Error::other(e)), None)),
        }
    });

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3600 * 6))
        .build()
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let send_result = client
        .put(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await;
    let pack_result = pack_handle
        .join()
        .map_err(|_| Error::Other(anyhow::anyhow!("pack thread panicked")))?;
    stop_progress.store(true, Ordering::Relaxed);
    let _ = progress_task.await;

    let done_bytes = written.load(Ordering::Relaxed);
    let tot_bytes = total.load(Ordering::Relaxed).max(done_bytes);
    let final_detail = format_bytes_progress(done_bytes, tot_bytes);

    let fail = |msg: String| async {
        let _ = report_target(
            &cfg,
            &task.target_id,
            "failed",
            (done_bytes as f64 / tot_bytes.max(1) as f64).clamp(0.0, 0.99),
            &msg,
            &final_detail,
            0,
            done_bytes as i64,
        )
        .await;
        Err(Error::Other(anyhow::anyhow!(msg)))
    };

    let resp = match send_result {
        Ok(r) => r,
        Err(err) => {
            return fail(format!("upload send failed: {err}")).await;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let _ = pack_result;
        return fail(format!("upload failed: {status} {body}")).await;
    }
    let header = match pack_result {
        Ok(h) => h,
        Err(e) => {
            return fail(format!("pack failed after upload: {e}")).await;
        }
    };
    if tot_bytes > 0 && done_bytes + 1024 * 1024 < tot_bytes {
        // More than 1MiB short of announced total — treat as truncated stream.
        return fail(format!(
            "upload truncated: wrote {done_bytes} of {tot_bytes} bytes ({final_detail})"
        ))
        .await;
    }
    let image: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(err) => {
            return fail(format!(
                "upload OK but ingest response invalid (Edge may still be hashing): {err}"
            ))
            .await;
        }
    };
    let done_detail = format_bytes_progress(done_bytes, done_bytes.max(tot_bytes));
    report_target(
        &cfg,
        &task.target_id,
        "done",
        1.0,
        "",
        &done_detail,
        0,
        done_bytes as i64,
    )
    .await?;
    println!(
        "capture_ok image_id={} checksum={} encoding={} payload={} logical={} progress={}",
        image["id"].as_str().unwrap_or(""),
        image["checksum_sha256"].as_str().unwrap_or(""),
        header.encoding,
        header.payload_size,
        header.logical_size,
        done_detail
    );
    Ok(())
}

async fn deploy_with_task(
    config_path: &PathBuf,
    task: &ClaimedTask,
    disk_file: &PathBuf,
    use_p2p: bool,
) -> Result<()> {
    use std::io::{Read, Write as _};
    use std::sync::mpsc::sync_channel;

    let cfg = load_cfg(config_path).await?;
    init_tracing(&cfg.log);
    let image_id = task.image_id.clone().ok_or_else(|| {
        Error::Other(anyhow::anyhow!("deploy task missing image_id"))
    })?;
    let disk = resolve_disk(disk_file);
    report_target(
        &cfg,
        &task.target_id,
        "running",
        0.05,
        "",
        "stream-deploy-start",
        0,
        0,
    )
    .await?;

    let edge = cfg.edge_url.trim_end_matches('/');
    let client = http_client();

    // P2P still reassembles in RAM — refuse on real disks to avoid OOM.
    if use_p2p {
        let msg = "P2P deploy not supported for streaming disk write yet; omit --p2p";
        fail_deploy(&cfg, task, &disk, msg).await?;
        return Err(Error::Other(anyhow::anyhow!(msg)));
    }

    let meta_url = format!("{edge}/api/v1/pe/images/{image_id}");
    let meta_resp = client
        .get(&meta_url)
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !meta_resp.status().is_success() {
        let status = meta_resp.status();
        let body = meta_resp.text().await.unwrap_or_default();
        let msg = format!("image meta failed: {status} {body}");
        fail_deploy(&cfg, task, &disk, &msg).await?;
        return Err(Error::Other(anyhow::anyhow!(msg)));
    }
    let meta: serde_json::Value = meta_resp
        .json()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    let expected = meta["checksum_sha256"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let total = meta["size_bytes"].as_u64().unwrap_or(0);
    eprintln!(
        "pe-agent: streaming deploy {} -> {} ({})",
        image_id,
        disk.display(),
        format_bytes_progress(0, total)
    );
    let _ = std::io::stderr().flush();

    let blob_url = format!("{edge}/api/v1/pe/images/{image_id}/blob");
    let blob_resp = client
        .get(&blob_url)
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !blob_resp.status().is_success() {
        let status = blob_resp.status();
        let body = blob_resp.text().await.unwrap_or_default();
        let msg = format!("image blob failed: {status} {body}");
        fail_deploy(&cfg, task, &disk, &msg).await?;
        return Err(Error::Other(anyhow::anyhow!(msg)));
    }

    let (tx, rx) = sync_channel::<std::result::Result<bytes::Bytes, String>>(16);
    let written = Arc::new(AtomicU64::new(0));
    let stop_progress = Arc::new(AtomicBool::new(false));
    let written_p = Arc::clone(&written);
    let stop_p = Arc::clone(&stop_progress);
    let progress_cfg = cfg.clone();
    let progress_target = task.target_id.clone();
    let progress_task = tokio::spawn(async move {
        let mut last = 0u64;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if stop_p.load(Ordering::Relaxed) {
                break;
            }
            let done = written_p.load(Ordering::Relaxed);
            if done == last {
                continue;
            }
            last = done;
            let detail = format_bytes_progress(done, total);
            eprintln!("pe-agent: deploy {detail}");
            let _ = std::io::stderr().flush();
            let ratio = if total > 0 {
                (done as f64 / total as f64).clamp(0.0, 0.99)
            } else {
                0.2
            };
            let _ = report_target(
                &progress_cfg,
                &progress_target,
                "running",
                ratio,
                "",
                &detail,
                0,
                done as i64,
            )
            .await;
        }
    });

    let pump = tokio::spawn(async move {
        let mut stream = blob_resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(b) => {
                    if tx.send(Ok(b)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });

    let disk_plan = task.disk_plan.clone();
    let disk_for_write = disk.clone();
    let written_w = Arc::clone(&written);
    let deploy_handle = tokio::task::spawn_blocking(move || -> anyhow::Result<ImageHeader> {
        struct ChanReader {
            rx: std::sync::mpsc::Receiver<std::result::Result<bytes::Bytes, String>>,
            buf: bytes::Bytes,
            pos: usize,
            written: Arc<AtomicU64>,
        }
        impl Read for ChanReader {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.pos >= self.buf.len() {
                    match self.rx.recv() {
                        Ok(Ok(b)) => {
                            self.buf = b;
                            self.pos = 0;
                        }
                        Ok(Err(msg)) => {
                            return Err(std::io::Error::other(msg));
                        }
                        Err(_) => return Ok(0),
                    }
                }
                if self.buf.is_empty() {
                    return Ok(0);
                }
                let n = std::cmp::min(out.len(), self.buf.len() - self.pos);
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                self.written.fetch_add(n as u64, Ordering::Relaxed);
                Ok(n)
            }
        }
        let mut reader = ChanReader {
            rx,
            buf: bytes::Bytes::new(),
            pos: 0,
            written: written_w,
        };
        let mut last_report = 0u64;
        let mut on_prog = |done: u64, _tot: u64| {
            // payload progress; also keep atomic roughly in sync for UI
            if done >= last_report + 64 * 1024 * 1024 {
                last_report = done;
            }
        };
        deploy_reader_to_disk(
            &mut reader,
            &disk_for_write,
            &disk_plan,
            Some(&mut on_prog),
        )
    });

    let deploy_result = deploy_handle
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let _ = pump.await;
    stop_progress.store(true, Ordering::Relaxed);
    let _ = progress_task.await;

    let header = match deploy_result {
        Ok(h) => h,
        Err(err) => {
            let msg = format!("stream deploy failed: {err}");
            fail_deploy(&cfg, task, &disk, &msg).await?;
            return Err(Error::Other(anyhow::anyhow!(msg)));
        }
    };
    if !expected.is_empty()
        && header.payload_sha256.to_ascii_lowercase() != expected
    {
        eprintln!(
            "pe-agent: checksum note meta={} header_payload={}",
            expected, header.payload_sha256
        );
    }

    let marker = failed_marker(&disk);
    if marker.exists() {
        let _ = tokio::fs::remove_file(&marker).await;
    }

    let done_bytes = written.load(Ordering::Relaxed).max(total);
    let detail = format_bytes_progress(done_bytes, total.max(done_bytes));
    report_target(
        &cfg,
        &task.target_id,
        "done",
        1.0,
        "",
        &detail,
        0,
        done_bytes as i64,
    )
    .await?;
    println!(
        "deploy_ok mac={} image_id={} disk={} encoding={} progress={}",
        task.mac,
        image_id,
        disk.display(),
        header.encoding,
        detail
    );
    Ok(())
}

async fn download_blob_with_progress(
    client: &reqwest::Client,
    cfg: &PeAgentConfig,
    task: &ClaimedTask,
    edge: &str,
    image_id: &str,
) -> Result<(Vec<u8>, i64, i64)> {
    let meta_url = format!("{edge}/api/v1/pe/images/{image_id}");
    let meta_resp = client
        .get(&meta_url)
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !meta_resp.status().is_success() {
        let status = meta_resp.status();
        let body = meta_resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "image meta failed: {status} {body}"
        )));
    }
    let meta: serde_json::Value = meta_resp
        .json()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    let expected = meta["checksum_sha256"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let total = meta["size_bytes"].as_u64().unwrap_or(0);
    let blob_url = format!("{edge}/api/v1/pe/images/{image_id}/blob");
    let blob_resp = client
        .get(&blob_url)
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !blob_resp.status().is_success() {
        let status = blob_resp.status();
        let body = blob_resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "image blob failed: {status} {body}"
        )));
    }
    let mut stream = blob_resp.bytes_stream();
    let mut bytes = Vec::new();
    if total > 0 {
        bytes.reserve(total as usize);
    }
    let mut last_bytes = 0u64;
    let mut last_at = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
        bytes.extend_from_slice(&chunk);
        let done = bytes.len() as u64;
        let due_bytes = done.saturating_sub(last_bytes) >= 256 * 1024 * 1024;
        let due_time = last_at.elapsed() >= std::time::Duration::from_secs(5);
        if due_bytes || due_time {
            last_bytes = done;
            last_at = std::time::Instant::now();
            let detail = format_bytes_progress(done, total);
            let ratio = if total > 0 {
                (done as f64 / total as f64).clamp(0.0, 0.99)
            } else {
                0.2
            };
            eprintln!("pe-agent: download {detail}");
            let _ = std::io::stderr().flush();
            let _ = report_target(
                cfg,
                &task.target_id,
                "running",
                ratio,
                "",
                &detail,
                0,
                done as i64,
            )
            .await;
        }
    }
    let got_file = hex_sha256(&bytes);
    let got = if got_file == expected {
        got_file
    } else {
        // Prefer payload SHA (matches Center capture ingest checksum).
        match payload_sha_from_image_bytes(&bytes) {
            Ok(p) if p == expected => p,
            Ok(p) => {
                return Err(Error::Other(anyhow::anyhow!(
                    "checksum mismatch: file={got_file} payload={p}, expected {expected}"
                )));
            }
            Err(e) => {
                return Err(Error::Other(anyhow::anyhow!(
                    "checksum mismatch: file={got_file}, expected {expected}; payload parse: {e}"
                )));
            }
        }
    };
    let _ = got;
    let n = bytes.len() as i64;
    let detail = format_bytes_progress(bytes.len() as u64, total.max(bytes.len() as u64));
    eprintln!("pe-agent: download {detail}");
    Ok((bytes, 0_i64, n))
}

async fn download_via_p2p(
    client: &reqwest::Client,
    cfg: &PeAgentConfig,
    task: &ClaimedTask,
    edge: &str,
    image_id: &str,
) -> Result<(Vec<u8>, i64, i64)> {
    let manifest_url = format!("{edge}/api/v1/p2p/manifest/{image_id}");
    let manifest: idv_common::chunking::ChunkManifest = client
        .get(&manifest_url)
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?
        .error_for_status()
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?
        .json()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;

    let mut out = vec![0u8; manifest.total_size as usize];
    let mut have = Vec::new();
    let mut p2p_bytes = 0i64;
    let mut edge_bytes = 0i64;
    let mut last_at = std::time::Instant::now();
    let mut last_done = 0u64;

    for chunk in &manifest.chunks {
        let url = format!("{edge}/api/v1/p2p/chunk/{image_id}/{}", chunk.index);
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|err| Error::Other(anyhow::anyhow!(err)))?
            .error_for_status()
            .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
        let source = resp
            .headers()
            .get("x-idv-chunk-source")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("edge")
            .to_string();
        let data = resp
            .bytes()
            .await
            .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
        if !idv_common::chunking::verify_chunk(chunk, &data) {
            return Err(Error::Other(anyhow::anyhow!(
                "chunk {} checksum failed",
                chunk.index
            )));
        }
        let start = chunk.offset as usize;
        let end = start + chunk.length as usize;
        out[start..end].copy_from_slice(&data);
        if source == "p2p" {
            p2p_bytes += data.len() as i64;
        } else {
            edge_bytes += data.len() as i64;
        }
        have.push(chunk.index);

        let done = (p2p_bytes + edge_bytes) as u64;
        let tot = manifest.total_size;
        if done.saturating_sub(last_done) >= 256 * 1024 * 1024
            || last_at.elapsed() >= std::time::Duration::from_secs(5)
        {
            last_done = done;
            last_at = std::time::Instant::now();
            let detail = format_bytes_progress(done, tot);
            let ratio = if tot > 0 {
                (done as f64 / tot as f64).clamp(0.0, 0.99)
            } else {
                0.2
            };
            eprintln!("pe-agent: download {detail}");
            let _ = std::io::stderr().flush();
            let _ = report_target(
                cfg,
                &task.target_id,
                "running",
                ratio,
                "",
                &detail,
                p2p_bytes,
                edge_bytes,
            )
            .await;
        }

        // Donate chunk back to swarm for subsequent peers.
        let put_url = format!("{edge}/api/v1/p2p/chunk/{image_id}/{}", chunk.index);
        let _ = client.put(&put_url).body(data.to_vec()).send().await;
    }

    let _ = client
        .post(format!("{edge}/api/v1/p2p/peers"))
        .json(&serde_json::json!({
            "mac": task.mac,
            "task_id": task.task_id,
            "image_id": image_id,
            "have": have,
        }))
        .send()
        .await;

    let got = hex_sha256(&out);
    if got != manifest.file_sha256 {
        return Err(Error::Other(anyhow::anyhow!(
            "reassembled file checksum mismatch"
        )));
    }
    Ok((out, p2p_bytes, edge_bytes))
}

async fn fail_deploy(
    cfg: &PeAgentConfig,
    task: &ClaimedTask,
    disk_file: &PathBuf,
    msg: &str,
) -> Result<()> {
    let marker = failed_marker(disk_file);
    let content = format!("deploy_failed\nreason={msg}\nimage={:?}\n", task.image_id);
    let _ = tokio::fs::write(&marker, content.as_bytes()).await;
    report_target(cfg, &task.target_id, "failed", 0.0, msg, "deploy-failed", 0, 0).await?;
    println!("deploy_failed marker={}", marker.display());
    Ok(())
}

fn failed_marker(disk_file: &PathBuf) -> PathBuf {
    let mut s = disk_file.as_os_str().to_os_string();
    s.push(".deploy-failed");
    PathBuf::from(s)
}

async fn report_target(
    cfg: &PeAgentConfig,
    target_id: &str,
    state: &str,
    progress: f64,
    error_message: &str,
    detail: &str,
    p2p_bytes: i64,
    edge_bytes: i64,
) -> Result<()> {
    let edge = cfg.edge_url.trim_end_matches('/');
    let url = format!("{edge}/api/v1/pe/task/targets/{target_id}/report");
    let client = http_client();
    let resp = client
        .post(url)
        .json(&TargetReport {
            state,
            progress,
            error_message: error_message.to_string(),
            detail: detail.to_string(),
            p2p_bytes,
            edge_bytes,
        })
        .send()
        .await
        .map_err(|err| Error::Other(anyhow::anyhow!(err)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "report failed: {status} {body}"
        )));
    }
    Ok(())
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("reqwest client")
}

fn payload_sha_from_image_bytes(bytes: &[u8]) -> anyhow::Result<String> {
    const MAGIC: &[u8] = b"YIXUE1\n";
    if bytes.len() < 11 || &bytes[..7] != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let header_len = u32::from_le_bytes(bytes[7..11].try_into()?) as usize;
    let start = 11usize.saturating_add(header_len);
    if start > bytes.len() {
        anyhow::bail!("truncated header");
    }
    Ok(hex_sha256(&bytes[start..]))
}

fn urlencoding_lite(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

