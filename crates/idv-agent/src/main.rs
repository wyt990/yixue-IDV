mod netinfo;
mod policy_store;
mod restore_driver;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set while a new exe is being downloaded/replaced. The heartbeat loop keeps
/// reporting but must not re-arm restore / reboot in the middle of an update.
static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// Set after `shutdown.exe /r|/s` succeeds; stop processing further commands.
static REBOOT_SCHEDULED: AtomicBool = AtomicBool::new(false);

/// Kept in the binary so Center can auto-detect the package version on upload.
const BUILD_VERSION_MARKER: &str = concat!("IDV_AGENT_BUILD_VERSION=", env!("CARGO_PKG_VERSION"));

use clap::{Parser, Subcommand};
use idv_common::endpoint_policy::DesiredEndpointState;
use idv_common::image_format::{ImageHeader, default_partitions, pack_image, unpack_image};
use idv_common::restore::{apply_partition_restore, should_restore_now};
use idv_common::usb_policy::apply_usb_policy;
use idv_common::volume::{
    BootHookInstall, LabVolumeSnapshot, VolumeBackend, VolumeLayout, apply_windows_lab_restore,
    write_boot_hook, write_lab_baseline,
};
use idv_common::{Error, Result, init_tracing, load_toml_file};
use policy_store::{LocalPolicyStore, default_state_dir};
use restore_driver::{apply_frozen_state, default_driver};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Parser)]
#[command(
    name = "idv-agent",
    about = "易学 IDV 客户端代理（还原/模式）",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Status {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
    },
    /// Heartbeat and print effective mode policy
    Heartbeat {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "boot")]
        trigger: String,
    },
    /// Apply partition restore (file disk or windows-lab volumes)
    Restore {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "file")]
        backend: String,
        #[arg(long)]
        disk_file: Option<PathBuf>,
        #[arg(long)]
        baseline_file: Option<PathBuf>,
        #[arg(long)]
        volume_root: Option<PathBuf>,
        #[arg(long)]
        baseline_root: Option<PathBuf>,
        /// Force manual trigger semantics
        #[arg(long, default_value_t = false)]
        manual: bool,
    },
    /// Heartbeat then restore if policy says so
    Boot {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "file")]
        backend: String,
        #[arg(long)]
        disk_file: Option<PathBuf>,
        #[arg(long)]
        baseline_file: Option<PathBuf>,
        #[arg(long)]
        volume_root: Option<PathBuf>,
        #[arg(long)]
        baseline_root: Option<PathBuf>,
    },
    /// Pack a disk file into yixue-image-v1 baseline (lab helper)
    PackBaseline {
        #[arg(long)]
        disk_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "windows")]
        os_family: String,
    },
    /// Prepare windows-lab baseline volume tree
    PackVolumeBaseline {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "SYS-BASE")]
        system_marker: String,
        #[arg(long, default_value = "PER-BASE")]
        persistent_marker: String,
        #[arg(long, default_value = "VOL-BASE")]
        volatile_marker: String,
    },
    /// Heartbeat then apply USB restriction policy locally and report
    EnforceUsb {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long)]
        state_file: PathBuf,
    },
    /// Claim pending software job: download + simulated apply + report
    SoftApply {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Ack pending assist session
    AssistAck {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "accepted")]
        state: String,
    },
    /// Install boot hook script + marker (Windows schtasks optional via deploy script)
    InstallBootHook {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long, default_value = "windows-lab")]
        backend: String,
        #[arg(long)]
        volume_root: PathBuf,
        #[arg(long)]
        baseline_root: PathBuf,
        #[arg(long, default_value = "")]
        agent_bin: String,
    },
    /// Volume-filter restore driver status / protect on|off (mock if no .sys)
    RestoreDriver {
        #[command(subcommand)]
        action: RestoreDriverCmd,
    },
    /// Heartbeat: queue DesiredState if newer; with --apply apply pending after "reboot"
    SyncPolicy {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Apply pending DesiredState now (boot hook should pass this)
        #[arg(long, default_value_t = false)]
        apply: bool,
        #[arg(long, default_value = "heartbeat")]
        trigger: String,
    },
    /// Resident daemon: heartbeat loop + long-poll command channel (console mode)
    Daemon {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
        #[arg(long, default_value_t = 5)]
        heartbeat_secs: u64,
    },
    /// Windows service entry (falls back to console daemon if not started by SCM)
    Service {
        #[arg(short, long, default_value = "deploy/config/agent.toml")]
        config: PathBuf,
    },
    /// Hidden: interactive remote-assist helper launched in the logged-in session.
    #[command(hide = true)]
    AssistHelper {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        center_url: String,
        #[arg(long)]
        mac: String,
        #[arg(long, default_value = "view")]
        kind: String,
        #[arg(long, default_value_t = 500)]
        frame_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum RestoreDriverCmd {
    Status,
    ProtectOn {
        #[arg(long, default_value = "system")]
        volumes: String,
    },
    ProtectOff,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentConfig {
    #[serde(default = "default_center")]
    center_url: String,
    /// Empty / "auto" = auto-detect the NIC MAC that routes to Center.
    #[serde(default = "default_mac")]
    mac: String,
    /// Empty = use the current Windows computer name.
    #[serde(default)]
    hostname: String,
    /// Heartbeat interval for daemon/service mode (seconds).
    #[serde(default = "default_heartbeat_secs")]
    heartbeat_secs: u64,
    #[serde(default = "default_log")]
    log: String,
    /// Restore backend: auto | uwf | state_file.
    #[serde(default = "default_restore_backend")]
    restore_backend: String,
}

fn default_restore_backend() -> String {
    "auto".into()
}

fn default_center() -> String {
    "http://127.0.0.1:8080".into()
}
fn default_mac() -> String {
    String::new()
}
fn default_log() -> String {
    "info,idv_agent=debug".into()
}
fn default_heartbeat_secs() -> u64 {
    5
}
fn default_ip_mode() -> String {
    "static".into()
}

#[derive(Debug, Deserialize)]
struct PolicyResponse {
    mac: String,
    should_restore: bool,
    message: String,
    mode: Option<ModePolicy>,
    #[allow(dead_code)]
    baseline_image_id: Option<String>,
    #[serde(default)]
    schedule_active: bool,
    #[serde(default)]
    usb_restricted: bool,
    #[serde(default)]
    pending_power: Option<serde_json::Value>,
    #[serde(default)]
    pending_software: Option<serde_json::Value>,
    #[serde(default)]
    pending_assist: Option<serde_json::Value>,
    #[serde(default)]
    desired_state: Option<DesiredEndpointState>,
    #[serde(default)]
    desired_identity: Option<DesiredIdentity>,
}

#[derive(Debug, Clone, Deserialize)]
struct DesiredIdentity {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    netmask: String,
    #[serde(default)]
    gateway: String,
    #[serde(default)]
    dns: String,
    /// dhcp = IP comes from Edge DHCP reservation; static = agent configures it.
    #[serde(default = "default_ip_mode")]
    ip_mode: String,
}

#[derive(Debug, Deserialize)]
struct ModePolicy {
    name: String,
    restore: String,
    system_action: String,
    persistent_action: String,
    volatile_action: String,
}

#[derive(Debug, Serialize)]
struct HeartbeatBody<'a> {
    mac: &'a str,
    hostname: &'a str,
    agent_version: &'a str,
    trigger: &'a str,
    applied_revision: u64,
    pending_reboot: bool,
    driver_status: &'a str,
    local_ip: &'a str,
    current_hostname: &'a str,
    identity_state: &'a str,
    last_error: &'a str,
    frozen: bool,
    armed: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Runtime reference keeps the build-version marker in the binary so Center
    // can auto-detect the package version on upload.
    if std::env::var_os("IDV_BUILD_INFO").is_some() {
        println!("{BUILD_VERSION_MARKER}");
    }
    match cli.command {
        Commands::Status { config } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            println!(
                "service=idv-agent status=ok center={} mac={}",
                cfg.center_url,
                effective_mac(&cfg).unwrap_or_else(|_| "auto".into())
            );
        }
        Commands::Heartbeat { config, trigger } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let store = LocalPolicyStore::load(&default_state_dir());
            let status = store.applied_status();
            let policy = heartbeat(&cfg, &trigger, &status).await?;
            println!(
                "mac={} should_restore={} schedule_active={} usb_restricted={} mode={} msg={}",
                policy.mac,
                policy.should_restore,
                policy.schedule_active,
                policy.usb_restricted,
                policy.mode.as_ref().map(|m| m.name.as_str()).unwrap_or("-"),
                policy.message
            );
            if let Some(ref d) = policy.desired_state {
                println!(
                    "desired_state revision={} frozen={} apply={:?}",
                    d.revision, d.restore.frozen, d.restore.apply
                );
            }
            if let Some(ref p) = policy.pending_power {
                println!("pending_power={}", p);
            }
            if let Some(ref p) = policy.pending_software {
                println!("pending_software={}", p);
            }
            if let Some(ref p) = policy.pending_assist {
                println!("pending_assist={}", p);
            }
        }
        Commands::Restore {
            config,
            backend,
            disk_file,
            baseline_file,
            volume_root,
            baseline_root,
            manual,
        } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let trigger = if manual { "manual" } else { "boot" };
            let st = LocalPolicyStore::load(&default_state_dir()).applied_status();
            let policy = heartbeat(&cfg, trigger, &st).await?;
            run_restore(
                &cfg,
                &policy,
                &backend,
                disk_file.as_ref(),
                baseline_file.as_ref(),
                volume_root.as_ref(),
                baseline_root.as_ref(),
                trigger,
            )
            .await?;
        }
        Commands::Boot {
            config,
            backend,
            disk_file,
            baseline_file,
            volume_root,
            baseline_root,
        } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            restore_driver::set_backend(&cfg.restore_backend);
            // Prefer freeze DesiredState apply on boot; keep legacy image restore as fallback.
            let state_dir = default_state_dir();
            let _ = sync_and_maybe_apply(&cfg, &state_dir, true, "boot").await?;
            let st = LocalPolicyStore::load(&state_dir).applied_status();
            let policy = heartbeat(&cfg, "boot", &st).await?;
            if policy.should_restore && policy.desired_state.is_none() {
                run_restore(
                    &cfg,
                    &policy,
                    &backend,
                    disk_file.as_ref(),
                    baseline_file.as_ref(),
                    volume_root.as_ref(),
                    baseline_root.as_ref(),
                    "boot",
                )
                .await?;
            } else {
                println!("skip_restore reason={}", policy.message);
            }
        }
        Commands::PackBaseline {
            disk_file,
            out,
            os_family,
        } => {
            let payload = tokio::fs::read(&disk_file).await?;
            let header = ImageHeader {
                format_version: 1,
                os_family,
                boot_mode: "uefi".into(),
                partitions: default_partitions(payload.len() as u64),
                payload_sha256: String::new(),
                payload_size: 0,
                encoding: "raw".into(),
                logical_size: 0,
            };
            let packed = pack_image(header, payload)?;
            tokio::fs::write(&out, &packed).await?;
            println!("pack_ok out={} bytes={}", out.display(), packed.len());
        }
        Commands::PackVolumeBaseline {
            out,
            system_marker,
            persistent_marker,
            volatile_marker,
        } => {
            write_lab_baseline(
                &out,
                &LabVolumeSnapshot {
                    format_version: 1,
                    system_marker,
                    persistent_marker,
                    volatile_marker,
                },
            )?;
            println!("volume_baseline_ok out={}", out.display());
        }
        Commands::EnforceUsb { config, state_file } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let st = LocalPolicyStore::load(&default_state_dir()).applied_status();
            let policy = heartbeat(&cfg, "heartbeat", &st).await?;
            let applied = apply_usb_policy(&state_file, policy.usb_restricted)
                .map_err(|e| anyhow::anyhow!(e))?;
            report_usb(&cfg, applied.restricted, &applied.detail).await?;
            println!(
                "usb_enforce_ok restricted={} file={}",
                applied.restricted,
                state_file.display()
            );
        }
        Commands::SoftApply { config, out_dir } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let st = LocalPolicyStore::load(&default_state_dir()).applied_status();
            let policy = heartbeat(&cfg, "heartbeat", &st).await?;
            let Some(job) = policy.pending_software else {
                println!("soft_apply_skip reason=no_pending_job");
                return Ok(());
            };
            let job_id = job
                .get("job_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("job_id missing"))?;
            let name = job
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("package");
            tokio::fs::create_dir_all(&out_dir).await?;
            let url = format!(
                "{}/api/v1/software/jobs/{}/blob?mac={}",
                cfg.center_url.trim_end_matches('/'),
                job_id,
                urlencoding_lite(&effective_mac(&cfg)?)
            );
            let client = http_client();
            let size_hint = job.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let resp = client.get(&url).send().await?.error_for_status()?;
            let total = resp.content_length().unwrap_or(size_hint);
            use futures_util::StreamExt;
            use idv_common::image_format::format_bytes_progress;
            let mut stream = resp.bytes_stream();
            let mut bytes = Vec::new();
            if total > 0 {
                bytes.reserve(total as usize);
            }
            let mut last_bytes = 0u64;
            let mut last_at = std::time::Instant::now();
            let report_url = format!(
                "{}/api/v1/software/jobs/{}/report",
                cfg.center_url.trim_end_matches('/'),
                job_id
            );
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                bytes.extend_from_slice(&chunk);
                let done = bytes.len() as u64;
                if done.saturating_sub(last_bytes) >= 8 * 1024 * 1024
                    || last_at.elapsed() >= std::time::Duration::from_secs(3)
                {
                    last_bytes = done;
                    last_at = std::time::Instant::now();
                    let detail = format_bytes_progress(done, total);
                    println!("soft_download {detail}");
                    let _ = client
                        .post(&report_url)
                        .json(&serde_json::json!({
                            "state": "running",
                            "detail": detail,
                        }))
                        .send()
                        .await;
                }
            }
            let out = out_dir.join(format!("{name}.bin"));
            tokio::fs::write(&out, &bytes).await?;
            let detail = format!(
                "{} 路 applied_simulated path={}",
                format_bytes_progress(bytes.len() as u64, total.max(bytes.len() as u64)),
                out.display()
            );
            client
                .post(&report_url)
                .json(&serde_json::json!({
                    "state": "done",
                    "detail": detail,
                }))
                .send()
                .await?
                .error_for_status()?;
            println!("soft_apply_ok job={} path={}", job_id, out.display());
        }
        Commands::AssistAck { config, state } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let st = LocalPolicyStore::load(&default_state_dir()).applied_status();
            let policy = heartbeat(&cfg, "heartbeat", &st).await?;
            let Some(sess) = policy.pending_assist else {
                println!("assist_ack_skip reason=no_pending");
                return Ok(());
            };
            let sid = sess
                .get("session_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("session_id missing"))?;
            let url = format!(
                "{}/api/v1/assist/sessions/{}/ack",
                cfg.center_url.trim_end_matches('/'),
                sid
            );
            http_client()
                .post(url)
                .json(&serde_json::json!({
                    "state": state,
                    "detail": "agent_ack",
                }))
                .send()
                .await?
                .error_for_status()?;
            println!("assist_ack_ok session={} state={}", sid, state);
        }
        Commands::InstallBootHook {
            config,
            state_dir,
            backend,
            volume_root,
            baseline_root,
            agent_bin,
        } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let _ = VolumeBackend::parse(&backend)?;
            let bin = if agent_bin.trim().is_empty() {
                std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "idv-agent".into())
            } else {
                agent_bin
            };
            let install = BootHookInstall {
                installed_at: chrono::Utc::now().to_rfc3339(),
                agent_bin: bin,
                config_path: config.display().to_string(),
                volume_backend: backend,
                volume_root: volume_root.display().to_string(),
                baseline_root: baseline_root.display().to_string(),
                hook_script: String::new(),
                trigger: "AtStartup".into(),
            };
            let (script, marker) = write_boot_hook(&state_dir, &install)?;
            println!(
                "boot_hook_ok script={} marker={}",
                script.display(),
                marker.display()
            );
        }
        Commands::RestoreDriver { action } => {
            init_tracing("info,idv_agent=debug");
            let drv = default_driver();
            match action {
                RestoreDriverCmd::Status => {
                    let st = drv.status()?;
                    println!(
                        "restore_driver backend={:?} available={} frozen={} armed={} detail={}",
                        st.backend, st.available, st.frozen, st.armed, st.detail
                    );
                }
                RestoreDriverCmd::ProtectOn { volumes } => {
                    let vols: Vec<String> = volumes
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let st = drv.protect_on(&vols)?;
                    println!("protect_on_ok frozen={} detail={}", st.frozen, st.detail);
                }
                RestoreDriverCmd::ProtectOff => {
                    let st = drv.protect_off()?;
                    println!("protect_off_ok frozen={} detail={}", st.frozen, st.detail);
                }
            }
        }
        Commands::SyncPolicy {
            config,
            state_dir,
            apply,
            trigger,
        } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let dir = state_dir.unwrap_or_else(default_state_dir);
            let summary = sync_and_maybe_apply(&cfg, &dir, apply, &trigger).await?;
            println!("{summary}");
        }
        Commands::Daemon {
            config,
            heartbeat_secs,
        } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let stop_ctrlc = stop.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    stop_ctrlc.store(true, Ordering::SeqCst);
                }
            });
            run_daemon(cfg, heartbeat_secs.max(2), stop).await?;
        }
        Commands::Service { config } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            #[cfg(windows)]
            {
                winsvc::run(config.clone(), cfg.heartbeat_secs.max(2)).await?;
            }
            #[cfg(not(windows))]
            {
                let stop = std::sync::Arc::new(AtomicBool::new(false));
                run_daemon(cfg, cfg.heartbeat_secs.max(2), stop).await?;
            }
        }
        Commands::AssistHelper {
            session_id,
            center_url,
            mac,
            kind,
            frame_ms,
        } => {
            #[cfg(windows)]
            run_assist_helper(&session_id, &center_url, &mac, &kind, frame_ms).await?;
            #[cfg(not(windows))]
            {
                let _ = (session_id, center_url, mac, kind, frame_ms);
                eprintln!("assist-helper is only supported on Windows");
            }
        }
    }
    Ok(())
}

async fn sync_and_maybe_apply(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    apply: bool,
    trigger: &str,
) -> Result<String> {
    consume_restore_now_marker(state_dir);
    let mut store = LocalPolicyStore::load(state_dir);
    let status = store.applied_status();
    let policy = heartbeat(cfg, trigger, &status).await?;
    let mut lines = Vec::new();
    lines.push(format!("msg={}", policy.message));
    if UPDATE_IN_PROGRESS.load(Ordering::SeqCst) {
        lines.push("update_in_progress=skip_rearm".into());
        return Ok(lines.join(" · "));
    }
    if let Some(desired) = policy.desired_state.clone() {
        if store.queue_if_newer(desired.clone()) {
            store.save(state_dir).map_err(|e| Error::Other(e))?;
            lines.push(format!(
                "queued_revision={} frozen={} pending_reboot=true",
                desired.revision, desired.restore.frozen
            ));
        } else {
            lines.push(format!(
                "no_queue revision={} applied={}",
                desired.revision, store.applied_revision
            ));
        }
    } else {
        lines.push("desired_state=none".into());
    }

    let power_kind = policy
        .pending_power
        .as_ref()
        .and_then(|p| p.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let power_id = policy
        .pending_power
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut need_rename_reboot = false;
    if let Some(ref ident) = policy.desired_identity {
        let driver = default_driver();
        let drv_status = driver
            .status()
            .map_err(|e| Error::Other(anyhow::anyhow!("driver status: {e}")))?;
        let want_host = ident.hostname.trim().to_string();
        let want_ip = ident.ip.trim().to_string();
        let ip_mode = if ident.ip_mode.trim().is_empty() {
            "static".to_string()
        } else {
            ident.ip_mode.trim().to_ascii_lowercase()
        };
        let have_host = current_computer_name();
        let have_ip = netinfo::detect_local_ip(&cfg.center_url);
        let want_frozen = policy
            .desired_state
            .as_ref()
            .map(|d| d.restore.enabled && d.restore.frozen)
            .unwrap_or(false);
        let volumes = policy
            .desired_state
            .as_ref()
            .map(|d| d.restore.protect_volumes.clone())
            .unwrap_or_else(|| vec!["system".into()]);

        let action = decide_identity_action(&IdentityDecisionInput {
            want_host: want_host.clone(),
            have_host: have_host.clone(),
            want_ip: want_ip.clone(),
            have_ip: have_ip.clone(),
            ip_mode: ip_mode.clone(),
            want_frozen,
            frozen: drv_status.frozen,
            armed: drv_status.armed,
        });
        match execute_identity_action(
            action,
            ident,
            &ip_mode,
            driver.as_ref(),
            want_frozen,
            &volumes,
            &cfg.center_url,
        ) {
            Ok((state, reboot)) => {
                lines.push(format!("identity={state}"));
                write_identity_report(&state, "");
                need_rename_reboot = reboot;
            }
            Err(err) => {
                let msg = format!("identity_err={err}");
                lines.push(msg);
                write_identity_report("error", &err.to_string());
            }
        }
    }

    // Identity convergence may need a reboot (rename / arm / disarm). Do it
    // before pending-store application to avoid fighting the protection state.
    if need_rename_reboot {
        // Identity arm/disarm/rename already applied. Clear the queued
        // DesiredState so the next boot does not apply it again and reboot a
        // second time.
        let applied_rev = policy
            .desired_state
            .as_ref()
            .map(|d| d.revision)
            .or_else(|| store.pending.as_ref().map(|p| p.revision));
        if let Some(revision) = applied_rev {
            store.clear_pending_applied(revision, "identity_reboot".into());
            store.save(state_dir)?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        issue_windows_power("reboot")?;
        lines.push("identity_reboot=1".into());
        return Ok(lines.join(" · "));
    }

    // Reboot from Center: apply pending freeze first, then reboot.
    let should_apply = apply || power_kind == "reboot";
    let mut pending_reboot = false;
    if should_apply {
        let (msg, reboot) = apply_pending_store(&mut store, state_dir)?;
        lines.push(msg);
        pending_reboot = reboot;
    }

    // A pending restore-scope change (e.g. teaching now protects D/E too) only
    // takes effect at next boot. Reboot automatically unless a power command is
    // already about to reboot/shutdown the machine.
    if pending_reboot && power_kind.is_empty() {
        write_identity_report("apply_pending_reboot", "");
        lines.push("pending_apply_reboot=1".into());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        issue_windows_power("reboot")?;
        return Ok(lines.join(" · "));
    }

    if !power_id.is_empty() && (power_kind == "reboot" || power_kind == "shutdown") {
        let _ = ack_power(cfg, &power_id, true, &format!("agent_{power_kind}_issued")).await;
        lines.push(format!("power_{power_kind}_id={power_id}"));
        // Give ack a moment to flush before process dies with the OS.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        issue_windows_power(&power_kind)?;
    }

    Ok(lines.join(" · "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityAction {
    None,
    /// Hostname mismatch while volume protection is on: disarm, reboot.
    DisarmReboot,
    /// Hostname mismatch while unprotected: set IP (static), rename, reboot.
    RenameReboot,
    /// Static IP mismatch only: configure the address, no reboot.
    SetStaticIp,
    /// Identity already correct and policy wants freeze: arm, reboot.
    ArmReboot,
    /// DHCP mode: release + renew to pick up Center's reservation.
    RenewDhcp,
}

#[derive(Debug, Clone)]
struct IdentityDecisionInput {
    want_host: String,
    have_host: String,
    want_ip: String,
    have_ip: String,
    ip_mode: String,
    want_frozen: bool,
    frozen: bool,
    armed: bool,
}

/// Pure decision logic (unit tested). At most one action per call.
fn decide_identity_action(i: &IdentityDecisionInput) -> IdentityAction {
    let host_mismatch =
        !i.want_host.is_empty() && !i.have_host.eq_ignore_ascii_case(i.want_host.trim());
    let ip_mismatch =
        i.ip_mode == "static" && !i.want_ip.is_empty() && i.have_ip.trim() != i.want_ip.trim();

    if host_mismatch {
        return if i.frozen || i.armed {
            IdentityAction::DisarmReboot
        } else {
            IdentityAction::RenameReboot
        };
    }
    // Mode switch off: always disarm once and reboot once.
    if !i.want_frozen && (i.frozen || i.armed) {
        return IdentityAction::DisarmReboot;
    }
    // Mode switch on: arm once and reboot once; ArmReboot also sets static IP.
    if i.want_frozen && !(i.frozen || i.armed) {
        return IdentityAction::ArmReboot;
    }
    if ip_mismatch {
        return IdentityAction::SetStaticIp;
    }
    if i.ip_mode == "dhcp" && !i.want_ip.is_empty() && i.have_ip.trim() != i.want_ip.trim() {
        return IdentityAction::RenewDhcp;
    }
    IdentityAction::None
}

/// Execute at most one convergence action. Returns `(state, reboot_required)`.
#[allow(clippy::too_many_arguments)]
fn execute_identity_action(
    action: IdentityAction,
    ident: &DesiredIdentity,
    ip_mode: &str,
    driver: &dyn restore_driver::RestoreDriver,
    want_frozen: bool,
    volumes: &[String],
    center_url: &str,
) -> Result<(String, bool)> {
    match action {
        IdentityAction::None => Ok(("ok".into(), false)),
        IdentityAction::DisarmReboot => {
            apply_frozen_state(driver, false, volumes)?;
            Ok(("disarm_reboot".into(), true))
        }
        IdentityAction::RenameReboot => {
            if ip_mode == "static" {
                let iface = netinfo::interface_for_center(center_url);
                set_static_ip_with_interface(ident, iface.as_ref())?;
            }
            rename_windows_computer(ident.hostname.trim())?;
            Ok(("rename_reboot".into(), true))
        }
        IdentityAction::SetStaticIp => {
            let iface = netinfo::interface_for_center(center_url);
            set_static_ip_with_interface(ident, iface.as_ref())?;
            Ok(("static_ip_set".into(), false))
        }
        IdentityAction::ArmReboot => {
            if ip_mode == "static" {
                let iface = netinfo::interface_for_center(center_url);
                let _ = set_static_ip_with_interface(ident, iface.as_ref());
            }
            apply_frozen_state(driver, want_frozen, volumes)?;
            Ok(("arm_reboot".into(), true))
        }
        IdentityAction::RenewDhcp => {
            let iface = netinfo::interface_for_center(center_url)
                .ok_or_else(|| anyhow::anyhow!("cannot find interface that routes to center"))?;
            if should_renew_dhcp(&iface.ip, ident.ip.trim()) {
                netinfo::renew_dhcp(&iface.alias)?;
                Ok(("dhcp_renew".into(), false))
            } else {
                Ok(("dhcp_wait".into(), false))
            }
        }
    }
}

fn set_static_ip_with_interface(
    ident: &DesiredIdentity,
    iface: Option<&netinfo::NetInterface>,
) -> Result<()> {
    let want = ident.ip.trim();
    if want.is_empty() {
        return Ok(());
    }
    let iface = iface.ok_or_else(|| {
        Error::Other(anyhow::anyhow!(
            "cannot find interface that routes to center (required for static IP)"
        ))
    })?;
    if iface.ip.trim() == want {
        return Ok(());
    }
    set_windows_static_ip(
        &iface.alias,
        want,
        ident.netmask.trim(),
        ident.gateway.trim(),
        ident.dns.trim(),
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IdentityReport {
    #[serde(default)]
    state: String,
    #[serde(default)]
    last_error: String,
    #[serde(default)]
    last_renew_ip: String,
    #[serde(default)]
    last_renew_at: u64,
}

fn identity_report_path() -> PathBuf {
    default_state_dir().join("identity-state.json")
}

fn load_identity_report() -> IdentityReport {
    std::fs::read(identity_report_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<IdentityReport>(&b).ok())
        .unwrap_or_default()
}

fn save_identity_report(report: &IdentityReport) {
    let path = identity_report_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(report) {
        let _ = std::fs::write(path, bytes);
    }
}

fn write_identity_report(state: &str, error: &str) {
    let mut report = load_identity_report();
    report.state = state.to_string();
    report.last_error = error.to_string();
    save_identity_report(&report);
}

/// Avoid DHCP release/renew storms: at most once per 5 minutes for the same IP.
fn should_renew_dhcp(have_ip: &str, want_ip: &str) -> bool {
    if want_ip.is_empty() || have_ip.trim() == want_ip.trim() {
        return false;
    }
    let mut report = load_identity_report();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if report.last_renew_ip == have_ip.trim() && now.saturating_sub(report.last_renew_at) < 300 {
        return false;
    }
    report.last_renew_ip = have_ip.trim().to_string();
    report.last_renew_at = now;
    save_identity_report(&report);
    true
}

fn current_computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

fn set_windows_static_ip(
    alias: &str,
    ip: &str,
    netmask: &str,
    gateway: &str,
    dns: &str,
) -> Result<()> {
    #[cfg(windows)]
    {
        let mask = if netmask.is_empty() {
            "255.255.255.0"
        } else {
            netmask
        };
        let dns_arg = if dns.is_empty() { gateway } else { dns };
        let alias = alias.trim();
        if alias.is_empty() {
            return Err(Error::Other(anyhow::anyhow!("empty adapter alias")));
        }
        let mut args = vec![
            "interface".into(),
            "ip".into(),
            "set".into(),
            "address".into(),
            format!("name={alias}"),
            "static".into(),
            ip.to_string(),
            mask.to_string(),
        ];
        if !gateway.is_empty() {
            args.push(gateway.to_string());
        }
        let st = std::process::Command::new("netsh")
            .args(&args)
            .status()
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        if !st.success() {
            return Err(Error::Other(anyhow::anyhow!("netsh set address failed")));
        }
        if !dns_arg.is_empty() {
            let st = std::process::Command::new("netsh")
                .args([
                    "interface",
                    "ip",
                    "set",
                    "dns",
                    &format!("name={alias}"),
                    "static",
                    dns_arg,
                ])
                .status()
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            if !st.success() {
                return Err(Error::Other(anyhow::anyhow!("netsh set dns failed")));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (alias, ip, netmask, gateway, dns);
        Err(Error::Other(anyhow::anyhow!(
            "static IP only supported on Windows"
        )))
    }
}

fn rename_windows_computer(name: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let safe: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(15)
            .collect();
        if safe.is_empty() {
            return Err(Error::Other(anyhow::anyhow!("invalid hostname")));
        }
        let st = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Rename-Computer -NewName '{safe}' -Force"),
            ])
            .status()
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        if !st.success() {
            return Err(Error::Other(anyhow::anyhow!(
                "Rename-Computer failed: {st}"
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        Err(Error::Other(anyhow::anyhow!(
            "rename only supported on Windows"
        )))
    }
}

fn apply_pending_store(
    store: &mut LocalPolicyStore,
    state_dir: &std::path::Path,
) -> Result<(String, bool)> {
    if let Some(pending) = store.pending.clone() {
        if pending.restore.enabled {
            let drv = default_driver();
            let st = apply_frozen_state(
                drv.as_ref(),
                pending.restore.frozen,
                &pending.restore.protect_volumes,
            )
            .map_err(|e| Error::Other(e))?;
            store.clear_pending_applied(pending.revision, st.detail.clone());
            store.save(state_dir).map_err(|e| Error::Other(e))?;
            Ok((
                format!(
                    "applied_revision={} frozen={} driver={}",
                    pending.revision, st.frozen, st.detail
                ),
                // Arming/disarming is already driven by the identity action,
                // which performs the single reboot. Do not reboot again here.
                false,
            ))
        } else {
            store.clear_pending_applied(pending.revision, "restore_disabled".into());
            store.save(state_dir).map_err(|e| Error::Other(e))?;
            // No filter state change and no reboot is required for an already
            // disarmed volume.
            Ok(("applied restore_disabled".into(), false))
        }
    } else {
        Ok(("apply_skip reason=no_pending".into(), false))
    }
}

const UPDATER_SCRIPT: &str = r#"
param(
  [string]$Job = "",
  [string]$NewExe,
  [string]$TargetExe,
  [string]$Service,
  [string]$Task,
  [string]$Config,
  [string]$Log,
  [string]$UpdaterTask = ""
)
if ($Job) {
  $j = Get-Content -LiteralPath $Job -Raw | ConvertFrom-Json
  $NewExe = $j.new_exe
  $TargetExe = $j.target_exe
  $Service = $j.service
  $Task = $j.task
  $Config = $j.config
  $Log = $j.log
  $UpdaterTask = $j.updater_task
}
function Write-Log([string]$m) {
  Add-Content -LiteralPath $Log -Value ("{0} {1}" -f (Get-Date -Format o), $m)
}
Start-Sleep -Seconds 2
$svc = Get-Service -Name $Service -ErrorAction SilentlyContinue
if ($svc) { Write-Log "stopping service"; Stop-Service -Name $Service -Force -ErrorAction SilentlyContinue }
$tsk = Get-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue
if ($tsk) { Write-Log "stopping task"; Stop-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue }
for ($i = 0; $i -lt 40; $i++) {
  if (-not (Get-Process -Name "idv-agent" -ErrorAction SilentlyContinue)) { break }
  Start-Sleep -Milliseconds 500
}
$copied = $false
$backup = $TargetExe + ".bak"
if (Test-Path -LiteralPath $TargetExe) {
  Copy-Item -LiteralPath $TargetExe -Destination $backup -Force
  Write-Log "backup saved"
}
for ($i = 0; $i -lt 20; $i++) {
  try {
    Copy-Item -LiteralPath $NewExe -Destination $TargetExe -Force -ErrorAction Stop
    $copied = $true
    break
  } catch {
    Write-Log ("copy retry: " + $_.Exception.Message)
    Start-Sleep -Milliseconds 500
  }
}
if (-not $copied) { Write-Log "copy failed"; exit 1 }
Write-Log "copied"
if ($svc) {
  for ($i = 0; $i -lt 10; $i++) {
    try { Start-Service -Name $Service -ErrorAction Stop; break } catch { Start-Sleep -Seconds 1 }
  }
  Write-Log "service start requested"
} elseif ($tsk) {
  Start-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue
  Write-Log "task start requested"
} else {
  Start-Process -FilePath $TargetExe -ArgumentList @("daemon", "--config", $Config) -WindowStyle Hidden
  Write-Log "process start requested"
}
Start-Sleep -Seconds 3
$running = $false
if ($svc) { $running = ((Get-Service -Name $Service -ErrorAction SilentlyContinue).Status -eq "Running") }
elseif ($tsk) { $running = ((Get-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue).State -eq "Running") }
else { $running = [bool](Get-Process -Name "idv-agent" -ErrorAction SilentlyContinue) }
Write-Log ("new agent running=" + $running)
if (-not $running -and (Test-Path -LiteralPath $backup)) {
  Copy-Item -LiteralPath $backup -Destination $TargetExe -Force -ErrorAction SilentlyContinue
  if ($svc) { Start-Service -Name $Service -ErrorAction SilentlyContinue }
  elseif ($tsk) { Start-ScheduledTask -TaskName $Task -ErrorAction SilentlyContinue }
  Write-Log "rollback to backup"
}
Remove-Item -LiteralPath $NewExe -Force -ErrorAction SilentlyContinue
if ($UpdaterTask) {
  cmd /c "schtasks /Delete /TN `"$UpdaterTask`" /F >nul 2>&1" | Out-Null
}
Write-Log "done"
"#;

fn default_client_install_dir() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(|p| PathBuf::from(p).join("YixueIDV"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\YixueIDV"))
}

struct UpdateFlagGuard {
    keep: bool,
}

impl Drop for UpdateFlagGuard {
    fn drop(&mut self) {
        if !self.keep {
            UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
    }
}

/// Download + schedule a detached PowerShell updater.
///
/// While restore is active, writing a new exe would be redirected/lost, so the
/// first phase disarms restore and reboots. The Center retries the update after
/// the client heartbeats again (now unfrozen).
async fn perform_agent_update(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    payload: &str,
) -> Result<(bool, String)> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| Error::Other(anyhow::anyhow!("invalid update payload: {e}")))?;
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let download_url = value
        .get("download_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let want_sha = value
        .get("sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if version.is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "update payload missing version"
        )));
    }
    let current = env!("CARGO_PKG_VERSION");
    if version == current {
        return Ok((true, format!("already_version={version}")));
    }

    let driver = default_driver();
    let st = driver
        .status()
        .map_err(|e| Error::Other(anyhow::anyhow!("driver status: {e}")))?;
    if st.frozen || st.armed {
        driver
            .protect_off()
            .map_err(|e| Error::Other(anyhow::anyhow!("disarm before update: {e}")))?;
        write_identity_report("update_disarm_reboot", "");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        issue_windows_power("reboot")?;
        return Ok((
            false,
            format!("reboot_for_update current={current} target={version}"),
        ));
    }

    #[cfg(not(windows))]
    {
        let _ = (cfg, state_dir, download_url, want_sha);
        Ok((false, "self-update is only supported on Windows".into()))
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        // Keep the heartbeat loop from re-arming restore while we replace the exe.
        UPDATE_IN_PROGRESS.store(true, Ordering::SeqCst);
        let mut update_guard = UpdateFlagGuard { keep: false };

        if download_url.is_empty() {
            return Err(Error::Other(anyhow::anyhow!(
                "update payload missing download_url"
            )));
        }
        let url = if download_url.starts_with("http://") || download_url.starts_with("https://") {
            download_url
        } else {
            format!(
                "{}{}",
                cfg.center_url.trim_end_matches('/'),
                if download_url.starts_with('/') {
                    download_url
                } else {
                    format!("/{download_url}")
                }
            )
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("download {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Other(anyhow::anyhow!(
                "download {url}: HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        if bytes.len() < 64 * 1024 {
            return Err(Error::Other(anyhow::anyhow!(
                "downloaded exe too small: {} bytes",
                bytes.len()
            )));
        }
        if !want_sha.is_empty() {
            use sha2::{Digest, Sha256};
            let got = hex::encode(Sha256::digest(&bytes));
            if got != want_sha {
                return Err(Error::Other(anyhow::anyhow!(
                    "sha256 mismatch: got {got} want {want_sha}"
                )));
            }
        }

        let update_dir = state_dir.join("update");
        tokio::fs::create_dir_all(&update_dir)
            .await
            .map_err(Error::Io)?;
        let new_exe = update_dir.join(format!("idv-agent-{version}.exe"));
        tokio::fs::write(&new_exe, &bytes)
            .await
            .map_err(Error::Io)?;
        let script = update_dir.join("apply-update.ps1");
        tokio::fs::write(&script, UPDATER_SCRIPT)
            .await
            .map_err(Error::Io)?;
        let current_exe = std::env::current_exe().map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        let log = update_dir.join("update.log");
        let config = default_client_install_dir().join("agent.toml");
        let updater_task = "YixueIdvAgentUpdate";
        let job = update_dir.join("update-job.json");
        let job_json = serde_json::json!({
            "new_exe": new_exe.to_string_lossy(),
            "target_exe": current_exe.to_string_lossy(),
            "service": "idv-agent",
            "task": "YixueIdvAgentDaemon",
            "config": config.to_string_lossy(),
            "log": log.to_string_lossy(),
            "updater_task": updater_task,
        });
        tokio::fs::write(&job, job_json.to_string())
            .await
            .map_err(Error::Io)?;

        // Launch through Task Scheduler (SYSTEM). A detached PowerShell child of
        // the service can be killed when the service stops (job object), which
        // leaves the old exe in place and the task forever "running".
        let _ = std::process::Command::new("schtasks")
            .args(["/Delete", "/TN", updater_task, "/F"])
            .output();
        let tr = format!(
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"{}\" -Job \"{}\"",
            script.display(),
            job.display()
        );
        let created = std::process::Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                updater_task,
                "/SC",
                "ONCE",
                "/ST",
                "00:00",
                "/RU",
                "SYSTEM",
                "/RL",
                "HIGHEST",
                "/TR",
                &tr,
                "/F",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let mut scheduled = false;
        if created {
            scheduled = std::process::Command::new("schtasks")
                .args(["/Run", "/TN", updater_task])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        }
        if !scheduled {
            // Fallback for environments where Task Scheduler is unavailable.
            let mut cmd = std::process::Command::new("powershell.exe");
            cmd.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&script)
                .arg("-Job")
                .arg(&job);
            cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
            cmd.spawn()
                .map_err(|e| Error::Other(anyhow::anyhow!("spawn updater: {e}")))?;
        }
        update_guard.keep = true;
        Ok((
            true,
            format!(
                "update_scheduled version={version} bytes={} trigger={} log={}",
                bytes.len(),
                if scheduled {
                    "task_scheduler"
                } else {
                    "detached"
                },
                log.display()
            ),
        ))
    }
}

fn issue_windows_power(kind: &str) -> Result<()> {
    #[cfg(windows)]
    {
        // Flush every fixed volume before the reboot. Warm reboots were
        // occasionally landing in the boot manager before the disk returned
        // ready; an explicit flush removes pending writeback in that window.
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:", letter as char);
            let root = format!("{drive}\\");
            if std::path::Path::new(&root).exists() {
                let _ = std::process::Command::new("fsutil")
                    .args(["volume", "flush", &drive])
                    .status();
            }
        }
        let arg = if kind == "reboot" { "/r" } else { "/s" };
        let status = std::process::Command::new("shutdown.exe")
            .args([arg, "/t", "5", "/f", "/c", "YixueIDV power command"])
            .status()
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        if !status.success() {
            return Err(Error::Other(anyhow::anyhow!(
                "shutdown.exe failed: {status}"
            )));
        }
        REBOOT_SCHEDULED.store(true, Ordering::SeqCst);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        Err(Error::Other(anyhow::anyhow!(
            "power command only supported on Windows"
        )))
    }
}

async fn ack_power(cfg: &AgentConfig, id: &str, ok: bool, detail: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/power/commands/{id}/ack",
        cfg.center_url.trim_end_matches('/')
    );
    let client = http_client();
    let resp = client
        .post(url)
        .json(&serde_json::json!({ "ok": ok, "detail": detail }))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "power ack failed: {status} {body}"
        )));
    }
    Ok(())
}

async fn load_cfg(path: &PathBuf) -> Result<AgentConfig> {
    if path.exists() {
        load_toml_file(path)
    } else {
        Ok(AgentConfig {
            center_url: default_center(),
            mac: default_mac(),
            hostname: String::new(),
            heartbeat_secs: default_heartbeat_secs(),
            log: default_log(),
            restore_backend: default_restore_backend(),
        })
    }
}

/// Explicit `mac` wins; empty/"auto" resolves the NIC that routes to Center.
fn effective_mac(cfg: &AgentConfig) -> Result<String> {
    let configured = cfg.mac.trim();
    if !configured.is_empty() && !configured.eq_ignore_ascii_case("auto") {
        let mac = netinfo::normalize_mac(configured);
        if mac.is_empty() {
            return Err(Error::Other(anyhow::anyhow!(
                "invalid mac in agent.toml: '{configured}'"
            )));
        }
        return Ok(mac);
    }
    let iface = netinfo::interface_for_center(&cfg.center_url).ok_or_else(|| {
        Error::Other(anyhow::anyhow!(
            "cannot auto-detect MAC for center {}; set mac explicitly or check network/PowerShell",
            cfg.center_url
        ))
    })?;
    if iface.mac.is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "auto-detected interface has empty MAC"
        )));
    }
    Ok(iface.mac)
}

fn effective_hostname(cfg: &AgentConfig) -> String {
    let configured = cfg.hostname.trim();
    if configured.is_empty() {
        current_computer_name()
    } else {
        configured.to_string()
    }
}

async fn heartbeat(
    cfg: &AgentConfig,
    trigger: &str,
    status: &idv_common::endpoint_policy::EndpointAppliedStatus,
) -> Result<PolicyResponse> {
    let url = format!(
        "{}/api/v1/agent/heartbeat",
        cfg.center_url.trim_end_matches('/')
    );
    let mac = effective_mac(cfg)?;
    let hostname = effective_hostname(cfg);
    let current_hostname = current_computer_name();
    let local_ip = netinfo::detect_local_ip(&cfg.center_url);
    let report = load_identity_report();
    let driver = default_driver().status().ok();
    let frozen = driver.as_ref().map(|s| s.frozen).unwrap_or(false);
    let armed = driver.as_ref().map(|s| s.armed).unwrap_or(false);
    let client = http_client();
    let resp = client
        .post(url)
        .json(&HeartbeatBody {
            mac: &mac,
            hostname: &hostname,
            agent_version: env!("CARGO_PKG_VERSION"),
            trigger,
            applied_revision: status.applied_revision,
            pending_reboot: status.pending_reboot,
            driver_status: &status.driver_status,
            local_ip: &local_ip,
            current_hostname: &current_hostname,
            identity_state: &report.state,
            last_error: &report.last_error,
            frozen,
            armed,
        })
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "heartbeat failed: {status} {body}"
        )));
    }
    resp.json()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))
}

async fn run_restore(
    cfg: &AgentConfig,
    policy: &PolicyResponse,
    backend: &str,
    disk_file: Option<&PathBuf>,
    baseline_file: Option<&PathBuf>,
    volume_root: Option<&PathBuf>,
    baseline_root: Option<&PathBuf>,
    trigger: &str,
) -> Result<()> {
    let mode = policy
        .mode
        .as_ref()
        .ok_or_else(|| Error::Other(anyhow::anyhow!("no mode policy; bind lab mode first")))?;
    if !should_restore_now(&mode.restore, trigger) {
        println!("skip_restore mode={} restore={}", mode.name, mode.restore);
        report(cfg, true, "skipped by policy").await?;
        return Ok(());
    }

    let kind = VolumeBackend::parse(backend)?;
    let log = match kind {
        VolumeBackend::File => {
            let disk_file = disk_file.ok_or_else(|| {
                Error::Other(anyhow::anyhow!("file backend requires --disk-file"))
            })?;
            let baseline_file = baseline_file.ok_or_else(|| {
                Error::Other(anyhow::anyhow!("file backend requires --baseline-file"))
            })?;
            let packed_bytes = tokio::fs::read(baseline_file)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("read baseline: {e}")))?;
            let baseline = unpack_image(&packed_bytes).map_err(|e| Error::Other(e))?;
            let mut disk = tokio::fs::read(disk_file)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("read disk: {e}")))?;
            if disk.len() < baseline.payload.len() {
                disk.resize(baseline.payload.len(), 0);
            }
            let log = apply_partition_restore(
                &mut disk,
                &baseline,
                &mode.system_action,
                &mode.persistent_action,
                &mode.volatile_action,
            )
            .map_err(|e| Error::Other(e))?;
            let tmp = disk_file.with_extension("restoring");
            tokio::fs::write(&tmp, &disk)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            tokio::fs::rename(&tmp, disk_file)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            log
        }
        VolumeBackend::WindowsLab => {
            let volume_root = volume_root.ok_or_else(|| {
                Error::Other(anyhow::anyhow!(
                    "windows-lab backend requires --volume-root"
                ))
            })?;
            let baseline_root = baseline_root.ok_or_else(|| {
                Error::Other(anyhow::anyhow!(
                    "windows-lab backend requires --baseline-root"
                ))
            })?;
            let live = VolumeLayout::windows_lab(volume_root);
            let base = VolumeLayout::windows_lab(baseline_root);
            apply_windows_lab_restore(
                &live,
                &base,
                &mode.system_action,
                &mode.persistent_action,
                &mode.volatile_action,
            )?
        }
    };

    let detail = log.join("; ");
    report(cfg, true, &detail).await?;
    info!(%detail, "restore applied");
    println!(
        "restore_ok mode={} backend={} detail={}",
        mode.name, backend, detail
    );
    Ok(())
}

async fn report(cfg: &AgentConfig, ok: bool, detail: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/agent/restore/report",
        cfg.center_url.trim_end_matches('/')
    );
    let mac = effective_mac(cfg)?;
    let client = http_client();
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "mac": mac,
            "ok": ok,
            "detail": detail,
        }))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "restore report failed: {status} {body}"
        )));
    }
    Ok(())
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        // A dead Center / network black hole must not stall a heartbeat task
        // indefinitely. Per-request timeouts (long-poll/file transfer) still
        // override these defaults.
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

fn urlencoding_lite(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn report_usb(cfg: &AgentConfig, enforced: bool, detail: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/agent/usb/report",
        cfg.center_url.trim_end_matches('/')
    );
    let mac = effective_mac(cfg)?;
    let client = http_client();
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "mac": mac,
            "enforced": enforced,
            "detail": detail,
        }))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "usb report failed: {status} {body}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct AgentCommand {
    id: String,
    kind: String,
    #[serde(default)]
    payload: String,
}

#[derive(Debug, Deserialize)]
struct RunProgramPayload {
    path: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExecPayload {
    #[serde(default = "default_exec_shell")]
    shell: String,
    #[serde(default)]
    script: String,
    #[serde(default = "default_exec_timeout")]
    timeout_secs: u64,
}

fn default_exec_shell() -> String {
    "powershell".into()
}

fn default_exec_timeout() -> u64 {
    60
}

/// Long-poll Center for one command (returns None on the wait timeout).
async fn poll_command(cfg: &AgentConfig, mac: &str) -> Result<Option<AgentCommand>> {
    poll_command_wait(cfg, mac, 20).await
}

async fn poll_command_wait(
    cfg: &AgentConfig,
    mac: &str,
    wait_secs: u64,
) -> Result<Option<AgentCommand>> {
    let url = format!(
        "{}/api/v1/agent/commands/next?mac={}&wait={wait_secs}",
        cfg.center_url.trim_end_matches('/'),
        urlencoding_lite(mac)
    );
    let resp = http_client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(wait_secs + 10))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "command poll failed: {status} {body}"
        )));
    }
    resp.json::<Option<AgentCommand>>()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))
}

async fn report_command(cfg: &AgentConfig, id: &str, ok: bool, detail: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/agent/commands/{id}/report",
        cfg.center_url.trim_end_matches('/')
    );
    let resp = http_client()
        .post(&url)
        .json(&serde_json::json!({ "ok": ok, "detail": detail }))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(anyhow::anyhow!(
            "command report failed: {status} {body}"
        )));
    }
    Ok(())
}

const RESTORE_NOW_MARKER: &str = "restore-now-armed";
#[cfg(windows)]
fn current_boot_ms() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(not(windows))]
fn current_boot_ms() -> u64 {
    0
}

/// One-shot manual restore: after the reboot, the driver has already restored
/// the protected volumes; now disarm so later edits persist again.
fn consume_restore_now_marker(state_dir: &std::path::Path) {
    let path = state_dir.join(RESTORE_NOW_MARKER);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let (armed, restore) = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        (
            v.get("armed_ms").and_then(|x| x.as_u64()).unwrap_or(0),
            v.get("restore")
                .and_then(|x| x.as_str())
                .unwrap_or("manual")
                .to_string(),
        )
    } else {
        (text.trim().parse::<u64>().unwrap_or(0), "manual".into())
    };
    let now = current_boot_ms();
    // Same boot: do not change protection before the reboot has happened.
    if armed == 0 || now >= armed.saturating_sub(2_000) {
        return;
    }
    if restore != "manual" && restore != "off" {
        // Teaching/exam mode wants protection on; keep it frozen after restore.
        let _ = std::fs::remove_file(&path);
        return;
    }
    match default_driver().protect_off() {
        Ok(st) => {
            let _ = std::fs::remove_file(&path);
            eprintln!(
                "idv-agent: manual restore finished; protection disarmed frozen={} detail={}",
                st.frozen, st.detail
            );
        }
        Err(err) => eprintln!("idv-agent: manual restore disarm failed: {err}; will retry"),
    }
}

async fn execute_script_command(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    cmd: &AgentCommand,
) -> Result<()> {
    let payload: ExecPayload = serde_json::from_str(&cmd.payload)
        .map_err(|e| Error::Other(anyhow::anyhow!("invalid exec payload: {e}")))?;
    if payload.script.trim().is_empty() {
        report_command(cfg, &cmd.id, false, "exec: empty script").await?;
        return Ok(());
    }
    let shell = payload.shell.trim().to_ascii_lowercase();
    let timeout = payload.timeout_secs.clamp(1, 300);
    let exec_dir = state_dir.join("exec");
    if let Err(err) = std::fs::create_dir_all(&exec_dir) {
        report_command(cfg, &cmd.id, false, &format!("exec_create_dir: {err}")).await?;
        return Ok(());
    }
    let extension = if shell == "cmd" { "cmd" } else { "ps1" };
    let script_path = exec_dir.join(format!("{}.{}", cmd.id, extension));
    let content = if shell == "cmd" {
        // Force UTF-8 for the redirected output of native commands.
        format!("@chcp 65001 >nul\r\n{}\r\n", payload.script)
    } else {
        // Windows PowerShell 5.1 otherwise emits GBK bytes and the browser
        // displays them as mojibake. Set both the console and pipeline encoding.
        format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\r\n$OutputEncoding = [System.Text.Encoding]::UTF8\r\n{}\r\n",
            payload.script
        )
    };
    let mut bytes = Vec::new();
    if extension == "ps1" {
        // PowerShell 5.1 treats BOM-less UTF-8 as ANSI for non-ASCII literals.
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.append(&mut content.as_bytes().to_vec());
    if let Err(err) = std::fs::write(&script_path, &bytes) {
        report_command(cfg, &cmd.id, false, &format!("exec_write_script: {err}")).await?;
        return Ok(());
    }

    let path = script_path.clone();
    let run = tokio::task::spawn_blocking(move || run_script_blocking(&shell, &path, timeout)).await;
    let _ = std::fs::remove_file(&script_path);
    let (ok, detail) = match run {
        Ok(Ok((ok, detail))) => (ok, detail),
        Ok(Err(err)) => (false, format!("exec_failed: {err}")),
        Err(err) => (false, format!("exec_join_failed: {err}")),
    };
    report_command(cfg, &cmd.id, ok, &detail).await?;
    Ok(())
}

fn run_script_blocking(
    shell: &str,
    script_path: &std::path::Path,
    timeout_secs: u64,
) -> anyhow::Result<(bool, String)> {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let out_path = script_path.with_extension("hexout");
    let wrapper_path = script_path.with_extension(if shell == "cmd" {
        "wrapper.cmd"
    } else {
        "wrapper.ps1"
    });
    let ps_quote = |p: &std::path::Path| p.to_string_lossy().replace('\'', "''");
    let wrapper = if shell == "cmd" {
        format!(
            "@echo off\r\nchcp 65001 >nul\r\ncall \"{}\" > \"{}\" 2>&1\r\nexit /b %errorlevel%\r\n",
            ps_quote(script_path),
            ps_quote(&out_path)
        )
    } else {
        // Capture the script's object output as a string, UTF-8 encode it,
        // then write hex ASCII. This bypasses the redirected-console encoding
        // completely so Chinese text survives unchanged.
        format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\r\n$OutputEncoding = [System.Text.Encoding]::UTF8\r\n$__text = & '{}' *>&1 | Out-String\r\n$__hex = [BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($__text)).Replace('-','')\r\n[IO.File]::WriteAllText('{}', $__hex, [Text.Encoding]::ASCII)\r\n$__rc = $LASTEXITCODE\r\nif ($null -eq $__rc) {{ $__rc = 0 }}\r\nexit $__rc\r\n",
            ps_quote(script_path),
            ps_quote(&out_path)
        )
    };
    std::fs::write(&wrapper_path, wrapper.as_bytes())?;

    let mut command = if shell == "cmd" {
        let mut c = Command::new("cmd.exe");
        c.args(["/D", "/S", "/C"]).arg(&wrapper_path);
        c
    } else {
        let mut c = Command::new("powershell.exe");
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&wrapper_path);
        c
    };
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<(&'static str, String)>();
    if let Some(mut s) = stdout {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(("stdout", read_capped(&mut s, 64 * 1024)));
        });
    }
    if let Some(mut s) = stderr {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(("stderr", read_capped(&mut s, 64 * 1024)));
        });
    }
    drop(tx);

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if start.elapsed() >= Duration::from_secs(timeout_secs) {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout_text = String::new();
    let mut stderr_text = String::new();
    for _ in 0..2 {
        if let Ok((kind, text)) = rx.recv_timeout(Duration::from_secs(2)) {
            if kind == "stdout" {
                stdout_text = text;
            } else {
                stderr_text = text;
            }
        }
    }
    // PowerShell wrapper writes UTF-8 text as hex ASCII; cmd writes plain text.
    if let Ok(text) = std::fs::read_to_string(&out_path) {
        let text = text.trim_start_matches('\u{feff}').trim().to_string();
        if !text.is_empty() {
            if shell != "cmd" {
                if let Ok(bytes) = hex::decode(&text) {
                    stdout_text = String::from_utf8_lossy(&bytes).to_string();
                }
            } else {
                stdout_text = text;
            }
        }
    }
    let _ = std::fs::remove_file(&wrapper_path);
    let _ = std::fs::remove_file(&out_path);
    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    let ok = !timed_out && code == 0;
    let detail = serde_json::json!({
        "shell": shell,
        "exit_code": code,
        "timed_out": timed_out,
        "stdout": stdout_text,
        "stderr": stderr_text,
    })
    .to_string();
    Ok((ok, detail))
}

fn read_capped<R: std::io::Read>(reader: &mut R, cap: usize) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if out.len() < cap {
                    let take = n.min(cap - out.len());
                    out.extend_from_slice(&buf[..take]);
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn execute_restore_now(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    sync_lock: &tokio::sync::Mutex<()>,
    cmd: &AgentCommand,
) -> Result<()> {
    let _guard = sync_lock.lock().await;
    let status = LocalPolicyStore::load(state_dir).applied_status();
    let policy = heartbeat(cfg, "manual", &status).await?;
    if policy.mode.is_none() {
        report_command(cfg, &cmd.id, false, "restore_now: no mode policy").await?;
        return Ok(());
    }
    let volumes = match serde_json::from_str::<serde_json::Value>(&cmd.payload)
        .ok()
        .and_then(|v| {
            v.get("scope")
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_ascii_lowercase())
        })
        .as_deref()
    {
        Some("all_fixed") => vec!["all_fixed".to_string()],
        Some("system") => vec!["system".to_string()],
        _ => policy
            .desired_state
            .as_ref()
            .map(|d| d.restore.protect_volumes.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec!["system".into()]),
    };
    let driver = default_driver();
    let st = driver
        .protect_on(&volumes)
        .map_err(|e| Error::Other(anyhow::anyhow!("arm restore: {e}")))?;
    let marker = state_dir.join(RESTORE_NOW_MARKER);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    let restore_mode = policy
        .mode
        .as_ref()
        .map(|m| m.restore.clone())
        .unwrap_or_else(|| "manual".into());
    let marker_body = serde_json::json!({
        "armed_ms": current_boot_ms(),
        "restore": restore_mode,
    });
    std::fs::write(&marker, marker_body.to_string())
        .map_err(|e| Error::Other(anyhow::anyhow!("write restore marker: {e}")))?;
    let detail = format!(
        "restore_armed volumes={:?} frozen={} detail={}; rebooting",
        volumes, st.frozen, st.detail
    );
    report_command(cfg, &cmd.id, true, &detail).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    issue_windows_power("reboot")?;
    Ok(())
}

async fn execute_agent_command(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    sync_lock: &tokio::sync::Mutex<()>,
    cmd: &AgentCommand,
    mac: &str,
) -> Result<()> {
    let kind = cmd.kind.as_str();
    if kind == "exec" {
        return execute_script_command(cfg, state_dir, cmd).await;
    }
    if kind == "restore_now" {
        return execute_restore_now(cfg, state_dir, sync_lock, cmd).await;
    }
    if matches!(kind, "apply_policy" | "sync_policy") {
        let _guard = sync_lock.lock().await;
        let summary = sync_and_maybe_apply(cfg, state_dir, true, "command").await?;
        report_command(cfg, &cmd.id, true, &summary).await?;
        return Ok(());
    }

    if kind == "update_agent" {
        match perform_agent_update(cfg, state_dir, &cmd.payload).await {
            Ok((ok, detail)) => report_command(cfg, &cmd.id, ok, &detail).await?,
            Err(err) => {
                report_command(cfg, &cmd.id, false, &format!("update_failed: {err}")).await?
            }
        }
        return Ok(());
    }

    if kind == "assist_start" {
        let value: serde_json::Value = serde_json::from_str(&cmd.payload).unwrap_or_default();
        let session_id = value
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mode = value
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("view")
            .to_string();
        let frame_ms = value
            .get("frame_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(500);
        if session_id.is_empty() {
            report_command(cfg, &cmd.id, false, "assist_start missing session_id").await?;
            return Ok(());
        }
        stop_assist_helper();
        #[cfg(windows)]
        {
            match launch_assist_helper(&session_id, &cfg.center_url, mac, &mode, frame_ms) {
                Ok(pid) => {
                    if let Ok(mut guard) = ASSIST_HELPER_SESSION.lock() {
                        *guard = Some((session_id.clone(), pid));
                    }
                    report_command(cfg, &cmd.id, true, &format!("helper_pid={pid}")).await?;
                }
                Err(err) => {
                    assist_helper_ack(
                        &cfg.center_url,
                        &session_id,
                        "denied",
                        &format!("cannot start helper: {err}"),
                    )
                    .await;
                    report_command(cfg, &cmd.id, false, &format!("helper_start_failed: {err}"))
                        .await?;
                }
            }
        }
        #[cfg(not(windows))]
        {
            report_command(cfg, &cmd.id, false, "assist helper requires Windows").await?;
        }
        return Ok(());
    }

    if kind == "assist_stop" {
        stop_assist_helper();
        report_command(cfg, &cmd.id, true, "assist_stopped").await?;
        return Ok(());
    }

    if kind == "fs_collect" {
        let value: serde_json::Value = serde_json::from_str(&cmd.payload).unwrap_or_default();
        match handle_fs_collect(cfg, state_dir, mac, &value).await {
            Ok(detail) => report_command(cfg, &cmd.id, true, &detail).await?,
            Err(err) => {
                fs_task_result(cfg, &value, mac, false, &err.to_string()).await;
                report_command(cfg, &cmd.id, false, &format!("fs_collect_failed: {err}")).await?
            }
        }
        return Ok(());
    }

    if kind == "fs_distribute" {
        let value: serde_json::Value = serde_json::from_str(&cmd.payload).unwrap_or_default();
        match handle_fs_distribute(cfg, state_dir, mac, &value).await {
            Ok(detail) => report_command(cfg, &cmd.id, true, &detail).await?,
            Err(err) => {
                fs_task_result(cfg, &value, mac, false, &err.to_string()).await;
                report_command(cfg, &cmd.id, false, &format!("fs_distribute_failed: {err}")).await?
            }
        }
        return Ok(());
    }

    if matches!(kind, "reboot" | "shutdown") {
        let power_id = serde_json::from_str::<serde_json::Value>(&cmd.payload)
            .ok()
            .and_then(|v| {
                v.get("power_id")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if !power_id.is_empty() {
            let _ = ack_power(cfg, &power_id, true, "agent_command").await;
        }
        report_command(cfg, &cmd.id, true, &format!("{kind}_issued")).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        issue_windows_power(kind)?;
        return Ok(());
    }

    if kind == "open_url" {
        let url = serde_json::from_str::<serde_json::Value>(&cmd.payload)
            .ok()
            .and_then(|v| v.get("url").and_then(|x| x.as_str()).map(str::to_string))
            .unwrap_or_else(|| cmd.payload.trim().to_string());
        if url.is_empty() {
            return Err(Error::Other(anyhow::anyhow!("open_url missing url")));
        }
        let mut via = "user_session";
        #[cfg(windows)]
        {
            let args = vec![
                "/c".to_string(),
                "start".to_string(),
                String::new(),
                url.clone(),
            ];
            if let Err(err) =
                launch_program_as_user("C:\\Windows\\System32\\cmd.exe", &args)
            {
                eprintln!("idv-agent: launch open_url in user session failed: {err}");
                via = "service_session";
                std::process::Command::new("cmd")
                    .args(["/C", "start", "", &url])
                    .spawn()
                    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            }
        }
        report_command(cfg, &cmd.id, true, &format!("open_url={url} via={via}")).await?;
        return Ok(());
    }

    if kind == "run_program" {
        let payload: RunProgramPayload = serde_json::from_str(&cmd.payload)
            .map_err(|e| Error::Other(anyhow::anyhow!("invalid run_program payload: {e}")))?;
        let mut via = "user_session";
        #[cfg(windows)]
        {
            if let Err(err) = launch_program_as_user(&payload.path, &payload.args) {
                eprintln!("idv-agent: launch program in user session failed: {err}");
                via = "service_session";
                std::process::Command::new(&payload.path)
                    .args(&payload.args)
                    .spawn()
                    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            }
        }
        report_command(
            cfg,
            &cmd.id,
            true,
            &format!("run_program={} via={via}", payload.path),
        )
        .await?;
        return Ok(());
    }

    if kind == "assist_ack" {
        report_command(cfg, &cmd.id, true, "assist_signaled").await?;
        return Ok(());
    }

    report_command(cfg, &cmd.id, false, &format!("unsupported kind: {kind}")).await?;
    let _ = mac;
    Ok(())
}

#[cfg(windows)]
struct AssistFrame {
    screen_width: u32,
    screen_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    jpeg: Vec<u8>,
}

#[cfg(windows)]
#[derive(Default)]
struct AssistDirtyCapture {
    previous_thumb: Option<Vec<u8>>,
}

#[cfg(windows)]
impl AssistDirtyCapture {
    fn capture(&mut self, quality: u8, force_full: bool) -> Result<Option<AssistFrame>> {
        use windows_sys::Win32::Graphics::Gdi::{
            BITMAPINFO, BI_RGB, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };

        let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let screen = unsafe { GetDC(std::ptr::null_mut()) };
        if screen.is_null() {
            return Err(Error::Other(anyhow::anyhow!("GetDC failed")));
        }
        let mem = unsafe { CreateCompatibleDC(screen) };
        let bmp = unsafe { CreateCompatibleBitmap(screen, width, height) };
        if mem.is_null() || bmp.is_null() {
            unsafe {
                if !bmp.is_null() {
                    DeleteObject(bmp);
                }
                if !mem.is_null() {
                    DeleteDC(mem);
                }
                ReleaseDC(std::ptr::null_mut(), screen);
            }
            return Err(Error::Other(anyhow::anyhow!("GDI bitmap creation failed")));
        }
        let old = unsafe { SelectObject(mem, bmp) };
        let blt = unsafe { BitBlt(mem, 0, 0, width, height, screen, vx, vy, SRCCOPY) };
        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader.biSize = std::mem::size_of_val(&bmi.bmiHeader) as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let lines = if blt != 0 {
            unsafe {
                GetDIBits(
                    mem,
                    bmp,
                    0,
                    height as u32,
                    bgra.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut bmi,
                    DIB_RGB_COLORS,
                )
            }
        } else {
            0
        };
        unsafe {
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
        if lines == 0 {
            return Err(Error::Other(anyhow::anyhow!("screen capture failed")));
        }
        let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
        for px in bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let full = image::RgbImage::from_raw(width as u32, height as u32, rgb)
            .ok_or_else(|| Error::Other(anyhow::anyhow!("invalid RGB buffer")))?;
        let thumb = image::imageops::resize(
            &full,
            160,
            90,
            image::imageops::FilterType::Triangle,
        );
        let thumb_bytes = thumb.as_raw().clone();
        let mut changed = false;
        let mut min_x = 160usize;
        let mut min_y = 90usize;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        if let Some(prev) = &self.previous_thumb {
            for y in 0..90 {
                for x in 0..160 {
                    let i = (y * 160 + x) * 3;
                    let diff = prev[i].abs_diff(thumb_bytes[i])
                        + prev[i + 1].abs_diff(thumb_bytes[i + 1])
                        + prev[i + 2].abs_diff(thumb_bytes[i + 2]);
                    if diff > 24 {
                        changed = true;
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
            }
        }
        if !changed && !force_full {
            self.previous_thumb = Some(thumb_bytes);
            return Ok(None);
        }
        let full_frame = force_full || self.previous_thumb.is_none() || changed_ratio_x(min_x, max_x, min_y, max_y) > 0.30;
        let (x0, y0, w0, h0) = if full_frame {
            (0, 0, width as u32, height as u32)
        } else {
            let pad_x = 4usize;
            let pad_y = 4usize;
            let x0 = min_x.saturating_sub(pad_x);
            let y0 = min_y.saturating_sub(pad_y);
            let x1 = (max_x + pad_x + 1).min(160);
            let y1 = (max_y + pad_y + 1).min(90);
            let sx = width as usize;
            let sy = height as usize;
            (
                (x0 * sx / 160) as u32,
                (y0 * sy / 90) as u32,
                (((x1 - x0) * sx / 160) as u32).max(1),
                (((y1 - y0) * sy / 90) as u32).max(1),
            )
        };
        let w0 = w0.min(width as u32);
        let h0 = h0.min(height as u32);
        let crop = image::imageops::crop_imm(&full, x0, y0, w0, h0).to_image();
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        encoder
            .encode_image(&crop)
            .map_err(|e| Error::Other(anyhow::anyhow!("jpeg encode: {e}")))?;
        self.previous_thumb = Some(thumb_bytes);
        Ok(Some(AssistFrame {
            screen_width: width as u32,
            screen_height: height as u32,
            x: x0,
            y: y0,
            width: w0,
            height: h0,
            jpeg: out,
        }))
    }
}

#[cfg(windows)]
fn changed_ratio_x(min_x: usize, max_x: usize, min_y: usize, max_y: usize) -> f64 {
    let w = max_x.saturating_sub(min_x) + 1;
    let h = max_y.saturating_sub(min_y) + 1;
    (w * h) as f64 / (160.0 * 90.0)
}

#[cfg(windows)]
fn capture_virtual_rgb() -> Result<(u32, u32, image::RgbImage)> {
    use windows_sys::Win32::Graphics::Gdi::{
        BITMAPINFO, BI_RGB, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        return Err(Error::Other(anyhow::anyhow!("invalid screen size")));
    }
    let screen = unsafe { GetDC(std::ptr::null_mut()) };
    if screen.is_null() {
        return Err(Error::Other(anyhow::anyhow!("GetDC failed")));
    }
    let mem = unsafe { CreateCompatibleDC(screen) };
    let bmp = unsafe { CreateCompatibleBitmap(screen, width, height) };
    if mem.is_null() || bmp.is_null() {
        unsafe {
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            if !mem.is_null() {
                DeleteDC(mem);
            }
            ReleaseDC(std::ptr::null_mut(), screen);
        }
        return Err(Error::Other(anyhow::anyhow!("GDI bitmap creation failed")));
    }
    let old = unsafe { SelectObject(mem, bmp) };
    let blt = unsafe { BitBlt(mem, 0, 0, width, height, screen, vx, vy, SRCCOPY) };
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of_val(&bmi.bmiHeader) as u32;
    bmi.bmiHeader.biWidth = width;
    bmi.bmiHeader.biHeight = -height;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
    let lines = if blt != 0 {
        unsafe {
            GetDIBits(
                mem,
                bmp,
                0,
                height as u32,
                bgra.as_mut_ptr() as *mut core::ffi::c_void,
                &mut bmi,
                DIB_RGB_COLORS,
            )
        }
    } else {
        0
    };
    unsafe {
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
    if lines == 0 {
        return Err(Error::Other(anyhow::anyhow!("screen capture failed")));
    }
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for px in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let image = image::RgbImage::from_raw(width as u32, height as u32, rgb)
        .ok_or_else(|| Error::Other(anyhow::anyhow!("invalid RGB buffer")))?;
    Ok((width as u32, height as u32, image))
}

#[cfg(windows)]
struct H264Session {
    encoder: openh264::encoder::Encoder,
    width: u32,
    height: u32,
    timestamp_ms: u64,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    avcc_sent: bool,
}

#[cfg(windows)]
impl H264Session {
    fn new(width: u32, height: u32) -> Result<Self> {
        use openh264::encoder::{
            BitRate, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode, UsageType,
        };
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(2_000_000))
            .max_frame_rate(FrameRate::from_hz(15.0))
            .rate_control_mode(RateControlMode::Bitrate)
            .usage_type(UsageType::ScreenContentRealTime)
            .intra_frame_period(IntraFramePeriod::from_num_frames(30))
            .skip_frames(false)
            .num_threads(0);
        let encoder = openh264::encoder::Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            config,
        )
        .map_err(|e| Error::Other(anyhow::anyhow!("openh264: {e}")))?;
        Ok(Self {
            encoder,
            width,
            height,
            timestamp_ms: 0,
            sps: None,
            pps: None,
            avcc_sent: false,
        })
    }

    fn encode(
        &mut self,
        rgb: &image::RgbImage,
    ) -> Result<(Vec<u8>, bool, Option<Vec<u8>>, Option<String>)> {
        use openh264::encoder::FrameType;
        use openh264::formats::{RgbSliceU8, YUVBuffer};
        let resized = if rgb.width() != self.width || rgb.height() != self.height {
            image::imageops::resize(
                rgb,
                self.width,
                self.height,
                image::imageops::FilterType::Triangle,
            )
        } else {
            rgb.clone()
        };
        let source = RgbSliceU8::new(
            resized.as_raw(),
            (self.width as usize, self.height as usize),
        );
        let yuv = YUVBuffer::from_rgb8_source(source);
        let timestamp = openh264::Timestamp::from_millis(self.timestamp_ms);
        self.timestamp_ms = self.timestamp_ms.saturating_add(66);
        let encoded = self
            .encoder
            .encode_at(&yuv, timestamp)
            .map_err(|e| Error::Other(anyhow::anyhow!("openh264 encode: {e}")))?;
        let keyframe = matches!(encoded.frame_type(), FrameType::IDR);
        let mut out = Vec::new();
        for l in 0..encoded.num_layers() {
            let Some(layer) = encoded.layer(l) else {
                continue;
            };
            for i in 0..layer.nal_count() {
                if let Some(nal) = layer.nal_unit(i) {
                    let nal = strip_h264_start_code(nal);
                    if nal.is_empty() {
                        continue;
                    }
                    let nal_type = nal[0] & 0x1f;
                    if nal_type == 7 {
                        self.sps = Some(nal.to_vec());
                    } else if nal_type == 8 {
                        self.pps = Some(nal.to_vec());
                    }
                    // AVCC: 4-byte big-endian length followed by NAL bytes.
                    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    out.extend_from_slice(nal);
                }
            }
        }
        let mut avcc = None;
        let mut codec = None;
        if keyframe
            && let (Some(sps), Some(pps)) = (&self.sps, &self.pps)
        {
            avcc = Some(build_avcc(sps, pps));
            codec = Some(format!(
                "avc1.{:02X}{:02X}{:02X}",
                sps.get(1).copied().unwrap_or(0x42),
                sps.get(2).copied().unwrap_or(0xE0),
                sps.get(3).copied().unwrap_or(0x1E)
            ));
            self.avcc_sent = true;
        }
        Ok((out, keyframe, avcc, codec))
    }
}

#[cfg(windows)]
fn build_avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(1u8);
    out.push(sps.get(1).copied().unwrap_or(0x42));
    out.push(sps.get(2).copied().unwrap_or(0xE0));
    out.push(sps.get(3).copied().unwrap_or(0x1E));
    out.push(0xFFu8);
    out.push(0xE1u8);
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1u8);
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);
    out
}

#[cfg(windows)]
fn strip_h264_start_code(nal: &[u8]) -> &[u8] {
    if nal.starts_with(&[0, 0, 0, 1]) {
        &nal[4..]
    } else if nal.starts_with(&[0, 0, 1]) {
        &nal[3..]
    } else {
        nal
    }
}

#[cfg(windows)]
fn capture_screen_jpeg(max_width: u32, quality: u8) -> Result<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        BITMAPINFO, BI_RGB, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
    };

    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if width <= 0 || height <= 0 {
        return Err(Error::Other(anyhow::anyhow!(
            "invalid screen size {width}x{height}"
        )));
    }
    let screen = unsafe { GetDC(std::ptr::null_mut()) };
    if screen.is_null() {
        return Err(Error::Other(anyhow::anyhow!("GetDC failed")));
    }
    let mem = unsafe { CreateCompatibleDC(screen) };
    let bmp = unsafe { CreateCompatibleBitmap(screen, width, height) };
    if mem.is_null() || bmp.is_null() {
        unsafe {
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            if !mem.is_null() {
                DeleteDC(mem);
            }
            ReleaseDC(std::ptr::null_mut(), screen);
        }
        return Err(Error::Other(anyhow::anyhow!("GDI bitmap creation failed")));
    }
    let old = unsafe { SelectObject(mem, bmp) };
    let blt = unsafe { BitBlt(mem, 0, 0, width, height, screen, 0, 0, SRCCOPY) };
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of_val(&bmi.bmiHeader) as u32;
    bmi.bmiHeader.biWidth = width;
    bmi.bmiHeader.biHeight = -height;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
    let lines = if blt != 0 {
        unsafe {
            GetDIBits(
                mem,
                bmp,
                0,
                height as u32,
                bgra.as_mut_ptr() as *mut core::ffi::c_void,
                &mut bmi,
                DIB_RGB_COLORS,
            )
        }
    } else {
        0
    };
    unsafe {
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
    if lines == 0 {
        return Err(Error::Other(anyhow::anyhow!(
            "screen capture failed (BitBlt/GetDIBits)"
        )));
    }

    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for px in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let img = image::RgbImage::from_raw(width as u32, height as u32, rgb)
        .ok_or_else(|| Error::Other(anyhow::anyhow!("invalid RGB buffer")))?;
    let img = if max_width > 0 && img.width() > max_width {
        let nh = (img.height() as u64 * max_width as u64 / img.width() as u64) as u32;
        image::imageops::resize(
            &img,
            max_width,
            nh.max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode_image(&img)
        .map_err(|e| Error::Other(anyhow::anyhow!("jpeg encode: {e}")))?;
    Ok(out)
}

#[cfg(not(windows))]
fn capture_screen_jpeg(_max_width: u32, _quality: u8) -> Result<Vec<u8>> {
    Err(Error::Other(anyhow::anyhow!(
        "screen capture is only supported on Windows"
    )))
}

fn run_powershell_command(script: &str) -> Result<std::process::Output> {
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|e| Error::Other(anyhow::anyhow!("powershell: {e}")))
}

#[cfg(windows)]
fn get_clipboard_text() -> Result<String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Ok(String::new());
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            CloseClipboard();
            return Ok(String::new());
        }
        let ptr = GlobalLock(handle as _);
        if ptr.is_null() {
            CloseClipboard();
            return Ok(String::new());
        }
        let bytes = GlobalSize(handle as _) as usize;
        let units = std::slice::from_raw_parts(ptr as *const u16, bytes / 2);
        let len = units.iter().position(|c| *c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..len]);
        GlobalUnlock(handle as _);
        CloseClipboard();
        Ok(text)
    }
}

#[cfg(not(windows))]
fn get_clipboard_text() -> Result<String> {
    let out = run_powershell_command("[Console]::Out.Write((Get-Clipboard -Raw))")?;
    if !out.status.success() {
        return Err(Error::Other(anyhow::anyhow!(
            "Get-Clipboard failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn assist_user_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("IDV_ASSIST_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TEMP").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("YixueIDV")
        .join("assist")
}

#[cfg(windows)]
fn set_clipboard_text(text: &str) -> Result<()> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let bytes = utf16.len() * 2;
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(Error::Other(anyhow::anyhow!("OpenClipboard failed")));
        }
        EmptyClipboard();
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            CloseClipboard();
            return Err(Error::Other(anyhow::anyhow!("GlobalAlloc failed")));
        }
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            CloseClipboard();
            return Err(Error::Other(anyhow::anyhow!("GlobalLock failed")));
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, ptr as *mut u8, bytes);
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT as u32, handle) == std::ptr::null_mut() {
            CloseClipboard();
            return Err(Error::Other(anyhow::anyhow!("SetClipboardData failed")));
        }
        CloseClipboard();
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_clipboard_text(text: &str) -> Result<()> {
    let dir = assist_user_dir();
    std::fs::create_dir_all(&dir).map_err(Error::Io)?;
    let path = dir.join("clipboard.txt");
    std::fs::write(&path, text).map_err(Error::Io)?;
    let p = path.to_string_lossy().replace('\'', "''");
    let script = format!("Set-Clipboard -Value ([System.IO.File]::ReadAllText('{p}'))");
    let out = run_powershell_command(&script)?;
    if !out.status.success() {
        return Err(Error::Other(anyhow::anyhow!(
            "Set-Clipboard failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn key_to_vk(key: &str) -> Option<u8> {
    let lower = key.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "enter" | "return" => 0x0D,
        "backspace" => 0x08,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "space" | " " => 0x20,
        "arrowup" | "up" => 0x26,
        "arrowdown" | "down" => 0x28,
        "arrowleft" | "left" => 0x25,
        "arrowright" | "right" => 0x27,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "shift" => 0x10,
        "control" | "ctrl" => 0x11,
        "alt" => 0x12,
        "meta" | "win" => 0x5B,
        "capslock" => 0x14,
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if c.is_ascii_alphabetic() {
                c.to_ascii_uppercase() as u8
            } else if c.is_ascii_digit() {
                c as u8
            } else if c == ' ' {
                0x20
            } else {
                return None;
            }
        }
    };
    Some(mapped)
}

#[cfg(windows)]
fn assist_input(value: &serde_json::Value) -> Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
        keybd_event, mouse_event,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    let ty = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ty {
        "mouse_move" => {
            let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
            let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
            let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
            let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
            let px = vx + ((x.clamp(0.0, 1.0)) * w as f64) as i32;
            let py = vy + ((y.clamp(0.0, 1.0)) * h as f64) as i32;
            unsafe {
                SetCursorPos(px, py);
            }
        }
        "mouse_click" => {
            let button = value
                .get("button")
                .and_then(|v| v.as_str())
                .unwrap_or("left");
            let down = value.get("down").and_then(|v| v.as_bool()).unwrap_or(true);
            let flag = match (button, down) {
                ("right", true) => MOUSEEVENTF_RIGHTDOWN,
                ("right", false) => MOUSEEVENTF_RIGHTUP,
                ("middle", true) => MOUSEEVENTF_MIDDLEDOWN,
                ("middle", false) => MOUSEEVENTF_MIDDLEUP,
                (_, true) => MOUSEEVENTF_LEFTDOWN,
                (_, false) => MOUSEEVENTF_LEFTUP,
            };
            unsafe {
                mouse_event(flag, 0, 0, 0, 0);
            }
        }
        "mouse_wheel" => {
            let delta = value.get("delta").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            unsafe {
                mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta, 0);
            }
        }
        "key" => {
            let key = value.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let down = value.get("down").and_then(|v| v.as_bool()).unwrap_or(true);
            if let Some(vk) = key_to_vk(key) {
                let flags = if down { 0 } else { KEYEVENTF_KEYUP };
                unsafe {
                    keybd_event(vk, 0, flags, 0);
                }
            }
        }
        "text" => {
            let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if !text.is_empty() {
                set_clipboard_text(text)?;
                unsafe {
                    keybd_event(0x11, 0, 0, 0);
                    keybd_event(0x56, 0, 0, 0);
                    keybd_event(0x56, 0, KEYEVENTF_KEYUP, 0);
                    keybd_event(0x11, 0, KEYEVENTF_KEYUP, 0);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(windows)]
fn launch_as_user(cmd: &str) -> anyhow::Result<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        DuplicateTokenEx, SecurityImpersonation, TOKEN_ALL_ACCESS, TokenPrimary,
    };
    use windows_sys::Win32::System::Environment::{
        CreateEnvironmentBlock, DestroyEnvironmentBlock,
    };
    use windows_sys::Win32::System::RemoteDesktop::{
        WTSGetActiveConsoleSessionId, WTSQueryUserToken,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == u32::MAX {
        anyhow::bail!("no active console session");
    }
    let mut user_token: HANDLE = std::ptr::null_mut();
    if unsafe { WTSQueryUserToken(session, &mut user_token) } == 0 {
        anyhow::bail!(
            "WTSQueryUserToken failed: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut primary: HANDLE = std::ptr::null_mut();
    let dup_ok = unsafe {
        DuplicateTokenEx(
            user_token,
            TOKEN_ALL_ACCESS,
            std::ptr::null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
    };
    unsafe {
        CloseHandle(user_token);
    }
    if dup_ok == 0 {
        anyhow::bail!(
            "DuplicateTokenEx failed: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut env: *mut core::ffi::c_void = std::ptr::null_mut();
    let env_ok = unsafe { CreateEnvironmentBlock(&mut env, primary, 0) };
    let mut cmdline: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    // The helper is a console binary launched in the user session. Without
    // these flags Windows creates a visible console window on the teacher's
    // desktop; it also covers the screen being captured.
    si.dwFlags = 0x0000_0001; // STARTF_USESHOWWINDOW
    si.wShowWindow = 0; // SW_HIDE
    // Explicit desktop: otherwise a helper launched from Session 0 may inherit
    // the service desktop, making CopyFromScreen/clipboard/input fail.
    let desktop: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
    si.lpDesktop = desktop.as_ptr() as *mut u16;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessAsUserW(
            primary,
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            // CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT
            0x0800_0000 | CREATE_UNICODE_ENVIRONMENT,
            env,
            std::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if env_ok != 0 && !env.is_null() {
        unsafe {
            DestroyEnvironmentBlock(env);
        }
    }
    unsafe {
        CloseHandle(primary);
    }
    if ok == 0 {
        anyhow::bail!(
            "CreateProcessAsUserW failed: {}",
            std::io::Error::last_os_error()
        );
    }
    unsafe {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(pi.dwProcessId)
}

#[cfg(windows)]
fn quote_cmd_arg(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

#[cfg(windows)]
fn launch_program_as_user(exe: &str, args: &[String]) -> anyhow::Result<u32> {
    let mut cmd = quote_cmd_arg(exe);
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&quote_cmd_arg(arg));
    }
    launch_as_user(&cmd)
}

#[cfg(windows)]
fn launch_assist_helper(
    session_id: &str,
    center_url: &str,
    mac: &str,
    kind: &str,
    frame_ms: u64,
) -> anyhow::Result<u32> {
    let exe = std::env::current_exe()?;
    let cmd = format!(
        "{} assist-helper --session-id {} --center-url {} --mac {} --kind {} --frame-ms {}",
        quote_cmd_arg(&exe.to_string_lossy()),
        session_id,
        center_url,
        mac,
        kind,
        frame_ms
    );
    launch_as_user(&cmd)
}

#[cfg(windows)]
#[derive(Debug)]
enum AssistOut {
    Binary(Vec<u8>),
    Text(String),
}

static ASSIST_HELPER_SESSION: std::sync::Mutex<Option<(String, u32)>> = std::sync::Mutex::new(None);

fn stop_assist_helper() {
    if let Ok(mut guard) = ASSIST_HELPER_SESSION.lock()
        && let Some((_sid, pid)) = guard.take()
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .output();
    }
}

async fn assist_helper_ack(center: &str, session_id: &str, state: &str, detail: &str) {
    let url = format!(
        "{}/api/v1/assist/sessions/{session_id}/ack",
        center.trim_end_matches('/')
    );
    let _ = http_client()
        .post(&url)
        .json(&serde_json::json!({ "state": state, "detail": detail }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}

async fn assist_session_state(center: &str, session_id: &str) -> Option<String> {
    let url = format!(
        "{}/api/v1/assist/sessions/{session_id}/state",
        center.trim_end_matches('/')
    );
    let resp = http_client().get(&url).send().await.ok()?;
    let value: serde_json::Value = resp.json().await.ok()?;
    value
        .get("state")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

#[cfg(windows)]
async fn assist_ws_loop(
    center: &str,
    session_id: &str,
    mac: &str,
    stop: std::sync::Arc<AtomicBool>,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let base = center.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    let url = format!(
        "{ws_base}/api/v1/assist/sessions/{session_id}/ws?role=agent&mac={}",
        urlencoding_lite(mac)
    );
    let (stream, _) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("assist websocket connect timeout"))??;
    let (mut sink, mut incoming) = stream.split();

    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<AssistOut>(2);
    let quality = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(60));
    let frame_interval = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(100));
    let codec_mode = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let cap_codec = codec_mode.clone();
    let cap_stop = stop.clone();
    let cap_quality = quality.clone();
    let cap_interval = frame_interval.clone();
    tokio::spawn(async move {
        let mut seq = 0u32;
        let capture = std::sync::Arc::new(std::sync::Mutex::new(AssistDirtyCapture::default()));
        let mut h264: Option<H264Session> = None;
        let mut force_counter = 0u32;
        loop {
            if cap_stop.load(Ordering::SeqCst) {
                break;
            }
            force_counter = force_counter.wrapping_add(1);
            if cap_codec.load(Ordering::Relaxed) == 1 {
                let rgb = tokio::task::spawn_blocking(capture_virtual_rgb).await;
                if let Ok(Ok((_sw, _sh, rgb))) = rgb {
                    if h264.is_none() {
                        h264 = H264Session::new(1280, 720).ok();
                    }
                    if let Some(session) = h264.as_mut()
                        && let Ok((payload, keyframe, avcc, codec)) = session.encode(&rgb)
                    {
                        if let (Some(avcc), Some(codec)) = (avcc, codec) {
                            let config = serde_json::json!({
                                "type": "h264_config",
                                "codec": codec,
                                "width": session.width,
                                "height": session.height,
                                "description": hex::encode(avcc),
                            })
                            .to_string();
                            let _ = frame_tx.try_send(AssistOut::Text(config));
                        }
                        let mut packet = Vec::with_capacity(24 + payload.len());
                        packet.extend_from_slice(b"IDVF");
                        packet.extend_from_slice(&3u16.to_le_bytes());
                        packet.extend_from_slice(&(if keyframe { 1u16 } else { 0u16 }).to_le_bytes());
                        packet.extend_from_slice(&seq.to_le_bytes());
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        packet.extend_from_slice(&ts.to_le_bytes());
                        packet.extend_from_slice(&(session.width as u16).to_le_bytes());
                        packet.extend_from_slice(&(session.height as u16).to_le_bytes());
                        packet.extend_from_slice(&payload);
                        seq = seq.wrapping_add(1);
                        let _ = frame_tx.try_send(AssistOut::Binary(packet));
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(66)).await;
                continue;
            }
            let force = force_counter % 30 == 0;
            let q = cap_quality.load(Ordering::Relaxed).clamp(30, 80) as u8;
            let capture_task = capture.clone();
            let captured = tokio::task::spawn_blocking(move || capture_task.lock().unwrap().capture(q, force)).await;
            if let Ok(Ok(Some(frame))) = captured {
                let mut packet = Vec::with_capacity(32 + frame.jpeg.len());
                packet.extend_from_slice(b"IDVF");
                packet.extend_from_slice(&2u16.to_le_bytes());
                packet.extend_from_slice(&(if force { 1u16 } else { 0u16 }).to_le_bytes());
                packet.extend_from_slice(&seq.to_le_bytes());
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                packet.extend_from_slice(&ts.to_le_bytes());
                packet.extend_from_slice(&(frame.screen_width as u16).to_le_bytes());
                packet.extend_from_slice(&(frame.screen_height as u16).to_le_bytes());
                packet.extend_from_slice(&(frame.x as u16).to_le_bytes());
                packet.extend_from_slice(&(frame.y as u16).to_le_bytes());
                packet.extend_from_slice(&(frame.width as u16).to_le_bytes());
                packet.extend_from_slice(&(frame.height as u16).to_le_bytes());
                packet.extend_from_slice(&frame.jpeg);
                seq = seq.wrapping_add(1);
                let _ = frame_tx.try_send(AssistOut::Binary(packet));
            }
            let interval = cap_interval.load(Ordering::Relaxed).clamp(66, 250);
            tokio::time::sleep(std::time::Duration::from_millis(interval as u64)).await;
        }
    });

    let mut last_clip_seq = unsafe {
        windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
    };
    let mut last_clip = get_clipboard_text().unwrap_or_default();
    let mut clip_tick = tokio::time::interval(std::time::Duration::from_millis(300));
    let mut fs_tick = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut uploads: std::collections::HashMap<String, std::fs::File> = std::collections::HashMap::new();
    let fs_base = base.to_string();

    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                let msg = match frame {
                    AssistOut::Binary(bytes) => WsMessage::Binary(bytes.into()),
                    AssistOut::Text(text) => WsMessage::Text(text.into()),
                };
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
            _ = fs_tick.tick() => {
                // Keep the existing HTTP file browser working while video,
                // input and clipboard use the persistent WebSocket.
                let url = format!("{fs_base}/api/v1/assist/sessions/{session_id}/commands/next?wait=0");
                if let Ok(resp) = http_client().get(&url).timeout(std::time::Duration::from_secs(2)).send().await
                    && resp.status().is_success()
                    && let Ok(Some(cmd)) = resp.json::<Option<serde_json::Value>>().await
                {
                    let kind = cmd.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    if kind == "assist_stop" {
                        break;
                    }
                    if kind.starts_with("fs_") {
                        let payload = cmd.get("payload").and_then(|v| v.as_str()).unwrap_or("");
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                            if let Err(err) = handle_assist_fs_command(&fs_base, kind, &value).await
                                && let Some(rid) = value.get("request_id").and_then(|v| v.as_str())
                            {
                                let _ = http_client()
                                    .put(format!("{fs_base}/api/v1/assist/fs/{rid}"))
                                    .json(&serde_json::json!({ "ok": false, "error": err.to_string() }))
                                    .send()
                                    .await;
                            }
                        }
                    }
                }
            }
            _ = clip_tick.tick() => {
                let seq = unsafe {
                    windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
                };
                if seq != last_clip_seq {
                    last_clip_seq = seq;
                    if let Ok(Ok(text)) = tokio::task::spawn_blocking(get_clipboard_text).await
                        && text != last_clip
                    {
                        last_clip = text.clone();
                        let msg = serde_json::json!({"type":"clipboard","text":text}).to_string();
                        if sink.send(WsMessage::Text(msg.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            msg = incoming.next() => {
                let Some(Ok(msg)) = msg else { break };
                let text = match msg {
                    WsMessage::Text(t) => t.to_string(),
                    WsMessage::Binary(_) | WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Close(_) | WsMessage::Frame(_) => continue,
                };
                let value: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let ty = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ty {
                    "codec" => {
                        let codec = value.get("codec").and_then(|v| v.as_str()).unwrap_or("jpeg");
                        codec_mode.store(if codec == "h264" { 1 } else { 0 }, Ordering::Relaxed);
                    }
                    "ping" => {
                        let t = value.get("t").cloned().unwrap_or(serde_json::Value::Null);
                        let msg = serde_json::json!({"type":"pong","t":t}).to_string();
                        let _ = sink.send(WsMessage::Text(msg.into())).await;
                    }
                    "ack" => {
                        let rtt = value.get("rtt").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let q = quality.load(Ordering::Relaxed);
                        let interval = frame_interval.load(Ordering::Relaxed);
                        if rtt > 250 {
                            quality.store(q.saturating_sub(5).max(30), Ordering::Relaxed);
                            frame_interval.store((interval + 20).min(250), Ordering::Relaxed);
                        } else if rtt > 0 && rtt < 90 {
                            quality.store((q + 1).min(75), Ordering::Relaxed);
                            frame_interval
                                .store(interval.saturating_sub(2).max(66), Ordering::Relaxed);
                        }
                    }
                    "input" => {
                        if let Some(event) = value.get("event") {
                            let _ = assist_input(event);
                        }
                    }
                    "clipboard_set" => {
                        let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if let Ok(()) = set_clipboard_text(text) {
                            last_clip = text.to_string();
                            last_clip_seq = unsafe {
                                windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
                            };
                        }
                    }
                    "clipboard_get" => {
                        if let Ok(Ok(text)) = tokio::task::spawn_blocking(get_clipboard_text).await {
                            last_clip = text.clone();
                            let msg = serde_json::json!({"type":"clipboard","text":text}).to_string();
                            let _ = sink.send(WsMessage::Text(msg.into())).await;
                        }
                    }
                    "file_get" => {
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        if !id.is_empty() {
                            if let Ok(bytes) = std::fs::read(path) {
                                let name = std::path::Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or("file");
                                let begin = serde_json::json!({"type":"file_begin","id":id,"name":name,"size":bytes.len()}).to_string();
                                let _ = sink.send(WsMessage::Text(begin.into())).await;
                                for (i, chunk) in bytes.chunks(128 * 1024).enumerate() {
                                    let msg = serde_json::json!({
                                        "type":"file_chunk","id":id,"offset":i * 128 * 1024,"data":hex::encode(chunk)
                                    }).to_string();
                                    if sink.send(WsMessage::Text(msg.into())).await.is_err() { break; }
                                }
                                let end = serde_json::json!({"type":"file_end","id":id}).to_string();
                                let _ = sink.send(WsMessage::Text(end.into())).await;
                            } else {
                                let err = serde_json::json!({"type":"file_error","id":id,"message":"open failed"}).to_string();
                                let _ = sink.send(WsMessage::Text(err.into())).await;
                            }
                        }
                    }
                    "file_put_start" => {
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        if !id.is_empty() {
                            match std::fs::File::create(path) {
                                Ok(file) => {
                                    uploads.insert(id.clone(), file);
                                    let _ = sink.send(WsMessage::Text(serde_json::json!({"type":"file_ack","id":id}).to_string().into())).await;
                                }
                                Err(_) => {
                                    let _ = sink.send(WsMessage::Text(serde_json::json!({"type":"file_error","id":id,"message":"create failed"}).to_string().into())).await;
                                }
                            }
                        }
                    }
                    "file_chunk" => {
                        use std::io::Write;
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(data) = value.get("data").and_then(|v| v.as_str())
                            && let Ok(bytes) = hex::decode(data)
                            && let Some(file) = uploads.get_mut(id)
                        {
                            let _ = file.write_all(&bytes);
                        }
                    }
                    "file_end" => {
                        let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        uploads.remove(id);
                        let _ = sink.send(WsMessage::Text(serde_json::json!({"type":"file_ack","id":id}).to_string().into())).await;
                    }
                    _ => {}
                }
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
    Ok(())
}

#[cfg(not(windows))]
async fn assist_ws_loop(
    _center: &str,
    _session_id: &str,
    _mac: &str,
    _stop: std::sync::Arc<AtomicBool>,
) -> anyhow::Result<()> {
    anyhow::bail!("assist websocket is only supported on Windows")
}

#[cfg(windows)]
async fn assist_frame_loop(
    center: String,
    session_id: String,
    mac: String,
    frame_ms: u64,
    stop: std::sync::Arc<AtomicBool>,
) {
    let mut last_error_report = std::time::Instant::now() - std::time::Duration::from_secs(60);
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let started = std::time::Instant::now();
        match tokio::task::spawn_blocking(|| capture_screen_jpeg(1280, 60)).await {
            Ok(Ok(bytes)) if bytes.len() > 256 => {
                let url = format!(
                    "{}/api/v1/assist/sessions/{session_id}/frame?mac={}",
                    center.trim_end_matches('/'),
                    urlencoding_lite(&mac)
                );
                match http_client()
                    .put(&url)
                    .header("Content-Type", "image/jpeg")
                    .body(bytes)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) if !resp.status().is_success() => {
                        let msg = format!("frame_upload_error: {}", resp.status());
                        if last_error_report.elapsed() > std::time::Duration::from_secs(10) {
                            last_error_report = std::time::Instant::now();
                            assist_helper_ack(&center, &session_id, "active", &msg).await;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("idv-agent: assist frame upload: {err}"),
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                eprintln!("idv-agent: assist capture: {err}");
                if last_error_report.elapsed() > std::time::Duration::from_secs(10) {
                    last_error_report = std::time::Instant::now();
                    assist_helper_ack(
                        &center,
                        &session_id,
                        "active",
                        &format!("capture_error: {err}"),
                    )
                    .await;
                }
            }
            Err(err) => {
                eprintln!("idv-agent: assist capture task: {err}");
            }
        }
        let elapsed = started.elapsed();
        let wait = std::time::Duration::from_millis(frame_ms.max(200));
        if elapsed < wait {
            tokio::time::sleep(wait - elapsed).await;
        }
    }
}

fn fs_list_json(path: &str) -> Result<serde_json::Value> {
    if path.trim().is_empty() {
        let mut entries = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if std::path::Path::new(&root).exists() {
                entries.push(serde_json::json!({
                    "name": format!("{}:", letter as char),
                    "path": root,
                    "is_dir": true,
                    "size": 0,
                    "modified": 0,
                }));
            }
        }
        return Ok(serde_json::json!({ "ok": true, "entries": entries }));
    }
    let dir = std::path::Path::new(path);
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| Error::Other(anyhow::anyhow!(e)))? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        entries.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "path": entry.path().to_string_lossy(),
            "is_dir": is_dir,
            "size": if is_dir { 0 } else { meta.as_ref().map(|m| m.len()).unwrap_or(0) },
            "modified": meta.as_ref().and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()).unwrap_or(0),
        }));
    }
    entries.sort_by_key(|e| {
        (
            !e.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false),
            e.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase(),
        )
    });
    Ok(serde_json::json!({ "ok": true, "entries": entries }))
}

async fn assist_report(center: &str, command_id: &str, ok: bool, detail: &str) {
    let url = format!(
        "{}/api/v1/assist/commands/{command_id}/report",
        center.trim_end_matches('/')
    );
    let _ = http_client()
        .post(&url)
        .json(&serde_json::json!({ "ok": ok, "detail": detail }))
        .send()
        .await;
}

async fn handle_assist_fs_command(
    center: &str,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let request_id = payload
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if request_id.is_empty() {
        return Err(Error::Other(anyhow::anyhow!("missing request_id")));
    }
    let base = center.trim_end_matches('/');
    match kind {
        "fs_list" => {
            let path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("C:\\");
            let body = fs_list_json(path)?;
            let _ = http_client()
                .put(format!("{base}/api/v1/assist/fs/{request_id}"))
                .json(&body)
                .send()
                .await;
        }
        "fs_download" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let meta = std::fs::metadata(path)
                .map_err(|e| Error::Other(anyhow::anyhow!("{path}: {e}")))?;
            let bytes = if meta.is_dir() {
                let zip_path = std::env::temp_dir().join(format!("idv-assist-{request_id}.zip"));
                let zip_s = zip_path.to_string_lossy().to_string();
                let script = format!(
                    "Compress-Archive -LiteralPath {} -DestinationPath {} -Force",
                    ps_quote(path),
                    ps_quote(&zip_s)
                );
                let out = run_powershell_command(&script)
                    .map_err(|e| Error::Other(anyhow::anyhow!("compress: {e}")))?;
                if !out.status.success() {
                    return Err(Error::Other(anyhow::anyhow!(
                        "compress failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    )));
                }
                let bytes = tokio::fs::read(&zip_path)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
                let _ = tokio::fs::remove_file(&zip_path).await;
                bytes
            } else {
                tokio::fs::read(path)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?
            };
            let _ = http_client()
                .put(format!("{base}/api/v1/assist/fs/{request_id}/data"))
                .body(bytes)
                .send()
                .await;
        }
        "fs_move" => {
            let from = payload
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let to = payload
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if from.is_empty() || to.is_empty() {
                return Err(Error::Other(anyhow::anyhow!("invalid fs_move payload")));
            }
            let from_path = std::path::Path::new(&from);
            let to_path = std::path::Path::new(&to);
            if !from_path.exists() {
                return Err(Error::Other(anyhow::anyhow!("source not found: {from}")));
            }
            if to_path.exists() {
                return Err(Error::Other(anyhow::anyhow!("target already exists: {to}")));
            }
            let from_prefix = format!("{}\\", from.trim_end_matches('\\'));
            if to.starts_with(&from_prefix) {
                return Err(Error::Other(anyhow::anyhow!(
                    "cannot move a directory into itself"
                )));
            }
            if let Some(parent) = to_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            }
            tokio::task::spawn_blocking(move || move_path(&from, &to))
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("move task: {e}")))??;
            let _ = http_client()
                .put(format!("{base}/api/v1/assist/fs/{request_id}"))
                .json(&serde_json::json!({ "ok": true }))
                .send()
                .await;
        }
        "fs_upload" => {
            let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let rel = payload
                .get("download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path.is_empty() || rel.is_empty() {
                return Err(Error::Other(anyhow::anyhow!("invalid fs_upload payload")));
            }
            let url = format!(
                "{base}{}",
                if rel.starts_with('/') {
                    rel.to_string()
                } else {
                    format!("/{rel}")
                }
            );
            let resp = http_client()
                .get(&url)
                .send()
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            if !resp.status().is_success() {
                return Err(Error::Other(anyhow::anyhow!(
                    "download failed: {}",
                    resp.status()
                )));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            if let Some(parent) = std::path::Path::new(path).parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::write(path, &bytes)
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
            let _ = http_client()
                .put(format!("{base}/api/v1/assist/fs/{request_id}"))
                .json(&serde_json::json!({ "ok": true }))
                .send()
                .await;
        }
        _ => {}
    }
    Ok(())
}

async fn run_assist_helper(
    session_id: &str,
    center_url: &str,
    mac: &str,
    kind: &str,
    frame_ms: u64,
) -> Result<()> {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
    }
    let state_dir = assist_user_dir().join(session_id);
    tokio::fs::create_dir_all(&state_dir).await.map_err(|e| {
        Error::Other(anyhow::anyhow!(
            "create assist dir {}: {e}",
            state_dir.display()
        ))
    })?;
    assist_helper_ack(center_url, session_id, "accepted", "helper_started").await;
    assist_helper_ack(center_url, session_id, "active", "helper_active").await;
    #[cfg(windows)]
    {
        // Reconnect briefly after a Center restart instead of immediately
        // falling back to the old HTTP path (which shows a black viewer).
        for _attempt in 0..3 {
            let ws_stop = std::sync::Arc::new(AtomicBool::new(false));
            if assist_ws_loop(center_url, session_id, mac, ws_stop).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        eprintln!("idv-agent: assist websocket unavailable, falling back to HTTP helper");
    }
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let frame_stop = stop.clone();
    let frame_task = tokio::spawn(assist_frame_loop(
        center_url.to_string(),
        session_id.to_string(),
        mac.to_string(),
        frame_ms,
        frame_stop,
    ));
    let base = center_url.trim_end_matches('/');
    let mut last_session_check = std::time::Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let url = format!("{base}/api/v1/assist/sessions/{session_id}/commands/next?wait=10");
        let cmd = match http_client()
            .get(&url)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp
                .json::<Option<serde_json::Value>>()
                .await
                .ok()
                .flatten(),
            _ => None,
        };
        if let Some(cmd) = cmd {
            let command_id = cmd.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let cmd_kind = cmd.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let payload_text = cmd.get("payload").and_then(|v| v.as_str()).unwrap_or("");
            let payload: serde_json::Value =
                serde_json::from_str(payload_text).unwrap_or(serde_json::Value::Null);
            let result: Result<()> = match cmd_kind {
                "input" => {
                    if kind == "control" {
                        #[cfg(windows)]
                        {
                            assist_input(&payload)
                        }
                        #[cfg(not(windows))]
                        {
                            Ok(())
                        }
                    } else {
                        Err(Error::Other(anyhow::anyhow!(
                            "observe session is read-only"
                        )))
                    }
                }
                "clipboard_get" => match get_clipboard_text() {
                    Ok(text) => {
                        let _ = http_client()
                            .put(format!(
                                "{base}/api/v1/assist/sessions/{session_id}/clipboard"
                            ))
                            .body(text)
                            .send()
                            .await;
                        Ok(())
                    }
                    Err(err) => Err(err),
                },
                "clipboard_set" => {
                    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    match set_clipboard_text(text) {
                        Ok(()) => Ok(()),
                        Err(err) => Err(err),
                    }
                }
                "fs_list" | "fs_download" | "fs_upload" | "fs_move" => {
                    handle_assist_fs_command(center_url, cmd_kind, &payload).await
                }
                "assist_stop" => {
                    stop.store(true, Ordering::SeqCst);
                    Ok(())
                }
                _ => Ok(()),
            };
            match result {
                Ok(()) => assist_report(center_url, command_id, true, "ok").await,
                Err(err) => {
                    if cmd_kind.starts_with("fs_")
                        && let Some(rid) = payload.get("request_id").and_then(|v| v.as_str())
                    {
                        let _ = http_client()
                            .put(format!("{base}/api/v1/assist/fs/{rid}"))
                            .json(&serde_json::json!({ "ok": false, "error": err.to_string() }))
                            .send()
                            .await;
                    }
                    assist_report(center_url, command_id, false, &err.to_string()).await
                }
            }
        }
        if last_session_check.elapsed() > std::time::Duration::from_secs(5) {
            last_session_check = std::time::Instant::now();
            if let Some(state) = assist_session_state(center_url, session_id).await
                && (state == "ended" || state == "denied")
            {
                stop.store(true, Ordering::SeqCst);
                break;
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
    frame_task.abort();
    assist_helper_ack(center_url, session_id, "ended", "helper_stopped").await;
    Ok(())
}

fn ps_quote(path: &str) -> String {
    path.replace('\'', "''")
}

fn move_path(from: &str, to: &str) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let meta = std::fs::metadata(from).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if meta.is_dir() {
        copy_dir_all(std::path::Path::new(from), std::path::Path::new(to))?;
        std::fs::remove_dir_all(from).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    } else {
        std::fs::copy(from, to).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        std::fs::remove_file(from).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    Ok(())
}

fn copy_dir_all(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    for entry in std::fs::read_dir(from).map_err(|e| Error::Other(anyhow::anyhow!(e)))? {
        let entry = entry.map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
    }
    Ok(())
}

fn center_url_join(center: &str, path: &str) -> String {
    format!(
        "{}{}",
        center.trim_end_matches('/'),
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    )
}

async fn fs_task_result(
    cfg: &AgentConfig,
    payload: &serde_json::Value,
    mac: &str,
    ok: bool,
    detail: &str,
) {
    let task_id = payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if task_id.is_empty() {
        return;
    }
    let url = center_url_join(
        &cfg.center_url,
        &format!(
            "/api/v1/file-tasks/{task_id}/{}/result",
            mac.replace(':', "%3A")
        ),
    );
    let _ = http_client()
        .put(&url)
        .json(&serde_json::json!({ "ok": ok, "detail": detail }))
        .send()
        .await;
}

async fn handle_fs_collect(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    mac: &str,
    payload: &serde_json::Value,
) -> Result<String> {
    let task_id = payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_dir = payload
        .get("source_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let upload_url = payload
        .get("upload_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if task_id.is_empty() || source_dir.is_empty() || upload_url.is_empty() {
        return Err(Error::Other(anyhow::anyhow!("invalid fs_collect payload")));
    }
    if !std::path::Path::new(source_dir).is_dir() {
        return Err(Error::Other(anyhow::anyhow!(
            "source_dir not found: {source_dir}"
        )));
    }
    let work = state_dir.join("file-tasks").join(task_id);
    tokio::fs::create_dir_all(&work).await.map_err(Error::Io)?;
    let mac_file = mac.replace(':', "-");
    let zip = work.join(format!("{mac_file}.zip"));
    let zip_s = zip.to_string_lossy().to_string();
    let script = format!(
        "Add-Type -AssemblyName System.IO.Compression.FileSystem; \
         [System.IO.Compression.ZipFile]::CreateFromDirectory('{}','{}','Optimal',$true)",
        ps_quote(source_dir),
        ps_quote(&zip_s)
    );
    let out = tokio::task::spawn_blocking(move || run_powershell_command(&script))
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))??;
    if !out.status.success() {
        return Err(Error::Other(anyhow::anyhow!(
            "zip failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let bytes = tokio::fs::read(&zip).await.map_err(Error::Io)?;
    let url = center_url_join(&cfg.center_url, upload_url);
    let resp = http_client()
        .put(&url)
        .header("Content-Type", "application/zip")
        .body(bytes.clone())
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let _ = tokio::fs::remove_file(&zip).await;
    if !resp.status().is_success() {
        return Err(Error::Other(anyhow::anyhow!(
            "upload failed: {}",
            resp.status()
        )));
    }
    Ok(format!("collected {} bytes", bytes.len()))
}

async fn handle_fs_distribute(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    mac: &str,
    payload: &serde_json::Value,
) -> Result<String> {
    let task_id = payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let target_dir = payload
        .get("target_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let download_url = payload
        .get("download_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if task_id.is_empty() || target_dir.is_empty() || download_url.is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "invalid fs_distribute payload"
        )));
    }
    let work = state_dir.join("file-tasks").join(task_id);
    tokio::fs::create_dir_all(&work).await.map_err(Error::Io)?;
    let mac_file = mac.replace(':', "-");
    let zip = work.join(format!("{mac_file}.zip"));
    let url = center_url_join(&cfg.center_url, download_url);
    let resp = http_client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    if !resp.status().is_success() {
        return Err(Error::Other(anyhow::anyhow!(
            "download bundle failed: {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    tokio::fs::write(&zip, &bytes).await.map_err(Error::Io)?;
    let zip_s = zip.to_string_lossy().to_string();
    let script = format!(
        "New-Item -ItemType Directory -Force -Path '{}' | Out-Null; \
         Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        ps_quote(target_dir),
        ps_quote(&zip_s),
        ps_quote(target_dir)
    );
    let out = tokio::task::spawn_blocking(move || run_powershell_command(&script))
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))??;
    let _ = tokio::fs::remove_file(&zip).await;
    if !out.status.success() {
        return Err(Error::Other(anyhow::anyhow!(
            "expand failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    fs_task_result(cfg, payload, mac, true, "expanded").await;
    Ok(format!("distributed {} bytes", bytes.len()))
}

async fn run_daemon(
    cfg: AgentConfig,
    heartbeat_secs: u64,
    stop: std::sync::Arc<AtomicBool>,
) -> Result<()> {
    let state_dir = default_state_dir();
    restore_driver::set_backend(&cfg.restore_backend);
    let cfg = std::sync::Arc::new(cfg);
    let sync_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));

    // Before the first heartbeat sync (which may re-arm restore and reboot),
    // give a pending update command a chance to run. This matters right after
    // the "disarm for update" reboot: Center pre-queues update_agent so it is
    // consumed here while the volumes are still thawed.
    if let Ok(mac) = effective_mac(&cfg)
        && let Ok(Some(cmd)) = poll_command_wait(&cfg, &mac, 3).await
    {
        let _ = execute_agent_command(&cfg, &state_dir, &sync_lock, &cmd, &mac).await;
    }

    let hb_cfg = cfg.clone();
    let hb_dir = state_dir.clone();
    let hb_lock = sync_lock.clone();
    let hb_stop = stop.clone();
    let heartbeat_task = tokio::spawn(async move {
        loop {
            if hb_stop.load(Ordering::SeqCst) || REBOOT_SCHEDULED.load(Ordering::SeqCst) {
                break;
            }
            {
                let _guard = hb_lock.lock().await;
                if let Err(err) = sync_and_maybe_apply(&hb_cfg, &hb_dir, true, "heartbeat").await {
                    eprintln!("idv-agent: heartbeat failed: {err}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(heartbeat_secs.max(2))).await;
        }
    });

    while !stop.load(Ordering::SeqCst) && !REBOOT_SCHEDULED.load(Ordering::SeqCst) {
        let mac = match effective_mac(&cfg) {
            Ok(mac) => mac,
            Err(err) => {
                eprintln!("idv-agent: {err}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };
        match poll_command(&cfg, &mac).await {
            Ok(Some(cmd)) => {
                if let Err(err) =
                    execute_agent_command(&cfg, &state_dir, &sync_lock, &cmd, &mac).await
                {
                    eprintln!("idv-agent: command {} failed: {err}", cmd.id);
                }
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("idv-agent: {err}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
    heartbeat_task.abort();
    Ok(())
}

#[cfg(windows)]
mod winsvc {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use windows_sys::Win32::Foundation::{ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, GetLastError};
    use windows_sys::Win32::System::Services::*;

    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    static HEARTBEAT: OnceLock<u64> = OnceLock::new();

    const SERVICE_NAME: &str = "idv-agent";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn report(handle: SERVICE_STATUS_HANDLE, state: u32, accepted: u32) {
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: accepted,
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 1,
            dwWaitHint: 10_000,
        };
        unsafe {
            SetServiceStatus(handle, &status);
        }
    }

    unsafe extern "system" fn handler(
        control: u32,
        _event: u32,
        _data: *mut c_void,
        _context: *mut c_void,
    ) -> u32 {
        if (control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN)
            && let Some(stop) = STOP.get()
        {
            stop.store(true, Ordering::SeqCst);
        }
        0
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
        let name = wide(SERVICE_NAME);
        let handle = unsafe {
            RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), std::ptr::null())
        };
        report(handle, SERVICE_START_PENDING, 0);
        report(
            handle,
            SERVICE_RUNNING,
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN,
        );
        let config = CONFIG.get().cloned().unwrap_or_default();
        let heartbeat = *HEARTBEAT.get().unwrap_or(&5);
        if let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            && let Ok(cfg) = runtime.block_on(super::load_cfg(&config))
        {
            super::init_tracing(&cfg.log);
            let stop = STOP
                .get()
                .cloned()
                .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
            let _ = runtime.block_on(super::run_daemon(cfg, heartbeat, stop));
        }
        report(handle, SERVICE_STOPPED, 0);
    }

    /// Run as SCM service; if launched from a console, fall back to daemon mode.
    pub async fn run(config: PathBuf, heartbeat_secs: u64) -> crate::Result<()> {
        let stop = Arc::new(AtomicBool::new(false));
        let _ = STOP.set(stop.clone());
        let _ = CONFIG.set(config.clone());
        let _ = HEARTBEAT.set(heartbeat_secs);
        let name = wide(SERVICE_NAME);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: name.as_ptr() as *mut u16,
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW::default(),
        ];
        let ok = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
                // Console fallback: behave like `daemon`.
                let cfg = super::load_cfg(&config).await?;
                return super::run_daemon(cfg, heartbeat_secs, stop).await;
            }
            return Err(crate::Error::Other(anyhow::anyhow!(
                "StartServiceCtrlDispatcherW failed: {err}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn input(
        want_host: &str,
        have_host: &str,
        want_ip: &str,
        have_ip: &str,
        ip_mode: &str,
        want_frozen: bool,
        frozen: bool,
        armed: bool,
    ) -> IdentityDecisionInput {
        IdentityDecisionInput {
            want_host: want_host.into(),
            have_host: have_host.into(),
            want_ip: want_ip.into(),
            have_ip: have_ip.into(),
            ip_mode: ip_mode.into(),
            want_frozen,
            frozen,
            armed,
        }
    }

    #[test]
    fn hostname_mismatch_while_protected_disarms_first() {
        let action = decide_identity_action(&input(
            "stu001", "BASE-PC", "", "", "static", true, true, true,
        ));
        assert_eq!(action, IdentityAction::DisarmReboot);
    }

    #[test]
    fn hostname_mismatch_unprotected_renames() {
        let action = decide_identity_action(&input(
            "stu001", "BASE-PC", "", "", "static", true, false, false,
        ));
        assert_eq!(action, IdentityAction::RenameReboot);
    }

    #[test]
    fn static_ip_mismatch_while_frozen_does_not_disarm() {
        let action = decide_identity_action(&input(
            "stu001",
            "stu001",
            "10.110.0.31",
            "10.110.0.50",
            "static",
            true,
            true,
            true,
        ));
        assert_eq!(action, IdentityAction::SetStaticIp);
    }

    #[test]
    fn identity_ok_arms_protection() {
        let action = decide_identity_action(&input(
            "stu001",
            "stu001",
            "10.110.0.31",
            "10.110.0.31",
            "dhcp",
            true,
            false,
            false,
        ));
        assert_eq!(action, IdentityAction::ArmReboot);
    }

    #[test]
    fn restore_off_disarms_protection() {
        let action = decide_identity_action(&input(
            "stu001",
            "stu001",
            "10.110.0.31",
            "10.110.0.31",
            "dhcp",
            false,
            true,
            true,
        ));
        assert_eq!(action, IdentityAction::DisarmReboot);
    }

    #[test]
    fn dhcp_ip_mismatch_renews() {
        let action = decide_identity_action(&input(
            "stu001",
            "stu001",
            "10.110.0.31",
            "10.110.0.50",
            "dhcp",
            false,
            false,
            false,
        ));
        assert_eq!(action, IdentityAction::RenewDhcp);
    }

    #[test]
    fn converged_is_noop() {
        let action = decide_identity_action(&input(
            "stu001",
            "stu001",
            "10.110.0.31",
            "10.110.0.31",
            "dhcp",
            true,
            true,
            true,
        ));
        assert_eq!(action, IdentityAction::None);
    }

}
