use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::{AuthUser, create_session};
use crate::db::verify_password;
use crate::models::{Admin, LoginRequest, LoginResponse};
use crate::state::AppState;

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let admin = sqlx::query_as::<_, Admin>(
        "SELECT id, username, password_hash, display_name, role, created_at FROM admins WHERE username = ?",
    )
    .bind(&body.username)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;

    let Some(admin) = admin else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid username or password".into(),
        ));
    };
    if !verify_password(&body.password, &admin.password_hash) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid username or password".into(),
        ));
    }

    let token = create_session(&state.pool, &admin.id)
        .await
        .map_err(internal)?;
    Ok(Json(LoginResponse {
        token,
        username: admin.username,
        display_name: admin.display_name,
        role: admin.role,
    }))
}

pub async fn me(AuthUser(admin): AuthUser) -> Json<Admin> {
    Json(admin)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
