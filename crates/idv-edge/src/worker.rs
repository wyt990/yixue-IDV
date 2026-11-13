use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

#[derive(Clone)]
pub struct EdgeRuntime {
    pub center_url: String,
    pub node_token: String,
    pub name: String,
    pub listen_url: String,
    pub cache_dir: PathBuf,
    /// Local default from edge.toml (or cache_dir/capture-tmp). Center may override via heartbeat.
    pub capture_tmp_default: PathBuf,
    pub capture_tmp_dir: Arc<std::sync::RwLock<PathBuf>>,
    pub cache_quota_bytes: u64,
    pub cache_warn_pct: f64,
    pub sync_bandwidth_bps: u64,
    pub center_reachable: Arc<AtomicBool>,
    pub client: reqwest::Client,
    pub sync_slots: Arc<tokio::sync::Semaphore>,
    pub host_metrics: Arc<crate::host_metrics::HostMetricsSampler>,
}

impl EdgeRuntime {
    pub fn resolved_capture_tmp(&self) -> PathBuf {
        self.capture_tmp_dir
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn apply_capture_tmp_from_center(&self, from_center: &str) {
        let next = {
            let trimmed = from_center.trim();
            if trimmed.is_empty() {
                self.capture_tmp_default.clone()
            } else {
                PathBuf::from(trimmed)
            }
        };
        let mut slot = self
            .capture_tmp_dir
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if *slot != next {
            info!(path = %next.display(), "capture_tmp_dir updated from center");
            *slot = next;
        }
    }
}

#[derive(Debug, Deserialize)]
struct EdgeCommand {
    id: String,
    kind: String,
    image_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageMeta {
    #[allow(dead_code)]
    id: String,
    checksum_sha256: String,
    size_bytes: i64,
    /// Center storage path; when reachable on this host, prefer hardlink/copy over HTTP.
    #[serde(default)]
    storage_uri: String,
}

#[derive(Debug, Serialize)]
struct ReportBody<'a> {
    state: &'a str,
    progress: f64,
    error_message: String,
    local_path: String,
    checksum_sha256: String,
}

impl EdgeRuntime {
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "X-Idv-Edge-Token",
            self.node_token.parse().expect("token header"),
        );
        headers
    }

    pub async fn report_pe_session(
        &self,
        session: &crate::pe_api::PeSession,
    ) -> anyhow::Result<crate::pe_api::PeEnrollInfo> {
        let url = format!("{}/api/v1/edge/pe-sessions", self.center_url);
        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(session)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("pe session report failed: {status} {body}");
        }
        let info = resp
            .json::<crate::pe_api::PeEnrollInfo>()
            .await
            .unwrap_or_default();
        Ok(info)
    }

    pub async fn claim_task(
        &self,
        mac: &str,
    ) -> anyhow::Result<Option<crate::pe_api::ClaimedTask>> {
        let url = format!("{}/api/v1/edge/tasks/claim", self.center_url);
        let resp = self
            .client
            .get(url)
            .headers(self.headers())
            .query(&[("mac", mac)])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("claim task failed: {status} {body}");
        }
        Ok(resp.json().await?)
    }

    pub async fn report_target(
        &self,
        target_id: &str,
        body: &crate::pe_api::TargetReport,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!(
            "{}/api/v1/edge/tasks/targets/{target_id}/report",
            self.center_url
        );
        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("report target failed: {status} {text}");
        }
        Ok(resp.json().await?)
    }

    pub async fn ingest_capture(
        &self,
        task_id: &str,
        target_id: &str,
        name: &str,
        version_label: &str,
        packed_path: &std::path::Path,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/api/v1/edge/images/ingest", self.center_url);
        let resp = if idv_common::fs_util::center_url_is_loopback(&self.center_url) {
            let local_path = tokio::fs::canonicalize(packed_path)
                .await
                .unwrap_or_else(|_| packed_path.to_path_buf())
                .to_string_lossy()
                .to_string();
            info!(
                path = %local_path,
                "ingest via local_path (loopback Center)"
            );
            self.client
                .put(&url)
                .headers(self.headers())
                .query(&[
                    ("task_id", task_id),
                    ("target_id", target_id),
                    ("name", name),
                    ("version_label", version_label),
                    ("local_path", local_path.as_str()),
                ])
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .header(reqwest::header::CONTENT_LENGTH, "0")
                .body(reqwest::Body::from(""))
                .send()
                .await?
        } else {
            let file = tokio::fs::File::open(packed_path).await?;
            let stream = tokio_util::io::ReaderStream::new(file);
            self.client
                .put(&url)
                .headers(self.headers())
                .query(&[
                    ("task_id", task_id),
                    ("target_id", target_id),
                    ("name", name),
                    ("version_label", version_label),
                ])
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await?
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ingest failed: {status} {text}");
        }
        Ok(resp.json().await?)
    }

    pub async fn fetch_image_meta(&self, image_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/api/v1/edge/images/{image_id}", self.center_url);
        let resp = self
            .client
            .get(url)
            .headers(self.headers())
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("image meta failed: {status} {text}");
        }
        Ok(resp.json().await?)
    }

    pub fn cache_file(&self, image_id: &str) -> PathBuf {
        self.cache_dir.join(format!("{image_id}.bin"))
    }

    pub async fn register(&self) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/edge/register", self.center_url);
        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(&serde_json::json!({
                "name": self.name,
                "listen_url": self.listen_url,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("register failed: {status} {body}");
        }
        info!(name = %self.name, "registered with center");
        Ok(())
    }

    async fn heartbeat(&self) -> anyhow::Result<()> {
        let used = self.cache_used_bytes().await.unwrap_or(0);
        if self.cache_quota_bytes > 0 {
            let pct = (used as f64 / self.cache_quota_bytes as f64) * 100.0;
            if pct >= self.cache_warn_pct {
                warn!(
                    used,
                    quota = self.cache_quota_bytes,
                    pct,
                    "edge cache watermark exceeded"
                );
            }
        }
        let metrics = self.host_metrics.sample();
        let url = format!("{}/api/v1/edge/heartbeat", self.center_url);
        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(&serde_json::json!({
                "cache_used_bytes": used,
                "cache_quota_bytes": self.cache_quota_bytes,
                "cache_warn_pct": self.cache_warn_pct,
                "cpu_pct": metrics.cpu_pct,
                "mem_used_bytes": metrics.mem_used_bytes,
                "mem_total_bytes": metrics.mem_total_bytes,
                "net_rx_bps": metrics.net_rx_bps,
                "net_tx_bps": metrics.net_tx_bps,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("heartbeat failed: {status} {body}");
        }
        #[derive(Debug, Deserialize)]
        struct HeartbeatResp {
            #[serde(default)]
            capture_tmp_dir: String,
        }
        if let Ok(body) = resp.json::<HeartbeatResp>().await {
            self.apply_capture_tmp_from_center(&body.capture_tmp_dir);
        }
        Ok(())
    }

    async fn cache_used_bytes(&self) -> anyhow::Result<u64> {
        let mut total = 0u64;
        let mut rd = tokio::fs::read_dir(&self.cache_dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
        Ok(total)
    }

    async fn next_command(&self) -> anyhow::Result<Option<EdgeCommand>> {
        let url = format!("{}/api/v1/edge/commands/next", self.center_url);
        let resp = self
            .client
            .get(url)
            .headers(self.headers())
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("pull command failed: {status} {body}");
        }
        Ok(resp.json::<Option<EdgeCommand>>().await?)
    }

    async fn report(
        &self,
        command_id: &str,
        state: &str,
        progress: f64,
        error_message: impl Into<String>,
        local_path: impl Into<String>,
        checksum: impl Into<String>,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/api/v1/edge/commands/{command_id}/report",
            self.center_url
        );
        let body = ReportBody {
            state,
            progress,
            error_message: error_message.into(),
            local_path: local_path.into(),
            checksum_sha256: checksum.into(),
        };
        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("report failed: {status} {text}");
        }
        Ok(())
    }

    pub async fn run_loops(self, heartbeat_secs: u64, poll_secs: u64) {
        let hb = self.clone();
        tokio::spawn(async move {
            loop {
                match hb.heartbeat().await {
                    Ok(()) => hb.center_reachable.store(true, Ordering::Relaxed),
                    Err(err) => {
                        hb.center_reachable.store(false, Ordering::Relaxed);
                        warn!(error = %err, "heartbeat/center unreachable; edge keeps running");
                        let _ = hb.register().await;
                    }
                }
                tokio::time::sleep(Duration::from_secs(heartbeat_secs)).await;
            }
        });

        loop {
            match self.next_command().await {
                Ok(Some(cmd)) => {
                    self.center_reachable.store(true, Ordering::Relaxed);
                    if let Err(err) = self.handle_command(cmd).await {
                        warn!(error = %err, "command handling failed");
                    }
                }
                Ok(None) => {
                    self.center_reachable.store(true, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(poll_secs)).await;
                }
                Err(err) => {
                    self.center_reachable.store(false, Ordering::Relaxed);
                    warn!(error = %err, "pull paused because center unreachable");
                    tokio::time::sleep(Duration::from_secs(poll_secs.max(3))).await;
                }
            }
        }
    }

    async fn handle_command(&self, cmd: EdgeCommand) -> anyhow::Result<()> {
        info!(id = %cmd.id, kind = %cmd.kind, "received command");
        match cmd.kind.as_str() {
            "purge" => {
                let image_id = cmd
                    .image_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("purge missing image_id"))?;
                self.purge_one(&image_id).await?;
                self.report(&cmd.id, "done", 1.0, "", "", "").await?;
            }
            "purge_all" => {
                self.purge_all().await?;
                self.report(&cmd.id, "done", 1.0, "", "", "").await?;
            }
            "sync" => {
                let image_id = cmd
                    .image_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("sync missing image_id"))?;
                let _permit = self.sync_slots.acquire().await?;
                self.ensure_cache_quota().await?;
                match self.sync_image(&cmd.id, &image_id).await {
                    Ok((path, checksum)) => {
                        self.report(&cmd.id, "done", 1.0, "", path, checksum)
                            .await?;
                    }
                    Err(err) => {
                        let _ = self
                            .report(&cmd.id, "failed", 0.0, err.to_string(), "", "")
                            .await;
                        return Err(err);
                    }
                }
            }
            "purge_and_sync" => {
                let image_id = cmd
                    .image_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("purge_and_sync missing image_id"))?;
                let _permit = self.sync_slots.acquire().await?;
                self.purge_one(&image_id).await?;
                self.ensure_cache_quota().await?;
                match self.sync_image(&cmd.id, &image_id).await {
                    Ok((path, checksum)) => {
                        self.report(&cmd.id, "done", 1.0, "", path, checksum)
                            .await?;
                    }
                    Err(err) => {
                        let _ = self
                            .report(&cmd.id, "failed", 0.0, err.to_string(), "", "")
                            .await;
                        return Err(err);
                    }
                }
            }
            other => {
                self.report(
                    &cmd.id,
                    "failed",
                    0.0,
                    format!("unknown command kind: {other}"),
                    "",
                    "",
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn ensure_cache_quota(&self) -> anyhow::Result<()> {
        if self.cache_quota_bytes == 0 {
            return Ok(());
        }
        let used = self.cache_used_bytes().await?;
        if used >= self.cache_quota_bytes {
            anyhow::bail!(
                "cache quota exceeded: used={used} quota={}",
                self.cache_quota_bytes
            );
        }
        Ok(())
    }

    async fn purge_one(&self, image_id: &str) -> anyhow::Result<()> {
        let path = self.cache_file(image_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
            info!(path = %path.display(), "purged cache file");
        }
        Ok(())
    }

    async fn purge_all(&self) -> anyhow::Result<()> {
        let mut rd = tokio::fs::read_dir(&self.cache_dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("bin") {
                tokio::fs::remove_file(&path).await?;
            }
        }
        info!("purged all edge cache bins");
        Ok(())
    }

    async fn sync_image(&self, command_id: &str, image_id: &str) -> anyhow::Result<(String, String)> {
        let meta_url = format!("{}/api/v1/edge/images/{image_id}", self.center_url);
        let meta = self
            .client
            .get(meta_url)
            .headers(self.headers())
            .send()
            .await?
            .error_for_status()?
            .json::<ImageMeta>()
            .await?;

        let dest = self.cache_file(image_id);
        let tmp = self.cache_dir.join(format!("{image_id}.bin.part"));
        if tmp.exists() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }

        let _ = self
            .report(command_id, "running", 0.02, "", tmp.display().to_string(), "")
            .await;

        // Same-machine / shared volume: hardlink or copy instead of HTTP-pulling tens of GB.
        if let Some(src) = local_storage_path(&meta.storage_uri) {
            if src.exists() {
                info!(
                    src = %src.display(),
                    dest = %dest.display(),
                    "sync via local hardlink/copy (skip HTTP blob)"
                );
                let _ = self
                    .report(
                        command_id,
                        "running",
                        0.5,
                        "",
                        dest.display().to_string(),
                        "",
                    )
                    .await;
                if dest.exists() {
                    tokio::fs::remove_file(&dest).await?;
                }
                let placed = match tokio::fs::hard_link(&src, &dest).await {
                    Ok(()) => {
                        info!(path = %dest.display(), "local hardlink ok");
                        true
                    }
                    Err(err) => {
                        warn!(error = %err, "hardlink failed; falling back to copy");
                        match tokio::fs::copy(&src, &dest).await {
                            Ok(_) => true,
                            Err(err2) => {
                                warn!(error = %err2, "local copy failed; falling back to HTTP");
                                false
                            }
                        }
                    }
                };
                if placed {
                    let expected = meta.checksum_sha256.to_ascii_lowercase();
                    let meta_len = meta.size_bytes.max(0) as u64;
                    let actual = tokio::fs::metadata(&dest).await?.len();
                    if meta_len > 0 && actual != meta_len {
                        let _ = tokio::fs::remove_file(&dest).await;
                        anyhow::bail!(
                            "local sync size mismatch: file={actual} expected={meta_len}"
                        );
                    }
                    info!(
                        path = %dest.display(),
                        checksum = %expected,
                        "image synced to edge cache (local)"
                    );
                    return Ok((dest.display().to_string(), expected));
                }
            }
        }

        let blob_url = format!("{}/api/v1/edge/images/{image_id}/blob", self.center_url);
        let resp = self
            .client
            .get(blob_url)
            .headers(self.headers())
            .send()
            .await?
            .error_for_status()?;

        let total = if meta.size_bytes > 0 {
            meta.size_bytes as u64
        } else {
            resp.content_length().unwrap_or(0)
        };

        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut last_report = 0.02_f64;
        let mut last_bytes_report = 0u64;

        let mut window_start = std::time::Instant::now();
        let mut window_bytes = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
            written += chunk.len() as u64;
            window_bytes += chunk.len() as u64;
            if self.sync_bandwidth_bps > 0 {
                let elapsed = window_start.elapsed().as_secs_f64().max(0.001);
                let allowed = self.sync_bandwidth_bps as f64 * elapsed;
                if window_bytes as f64 > allowed {
                    let sleep_secs = (window_bytes as f64 / self.sync_bandwidth_bps as f64) - elapsed;
                    if sleep_secs > 0.0 {
                        tokio::time::sleep(Duration::from_secs_f64(sleep_secs.min(2.0))).await;
                    }
                    window_start = std::time::Instant::now();
                    window_bytes = 0;
                }
            }
            let progress = if total > 0 {
                (written as f64 / total as f64).clamp(0.0, 0.99)
            } else {
                0.5
            };
            let due_pct = progress - last_report >= 0.02;
            let due_bytes = written.saturating_sub(last_bytes_report) >= 256 * 1024 * 1024;
            if due_pct || due_bytes {
                let _ = self
                    .report(
                        command_id,
                        "running",
                        progress,
                        "",
                        tmp.display().to_string(),
                        "",
                    )
                    .await;
                last_report = progress;
                last_bytes_report = written;
            }
        }
        file.flush().await?;
        drop(file);

        let whole = hex::encode(hasher.finalize());
        let expected = meta.checksum_sha256.to_ascii_lowercase();
        let checksum = if whole == expected {
            whole
        } else {
            // Capture ingest stores payload SHA (avoids second full-file hash on Center).
            let (payload_sha, _) = tokio::task::spawn_blocking({
                let tmp = tmp.clone();
                move || idv_common::image_format::hash_image_payload(&tmp)
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))??;
            if payload_sha != expected {
                let _ = tokio::fs::remove_file(&tmp).await;
                anyhow::bail!(
                    "checksum mismatch: file={whole} payload={payload_sha}, expected {expected}"
                );
            }
            payload_sha
        };

        if dest.exists() {
            tokio::fs::remove_file(&dest).await?;
        }
        tokio::fs::rename(&tmp, &dest).await?;
        info!(
            path = %dest.display(),
            checksum = %checksum,
            "image synced to edge cache"
        );
        Ok((dest.display().to_string(), checksum))
    }
}

fn local_storage_path(storage_uri: &str) -> Option<PathBuf> {
    let raw = storage_uri.trim();
    if raw.is_empty() {
        return None;
    }
    let path = if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else if let Some(rest) = raw.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(raw)
    };
    Some(path)
}

#[allow(dead_code)]
fn ensure_parent(path: &Path) -> PathBuf {
    path.to_path_buf()
}
