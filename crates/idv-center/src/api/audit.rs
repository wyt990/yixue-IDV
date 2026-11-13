use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::AuthUser;
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AuditEvent {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub action: String,
}

fn default_limit() -> i64 {
    100
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEvent>>, (StatusCode, String)> {
    let limit = q.limit.clamp(1, 500);
    let rows = if q.action.trim().is_empty() {
        sqlx::query_as::<_, AuditEvent>(
            "SELECT id, actor, action, resource_type, resource_id, detail, created_at FROM audit_events ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query_as::<_, AuditEvent>(
            "SELECT id, actor, action, resource_type, resource_id, detail, created_at FROM audit_events WHERE action = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(q.action.trim())
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(rows))
}

pub async fn record(
    pool: &SqlitePool,
    actor: &str,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    detail: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audit_events (id, actor, action, resource_type, resource_id, detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(actor)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(detail)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn export_csv(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Response, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, AuditEvent>(
        "SELECT id, actor, action, resource_type, resource_id, detail, created_at FROM audit_events ORDER BY created_at DESC LIMIT 2000",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut csv = String::from("id,actor,action,resource_type,resource_id,detail,created_at\n");
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            r.id,
            csv_escape(&r.actor),
            csv_escape(&r.action),
            csv_escape(&r.resource_type),
            csv_escape(&r.resource_id),
            csv_escape(&r.detail),
            r.created_at.to_rfc3339()
        ));
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"idv-audit.csv\"",
        )
        .body(Body::from(csv))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
