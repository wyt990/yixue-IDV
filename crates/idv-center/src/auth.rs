use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use chrono::{Duration, Utc};
use sqlx::SqlitePool;

use crate::models::{Admin, Edge, new_id};
use crate::state::AppState;

pub async fn create_session(pool: &SqlitePool, admin_id: &str) -> anyhow::Result<String> {
    let token = new_id();
    let expires = Utc::now() + Duration::days(7);
    sqlx::query("INSERT INTO sessions (token, admin_id, expires_at) VALUES (?, ?, ?)")
        .bind(&token)
        .bind(admin_id)
        .bind(expires)
        .execute(pool)
        .await?;
    Ok(token)
}

pub async fn admin_from_token(pool: &SqlitePool, token: &str) -> Option<Admin> {
    sqlx::query_as::<_, Admin>(
        r#"
        SELECT a.id, a.username, a.password_hash, a.display_name, a.role, a.created_at
        FROM sessions s
        JOIN admins a ON a.id = s.admin_id
        WHERE s.token = ? AND s.expires_at > ?
        "#,
    )
    .bind(token)
    .bind(Utc::now())
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

pub struct AuthUser(pub Admin);

impl AuthUser {
    pub fn require_write(&self) -> Result<(), (StatusCode, String)> {
        if self.0.role == "readonly" {
            Err((
                StatusCode::FORBIDDEN,
                "readonly role cannot mutate".into(),
            ))
        } else {
            Ok(())
        }
    }

    pub fn require_super(&self) -> Result<(), (StatusCode, String)> {
        if self.0.role != "super" {
            Err((StatusCode::FORBIDDEN, "super role required".into()))
        } else {
            Ok(())
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".into()))?;
        let admin = admin_from_token(&state.pool, &token).await.ok_or((
            StatusCode::UNAUTHORIZED,
            "invalid or expired session".into(),
        ))?;
        Ok(AuthUser(admin))
    }
}

pub fn edge_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-idv-edge-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn edge_from_token(pool: &SqlitePool, token: &str) -> Option<Edge> {
    sqlx::query_as::<_, Edge>(
        "SELECT id, name, node_token, listen_url, last_seen, cache_used_bytes, cache_quota_bytes, cache_warn_pct, cache_level, capture_tmp_dir, created_at FROM edges WHERE node_token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub struct AuthEdge(pub Edge);

impl FromRequestParts<AppState> for AuthEdge {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = edge_token(&parts.headers).ok_or((
            StatusCode::UNAUTHORIZED,
            "missing X-Idv-Edge-Token".into(),
        ))?;
        let edge = edge_from_token(&state.pool, &token)
            .await
            .ok_or((StatusCode::UNAUTHORIZED, "invalid edge token".into()))?;
        Ok(AuthEdge(edge))
    }
}
