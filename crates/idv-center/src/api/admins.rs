use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::db::hash_password;
use crate::models::{Admin, CreateAdmin, new_id};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<Admin>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, Admin>(
        "SELECT id, username, password_hash, display_name, role, created_at FROM admins ORDER BY username",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateAdmin>,
) -> Result<(StatusCode, Json<Admin>), (StatusCode, String)> {
    user.require_super()?;
    let username = body.username.trim();
    if username.is_empty() || body.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            "username required and password >= 6 chars".into(),
        ));
    }
    let role = body.role.trim().to_ascii_lowercase();
    if !matches!(role.as_str(), "super" | "lab_admin" | "readonly") {
        return Err((
            StatusCode::BAD_REQUEST,
            "role must be super|lab_admin|readonly".into(),
        ));
    }
    let display_name = if body.display_name.trim().is_empty() {
        username.to_string()
    } else {
        body.display_name.trim().to_string()
    };
    let password_hash = hash_password(&body.password).map_err(internal)?;
    let admin = Admin {
        id: new_id(),
        username: username.to_string(),
        password_hash,
        display_name,
        role,
        created_at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO admins (id, username, password_hash, display_name, role, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&admin.id)
    .bind(&admin.username)
    .bind(&admin.password_hash)
    .bind(&admin.display_name)
    .bind(&admin.role)
    .bind(admin.created_at)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "username already exists".into())
        } else {
            internal(err)
        }
    })?;
    Ok((StatusCode::CREATED, Json(admin)))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
