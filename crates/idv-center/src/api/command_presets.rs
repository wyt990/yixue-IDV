//! Reusable remote command presets shown in the terminal page's command menu.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CommandPreset {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub command: String,
    pub url: String,
    pub program_path: String,
    pub args: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PresetBody {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub program_path: String,
    #[serde(default)]
    pub args: String,
}

const COLS: &str = "id, kind, name, command, url, program_path, args, created_at, updated_at";

fn validate(body: &PresetBody) -> Result<String, (StatusCode, String)> {
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "command" | "url" | "program") {
        return Err((
            StatusCode::BAD_REQUEST,
            "kind must be command|url|program".into(),
        ));
    }
    if body.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    match kind.as_str() {
        "command" if body.command.trim().is_empty() => {
            Err((StatusCode::BAD_REQUEST, "command required".into()))
        }
        "url" if body.url.trim().is_empty() => {
            Err((StatusCode::BAD_REQUEST, "url required".into()))
        }
        "program" if body.program_path.trim().is_empty() => Err((
            StatusCode::BAD_REQUEST,
            "program_path required".into(),
        )),
        _ => Ok(kind),
    }
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<CommandPreset>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, CommandPreset>(&format!(
        "SELECT {COLS} FROM command_presets ORDER BY kind, name"
    ))
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<PresetBody>,
) -> Result<(StatusCode, Json<CommandPreset>), (StatusCode, String)> {
    user.require_write()?;
    let kind = validate(&body)?;
    let now = Utc::now().to_rfc3339();
    let row = CommandPreset {
        id: new_id(),
        kind,
        name: body.name.trim().to_string(),
        command: body.command.trim().to_string(),
        url: body.url.trim().to_string(),
        program_path: body.program_path.trim().to_string(),
        args: body.args.trim().to_string(),
        created_at: now.clone(),
        updated_at: now,
    };
    sqlx::query(&format!(
        "INSERT INTO command_presets ({COLS}, created_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    ))
    .bind(&row.id)
    .bind(&row.kind)
    .bind(&row.name)
    .bind(&row.command)
    .bind(&row.url)
    .bind(&row.program_path)
    .bind(&row.args)
    .bind(&row.created_at)
    .bind(&row.updated_at)
    .bind(&user.0.username)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<PresetBody>,
) -> Result<Json<CommandPreset>, (StatusCode, String)> {
    user.require_write()?;
    let kind = validate(&body)?;
    let now = Utc::now().to_rfc3339();
    let affected = sqlx::query(
        "UPDATE command_presets SET kind = ?, name = ?, command = ?, url = ?, \
         program_path = ?, args = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&kind)
    .bind(body.name.trim())
    .bind(body.command.trim())
    .bind(body.url.trim())
    .bind(body.program_path.trim())
    .bind(body.args.trim())
    .bind(&now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?
    .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "preset not found".into()));
    }
    let row = sqlx::query_as::<_, CommandPreset>(&format!(
        "SELECT {COLS} FROM command_presets WHERE id = ?"
    ))
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(row))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    sqlx::query("DELETE FROM command_presets WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
