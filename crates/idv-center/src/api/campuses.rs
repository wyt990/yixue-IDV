use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::models::{Campus, CreateCampus, UpdateCampus, new_id};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<Campus>>, (StatusCode, String)> {
    let rows =
        sqlx::query_as::<_, Campus>("SELECT id, name, created_at FROM campuses ORDER BY name")
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Campus>, (StatusCode, String)> {
    sqlx::query_as::<_, Campus>("SELECT id, name, created_at FROM campuses WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "campus not found".into()))
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
        created_at: Utc::now(),
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
    sqlx::query_as::<_, Campus>("SELECT id, name, created_at FROM campuses WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "campus not found".into()))
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
