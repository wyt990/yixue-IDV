use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::db::hash_password;
use crate::models::{Admin, CreateAdmin, UpdateAdmin, new_id};
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
            "请填写用户名，密码至少 6 位".into(),
        ));
    }
    let role = normalize_role(&body.role)?;
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
            (StatusCode::CONFLICT, "用户名已存在".into())
        } else {
            internal(err)
        }
    })?;
    Ok((StatusCode::CREATED, Json(admin)))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateAdmin>,
) -> Result<Json<Admin>, (StatusCode, String)> {
    user.require_super()?;
    let username = body.username.trim();
    if username.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "请填写用户名".into()));
    }
    let role = normalize_role(&body.role)?;
    if !body.password.is_empty() && body.password.len() < 6 {
        return Err((StatusCode::BAD_REQUEST, "密码至少 6 位".into()));
    }
    let existing = load_admin(&state, &id).await?;
    ensure_super_remains(&state, &existing, &role).await?;
    let display_name = if body.display_name.trim().is_empty() {
        username.to_string()
    } else {
        body.display_name.trim().to_string()
    };
    let password_hash = if body.password.is_empty() {
        existing.password_hash.clone()
    } else {
        hash_password(&body.password).map_err(internal)?
    };
    sqlx::query(
        "UPDATE admins SET username = ?, password_hash = ?, display_name = ?, role = ? WHERE id = ?",
    )
    .bind(username)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(&role)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(unique_or_internal)?;
    load_admin(&state, &id).await.map(Json)
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_super()?;
    if user.0.id == id {
        return Err((
            StatusCode::BAD_REQUEST,
            "不能删除当前登录的账号".into(),
        ));
    }
    let existing = load_admin(&state, &id).await?;
    ensure_super_remains(&state, &existing, "lab_admin").await?;
    sqlx::query("DELETE FROM sessions WHERE admin_id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM admins WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn normalize_role(raw: &str) -> Result<String, (StatusCode, String)> {
    let role = raw.trim().to_ascii_lowercase();
    if matches!(role.as_str(), "super" | "lab_admin" | "readonly") {
        Ok(role)
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "角色只能是超级管理员、机房管理员或只读".into(),
        ))
    }
}

async fn load_admin(state: &AppState, id: &str) -> Result<Admin, (StatusCode, String)> {
    sqlx::query_as::<_, Admin>(
        "SELECT id, username, password_hash, display_name, role, created_at FROM admins WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "管理员不存在".into()))
}

async fn ensure_super_remains(
    state: &AppState,
    existing: &Admin,
    next_role: &str,
) -> Result<(), (StatusCode, String)> {
    if existing.role == "super" && next_role != "super" {
        let supers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins WHERE role = 'super'")
            .fetch_one(&state.pool)
            .await
            .map_err(internal)?;
        if supers <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                "至少保留一名超级管理员".into(),
            ));
        }
    }
    Ok(())
}

fn unique_or_internal(err: sqlx::Error) -> (StatusCode, String) {
    if err.to_string().contains("UNIQUE") {
        (StatusCode::CONFLICT, "用户名已存在".into())
    } else {
        internal(err)
    }
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
