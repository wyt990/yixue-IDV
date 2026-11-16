//! Instant agent command channel (long-poll push).
//!
//! Center queues commands per MAC; the persistent agent holds
//! `GET /api/v1/agent/commands/next?mac=..&wait=20` open and receives them
//! within ~1s of being queued. Heartbeat remains the fallback/online signal.

use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AgentCommand {
    pub id: String,
    pub kind: String,
    pub payload: String,
}

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    pub mac: String,
    #[serde(default = "default_wait")]
    pub wait: u64,
}

fn default_wait() -> u64 {
    20
}

#[derive(Debug, Deserialize)]
pub struct ReportBody {
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommandBody {
    pub kind: String,
    #[serde(default)]
    pub payload: String,
}

/// Queue a command for one client (wakes any long-poll waiter).
pub async fn enqueue(
    state: &AppState,
    mac: &str,
    kind: &str,
    payload: &str,
    created_by: &str,
) -> anyhow::Result<String> {
    let id = new_id();
    sqlx::query(
        "INSERT INTO agent_commands (id, mac, kind, payload, state, created_by, created_at) VALUES (?, ?, ?, ?, 'pending', ?, ?)",
    )
    .bind(&id)
    .bind(mac)
    .bind(kind)
    .bind(payload)
    .bind(created_by)
    .bind(Utc::now())
    .execute(&state.pool)
    .await?;
    state.agent_notify.notify_waiters();
    Ok(id)
}

async fn claim(state: &AppState, mac: &str) -> anyhow::Result<Option<AgentCommand>> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, AgentCommand>(
        "SELECT id, kind, payload FROM agent_commands WHERE mac = ? AND state = 'pending' ORDER BY created_at ASC LIMIT 1",
    )
    .bind(mac)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(cmd) = row else {
        tx.rollback().await?;
        return Ok(None);
    };
    sqlx::query("UPDATE agent_commands SET state = 'delivered', delivered_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(&cmd.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(cmd))
}

/// Long-poll for the next command. Returns `null` after `wait` seconds.
pub async fn next(
    State(state): State<AppState>,
    Query(q): Query<NextQuery>,
) -> Result<Json<Option<AgentCommand>>, (StatusCode, String)> {
    let mac = normalize_mac(&q.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let wait = q.wait.min(30);
    let deadline = Instant::now() + Duration::from_secs(wait);
    loop {
        // Register the wake future *before* checking the table so a command
        // enqueued between the check and the await cannot be missed.
        let notified = state.agent_notify.notified();
        match claim(&state, &mac).await {
            Ok(Some(cmd)) => return Ok(Json(Some(cmd))),
            Ok(None) => {}
            Err(err) => return Err(internal(err)),
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(Json(None));
        }
        let remaining = deadline - now;
        let _ = tokio::time::timeout(remaining, notified).await;
    }
}

/// Agent acknowledges command execution.
pub async fn report(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let state_name = if body.ok { "done" } else { "failed" };
    let meta: Option<(String, String)> =
        sqlx::query_as("SELECT mac, kind FROM agent_commands WHERE id = ?")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    sqlx::query(
        "UPDATE agent_commands SET state = ?, finished_at = ?, error_message = ? WHERE id = ?",
    )
    .bind(state_name)
    .bind(Utc::now())
    .bind(&body.detail)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if let Some((mac, kind)) = &meta
        && kind == "update_agent"
    {
        let progress = if body.ok && body.detail.contains("update_scheduled") {
            0.5
        } else if body.detail.contains("reboot_for_update") {
            0.2
        } else {
            0.1
        };
        let _ = super::tasks::update_agent_progress(&state, mac, progress, &body.detail).await;
    }
    // The client is about to reboot to disarm restore; pre-queue the retry so
    // the agent can consume it before its first post-boot heartbeat re-arms.
    if !body.ok
        && body.detail.contains("reboot_for_update")
        && let Some((mac, _)) = &meta
    {
        let _ = super::tasks::requeue_agent_update(&state, mac).await;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

const ALLOWED_KINDS: &[&str] = &[
    "apply_policy",
    "sync_policy",
    "reboot",
    "shutdown",
    "open_url",
    "run_program",
    "assist_ack",
    "update_agent",
];

/// Admin/API: queue a command for a MAC (used by UI/automation).
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Path(mac_raw): Path<String>,
    Json(body): Json<CreateCommandBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    user.require_write()?;
    let kind = body.kind.trim();
    if !ALLOWED_KINDS.contains(&kind) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("unsupported command kind: {kind}"),
        ));
    }
    let mac = normalize_mac(&mac_raw).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let id = enqueue(&state, &mac, kind, &body.payload, &user.0.username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
