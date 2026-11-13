use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::models::{
    CreateEdge, Edge, EdgeCommand, EdgeView, ImageReplica, ScaleLimits, SyncImageRequest,
    UpdateEdge, edge_online, new_id,
};
use crate::state::AppState;

const EDGE_COLS: &str = "id, name, node_token, listen_url, last_seen, cache_used_bytes, cache_quota_bytes, cache_warn_pct, cache_level, capture_tmp_dir, created_at";

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<EdgeView>>, (StatusCode, String)> {
    let sql = format!("SELECT {EDGE_COLS} FROM edges ORDER BY name");
    let rows = sqlx::query_as::<_, Edge>(&sql)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|edge| {
                let online = edge_online(edge.last_seen);
                EdgeView { edge, online }
            })
            .collect(),
    ))
}

pub async fn create(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<CreateEdge>,
) -> Result<(StatusCode, Json<Edge>), (StatusCode, String)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let token = body
        .node_token
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(new_id);
    let edge = Edge {
        id: new_id(),
        name: name.to_string(),
        node_token: token,
        listen_url: String::new(),
        last_seen: None,
        cache_used_bytes: 0,
        cache_quota_bytes: 0,
        cache_warn_pct: 85.0,
        cache_level: "ok".into(),
        capture_tmp_dir: String::new(),
        created_at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO edges (id, name, node_token, listen_url, last_seen, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&edge.id)
    .bind(&edge.name)
    .bind(&edge.node_token)
    .bind(&edge.listen_url)
    .bind(edge.last_seen)
    .bind(edge.created_at)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "edge name or token already exists".into())
        } else {
            internal(err)
        }
    })?;
    Ok((StatusCode::CREATED, Json(edge)))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<EdgeView>, (StatusCode, String)> {
    let edge = load_edge(&state, &id).await?;
    let online = edge_online(edge.last_seen);
    Ok(Json(EdgeView { edge, online }))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateEdge>,
) -> Result<Json<EdgeView>, (StatusCode, String)> {
    user.require_write()?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let capture_tmp_dir = body
        .capture_tmp_dir
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let result = sqlx::query("UPDATE edges SET name = ?, capture_tmp_dir = ? WHERE id = ?")
        .bind(name)
        .bind(&capture_tmp_dir)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|err| {
            if err.to_string().contains("UNIQUE") {
                (StatusCode::CONFLICT, "edge name already exists".into())
            } else {
                internal(err)
            }
        })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "edge not found".into()));
    }
    let edge = load_edge(&state, &id).await?;
    let online = edge_online(edge.last_seen);
    Ok(Json(EdgeView { edge, online }))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    let result = sqlx::query("DELETE FROM edges WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "edge not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_replicas(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<ImageReplica>>, (StatusCode, String)> {
    let _ = load_edge(&state, &id).await?;
    let rows = sqlx::query_as::<_, ImageReplica>(
        "SELECT id, edge_id, image_id, state, progress, local_path, checksum_sha256, error_message, updated_at FROM image_replicas WHERE edge_id = ? ORDER BY updated_at DESC",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn scale_limits(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<ScaleLimits>, (StatusCode, String)> {
    Ok(Json(ScaleLimits {
        max_concurrent_deploy_targets: state.max_concurrent_deploy_targets,
        max_pending_sync_per_edge: state.max_pending_sync_per_edge,
        recommended_deploy_batch: state.recommended_deploy_batch,
    }))
}

pub async fn rotate_token(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Edge>, (StatusCode, String)> {
    let _ = load_edge(&state, &id).await?;
    let token = new_id();
    sqlx::query("UPDATE edges SET node_token = ? WHERE id = ?")
        .bind(&token)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let edge = load_edge(&state, &id).await?;
    Ok(Json(edge))
}

pub async fn sync_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<SyncImageRequest>,
) -> Result<(StatusCode, Json<EdgeCommand>), (StatusCode, String)> {
    let _ = load_edge(&state, &id).await?;
    let image_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM images WHERE id = ?")
            .bind(&body.image_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    if image_exists.is_none() {
        return Err((StatusCode::BAD_REQUEST, "image not found".into()));
    }

    let queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM edge_commands WHERE edge_id = ? AND kind IN ('sync','purge_and_sync') AND state IN ('pending','running')",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    if queued >= state.max_pending_sync_per_edge {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "sync queue full for edge (max {})",
                state.max_pending_sync_per_edge
            ),
        ));
    }

    upsert_replica(&state, &id, &body.image_id, "pending", 0.0, "", "", "").await?;

    let kind = if body.purge_first {
        "purge_and_sync"
    } else {
        "sync"
    };
    let cmd = enqueue_command(&state, &id, kind, Some(&body.image_id)).await?;
    Ok((StatusCode::ACCEPTED, Json(cmd)))
}

pub async fn purge_replica(
    State(state): State<AppState>,
    _user: AuthUser,
    Path((id, image_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<EdgeCommand>), (StatusCode, String)> {
    let _ = load_edge(&state, &id).await?;
    upsert_replica(&state, &id, &image_id, "purging", 0.0, "", "", "").await?;
    let cmd = enqueue_command(&state, &id, "purge", Some(&image_id)).await?;
    Ok((StatusCode::ACCEPTED, Json(cmd)))
}

pub async fn purge_all(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<EdgeCommand>), (StatusCode, String)> {
    let _ = load_edge(&state, &id).await?;
    sqlx::query("UPDATE image_replicas SET state = 'purging', updated_at = ? WHERE edge_id = ?")
        .bind(Utc::now())
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let cmd = enqueue_command(&state, &id, "purge_all", None).await?;
    Ok((StatusCode::ACCEPTED, Json(cmd)))
}

async fn load_edge(state: &AppState, id: &str) -> Result<Edge, (StatusCode, String)> {
    let sql = format!("SELECT {EDGE_COLS} FROM edges WHERE id = ?");
    sqlx::query_as::<_, Edge>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "edge not found".into()))
}

async fn enqueue_command(
    state: &AppState,
    edge_id: &str,
    kind: &str,
    image_id: Option<&str>,
) -> Result<EdgeCommand, (StatusCode, String)> {
    let now = Utc::now();
    let cmd = EdgeCommand {
        id: new_id(),
        edge_id: edge_id.to_string(),
        kind: kind.to_string(),
        image_id: image_id.map(str::to_string),
        state: "pending".into(),
        progress: 0.0,
        error_message: String::new(),
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO edge_commands (id, edge_id, kind, image_id, state, progress, error_message, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&cmd.id)
    .bind(&cmd.edge_id)
    .bind(&cmd.kind)
    .bind(&cmd.image_id)
    .bind(&cmd.state)
    .bind(cmd.progress)
    .bind(&cmd.error_message)
    .bind(cmd.created_at)
    .bind(cmd.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(cmd)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_replica(
    state: &AppState,
    edge_id: &str,
    image_id: &str,
    state_name: &str,
    progress: f64,
    local_path: &str,
    checksum: &str,
    error: &str,
) -> Result<(), (StatusCode, String)> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM image_replicas WHERE edge_id = ? AND image_id = ?",
    )
    .bind(edge_id)
    .bind(image_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;

    let now = Utc::now();
    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE image_replicas SET state = ?, progress = ?, local_path = ?, checksum_sha256 = ?, error_message = ?, updated_at = ? WHERE id = ?",
        )
        .bind(state_name)
        .bind(progress)
        .bind(local_path)
        .bind(checksum)
        .bind(error)
        .bind(now)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    } else {
        sqlx::query(
            "INSERT INTO image_replicas (id, edge_id, image_id, state, progress, local_path, checksum_sha256, error_message, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(edge_id)
        .bind(image_id)
        .bind(state_name)
        .bind(progress)
        .bind(local_path)
        .bind(checksum)
        .bind(error)
        .bind(now)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
