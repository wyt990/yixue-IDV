use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::{AuthUser, create_session};
use crate::db::{hash_password, verify_password};
use crate::models::{Admin, LoginRequest, LoginResponse, UpdateMe};
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

pub async fn update_me(
    State(state): State<AppState>,
    AuthUser(admin): AuthUser,
    Json(body): Json<UpdateMe>,
) -> Result<Json<Admin>, (StatusCode, String)> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "请填写用户名".into()));
    }
    if !body.new_password.is_empty() {
        if body.new_password.len() < 6 {
            return Err((StatusCode::BAD_REQUEST, "新密码至少 6 位".into()));
        }
        if !verify_password(&body.current_password, &admin.password_hash) {
            return Err((StatusCode::BAD_REQUEST, "当前密码不正确".into()));
        }
    }
    let display_name = if body.display_name.trim().is_empty() {
        username.to_string()
    } else {
        body.display_name.trim().to_string()
    };
    let password_hash = if body.new_password.is_empty() {
        admin.password_hash.clone()
    } else {
        hash_password(&body.new_password).map_err(internal)?
    };
    sqlx::query(
        "UPDATE admins SET username = ?, password_hash = ?, display_name = ? WHERE id = ?",
    )
    .bind(username)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(&admin.id)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "用户名已存在".into())
        } else {
            internal(err)
        }
    })?;
    let updated = sqlx::query_as::<_, Admin>(
        "SELECT id, username, password_hash, display_name, role, created_at FROM admins WHERE id = ?",
    )
    .bind(&admin.id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(updated))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
