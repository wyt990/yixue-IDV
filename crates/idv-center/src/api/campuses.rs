use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::lab_naming::CAMPUS_COLS;
use crate::models::{Campus, CreateCampus, UpdateCampus, new_id};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<Campus>>, (StatusCode, String)> {
    let sql = format!("SELECT {CAMPUS_COLS} FROM campuses ORDER BY name");
    let mut rows = sqlx::query_as::<_, Campus>(&sql)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    crate::site_defaults::attach_campus_edges(&state.pool, &mut rows).await?;
    Ok(Json(rows))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Campus>, (StatusCode, String)> {
    let sql = format!("SELECT {CAMPUS_COLS} FROM campuses WHERE id = ?");
    let mut campus = sqlx::query_as::<_, Campus>(&sql)
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "campus not found".into()))?;
    campus.edge_ids = crate::site_defaults::load_campus_edge_ids(&state.pool, &id).await?;
    Ok(Json(campus))
}

pub async fn create(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<CreateCampus>,
) -> Result<(StatusCode, Json<Campus>), (StatusCode, String)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let campus = Campus {
        id: new_id(),
        name: name.to_string(),
        default_image_id: None,
        created_at: Utc::now(),
        edge_ids: Vec::new(),
    };
    sqlx::query("INSERT INTO campuses (id, name, created_at) VALUES (?, ?, ?)")
        .bind(&campus.id)
        .bind(&campus.name)
        .bind(campus.created_at)
        .execute(&state.pool)
        .await
        .map_err(|err| {
            if err.to_string().contains("UNIQUE") {
                (StatusCode::CONFLICT, "campus name already exists".into())
            } else {
                internal(err)
            }
        })?;
    Ok((StatusCode::CREATED, Json(campus)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateCampus>,
) -> Result<Json<Campus>, (StatusCode, String)> {
    user.require_write()?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let result = sqlx::query("UPDATE campuses SET name = ? WHERE id = ?")
        .bind(name)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|err| {
            if err.to_string().contains("UNIQUE") {
                (StatusCode::CONFLICT, "campus name already exists".into())
            } else {
                internal(err)
            }
        })?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "campus not found".into()));
    }

    if let Some(edge_ids) = body.edge_ids {
        let image_id =
            crate::site_defaults::normalize_default_image_id(&state.pool, body.default_image_id)
                .await?;
        sqlx::query("UPDATE campuses SET default_image_id = ? WHERE id = ?")
            .bind(&image_id)
            .bind(&id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        crate::site_defaults::replace_campus_edges(&state.pool, &id, &edge_ids).await?;
    }

    get(State(state), user, Path(id)).await
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    let result = sqlx::query("DELETE FROM campuses WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "campus not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
