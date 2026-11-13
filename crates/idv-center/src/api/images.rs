use std::path::{Component, Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::auth::AuthUser;
use crate::models::{CreateImage, Image, new_id};
use crate::state::AppState;

const IMAGE_COLS: &str = "id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at";

const ALLOWED_EXTS: &[&str] = &[
    "img", "bin", "wim", "iso", "vhdx", "vhd", "raw", "qcow2", "delta",
];

#[derive(Debug, Clone, Serialize)]
pub struct ImageFileEntry {
    pub name: String,
    pub size_bytes: i64,
    pub storage_uri: String,
    pub checksum_sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct FileNameQuery {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    pub filename: String,
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<Image>>, (StatusCode, String)> {
    let sql = format!("SELECT {IMAGE_COLS} FROM images ORDER BY created_at DESC");
    let rows = sqlx::query_as::<_, Image>(&sql)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Image>, (StatusCode, String)> {
    let sql = format!("SELECT {IMAGE_COLS} FROM images WHERE id = ?");
    sqlx::query_as::<_, Image>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "image not found".into()))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateImage>,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    user.require_write()?;
    let name = body.name.trim();
    let checksum = body.checksum_sha256.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    if checksum.len() != 64 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "checksum_sha256 must be 64 hex chars".into(),
        ));
    }
    let os = body.os_family.trim().to_ascii_lowercase();
    if !matches!(os.as_str(), "windows" | "linux" | "other") {
        return Err((
            StatusCode::BAD_REQUEST,
            "os_family must be windows|linux|other".into(),
        ));
    }

    let content_type = body.content_type.trim().to_ascii_lowercase();
    if !matches!(content_type.as_str(), "full" | "delta") {
        return Err((StatusCode::BAD_REQUEST, "content_type must be full|delta".into()));
    }
    if content_type == "delta" && body.base_image_id.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "delta image requires base_image_id".into(),
        ));
    }

    let image = Image {
        id: new_id(),
        name: name.to_string(),
        os_family: os,
        version_label: body.version_label,
        checksum_sha256: checksum,
        size_bytes: body.size_bytes,
        storage_uri: body.storage_uri,
        boot_mode: body.boot_mode,
        notes: body.notes,
        partition_map: body.partition_map,
        content_type,
        base_image_id: body.base_image_id.filter(|s| !s.trim().is_empty()),
        created_at: Utc::now(),
    };

    sqlx::query(
        "INSERT INTO images (id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&image.id)
    .bind(&image.name)
    .bind(&image.os_family)
    .bind(&image.version_label)
    .bind(&image.checksum_sha256)
    .bind(image.size_bytes)
    .bind(&image.storage_uri)
    .bind(&image.boot_mode)
    .bind(&image.notes)
    .bind(&image.partition_map)
    .bind(&image.content_type)
    .bind(&image.base_image_id)
    .bind(image.created_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    Ok((StatusCode::CREATED, Json(image)))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    let result = sqlx::query("DELETE FROM images WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "image not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// List files under center `images_dir` (no checksum; call meta to hash).
pub async fn list_files(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<ImageFileEntry>>, (StatusCode, String)> {
    tokio::fs::create_dir_all(&state.images_dir)
        .await
        .map_err(internal)?;
    let mut rd = tokio::fs::read_dir(&state.images_dir)
        .await
        .map_err(internal)?;
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await.map_err(internal)? {
        let meta = entry.metadata().await.map_err(internal)?;
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_allowed_image_name(&name) {
            continue;
        }
        let path = entry.path();
        out.push(ImageFileEntry {
            name,
            size_bytes: meta.len() as i64,
            storage_uri: path_uri(&path),
            checksum_sha256: String::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// Compute SHA-256 for an existing file under `images_dir`.
pub async fn file_meta(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<FileNameQuery>,
) -> Result<Json<ImageFileEntry>, (StatusCode, String)> {
    let path = resolve_images_file(&state.images_dir, &q.name)?;
    let meta = tokio::fs::metadata(&path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            (StatusCode::NOT_FOUND, "file not found".into())
        } else {
            internal(err)
        }
    })?;
    if !meta.is_file() {
        return Err((StatusCode::BAD_REQUEST, "not a file".into()));
    }
    let (checksum, size) = hash_file(&path).await?;
    Ok(Json(ImageFileEntry {
        name: q.name.trim().to_string(),
        size_bytes: size as i64,
        storage_uri: path_uri(&path),
        checksum_sha256: checksum,
    }))
}

/// Upload raw body into `images_dir` as `?filename=`.
pub async fn upload_file(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> Result<(StatusCode, Json<ImageFileEntry>), (StatusCode, String)> {
    user.require_write()?;
    let name = sanitize_upload_name(&q.filename)?;
    tokio::fs::create_dir_all(&state.images_dir)
        .await
        .map_err(internal)?;
    let dest = state.images_dir.join(&name);
    if dest.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("file already exists: {name}"),
        ));
    }

    let mut file = tokio::fs::File::create(&dest).await.map_err(internal)?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| internal(e))?;
        if chunk.is_empty() {
            continue;
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(internal)?;
        size += chunk.len() as u64;
    }
    file.flush().await.map_err(internal)?;
    if size == 0 {
        let _ = tokio::fs::remove_file(&dest).await;
        return Err((StatusCode::BAD_REQUEST, "empty upload body".into()));
    }
    let checksum = hex::encode(hasher.finalize());
    Ok((
        StatusCode::CREATED,
        Json(ImageFileEntry {
            name,
            size_bytes: size as i64,
            storage_uri: path_uri(&dest),
            checksum_sha256: checksum,
        }),
    ))
}

fn sanitize_upload_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let name = Path::new(raw.trim())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() || name == "." || name == ".." {
        return Err((StatusCode::BAD_REQUEST, "filename required".into()));
    }
    if name.contains('/') || name.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
    }
    if !is_allowed_image_name(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "filename extension must be one of: {}",
                ALLOWED_EXTS.join(", ")
            ),
        ));
    }
    Ok(name)
}

fn is_allowed_image_name(name: &str) -> bool {
    let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else {
        return false;
    };
    ALLOWED_EXTS
        .iter()
        .any(|a| ext.eq_ignore_ascii_case(a))
}

fn resolve_images_file(
    images_dir: &Path,
    name: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    let name = sanitize_upload_name(name)?;
    let path = images_dir.join(&name);
    // Reject any unexpected components after join (defense in depth).
    for c in path.components() {
        if matches!(c, Component::ParentDir) {
            return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
        }
    }
    let canon_dir = images_dir
        .canonicalize()
        .unwrap_or_else(|_| images_dir.to_path_buf());
    if let Ok(canon_file) = path.canonicalize() {
        if !canon_file.starts_with(&canon_dir) {
            return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
        }
        return Ok(canon_file);
    }
    Ok(path)
}

async fn hash_file(path: &Path) -> Result<(String, u64), (StatusCode, String)> {
    let mut file = tokio::fs::File::open(path).await.map_err(internal)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf)
            .await
            .map_err(internal)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

fn path_uri(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
