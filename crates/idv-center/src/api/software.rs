use std::path::PathBuf;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use idv_common::image_format::hex_sha256;

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SoftwarePackage {
    pub id: String,
    pub name: String,
    pub version_label: String,
    pub checksum_sha256: String,
    pub size_bytes: i64,
    pub storage_uri: String,
    pub notes: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePackageMeta {
    pub name: String,
    #[serde(default)]
    pub version_label: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SoftwareJob {
    pub id: String,
    pub package_id: String,
    pub mac: String,
    pub state: String,
    pub detail: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateSoftwareJob {
    pub package_id: String,
    pub mac: String,
}

pub async fn list_packages(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<SoftwarePackage>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, SoftwarePackage>(
        "SELECT id, name, version_label, checksum_sha256, size_bytes, storage_uri, notes, created_at FROM software_packages ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create_package(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreatePackageMeta>,
) -> Result<(StatusCode, Json<SoftwarePackage>), (StatusCode, String)> {
    user.require_write()?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    let now = Utc::now();
    let pkg = SoftwarePackage {
        id: new_id(),
        name: name.to_string(),
        version_label: body.version_label,
        checksum_sha256: String::new(),
        size_bytes: 0,
        storage_uri: String::new(),
        notes: body.notes,
        created_at: now.to_rfc3339(),
    };
    sqlx::query(
        "INSERT INTO software_packages (id, name, version_label, checksum_sha256, size_bytes, storage_uri, notes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&pkg.id)
    .bind(&pkg.name)
    .bind(&pkg.version_label)
    .bind(&pkg.checksum_sha256)
    .bind(pkg.size_bytes)
    .bind(&pkg.storage_uri)
    .bind(&pkg.notes)
    .bind(&pkg.created_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(pkg)))
}

pub async fn upload_package(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<SoftwarePackage>, (StatusCode, String)> {
    user.require_write()?;
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty package body".into()));
    }
    let checksum = hex_sha256(&body);
    let path: PathBuf = state.software_dir.join(format!("{id}.bin"));
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| internal(e))?;
    let uri = path.to_string_lossy().replace('\\', "/");
    sqlx::query(
        "UPDATE software_packages SET checksum_sha256 = ?, size_bytes = ?, storage_uri = ? WHERE id = ?",
    )
    .bind(&checksum)
    .bind(body.len() as i64)
    .bind(&uri)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    load_package(&state, &id).await.map(Json)
}

pub async fn create_job(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateSoftwareJob>,
) -> Result<(StatusCode, Json<SoftwareJob>), (StatusCode, String)> {
    user.require_write()?;
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let _ = load_package(&state, &body.package_id).await?;
    let now = Utc::now().to_rfc3339();
    let job = SoftwareJob {
        id: new_id(),
        package_id: body.package_id,
        mac,
        state: "pending".into(),
        detail: String::new(),
        created_at: now.clone(),
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO software_jobs (id, package_id, mac, state, detail, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&job.id)
    .bind(&job.package_id)
    .bind(&job.mac)
    .bind(&job.state)
    .bind(&job.detail)
    .bind(&job.created_at)
    .bind(&job.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(job)))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<SoftwareJob>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, SoftwareJob>(
        "SELECT id, package_id, mac, state, detail, created_at, updated_at FROM software_jobs ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct SoftwareReport {
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

pub async fn report_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SoftwareReport>,
) -> Result<Json<SoftwareJob>, (StatusCode, String)> {
    let state_name = body.state.trim().to_ascii_lowercase();
    if !matches!(state_name.as_str(), "running" | "done" | "failed") {
        return Err((StatusCode::BAD_REQUEST, "state must be running|done|failed".into()));
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE software_jobs SET state = ?, detail = ?, updated_at = ? WHERE id = ?")
        .bind(&state_name)
        .bind(&body.detail)
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query_as::<_, SoftwareJob>(
        "SELECT id, package_id, mac, state, detail, created_at, updated_at FROM software_jobs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .map(Json)
    .ok_or((StatusCode::NOT_FOUND, "job not found".into()))
}

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub mac: String,
}

/// Agent downloads package blob for a pending/running job.
pub async fn download_for_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Query(q): Query<DownloadQuery>,
) -> Result<Bytes, (StatusCode, String)> {
    let mac = normalize_mac(&q.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let job = sqlx::query_as::<_, SoftwareJob>(
        "SELECT id, package_id, mac, state, detail, created_at, updated_at FROM software_jobs WHERE id = ?",
    )
    .bind(&job_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "job not found".into()))?;
    if job.mac != mac {
        return Err((StatusCode::FORBIDDEN, "mac mismatch".into()));
    }
    let pkg = load_package(&state, &job.package_id).await?;
    if pkg.storage_uri.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "package has no blob".into()));
    }
    let data = tokio::fs::read(&pkg.storage_uri)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Bytes::from(data))
}

pub async fn pending_for_mac(
    state: &AppState,
    mac: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT j.id, j.package_id, p.name, p.checksum_sha256
        FROM software_jobs j
        JOIN software_packages p ON p.id = j.package_id
        WHERE j.mac = ? AND j.state = 'pending'
        ORDER BY j.created_at ASC LIMIT 1
        "#,
    )
    .bind(mac)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|(id, package_id, name, checksum)| {
        serde_json::json!({
            "job_id": id,
            "package_id": package_id,
            "name": name,
            "checksum_sha256": checksum,
        })
    }))
}

async fn load_package(state: &AppState, id: &str) -> Result<SoftwarePackage, (StatusCode, String)> {
    sqlx::query_as::<_, SoftwarePackage>(
        "SELECT id, name, version_label, checksum_sha256, size_bytes, storage_uri, notes, created_at FROM software_packages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "package not found".into()))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
