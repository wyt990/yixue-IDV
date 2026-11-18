//! Real remote-assist relay: screen frames, mouse/keyboard input, clipboard and
//! one-to-one file browsing. The interactive helper runs in the logged-in
//! Windows user session and long-polls `assist_commands`.

use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::sync::mpsc;

use crate::auth::{AuthUser, admin_from_token};
use crate::models::new_id;
use crate::state::{AppState, AssistWire};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistCommand {
    pub id: String,
    pub kind: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistFileRequest {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub path: String,
    pub state: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub payload: String,
    pub error_message: String,
}

#[derive(Debug, Deserialize)]
pub struct FrameQuery {
    #[serde(default)]
    pub mac: String,
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    #[serde(default = "default_wait")]
    pub wait: u64,
}

fn default_wait() -> u64 {
    10
}

#[derive(Debug, Deserialize)]
pub struct ReportBody {
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct TextBody {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct PathBody {
    pub path: String,
}

async fn load_session(
    state: &AppState,
    id: &str,
) -> Result<(String, String, String), (StatusCode, String)> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT mac, kind, state FROM assist_sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    row.ok_or((StatusCode::NOT_FOUND, "assist session not found".into()))
}

async fn enqueue_command(
    state: &AppState,
    session_id: &str,
    kind: &str,
    payload: &str,
) -> anyhow::Result<String> {
    let id = new_id();
    sqlx::query(
        "INSERT INTO assist_commands (id, session_id, kind, payload, state, created_at) \
         VALUES (?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(kind)
    .bind(payload)
    .bind(Utc::now())
    .execute(&state.pool)
    .await?;
    state.assist_notify.notify_waiters();
    Ok(id)
}

// ---------------------------------------------------------------- sessions

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = sqlx::query(
        "SELECT id, mac, kind, state, requested_by, detail, clipboard, last_frame_at, frame_count, \
         created_at, updated_at FROM assist_sessions WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "assist session not found".into()))?;
    Ok(Json(serde_json::json!({
        "id": row.get::<String, _>("id"),
        "mac": row.get::<String, _>("mac"),
        "kind": row.get::<String, _>("kind"),
        "state": row.get::<String, _>("state"),
        "requested_by": row.get::<String, _>("requested_by"),
        "detail": row.get::<String, _>("detail"),
        "clipboard": row.get::<String, _>("clipboard"),
        "last_frame_at": row.get::<Option<String>, _>("last_frame_at"),
        "frame_count": row.get::<i64, _>("frame_count"),
        "created_at": row.get::<String, _>("created_at"),
        "updated_at": row.get::<String, _>("updated_at"),
    })))
}

pub async fn end(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE assist_sessions SET state = 'ended', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let _ = enqueue_command(&state, &id, "assist_stop", "").await;
    let mut frames = state.assist_frames.lock().await;
    frames.remove(&id);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Unauthenticated state check used by the in-session helper (random session id).
pub async fn helper_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let state_name: Option<String> =
        sqlx::query_scalar("SELECT state FROM assist_sessions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    Ok(Json(
        serde_json::json!({ "state": state_name.unwrap_or_else(|| "ended".into()) }),
    ))
}

// ---------------------------------------------------------------- screen frames

pub async fn frame_put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FrameQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.len() < 256 || body.len() > 12 * 1024 * 1024 {
        return Err((StatusCode::BAD_REQUEST, "invalid frame size".into()));
    }
    let (_mac, _kind, state_name) = load_session(&state, &id).await?;
    if state_name == "ended" || state_name == "denied" {
        return Err((StatusCode::CONFLICT, "assist session not active".into()));
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE assist_sessions SET state = 'active', last_frame_at = ?, \
         frame_count = frame_count + 1, updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(&now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let mut frames = state.assist_frames.lock().await;
    frames.insert(id.clone(), body.to_vec());
    if frames.len() > 64
        && let Some(k) = frames.keys().next().cloned()
        && k != id
    {
        frames.remove(&k);
    }
    let _ = q.mac;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn frame_get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let frames = state.assist_frames.lock().await;
    let Some(bytes) = frames.get(&id) else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let mut resp = Response::new(Body::from(bytes.clone()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("image/jpeg"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(resp)
}

// ---------------------------------------------------------------- helper commands

pub async fn commands_next(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<NextQuery>,
) -> Result<Json<Option<AssistCommand>>, (StatusCode, String)> {
    let wait = q.wait.min(20);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
    loop {
        let notified = state.assist_notify.notified();
        let mut tx = state.pool.begin().await.map_err(internal)?;
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT id, kind, payload FROM assist_commands \
             WHERE session_id = ? AND state = 'pending' ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?;
        if let Some((cmd_id, kind, payload)) = row {
            sqlx::query(
                "UPDATE assist_commands SET state = 'delivered', delivered_at = ? WHERE id = ?",
            )
            .bind(Utc::now())
            .bind(&cmd_id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
            tx.commit().await.map_err(internal)?;
            return Ok(Json(Some(AssistCommand {
                id: cmd_id,
                kind,
                payload,
            })));
        }
        tx.commit().await.map_err(internal)?;
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(Json(None));
        }
        let _ = tokio::time::timeout(deadline - now, notified).await;
    }
}

pub async fn command_report(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    sqlx::query(
        "UPDATE assist_commands SET state = ?, finished_at = ?, error_message = ? WHERE id = ?",
    )
    .bind(if body.ok { "done" } else { "failed" })
    .bind(Utc::now())
    .bind(&body.detail)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------- input / clipboard

pub async fn input(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (_mac, kind, state_name) = load_session(&state, &id).await?;
    if kind != "control" {
        return Err((StatusCode::FORBIDDEN, "observe session is read-only".into()));
    }
    if state_name == "ended" || state_name == "denied" {
        return Err((StatusCode::CONFLICT, "session not active".into()));
    }
    enqueue_command(&state, &id, "input", &body.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn clipboard_set(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<TextBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (_mac, kind, _) = load_session(&state, &id).await?;
    if kind != "control" {
        return Err((StatusCode::FORBIDDEN, "observe session is read-only".into()));
    }
    sqlx::query("UPDATE assist_sessions SET clipboard = ?, updated_at = ? WHERE id = ?")
        .bind(&body.text)
        .bind(Utc::now())
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    enqueue_command(
        &state,
        &id,
        "clipboard_set",
        &serde_json::json!({ "text": body.text }).to_string(),
    )
    .await
    .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn clipboard_read(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    enqueue_command(&state, &id, "clipboard_get", "")
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn clipboard_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let text = String::from_utf8_lossy(&body).to_string();
    sqlx::query("UPDATE assist_sessions SET clipboard = ?, updated_at = ? WHERE id = ?")
        .bind(&text)
        .bind(Utc::now())
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn clipboard_show(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let text: Option<String> =
        sqlx::query_scalar("SELECT clipboard FROM assist_sessions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    Ok(Json(
        serde_json::json!({ "text": text.unwrap_or_default() }),
    ))
}

// ---------------------------------------------------------------- one-to-one files

async fn create_fs_request(
    state: &AppState,
    session_id: &str,
    kind: &str,
    path: &str,
    file_name: &str,
    payload: &str,
) -> Result<String, (StatusCode, String)> {
    let (mac, _sess_kind, sess_state) = load_session(state, session_id).await?;
    if sess_state == "ended" || sess_state == "denied" {
        return Err((StatusCode::CONFLICT, "session not active".into()));
    }
    let id = new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO assist_file_requests \
         (id, session_id, mac, kind, path, state, file_name, size_bytes, payload, error_message, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'pending', ?, 0, ?, '', ?, ?)",
    )
    .bind(&id)
    .bind(session_id)
    .bind(&mac)
    .bind(kind)
    .bind(path)
    .bind(file_name)
    .bind(payload)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let cmd_payload = serde_json::json!({
        "request_id": id,
        "path": path,
    })
    .to_string();
    let cmd_kind = match kind {
        "list" => "fs_list",
        "download" => "fs_download",
        _ => "fs_upload",
    };
    enqueue_command(state, session_id, cmd_kind, &cmd_payload)
        .await
        .map_err(internal)?;
    Ok(id)
}

fn request_row(row: &sqlx::sqlite::SqliteRow) -> AssistFileRequest {
    AssistFileRequest {
        id: row.get("id"),
        session_id: row.get("session_id"),
        kind: row.get("kind"),
        path: row.get("path"),
        state: row.get("state"),
        file_name: row.get("file_name"),
        size_bytes: row.get("size_bytes"),
        payload: row.get("payload"),
        error_message: row.get("error_message"),
    }
}

pub async fn fs_list(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PathBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rid = create_fs_request(&state, &id, "list", &body.path, "", "").await?;
    Ok(Json(serde_json::json!({ "request_id": rid })))
}

pub async fn fs_download(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PathBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = std::path::Path::new(&body.path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let rid = create_fs_request(&state, &id, "download", &body.path, &name, "").await?;
    Ok(Json(serde_json::json!({ "request_id": rid })))
}

pub async fn fs_upload(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<FsUploadQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.len() > 512 * 1024 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "file too large".into()));
    }
    let (mac, _kind, sess_state) = load_session(&state, &id).await?;
    if sess_state == "ended" || sess_state == "denied" {
        return Err((StatusCode::CONFLICT, "session not active".into()));
    }
    let request_id = new_id();
    let target = q.path.trim();
    if target.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path required".into()));
    }
    let file_name = q
        .file_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::path::Path::new(target)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "upload.bin".into());
    let temp_path = state.assist_fs_dir.join(&request_id);
    tokio::fs::write(&temp_path, &body)
        .await
        .map_err(internal)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO assist_file_requests \
         (id, session_id, mac, kind, path, state, file_name, size_bytes, payload, error_message, created_at, updated_at) \
         VALUES (?, ?, ?, 'upload', ?, 'pending', ?, ?, '', '', ?, ?)",
    )
    .bind(&request_id)
    .bind(&id)
    .bind(&mac)
    .bind(target)
    .bind(&file_name)
    .bind(body.len() as i64)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let download_url = format!("/api/v1/assist/fs/{request_id}/data");
    let payload = serde_json::json!({
        "request_id": request_id,
        "path": target,
        "download_url": download_url,
    })
    .to_string();
    enqueue_command(&state, &id, "fs_upload", &payload)
        .await
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "request_id": request_id })))
}

#[derive(Debug, Deserialize)]
pub struct FsUploadQuery {
    pub path: String,
    #[serde(default)]
    pub file_name: Option<String>,
}

pub async fn fs_status(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<AssistFileRequest>, (StatusCode, String)> {
    let row = sqlx::query(
        "SELECT id, session_id, kind, path, state, file_name, size_bytes, payload, error_message \
         FROM assist_file_requests WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "file request not found".into()))?;
    Ok(Json(request_row(&row)))
}

pub async fn fs_data_get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let row = sqlx::query("SELECT file_name, state, kind FROM assist_file_requests WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "file request not found".into()))?;
    let file_name: String = row.get("file_name");
    let kind: String = row.get("kind");
    if kind == "download" && row.get::<String, _>("state") != "done" {
        return Err((StatusCode::CONFLICT, "file not ready".into()));
    }
    let bytes = tokio::fs::read(state.assist_fs_dir.join(&id))
        .await
        .map_err(internal)?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    let disp = format!("attachment; filename=\"{}\"", file_name.replace('"', ""));
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&disp)
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

pub async fn fs_data_put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.len() > 512 * 1024 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "file too large".into()));
    }
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT kind FROM assist_file_requests WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some((kind,)) = exists else {
        return Err((StatusCode::NOT_FOUND, "file request not found".into()));
    };
    if kind != "download" {
        return Err((StatusCode::BAD_REQUEST, "not a download request".into()));
    }
    tokio::fs::write(state.assist_fs_dir.join(&id), &body)
        .await
        .map_err(internal)?;
    sqlx::query(
        "UPDATE assist_file_requests SET state = 'done', size_bytes = ?, updated_at = ? WHERE id = ?",
    )
    .bind(body.len() as i64)
    .bind(Utc::now())
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn fs_result_put(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    let entries = body
        .get("entries")
        .map(|v| v.to_string())
        .unwrap_or_default();
    let error = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    sqlx::query(
        "UPDATE assist_file_requests SET state = ?, payload = ?, error_message = ?, updated_at = ? WHERE id = ?",
    )
    .bind(if ok { "done" } else { "failed" })
    .bind(&entries)
    .bind(&error)
    .bind(Utc::now())
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------- batch file tasks

#[derive(Debug, Deserialize)]
pub struct CreateFileCollect {
    pub macs: Vec<String>,
    pub source_dir: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct FileDistributeQuery {
    pub target_dir: String,
    /// Comma-separated MACs.
    pub macs: String,
    #[serde(default)]
    pub name: String,
}

pub async fn create_file_collect(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateFileCollect>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    user.require_write()?;
    if body.macs.is_empty() || body.source_dir.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "macs and source_dir required".into(),
        ));
    }
    let macs = normalize_macs(&body.macs)?;
    let now = Utc::now();
    let task_id = new_id();
    let name = if body.name.trim().is_empty() {
        format!("收集文件-{}", now.format("%H%M%S"))
    } else {
        body.name.clone()
    };
    sqlx::query(
        "INSERT INTO maintenance_tasks \
         (id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, payload, created_at, updated_at) \
         VALUES (?, 'file_collect', 'running', NULL, NULL, 'none', ?, '', ?, ?, ?, ?)",
    )
    .bind(&task_id)
    .bind(&name)
    .bind(&user.0.username)
    .bind(serde_json::json!({ "source_dir": body.source_dir }).to_string())
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    insert_file_targets(&state, &task_id, &macs, now).await?;
    for mac in &macs {
        let mac_path = mac.replace(':', "%3A");
        let payload = serde_json::json!({
            "task_id": task_id,
            "source_dir": body.source_dir,
            "upload_url": format!("/api/v1/file-tasks/{task_id}/{mac_path}/collect"),
        })
        .to_string();
        let _ =
            super::agent_commands::enqueue(&state, mac, "fs_collect", &payload, &user.0.username)
                .await;
    }
    Ok(Json(
        serde_json::json!({ "task_id": task_id, "targets": macs.len() }),
    ))
}

pub async fn create_file_distribute(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<FileDistributeQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    user.require_write()?;
    if body.len() < 4 || body.len() > 512 * 1024 * 1024 {
        return Err((StatusCode::BAD_REQUEST, "zip size invalid".into()));
    }
    if q.target_dir.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "target_dir required".into()));
    }
    let macs = normalize_macs(
        &q.macs
            .split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>(),
    )?;
    if macs.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "macs required".into()));
    }
    let now = Utc::now();
    let task_id = new_id();
    let task_dir = state
        .images_dir
        .parent()
        .unwrap_or(&state.images_dir)
        .join("file-tasks")
        .join(&task_id);
    tokio::fs::create_dir_all(&task_dir)
        .await
        .map_err(internal)?;
    tokio::fs::write(task_dir.join("bundle.zip"), &body)
        .await
        .map_err(internal)?;
    let name = if q.name.trim().is_empty() {
        format!("下发文件-{}", now.format("%H%M%S"))
    } else {
        q.name.clone()
    };
    sqlx::query(
        "INSERT INTO maintenance_tasks \
         (id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, payload, created_at, updated_at) \
         VALUES (?, 'file_distribute', 'running', NULL, NULL, 'none', ?, '', ?, ?, ?, ?)",
    )
    .bind(&task_id)
    .bind(&name)
    .bind(&user.0.username)
    .bind(serde_json::json!({ "target_dir": q.target_dir }).to_string())
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    insert_file_targets(&state, &task_id, &macs, now).await?;
    for mac in &macs {
        let payload = serde_json::json!({
            "task_id": task_id,
            "target_dir": q.target_dir,
            "download_url": format!("/api/v1/file-tasks/{task_id}/bundle"),
        })
        .to_string();
        let _ = super::agent_commands::enqueue(
            &state,
            mac,
            "fs_distribute",
            &payload,
            &user.0.username,
        )
        .await;
    }
    Ok(Json(
        serde_json::json!({ "task_id": task_id, "targets": macs.len() }),
    ))
}

fn normalize_macs(raw: &[String]) -> Result<Vec<String>, (StatusCode, String)> {
    let mut out = Vec::new();
    for mac in raw {
        if mac.trim().is_empty() {
            continue;
        }
        out.push(crate::mac::normalize_mac(mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

async fn insert_file_targets(
    state: &AppState,
    task_id: &str,
    macs: &[String],
    now: chrono::DateTime<Utc>,
) -> Result<(), (StatusCode, String)> {
    for mac in macs {
        sqlx::query(
            "INSERT INTO maintenance_task_targets \
             (id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at) \
             VALUES (?, ?, ?, 'pending', 0, '', 'waiting', 0, 0, ?)",
        )
        .bind(new_id())
        .bind(task_id)
        .bind(mac)
        .bind(now)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }
    Ok(())
}

pub async fn file_task_bundle(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let task_dir = state
        .images_dir
        .parent()
        .unwrap_or(&state.images_dir)
        .join("file-tasks")
        .join(&task_id);
    let bytes = tokio::fs::read(task_dir.join("bundle.zip"))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "bundle not found".into()))?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/zip"),
    );
    Ok(resp)
}

pub async fn collect_upload(
    State(state): State<AppState>,
    Path((task_id, mac)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.len() < 4 || body.len() > 512 * 1024 * 1024 {
        return Err((StatusCode::BAD_REQUEST, "zip size invalid".into()));
    }
    let task_dir = state
        .images_dir
        .parent()
        .unwrap_or(&state.images_dir)
        .join("file-tasks")
        .join(&task_id);
    tokio::fs::create_dir_all(&task_dir)
        .await
        .map_err(internal)?;
    let mac_norm = crate::mac::normalize_mac(&mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let mac_file = mac_norm.replace(':', "-");
    tokio::fs::write(task_dir.join(format!("{mac_file}.zip")), &body)
        .await
        .map_err(internal)?;
    sqlx::query(
        "UPDATE maintenance_task_targets SET state = 'done', progress = 1.0, detail = ?, updated_at = ? \
         WHERE task_id = ? AND mac = ?",
    )
    .bind(format!("collected {} bytes", body.len()))
    .bind(Utc::now())
    .bind(&task_id)
    .bind(&mac_norm)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::tasks::refresh_task_aggregate(&state, &task_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn file_target_result(
    State(state): State<AppState>,
    Path((task_id, mac)): Path<(String, String)>,
    Json(body): Json<ReportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mac_norm = crate::mac::normalize_mac(&mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    sqlx::query(
        "UPDATE maintenance_task_targets SET state = ?, progress = ?, error_message = ?, detail = ?, updated_at = ? \
         WHERE task_id = ? AND mac = ?",
    )
    .bind(if body.ok { "done" } else { "failed" })
    .bind(if body.ok { 1.0 } else { 0.0 })
    .bind(if body.ok { "" } else { &body.detail })
    .bind(&body.detail)
    .bind(Utc::now())
    .bind(&task_id)
    .bind(&mac_norm)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::tasks::refresh_task_aggregate(&state, &task_id).await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn collect_download(
    State(state): State<AppState>,
    _user: AuthUser,
    Path((task_id, mac)): Path<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    let task_dir = state
        .images_dir
        .parent()
        .unwrap_or(&state.images_dir)
        .join("file-tasks")
        .join(&task_id);
    let mac_file = mac.replace(':', "-");
    let bytes = tokio::fs::read(task_dir.join(format!("{mac_file}.zip")))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "collected file not found".into()))?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/zip"),
    );
    Ok(resp)
}

// ---------------------------------------------------------------- websocket

#[derive(Debug, Deserialize)]
pub struct AssistWsQuery {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub mac: String,
    #[serde(default)]
    pub token: String,
}

/// Multiplexed Assist transport: one Agent side and N viewer sides per session.
pub async fn ws(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<AssistWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let (mac, _kind, session_state) = load_session(&state, &id).await?;
    if session_state == "ended" || session_state == "denied" {
        return Err((StatusCode::CONFLICT, "assist session not active".into()));
    }
    let role = q.role.trim().to_ascii_lowercase();
    if role == "viewer" {
        let admin = admin_from_token(&state.pool, q.token.trim())
            .await
            .ok_or((StatusCode::UNAUTHORIZED, "invalid assist token".into()))?;
        if admin.role == "readonly" {
            // readonly can watch, but not control
        }
    } else if role == "agent" {
        if q.mac.trim().is_empty() || !q.mac.eq_ignore_ascii_case(&mac) {
            return Err((StatusCode::FORBIDDEN, "assist mac mismatch".into()));
        }
    } else {
        return Err((StatusCode::BAD_REQUEST, "role must be agent|viewer".into()));
    }
    let hub = state.assist_hub.clone();
    Ok(ws
        .on_upgrade(move |socket| async move {
            if role == "agent" {
                handle_agent_ws(socket, hub, id).await;
            } else {
                handle_viewer_ws(socket, hub, id).await;
            }
        })
        .into_response())
}

async fn handle_agent_ws(socket: WebSocket, hub: std::sync::Arc<crate::state::AssistHub>, id: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<AssistWire>(64);
    hub.set_agent(&id, tx).await;
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(out) = out else { break };
                let msg = match out {
                    AssistWire::Text(s) => Message::Text(s.into()),
                    AssistWire::Binary(b) => Message::Binary(b.into()),
                };
                if sink.send(msg).await.is_err() { break; }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => hub.broadcast(&id, AssistWire::Text(text.to_string())).await,
                    Some(Ok(Message::Binary(bytes))) => hub.broadcast(&id, AssistWire::Binary(bytes.to_vec())).await,
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
            }
        }
    }
    hub.clear_agent(&id).await;
}

async fn handle_viewer_ws(socket: WebSocket, hub: std::sync::Arc<crate::state::AssistHub>, id: String) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = hub.subscribe(&id).await;
    loop {
        tokio::select! {
            out = rx.recv() => {
                match out {
                    Ok(AssistWire::Text(s)) => {
                        if sink.send(Message::Text(s.into())).await.is_err() { break; }
                    }
                    Ok(AssistWire::Binary(b)) => {
                        if sink.send(Message::Binary(b.into())).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let _ = hub.send_agent(&id, AssistWire::Text(text.to_string())).await;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let _ = hub.send_agent(&id, AssistWire::Binary(bytes.to_vec())).await;
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
