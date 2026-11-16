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
const BUILD_VERSION_MARKER: &str = concat!(
    "IDV_AGENT_BUILD_VERSION=",
    env!("CARGO_PKG_VERSION")
);

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
    }
    Ok(())
}

async fn sync_and_maybe_apply(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    apply: bool,
    trigger: &str,
) -> Result<String> {
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
    if ip_mismatch {
        return IdentityAction::SetStaticIp;
    }
    if i.want_frozen && !(i.frozen || i.armed) {
        return IdentityAction::ArmReboot;
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
                true,
            ))
        } else {
            store.clear_pending_applied(pending.revision, "restore_disabled".into());
            store.save(state_dir).map_err(|e| Error::Other(e))?;
            Ok(("applied restore_disabled".into(), true))
        }
    } else {
        Ok(("apply_skip reason=no_pending".into(), false))
    }
}

const UPDATER_SCRIPT: &str = r#"
param(
  [string]$NewExe,
  [string]$TargetExe,
  [string]$Service,
  [string]$Task,
  [string]$Config,
  [string]$Log
)
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
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("-NewExe")
            .arg(&new_exe)
            .arg("-TargetExe")
            .arg(&current_exe)
            .arg("-Service")
            .arg("idv-agent")
            .arg("-Task")
            .arg("YixueIdvAgentDaemon")
            .arg("-Config")
            .arg(&config)
            .arg("-Log")
            .arg(&log);
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        cmd.spawn()
            .map_err(|e| Error::Other(anyhow::anyhow!("spawn updater: {e}")))?;
        update_guard.keep = true;
        Ok((
            true,
            format!(
                "update_scheduled version={version} bytes={} log={}",
                bytes.len(),
                log.display()
            ),
        ))
    }
}

fn issue_windows_power(kind: &str) -> Result<()> {
    #[cfg(windows)]
    {
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

async fn execute_agent_command(
    cfg: &AgentConfig,
    state_dir: &std::path::Path,
    sync_lock: &tokio::sync::Mutex<()>,
    cmd: &AgentCommand,
    mac: &str,
) -> Result<()> {
    let kind = cmd.kind.as_str();
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
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &url])
                .spawn()
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
        report_command(cfg, &cmd.id, true, &format!("open_url={url}")).await?;
        return Ok(());
    }

    if kind == "run_program" {
        let payload: RunProgramPayload = serde_json::from_str(&cmd.payload)
            .map_err(|e| Error::Other(anyhow::anyhow!("invalid run_program payload: {e}")))?;
        #[cfg(windows)]
        {
            std::process::Command::new(&payload.path)
                .args(&payload.args)
                .spawn()
                .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
        report_command(cfg, &cmd.id, true, &format!("run_program={}", payload.path)).await?;
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

/// Resident loop shared by console daemon and Windows service.
async fn run_daemon(
    cfg: AgentConfig,
    heartbeat_secs: u64,
    stop: std::sync::Arc<AtomicBool>,
) -> Result<()> {
    let state_dir = default_state_dir();
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
