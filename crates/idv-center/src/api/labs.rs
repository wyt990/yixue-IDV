use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::lab_naming::LAB_COLS;
use crate::models::{CreateLab, Lab, UpdateLab, new_id};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LabFilter {
    pub campus_id: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(filter): Query<LabFilter>,
) -> Result<Json<Vec<Lab>>, (StatusCode, String)> {
    let rows = if let Some(campus_id) = filter.campus_id {
        let sql = format!("SELECT {LAB_COLS} FROM labs WHERE campus_id = ? ORDER BY name");
        sqlx::query_as::<_, Lab>(&sql)
            .bind(campus_id)
            .fetch_all(&state.pool)
            .await
    } else {
        let sql = format!("SELECT {LAB_COLS} FROM labs ORDER BY name");
        sqlx::query_as::<_, Lab>(&sql)
            .fetch_all(&state.pool)
            .await
    }
    .map_err(internal)?;
    let mut rows = rows;
    crate::site_defaults::attach_lab_edges(&state.pool, &mut rows).await?;
    Ok(Json(rows))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Lab>, (StatusCode, String)> {
    let mut lab = crate::lab_naming::load_lab(&state, &id).await?;
    lab.edge_ids = crate::site_defaults::load_lab_edge_ids(&state.pool, &id).await?;
    Ok(Json(lab))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateLab>,
) -> Result<(StatusCode, Json<Lab>), (StatusCode, String)> {
    user.require_write()?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let width = body.suffix_width.clamp(1, 8);
    let start = body.suffix_start.max(1);
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM campuses WHERE id = ?")
        .bind(&body.campus_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        return Err((StatusCode::BAD_REQUEST, "campus not found".into()));
    }

    let lab = Lab {
        id: new_id(),
        campus_id: body.campus_id,
        name: name.to_string(),
        name_prefix: body.name_prefix.trim().to_string(),
        suffix_width: width,
        suffix_start: start,
        next_suffix: start,
        ip_start: body.ip_start.trim().to_string(),
        ip_end: body.ip_end.trim().to_string(),
        default_image_id: None,
        created_at: Utc::now(),
        edge_ids: Vec::new(),
    };
    sqlx::query(
        "INSERT INTO labs (id, campus_id, name, name_prefix, suffix_width, suffix_start, next_suffix, ip_start, ip_end, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&lab.id)
    .bind(&lab.campus_id)
    .bind(&lab.name)
    .bind(&lab.name_prefix)
    .bind(lab.suffix_width)
    .bind(lab.suffix_start)
    .bind(lab.next_suffix)
    .bind(&lab.ip_start)
    .bind(&lab.ip_end)
    .bind(lab.created_at)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                "lab name already exists in campus".into(),
            )
        } else {
            internal(err)
        }
    })?;
    Ok((StatusCode::CREATED, Json(lab)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateLab>,
) -> Result<Json<Lab>, (StatusCode, String)> {
    user.require_write()?;
    let existing = crate::lab_naming::load_lab(&state, &id).await?;
    let campus_id = body
        .campus_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(existing.campus_id);
    let name = body
        .name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(existing.name);
    let name_prefix = body
        .name_prefix
        .map(|s| s.trim().to_string())
        .unwrap_or(existing.name_prefix);
    let suffix_width = body
        .suffix_width
        .unwrap_or(existing.suffix_width)
        .clamp(1, 8);
    let suffix_start = body.suffix_start.unwrap_or(existing.suffix_start).max(1);
    let next_suffix = body
        .next_suffix
        .unwrap_or(existing.next_suffix)
        .max(suffix_start);
    let ip_start = body
        .ip_start
        .map(|s| s.trim().to_string())
        .unwrap_or(existing.ip_start);
    let ip_end = body
        .ip_end
        .map(|s| s.trim().to_string())
        .unwrap_or(existing.ip_end);

    let campus_ok: Option<(String,)> = sqlx::query_as("SELECT id FROM campuses WHERE id = ?")
        .bind(&campus_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if campus_ok.is_none() {
        return Err((StatusCode::BAD_REQUEST, "campus not found".into()));
    }
    sqlx::query(
        "UPDATE labs SET campus_id = ?, name = ?, name_prefix = ?, suffix_width = ?, suffix_start = ?, next_suffix = ?, ip_start = ?, ip_end = ? WHERE id = ?",
    )
    .bind(&campus_id)
    .bind(&name)
    .bind(&name_prefix)
    .bind(suffix_width)
    .bind(suffix_start)
    .bind(next_suffix)
    .bind(&ip_start)
    .bind(&ip_end)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (
                StatusCode::CONFLICT,
                "lab name already exists in campus".into(),
            )
        } else {
            internal(err)
        }
    })?;

    if let Some(edge_ids) = body.edge_ids {
        let image_id =
            crate::site_defaults::normalize_default_image_id(&state.pool, body.default_image_id)
                .await?;
        sqlx::query("UPDATE labs SET default_image_id = ? WHERE id = ?")
            .bind(&image_id)
            .bind(&id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        crate::site_defaults::replace_lab_edges(&state.pool, &id, &edge_ids).await?;
    }

    get(State(state), user, Path(id)).await
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    let result = sqlx::query("DELETE FROM labs WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "lab not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
