use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssistSession {
    pub id: String,
    pub mac: String,
    pub kind: String,
    pub state: String,
    pub requested_by: String,
    pub detail: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAssist {
    pub mac: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub detail: String,
}

fn default_kind() -> String {
    "view".into()
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<AssistSession>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, AssistSession>(
        "SELECT id, mac, kind, state, requested_by, detail, created_at, updated_at FROM assist_sessions ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateAssist>,
) -> Result<(StatusCode, Json<AssistSession>), (StatusCode, String)> {
    user.require_write()?;
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "view" | "control") {
        return Err((StatusCode::BAD_REQUEST, "kind must be view|control".into()));
    }
    let now = Utc::now().to_rfc3339();
    let row = AssistSession {
        id: new_id(),
        mac,
        kind,
        state: "pending".into(),
        requested_by: user.0.username.clone(),
        detail: body.detail,
        created_at: now.clone(),
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO assist_sessions (id, mac, kind, state, requested_by, detail, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.mac)
    .bind(&row.kind)
    .bind(&row.state)
    .bind(&row.requested_by)
    .bind(&row.detail)
    .bind(&row.created_at)
    .bind(&row.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "assist.create",
        "assist_session",
        &row.id,
        &format!("mac={} kind={}", row.mac, row.kind),
    )
    .await;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Debug, Deserialize)]
pub struct AssistAck {
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

pub async fn ack(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AssistAck>,
) -> Result<Json<AssistSession>, (StatusCode, String)> {
    let state_name = body.state.trim().to_ascii_lowercase();
    if !matches!(
        state_name.as_str(),
        "accepted" | "active" | "ended" | "denied"
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            "state must be accepted|active|ended|denied".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE assist_sessions SET state = ?, detail = ?, updated_at = ? WHERE id = ?")
        .bind(&state_name)
        .bind(&body.detail)
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query_as::<_, AssistSession>(
        "SELECT id, mac, kind, state, requested_by, detail, created_at, updated_at FROM assist_sessions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .map(Json)
    .ok_or((StatusCode::NOT_FOUND, "assist session not found".into()))
}

pub async fn pending_for_mac(
    state: &AppState,
    mac: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, kind, requested_by FROM assist_sessions WHERE mac = ? AND state = 'pending' ORDER BY created_at ASC LIMIT 1",
    )
    .bind(mac)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|(id, kind, requested_by)| {
        serde_json::json!({
            "session_id": id,
            "kind": kind,
            "requested_by": requested_by,
        })
    }))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
