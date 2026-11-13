use std::path::PathBuf;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use futures_util::StreamExt;
use idv_common::fs_util::relocate_file;
use idv_common::image_format::finalize_captured_image;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::api::edges::upsert_replica;
use crate::auth::AuthEdge;
use crate::mac::normalize_mac;
use crate::models::{
    Edge, EdgeCommand, EdgeCommandReport, EdgeHeartbeatRequest, EdgeRegisterRequest, Image,
    MaintenanceTask, MaintenanceTaskTarget, new_id,
};
use crate::state::AppState;

const IMAGE_COLS: &str = "id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at";

#[derive(Debug, Deserialize)]
pub struct ClaimQuery {
    pub mac: String,
}

#[derive(Debug, Serialize)]
pub struct ClaimedTask {
    pub target_id: String,
    pub task_id: String,
    pub kind: String,
    pub disk_plan: String,
    pub image_id: Option<String>,
    pub mac: String,
    pub os_family: String,
    pub task_name: String,
    #[serde(default = "default_exclude_true")]
    pub exclude_temp: bool,
}

fn default_exclude_true() -> bool {
    true
}

fn parse_capture_detail(detail: &str) -> (String, bool) {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return ("windows".into(), true);
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let os = v
            .get("os_family")
            .and_then(|x| x.as_str())
            .unwrap_or("windows")
            .to_string();
        let excl = v
            .get("exclude_temp")
            .and_then(|x| x.as_bool())
            .unwrap_or(true);
        return (os, excl);
    }
    // Legacy: detail was plain os_family string.
    (trimmed.to_string(), true)
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    pub task_id: String,
    pub target_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_label: String,
    /// When set (same-machine Edge), Center relocates this path instead of reading HTTP body.
    #[serde(default)]
    pub local_path: String,
}

pub async fn register(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
    Json(body): Json<EdgeRegisterRequest>,
) -> Result<Json<Edge>, (StatusCode, String)> {
    let now = Utc::now();
    let listen_url = if body.listen_url.trim().is_empty() {
        edge.listen_url.clone()
    } else {
        body.listen_url.trim().to_string()
    };
    let name = if body.name.trim().is_empty() {
        edge.name.clone()
    } else {
        body.name.trim().to_string()
    };

    sqlx::query("UPDATE edges SET name = ?, listen_url = ?, last_seen = ? WHERE id = ?")
        .bind(&name)
        .bind(&listen_url)
        .bind(now)
        .bind(&edge.id)
        .execute(&state.pool)
        .await
        .map_err(|err| {
            if err.to_string().contains("UNIQUE") {
                (StatusCode::CONFLICT, "edge name already exists".into())
            } else {
                internal(err)
            }
        })?;

    Ok(Json(Edge {
        id: edge.id,
        name,
        node_token: edge.node_token,
        listen_url,
        last_seen: Some(now),
        cache_used_bytes: edge.cache_used_bytes,
        cache_quota_bytes: edge.cache_quota_bytes,
        cache_warn_pct: edge.cache_warn_pct,
        cache_level: edge.cache_level,
        capture_tmp_dir: edge.capture_tmp_dir,
        cpu_pct: edge.cpu_pct,
        mem_used_bytes: edge.mem_used_bytes,
        mem_total_bytes: edge.mem_total_bytes,
        net_rx_bps: edge.net_rx_bps,
        net_tx_bps: edge.net_tx_bps,
        created_at: edge.created_at,
    }))
}

pub async fn heartbeat(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
    Json(hb): Json<EdgeHeartbeatRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let now = Utc::now();
    let level = cache_level(hb.cache_used_bytes, hb.cache_quota_bytes, hb.cache_warn_pct);
    let cpu_pct = hb.cpu_pct.clamp(0.0, 100.0);
    sqlx::query(
        "UPDATE edges SET last_seen = ?, cache_used_bytes = ?, cache_quota_bytes = ?, cache_warn_pct = ?, cache_level = ?, cpu_pct = ?, mem_used_bytes = ?, mem_total_bytes = ?, net_rx_bps = ?, net_tx_bps = ? WHERE id = ?",
    )
    .bind(now)
    .bind(hb.cache_used_bytes)
    .bind(hb.cache_quota_bytes)
    .bind(hb.cache_warn_pct)
    .bind(&level)
    .bind(cpu_pct)
    .bind(hb.mem_used_bytes.max(0))
    .bind(hb.mem_total_bytes.max(0))
    .bind(hb.net_rx_bps.max(0))
    .bind(hb.net_tx_bps.max(0))
    .bind(&edge.id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "server_time": now,
        "cache_level": level,
        "capture_tmp_dir": edge.capture_tmp_dir,
    })))
}

fn cache_level(used: i64, quota: i64, warn_pct: f64) -> String {
    if quota <= 0 {
        return "ok".into();
    }
    let pct = (used as f64 / quota as f64) * 100.0;
    let warn = if warn_pct <= 0.0 { 85.0 } else { warn_pct };
    let crit = (warn + (100.0 - warn) / 2.0).min(95.0);
    if pct >= crit {
        "crit".into()
    } else if pct >= warn {
        "warn".into()
    } else {
        "ok".into()
    }
}

pub async fn next_command(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
) -> Result<Json<Option<EdgeCommand>>, (StatusCode, String)> {
    let now = Utc::now();
    sqlx::query("UPDATE edges SET last_seen = ? WHERE id = ?")
        .bind(now)
        .bind(&edge.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;

    let cmd = sqlx::query_as::<_, EdgeCommand>(
        "SELECT id, edge_id, kind, image_id, state, progress, error_message, created_at, updated_at FROM edge_commands WHERE edge_id = ? AND state = 'pending' ORDER BY created_at ASC LIMIT 1",
    )
    .bind(&edge.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;

    if let Some(ref c) = cmd {
        sqlx::query("UPDATE edge_commands SET state = 'running', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(&c.id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        // Flip replica out of "pending" as soon as Edge claims the command,
        // so UI does not sit at 0% until the first byte-progress report.
        if let Some(image_id) = &c.image_id {
            if matches!(c.kind.as_str(), "sync" | "purge_and_sync") {
                let _ = crate::api::edges::upsert_replica(
                    &state,
                    &edge.id,
                    image_id,
                    "syncing",
                    0.01,
                    "",
                    "",
                    "",
                )
                .await;
            }
        }
    }

    Ok(Json(cmd.map(|mut c| {
        c.state = "running".into();
        c
    })))
}

pub async fn report_command(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
    Path(id): Path<String>,
    Json(body): Json<EdgeCommandReport>,
) -> Result<Json<EdgeCommand>, (StatusCode, String)> {
    let mut cmd = sqlx::query_as::<_, EdgeCommand>(
        "SELECT id, edge_id, kind, image_id, state, progress, error_message, created_at, updated_at FROM edge_commands WHERE id = ? AND edge_id = ?",
    )
    .bind(&id)
    .bind(&edge.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "command not found".into()))?;

    let state_name = body.state.trim();
    if !matches!(state_name, "running" | "done" | "failed") {
        return Err((
            StatusCode::BAD_REQUEST,
            "state must be running|done|failed".into(),
        ));
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE edge_commands SET state = ?, progress = ?, error_message = ?, updated_at = ? WHERE id = ?",
    )
    .bind(state_name)
    .bind(body.progress)
    .bind(&body.error_message)
    .bind(now)
    .bind(&cmd.id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    if let Some(image_id) = &cmd.image_id {
        let replica_state = match (cmd.kind.as_str(), state_name) {
            ("purge", "done") => "absent",
            ("sync", "running") | ("purge_and_sync", "running") => "syncing",
            ("purge", "running") => "purging",
            ("sync", "done") | ("purge_and_sync", "done") => "ready",
            (_, "failed") => "error",
            _ => "syncing",
        };
        let local = if replica_state == "absent" {
            ""
        } else {
            body.local_path.as_str()
        };
        let checksum = if matches!(replica_state, "ready" | "syncing" | "error") {
            body.checksum_sha256.as_str()
        } else {
            ""
        };
        upsert_replica(
            &state,
            &edge.id,
            image_id,
            replica_state,
            body.progress,
            local,
            checksum,
            &body.error_message,
        )
        .await?;
    } else if cmd.kind == "purge_all" && state_name == "done" {
        sqlx::query(
            "UPDATE image_replicas SET state = 'absent', progress = 0, local_path = '', checksum_sha256 = '', error_message = '', updated_at = ? WHERE edge_id = ?",
        )
        .bind(now)
        .bind(&edge.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }

    cmd.state = state_name.to_string();
    cmd.progress = body.progress;
    cmd.error_message = body.error_message;
    cmd.updated_at = now;
    Ok(Json(cmd))
}

pub async fn image_meta(
    State(state): State<AppState>,
    AuthEdge(_edge): AuthEdge,
    Path(id): Path<String>,
) -> Result<Json<Image>, (StatusCode, String)> {
    let sql = format!("SELECT {IMAGE_COLS} FROM images WHERE id = ?");
    sqlx::query_as::<_, Image>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "image not found".into()))
}

pub async fn image_blob(
    State(state): State<AppState>,
    AuthEdge(_edge): AuthEdge,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let sql = format!("SELECT {IMAGE_COLS} FROM images WHERE id = ?");
    let image = sqlx::query_as::<_, Image>(&sql)
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "image not found".into()))?;

    let path = resolve_storage_path(&image.storage_uri)?;
    let file = File::open(&path).await.map_err(|err| {
        (
            StatusCode::NOT_FOUND,
            format!("image blob missing at {}: {err}", path.display()),
        )
    })?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}.bin\"", image.id),
        )
        .body(body)
        .map_err(internal)
}

pub async fn claim_task(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
    Query(q): Query<ClaimQuery>,
) -> Result<Json<Option<ClaimedTask>>, (StatusCode, String)> {
    let mac = normalize_mac(&q.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();

    let target = sqlx::query_as::<_, MaintenanceTaskTarget>(
        r#"
        SELECT tg.id, tg.task_id, tg.mac, tg.state, tg.progress, tg.error_message, tg.detail, tg.p2p_bytes, tg.edge_bytes, tg.updated_at
        FROM maintenance_task_targets tg
        JOIN maintenance_tasks mt ON mt.id = tg.task_id
        WHERE tg.mac = ?
          AND tg.state IN ('pending', 'running')
          AND mt.state IN ('pending', 'running')
          AND (mt.edge_id IS NULL OR mt.edge_id = '' OR mt.edge_id = ?)
        ORDER BY mt.created_at ASC
        LIMIT 1
        "#,
    )
    .bind(&mac)
    .bind(&edge.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;

    let Some(target) = target else {
        return Ok(Json(None));
    };

    let task = sqlx::query_as::<_, MaintenanceTask>(
        "SELECT id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at FROM maintenance_tasks WHERE id = ?",
    )
    .bind(&target.task_id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;

    sqlx::query("UPDATE maintenance_task_targets SET state = 'running', updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(&target.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query(
        "UPDATE maintenance_tasks SET state = 'running', edge_id = COALESCE(NULLIF(edge_id, ''), ?), updated_at = ? WHERE id = ?",
    )
    .bind(&edge.id)
    .bind(now)
    .bind(&task.id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    let (os_family, exclude_temp) = if task.kind == "capture" {
        parse_capture_detail(&target.detail)
    } else {
        (
            if target.detail.trim().is_empty() {
                "windows".into()
            } else {
                target.detail.clone()
            },
            true,
        )
    };

    tracing::info!(
        task_id = %task.id,
        kind = %task.kind,
        mac = %mac,
        exclude_temp,
        "[claim] task"
    );

    Ok(Json(Some(ClaimedTask {
        target_id: target.id,
        task_id: task.id,
        kind: task.kind,
        disk_plan: task.disk_plan,
        image_id: task.image_id,
        mac,
        os_family,
        task_name: task.name,
        exclude_temp,
    })))
}

pub async fn report_target(
    State(state): State<AppState>,
    AuthEdge(_edge): AuthEdge,
    Path(id): Path<String>,
    Json(body): Json<TargetReport>,
) -> Result<Json<MaintenanceTaskTarget>, (StatusCode, String)> {
    if !matches!(
        body.state.as_str(),
        "pending" | "running" | "done" | "failed"
    ) {
        return Err((StatusCode::BAD_REQUEST, "invalid target state".into()));
    }
    let now = Utc::now();
    let existing = sqlx::query_as::<_, MaintenanceTaskTarget>(
        "SELECT id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at FROM maintenance_task_targets WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;

    let detail = if body.detail.is_empty() {
        existing.detail.clone()
    } else {
        body.detail.clone()
    };
    let p2p_bytes = if body.p2p_bytes > 0 {
        body.p2p_bytes
    } else {
        existing.p2p_bytes
    };
    let edge_bytes = if body.edge_bytes > 0 {
        body.edge_bytes
    } else {
        existing.edge_bytes
    };

    sqlx::query(
        "UPDATE maintenance_task_targets SET state = ?, progress = ?, error_message = ?, detail = ?, p2p_bytes = ?, edge_bytes = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&body.state)
    .bind(body.progress)
    .bind(&body.error_message)
    .bind(&detail)
    .bind(p2p_bytes)
    .bind(edge_bytes)
    .bind(now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    refresh_task_aggregate(&state, &existing.task_id).await?;

    let updated = sqlx::query_as::<_, MaintenanceTaskTarget>(
        "SELECT id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at FROM maintenance_task_targets WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(updated))
}

pub async fn ingest_image(
    State(state): State<AppState>,
    AuthEdge(_edge): AuthEdge,
    Query(q): Query<IngestQuery>,
    request: Request,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    let target = sqlx::query_as::<_, MaintenanceTaskTarget>(
        "SELECT id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at FROM maintenance_task_targets WHERE id = ?",
    )
    .bind(&q.target_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "target not found".into()))?;
    if target.task_id != q.task_id {
        return Err((StatusCode::BAD_REQUEST, "target/task mismatch".into()));
    }

    let task = sqlx::query_as::<_, MaintenanceTask>(
        "SELECT id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at FROM maintenance_tasks WHERE id = ?",
    )
    .bind(&q.task_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "task not found".into()))?;
    if task.kind != "capture" {
        return Err((StatusCode::BAD_REQUEST, "ingest only for capture tasks".into()));
    }

    let image_id = new_id();
    tokio::fs::create_dir_all(&state.images_dir)
        .await
        .map_err(internal)?;
    let dest = state.images_dir.join(format!("{image_id}.bin"));
    let size_bytes = if !q.local_path.trim().is_empty() {
        let src = PathBuf::from(q.local_path.trim());
        if !src.is_file() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("local_path not a file: {}", src.display()),
            ));
        }
        let meta = tokio::fs::metadata(&src).await.map_err(internal)?;
        let size = meta.len() as i64;
        if size == 0 {
            return Err((StatusCode::BAD_REQUEST, "empty local image".into()));
        }
        let src2 = src.clone();
        let dest2 = dest.clone();
        let how = tokio::task::spawn_blocking(move || relocate_file(&src2, &dest2))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(internal)?;
        tracing::info!(
            image_id = %image_id,
            how,
            src = %src.display(),
            dest = %dest.display(),
            "local ingest relocate"
        );
        // Consume body if any (clients may send empty).
        let mut stream = request.into_body().into_data_stream();
        while let Some(chunk) = stream.next().await {
            let _ = chunk;
        }
        size
    } else {
        let mut file = File::create(&dest).await.map_err(internal)?;
        let mut stream = request.into_body().into_data_stream();
        let mut size_bytes = 0i64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            size_bytes += chunk.len() as i64;
            file.write_all(&chunk).await.map_err(internal)?;
        }
        file.flush().await.map_err(internal)?;
        drop(file);
        if size_bytes == 0 {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err((StatusCode::BAD_REQUEST, "empty image body".into()));
        }
        size_bytes
    };

    let dest_for_finalize = dest.clone();
    let size_for_log = size_bytes;
    // Keep PE HTTP request alive while we hash; surface state for UI.
    let _ = sqlx::query(
        "UPDATE maintenance_task_targets SET detail = ?, progress = 0.99, updated_at = ? WHERE id = ?",
    )
    .bind(format!(
        "server hashing {:.2} GB (payload+file SHA)",
        size_for_log as f64 / (1024.0 * 1024.0 * 1024.0)
    ))
    .bind(Utc::now())
    .bind(&target.id)
    .execute(&state.pool)
    .await;
    tracing::info!(
        image_id = %image_id,
        size_bytes,
        path = %dest.display(),
        "finalize_captured_image start"
    );
    let finalize_started = std::time::Instant::now();
    let (header, checksum) =
        tokio::task::spawn_blocking(move || finalize_captured_image(&dest_for_finalize))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    tracing::info!(
        image_id = %image_id,
        elapsed_ms = finalize_started.elapsed().as_millis() as u64,
        "finalize_captured_image done"
    );

    let partition_map =
        serde_json::to_string(&header.partitions).unwrap_or_else(|_| "[]".into());
    let name = if q.name.trim().is_empty() {
        if task.name.trim().is_empty() {
            format!("capture-{}", &image_id[..8])
        } else {
            task.name.clone()
        }
    } else {
        q.name.trim().to_string()
    };
    let now = Utc::now();
    let storage_uri = dest
        .canonicalize()
        .unwrap_or(dest.clone())
        .to_string_lossy()
        .to_string();
    let image = Image {
        id: image_id.clone(),
        name,
        os_family: header.os_family.clone(),
        version_label: q.version_label,
        checksum_sha256: checksum,
        size_bytes,
        storage_uri,
        boot_mode: header.boot_mode.clone(),
        notes: format!("ingested from capture task {}", task.id),
        partition_map,
        content_type: "full".into(),
        base_image_id: None,
        created_at: now,
    };

    sqlx::query(
        "INSERT INTO images (id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&image.id)
    .bind(&image.name)
    .bind(&image.os_family)
    .bind(&image.version_label)
    .bind(&image.checksum_sha256)
    .bind(image.size_bytes)
    .bind(&image.storage_uri)
    .bind(&image.boot_mode)
    .bind(&image.notes)
    .bind(&image.partition_map)
    .bind(&image.content_type)
    .bind(&image.base_image_id)
    .bind(image.created_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    sqlx::query("UPDATE maintenance_tasks SET image_id = ?, updated_at = ? WHERE id = ?")
        .bind(&image.id)
        .bind(now)
        .bind(&task.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query(
        "UPDATE maintenance_task_targets SET state = 'done', progress = 1.0, error_message = '', updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(&target.id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    refresh_task_aggregate(&state, &task.id).await?;

    Ok((StatusCode::CREATED, Json(image)))
}

async fn refresh_task_aggregate(
    state: &AppState,
    task_id: &str,
) -> Result<(), (StatusCode, String)> {
    let states: Vec<(String,)> =
        sqlx::query_as("SELECT state FROM maintenance_task_targets WHERE task_id = ?")
            .bind(task_id)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    if states.is_empty() {
        return Ok(());
    }
    let all_done = states.iter().all(|(s,)| s == "done");
    let any_failed = states.iter().any(|(s,)| s == "failed");
    let any_active = states
        .iter()
        .any(|(s,)| s == "pending" || s == "running");
    let state_name = if all_done {
        "done"
    } else if any_failed && !any_active {
        "failed"
    } else {
        "running"
    };
    let now = Utc::now();
    sqlx::query("UPDATE maintenance_tasks SET state = ?, updated_at = ? WHERE id = ?")
        .bind(state_name)
        .bind(now)
        .bind(task_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(())
}

fn resolve_storage_path(storage_uri: &str) -> Result<PathBuf, (StatusCode, String)> {
    let raw = storage_uri.trim();
    if raw.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "image has empty storage_uri".into()));
    }
    if let Some(rest) = raw.strip_prefix("file:///") {
        return Ok(PathBuf::from(rest));
    }
    if let Some(rest) = raw.strip_prefix("file://") {
        return Ok(PathBuf::from(rest));
    }
    Ok(PathBuf::from(raw))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
