use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::worker::EdgeRuntime;

#[derive(Clone)]
pub struct PeState {
    pub sessions: Arc<RwLock<HashMap<String, PeSession>>>,
    pub runtime: EdgeRuntime,
    /// Shared secret for WiFi PE sessions (X-Idv-Pe-Token).
    pub pe_deploy_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeSession {
    pub id: String,
    pub mac: String,
    pub source: String,
    pub status: String,
    pub message: String,
    pub has_task: bool,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CheckinQuery {
    pub mac: String,
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "ipxe".into()
}

#[derive(Debug, Deserialize)]
pub struct CheckinBody {
    pub mac: String,
    #[serde(default = "default_pe_source")]
    pub source: String,
}

fn default_pe_source() -> String {
    "pe-agent".into()
}

#[derive(Debug, Serialize)]
pub struct CheckinResponse {
    pub session_id: String,
    pub mac: String,
    pub status: String,
    pub has_task: bool,
    pub message: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<ClaimedTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lab_id: Option<String>,
    pub client_created: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeEnrollInfo {
    #[serde(default)]
    pub ok: bool,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    pub lab_id: Option<String>,
    #[serde(default)]
    pub client_created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedTask {
    pub target_id: String,
    pub task_id: String,
    pub kind: String,
    pub disk_plan: String,
    pub image_id: Option<String>,
    pub mac: String,
    pub os_family: String,
    pub task_name: String,
}

#[derive(Debug, Deserialize)]
pub struct MacQuery {
    pub mac: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TargetReport {
    pub state: String,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub error_message: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub p2p_bytes: i64,
    #[serde(default)]
    pub edge_bytes: i64,
}

pub async fn checkin_get(
    State(state): State<PeState>,
    headers: HeaderMap,
    Query(q): Query<CheckinQuery>,
) -> Result<Json<CheckinResponse>, (StatusCode, String)> {
    enforce_wifi_token(&state, &q.source, &headers)?;
    checkin_inner(&state, &q.mac, &q.source).await
}

pub async fn checkin_post(
    State(state): State<PeState>,
    headers: HeaderMap,
    Json(body): Json<CheckinBody>,
) -> Result<Json<CheckinResponse>, (StatusCode, String)> {
    enforce_wifi_token(&state, &body.source, &headers)?;
    checkin_inner(&state, &body.mac, &body.source).await
}

fn is_wifi_source(source: &str) -> bool {
    let s = source.to_ascii_lowercase();
    s == "wifi" || s.starts_with("wifi-")
}

fn enforce_wifi_token(
    state: &PeState,
    source: &str,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, String)> {
    if !is_wifi_source(source) {
        return Ok(());
    }
    if state.pe_deploy_token.trim().is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "wifi PE deploy token not configured on edge".into(),
        ));
    }
    let presented = headers
        .get("x-idv-pe-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if presented != state.pe_deploy_token {
        return Err((
            StatusCode::UNAUTHORIZED,
            "wifi session requires valid X-Idv-Pe-Token".into(),
        ));
    }
    Ok(())
}

async fn checkin_inner(
    state: &PeState,
    mac_raw: &str,
    source: &str,
) -> Result<Json<CheckinResponse>, (StatusCode, String)> {
    let mac = normalize_mac(mac_raw).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();
    let task = state
        .runtime
        .claim_task(&mac)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let has_task = task.is_some();
    let message = if let Some(ref t) = task {
        format!("维护任务已就绪：{} ({})", t.kind, t.task_name)
    } else {
        "当前无维护任务。请从本地硬盘启动，或联系网管下发采集/部署任务。".to_string()
    };
    let action = if has_task {
        "await_task".to_string()
    } else {
        "exit_to_local_disk".to_string()
    };

    let session = {
        let mut guard = state.sessions.write().await;
        if let Some(existing) = guard.get_mut(&mac) {
            existing.last_seen = now;
            existing.source = source.to_string();
            existing.message = message.clone();
            existing.has_task = has_task;
            existing.status = "maintenance".into();
            existing.clone()
        } else {
            let s = PeSession {
                id: Uuid::new_v4().to_string(),
                mac: mac.clone(),
                source: source.to_string(),
                status: "maintenance".into(),
                message: message.clone(),
                has_task,
                created_at: now,
                last_seen: now,
            };
            guard.insert(mac.clone(), s.clone());
            s
        }
    };

    let enroll = state
        .runtime
        .report_pe_session(&session)
        .await
        .unwrap_or_default();

    let message = match (&enroll.client_name, has_task) {
        (Some(name), true) => format!("Registered as {name}. {message}"),
        (Some(name), false) => format!(
            "Registered as {name}. No maintenance task; boot local disk."
        ),
        (None, _) => message,
    };

    Ok(Json(CheckinResponse {
        session_id: session.id,
        mac: session.mac,
        status: session.status,
        has_task,
        message,
        action,
        task,
        client_id: enroll.client_id,
        client_name: enroll.client_name,
        lab_id: enroll.lab_id,
        client_created: enroll.client_created,
    }))
}

/// iPXE-friendly check-in: returns a script that prints enroll name and boots local disk.
pub async fn checkin_ipxe(
    State(state): State<PeState>,
    headers: HeaderMap,
    Query(q): Query<CheckinQuery>,
) -> Result<Response, (StatusCode, String)> {
    enforce_wifi_token(&state, &q.source, &headers)?;
    let Json(body) = checkin_inner(&state, &q.mac, &q.source).await?;

    let name = body
        .client_name
        .clone()
        .unwrap_or_else(|| "(unregistered)".into());
    let created = if body.client_created {
        "new"
    } else {
        "known"
    };
    let mut script = String::from("#!ipxe\n");
    script.push_str("echo\n");
    script.push_str("echo === Yixue IDV Check-in ===\n");
    script.push_str(&format!("echo MAC: {}\n", body.mac));
    script.push_str(&format!("echo Registered: {name} ({created})\n"));
    let kind = body
        .task
        .as_ref()
        .map(|t| t.kind.as_str())
        .unwrap_or("");
    if body.has_task && (kind == "capture" || kind == "deploy") {
        // Alpine netboot + idv apkovl; pe-agent run handles capture or deploy (see pe/linux).
        script.push_str("isset ${edge-http} || set edge-http http://${next-server}:8081\n");
        if kind == "capture" {
            script.push_str("echo Capture task - booting Linux PE...\n");
        } else {
            script.push_str("echo Deploy task - booting Linux PE...\n");
        }
        script.push_str("kernel ${edge-http}/pxe/linux/vmlinuz-lts console=tty0 earlyprintk=efi ip=dhcp modules=loop,squashfs,sd-mod,usb-storage,nvme,ahci,xhci-hcd apkovl=${edge-http}/pxe/linux/idv.apkovl.tar.gz modloop=${edge-http}/pxe/linux/modloop-lts idv.edge=${edge-http}\n");
        script.push_str("initrd ${edge-http}/pxe/linux/initramfs-lts\n");
        script.push_str("boot || goto pe_failed\n");
        script.push_str(":pe_failed\n");
        script.push_str("echo ERROR: Linux PE boot failed. Check pe/linux assets.\n");
        script.push_str("shell\n");
    } else if body.has_task {
        script.push_str(&format!(
            "echo Unsupported task kind '{kind}' - not starting PE.\n"
        ));
        script.push_str("sleep 8\n");
        script.push_str("shell\n");
    } else {
        script.push_str("echo No capture/deploy task; booting local disk...\n");
        script.push_str("sleep 2\n");
        script.push_str("sanboot --no-describe --drive 0x80 && exit 0 ||\n");
        script.push_str("sanboot --no-describe --drive 0x81 && exit 0 ||\n");
        script.push_str("echo ERROR: sanboot failed\n");
        script.push_str("shell\n");
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(script))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn list_sessions(State(state): State<PeState>) -> Json<Vec<PeSession>> {
    let guard = state.sessions.read().await;
    let mut rows: Vec<_> = guard.values().cloned().collect();
    rows.sort_by_key(|a| std::cmp::Reverse(a.last_seen));
    Json(rows)
}

pub async fn claim(
    State(state): State<PeState>,
    Query(q): Query<MacQuery>,
) -> Result<Json<Option<ClaimedTask>>, (StatusCode, String)> {
    let mac = normalize_mac(&q.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state
        .runtime
        .claim_task(&mac)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

pub async fn report(
    State(state): State<PeState>,
    Path(id): Path<String>,
    Json(body): Json<TargetReport>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .runtime
        .report_target(&id, &body)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

pub async fn capture_upload(
    State(state): State<PeState>,
    Path(target_id): Path<String>,
    Query(q): Query<CaptureUploadQuery>,
    request: axum::extract::Request,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    // Configurable via Center UI (heartbeat) or edge.toml capture_tmp_dir.
    // Default: cache_dir/capture-tmp (avoid tiny system TEMP drives).
    let tmp_dir = state.runtime.resolved_capture_tmp();
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Drop leftover staging files from prior failed uploads of the same target.
    if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
        let prefix = format!("{target_id}-");
        while let Ok(Some(ent)) = entries.next_entry().await {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".bin") {
                let _ = tokio::fs::remove_file(ent.path()).await;
            }
        }
    }

    let tmp = tmp_dir.join(format!("{target_id}-{}.bin", Uuid::new_v4()));
    let cleanup_tmp = tmp.clone();
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut stream = request.into_body().into_data_stream();
    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tokio::fs::remove_file(&cleanup_tmp).await;
                return Err((StatusCode::BAD_REQUEST, e.to_string()));
            }
        };
        size += chunk.len() as u64;
        if let Err(e) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&cleanup_tmp).await;
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    }
    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&cleanup_tmp).await;
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }
    drop(file);
    if size == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err((StatusCode::BAD_REQUEST, "empty capture body".into()));
    }

    let image = match state
        .runtime
        .ingest_capture(
            &q.task_id,
            &target_id,
            &q.name,
            &q.version_label,
            &tmp,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&cleanup_tmp).await;
            return Err((StatusCode::BAD_GATEWAY, e.to_string()));
        }
    };
    let _ = tokio::fs::remove_file(&tmp).await;
    Ok((StatusCode::CREATED, Json(image)))
}

#[derive(Debug, Deserialize)]
pub struct CaptureUploadQuery {
    pub task_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_label: String,
}

pub async fn image_meta(
    State(state): State<PeState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Prefer local cache presence; meta always from center.
    let meta = state
        .runtime
        .fetch_image_meta(&id)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let cached = state.runtime.cache_file(&id).exists();
    Ok(Json(serde_json::json!({
        "id": meta["id"],
        "checksum_sha256": meta["checksum_sha256"],
        "size_bytes": meta["size_bytes"],
        "partition_map": meta["partition_map"],
        "os_family": meta["os_family"],
        "boot_mode": meta["boot_mode"],
        "cached": cached
    })))
}

pub async fn image_blob(
    State(state): State<PeState>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let path = state.runtime.cache_file(&id);
    if !path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("image {id} not in edge cache; sync first"),
        ));
    }
    let file = File::open(&path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("open cache failed: {e}"),
        )
    })?;
    let stream = ReaderStream::new(file);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from_stream(stream))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn normalize_mac(input: &str) -> Result<String, String> {
    let hex: String = input
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return Err(format!("invalid MAC: {input}"));
    }
    Ok(hex
        .as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(":"))
}

#[allow(dead_code)]
fn _path_buf() -> PathBuf {
    PathBuf::new()
}
