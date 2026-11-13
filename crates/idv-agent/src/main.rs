mod policy_store;
mod restore_driver;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use idv_common::endpoint_policy::DesiredEndpointState;
use idv_common::image_format::{
    ImageHeader, default_partitions, pack_image, unpack_image,
};
use idv_common::restore::{apply_partition_restore, should_restore_now};
use idv_common::usb_policy::apply_usb_policy;
use idv_common::volume::{
    BootHookInstall, VolumeBackend, VolumeLayout, apply_windows_lab_restore, write_boot_hook,
    write_lab_baseline, LabVolumeSnapshot,
};
use idv_common::{Error, Result, init_tracing, load_toml_file};
use policy_store::{LocalPolicyStore, default_state_dir};
use restore_driver::{apply_frozen_state, default_driver};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Parser)]
#[command(name = "idv-agent", about = "易学 IDV 客户端代理（还原/模式）", version)]
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
    #[serde(default = "default_mac")]
    mac: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_log")]
    log: String,
}

fn default_center() -> String {
    "http://127.0.0.1:8080".into()
}
fn default_mac() -> String {
    "02:00:00:00:00:01".into()
}
fn default_log() -> String {
    "info,idv_agent=debug".into()
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status { config } => {
            let cfg = load_cfg(&config).await?;
            init_tracing(&cfg.log);
            println!(
                "service=idv-agent status=ok center={} mac={}",
                cfg.center_url, cfg.mac
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
                policy
                    .mode
                    .as_ref()
                    .map(|m| m.name.as_str())
                    .unwrap_or("-"),
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
                urlencoding_lite(&cfg.mac)
            );
            let client = http_client();
            let size_hint = job
                .get("size_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let resp = client.get(&url).send().await?.error_for_status()?;
            let total = resp
                .content_length()
                .unwrap_or(size_hint);
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
                    println!(
                        "protect_on_ok frozen={} detail={}",
                        st.frozen, st.detail
                    );
                }
                RestoreDriverCmd::ProtectOff => {
                    let st = drv.protect_off()?;
                    println!(
                        "protect_off_ok frozen={} detail={}",
                        st.frozen, st.detail
                    );
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
    if let Some(desired) = policy.desired_state {
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

    if apply {
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
                lines.push(format!(
                    "applied_revision={} frozen={} driver={}",
                    pending.revision, st.frozen, st.detail
                ));
            } else {
                store.clear_pending_applied(pending.revision, "restore_disabled".into());
                store.save(state_dir).map_err(|e| Error::Other(e))?;
                lines.push("applied restore_disabled".into());
            }
        } else {
            lines.push("apply_skip reason=no_pending".into());
        }
    }
    Ok(lines.join(" · "))
}

async fn load_cfg(path: &PathBuf) -> Result<AgentConfig> {
    if path.exists() {
        load_toml_file(path)
    } else {
        Ok(AgentConfig {
            center_url: default_center(),
            mac: default_mac(),
            hostname: "lab-pc".into(),
            log: default_log(),
        })
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
    let client = http_client();
    let resp = client
        .post(url)
        .json(&HeartbeatBody {
            mac: &cfg.mac,
            hostname: &cfg.hostname,
            agent_version: env!("CARGO_PKG_VERSION"),
            trigger,
            applied_revision: status.applied_revision,
            pending_reboot: status.pending_reboot,
            driver_status: &status.driver_status,
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
    let mode = policy.mode.as_ref().ok_or_else(|| {
        Error::Other(anyhow::anyhow!("no mode policy; bind lab mode first"))
    })?;
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
    println!("restore_ok mode={} backend={} detail={}", mode.name, backend, detail);
    Ok(())
}

async fn report(cfg: &AgentConfig, ok: bool, detail: &str) -> Result<()> {
    let url = format!(
        "{}/api/v1/agent/restore/report",
        cfg.center_url.trim_end_matches('/')
    );
    let client = http_client();
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "mac": cfg.mac,
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
    let client = http_client();
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "mac": cfg.mac,
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

