use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{Duration, Utc};

use crate::auth::AuthUser;
use crate::models::DashboardStats;
use crate::state::AppState;

pub async fn stats(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<DashboardStats>, (StatusCode, String)> {
    let campuses = count(&state, "SELECT COUNT(*) FROM campuses").await?;
    let labs = count(&state, "SELECT COUNT(*) FROM labs").await?;
    let clients = count(&state, "SELECT COUNT(*) FROM clients").await?;
    let images = count(&state, "SELECT COUNT(*) FROM images").await?;
    let cutoff = Utc::now() - Duration::seconds(45);
    let edges_online: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM edges WHERE last_seen IS NOT NULL AND last_seen >= ?",
    )
    .bind(cutoff)
    .fetch_one(&state.pool)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let sync_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM edge_commands WHERE state IN ('pending', 'running')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let maint_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maintenance_tasks WHERE state IN ('pending', 'running')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let tasks_running = sync_running + maint_running;
    Ok(Json(DashboardStats {
        campuses,
        labs,
        clients,
        images,
        edges_online,
        tasks_running,
    }))
}

async fn count(state: &AppState, sql: &str) -> Result<i64, (StatusCode, String)> {
    sqlx::query_scalar(sql)
        .fetch_one(&state.pool)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}
