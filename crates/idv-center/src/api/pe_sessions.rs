use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::auth::{AuthEdge, AuthUser};
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PeSessionRow {
    pub id: String,
    pub edge_id: String,
    pub mac: String,
    pub source: String,
    pub status: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct EdgePeSessionReport {
    pub id: String,
    pub mac: String,
    pub source: String,
    pub status: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct EdgePeSessionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lab_id: Option<String>,
    pub client_created: bool,
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<PeSessionRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, PeSessionRow>(
        "SELECT id, edge_id, mac, source, status, message, created_at, last_seen FROM pe_sessions ORDER BY last_seen DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn upsert_from_edge(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
    Json(body): Json<EdgePeSessionReport>,
) -> Result<Json<EdgePeSessionResponse>, (StatusCode, String)> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM pe_sessions WHERE edge_id = ? AND mac = ?")
            .bind(&edge.id)
            .bind(&body.mac)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;

    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE pe_sessions SET source = ?, status = ?, message = ?, last_seen = ? WHERE id = ?",
        )
        .bind(&body.source)
        .bind(&body.status)
        .bind(&body.message)
        .bind(body.last_seen)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    } else {
        let id = if body.id.trim().is_empty() {
            new_id()
        } else {
            body.id.clone()
        };
        sqlx::query(
            "INSERT INTO pe_sessions (id, edge_id, mac, source, status, message, created_at, last_seen) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&edge.id)
        .bind(&body.mac)
        .bind(&body.source)
        .bind(&body.status)
        .bind(&body.message)
        .bind(body.created_at)
        .bind(body.last_seen)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }

    let note = format!("pe-checkin source={}", body.source);
    let enrolled = crate::enroll::ensure_client_enrolled(
        &state,
        &body.mac,
        &note,
        body.last_seen,
    )
    .await?;

    if let Some(ref e) = enrolled {
        info!(
            mac = %body.mac,
            client_name = %e.client_name,
            created = e.created,
            "PE check-in client enrolled"
        );
    }

    Ok(Json(EdgePeSessionResponse {
        ok: true,
        client_id: enrolled.as_ref().map(|e| e.client_id.clone()),
        client_name: enrolled.as_ref().map(|e| e.client_name.clone()),
        lab_id: enrolled.as_ref().map(|e| e.lab_id.clone()),
        client_created: enrolled.as_ref().map(|e| e.created).unwrap_or(false),
    }))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
