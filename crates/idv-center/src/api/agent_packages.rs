//! Agent software packages (self-update).
//!
//! Packages are plain files under `AppState.agent_dir`:
//!   agent-<version>.exe
//!   agent-<version>.sha256
//!
//! Upload is admin-only; download is unauthenticated because the Windows agent
//! has no device token yet. Keep the Center on a trusted management VLAN.

use std::path::PathBuf;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct AgentPackage {
    pub version: String,
    pub size_bytes: u64,
    pub sha256: String,
}

const VERSION_MARKER: &[u8] = b"IDV_AGENT_BUILD_VERSION=";

/// Extract `0.2.0` from the marker embedded by the agent build.
pub fn extract_embedded_version(body: &[u8]) -> Option<String> {
    let start = body
        .windows(VERSION_MARKER.len())
        .position(|w| w == VERSION_MARKER)?
        + VERSION_MARKER.len();
    let mut end = start;
    while end < body.len() && end - start < 64 {
        let b = body[end];
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+') {
            end += 1;
        } else {
            break;
        }
    }
    if end == start {
        return None;
    }
    let version = String::from_utf8_lossy(&body[start..end])
        .trim()
        .to_string();
    if valid_version(&version) {
        Some(version)
    } else {
        None
    }
}

pub fn valid_version(version: &str) -> bool {
    let v = version.trim();
    !v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !v.contains("..")
}

fn package_path(state: &AppState, version: &str) -> PathBuf {
    state.agent_dir.join(format!("agent-{version}.exe"))
}

fn sha_path(state: &AppState, version: &str) -> PathBuf {
    state.agent_dir.join(format!("agent-{version}.sha256"))
}

pub async fn package_meta(state: &AppState, version: &str) -> Option<AgentPackage> {
    let path = package_path(state, version);
    let meta = tokio::fs::metadata(&path).await.ok()?;
    let sha = tokio::fs::read_to_string(sha_path(state, version))
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Some(AgentPackage {
        version: version.to_string(),
        size_bytes: meta.len(),
        sha256: sha,
    })
}

async fn read_package(state: &AppState, version: &str) -> Result<Vec<u8>, (StatusCode, String)> {
    let path = package_path(state, version);
    tokio::fs::read(&path).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("agent package {version} not found"),
        )
    })
}

/// PUT /api/v1/agent/packages with the raw exe body.
pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    body: Bytes,
) -> Result<Json<AgentPackage>, (StatusCode, String)> {
    user.require_write()?;
    if body.len() < 64 * 1024 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("agent package too small ({} bytes)", body.len()),
        ));
    }
    let version = extract_embedded_version(&body).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "无法从 exe 中识别版本号：请使用 0.2.0+ 构建的 idv-agent（内含 IDV_AGENT_BUILD_VERSION 标记）"
                .to_string(),
        )
    })?;
    let sha = hex::encode(Sha256::digest(&body));
    let final_path = package_path(&state, &version);
    let tmp_path = state.agent_dir.join(format!(".upload-{version}.tmp"));
    tokio::fs::write(&tmp_path, &body).await.map_err(internal)?;
    tokio::fs::rename(&tmp_path, &final_path)
        .await
        .map_err(internal)?;
    tokio::fs::write(sha_path(&state, &version), &sha)
        .await
        .map_err(internal)?;
    Ok(Json(AgentPackage {
        version,
        size_bytes: body.len() as u64,
        sha256: sha,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<AgentPackage>>, (StatusCode, String)> {
    let mut out = Vec::new();
    let mut dir = tokio::fs::read_dir(&state.agent_dir)
        .await
        .map_err(internal)?;
    while let Some(entry) = dir.next_entry().await.map_err(internal)? {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(version) = name
            .strip_prefix("agent-")
            .and_then(|n| n.strip_suffix(".exe"))
            && let Some(meta) = package_meta(&state, version).await
        {
            out.push(meta);
        }
    }
    out.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(Json(out))
}

/// GET /api/v1/agent/packages/{version}/download (agent-facing, no token).
pub async fn download(
    State(state): State<AppState>,
    AxumPath(version): AxumPath<String>,
) -> Result<Response, (StatusCode, String)> {
    if !valid_version(&version) {
        return Err((StatusCode::BAD_REQUEST, "invalid version".into()));
    }
    let bytes = read_package(&state, &version).await?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"idv-agent-{version}.exe\""))
            .unwrap_or(header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
