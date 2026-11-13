use std::path::PathBuf;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use idv_common::delta::{DEFAULT_BLOCK, apply_delta, build_delta, hex_sha256, unpack_delta};
use idv_common::image_format::hex_sha256 as file_sha;

use crate::auth::AuthUser;
use crate::models::{ApplyDeltaRequest, BuildDeltaRequest, Image, new_id};
use crate::state::AppState;

const IMAGE_COLS: &str = "id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at";

pub async fn build_delta_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<BuildDeltaRequest>,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    let base = load_image(&state, &body.base_image_id).await?;
    let target = load_image(&state, &body.target_image_id).await?;
    if base.content_type != "full" || target.content_type != "full" {
        return Err((
            StatusCode::BAD_REQUEST,
            "build-delta requires two full images".into(),
        ));
    }
    if base.os_family != target.os_family {
        return Err((StatusCode::BAD_REQUEST, "os_family mismatch".into()));
    }
    let base_bytes = read_blob(&base)?;
    let target_bytes = read_blob(&target)?;
    let delta_bytes = build_delta(&base_bytes, &target_bytes, DEFAULT_BLOCK)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let packed = unpack_delta(&delta_bytes).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let id = new_id();
    tokio::fs::create_dir_all(&state.images_dir)
        .await
        .map_err(internal)?;
    let dest = state.images_dir.join(format!("{id}.delta"));
    tokio::fs::write(&dest, &delta_bytes)
        .await
        .map_err(internal)?;
    let now = Utc::now();
    let name = if body.name.trim().is_empty() {
        format!("delta-{}-to-{}", &base.id[..8], &target.id[..8])
    } else {
        body.name.trim().to_string()
    };
    let image = Image {
        id: id.clone(),
        name,
        os_family: target.os_family.clone(),
        version_label: format!("{}→{}", base.version_label, target.version_label),
        checksum_sha256: file_sha(&delta_bytes),
        size_bytes: delta_bytes.len() as i64,
        storage_uri: dest
            .canonicalize()
            .unwrap_or(dest)
            .to_string_lossy()
            .to_string(),
        boot_mode: target.boot_mode.clone(),
        notes: format!(
            "delta hunks={} base={} target={}",
            packed.header.hunks.len(),
            base.id,
            target.id
        ),
        partition_map: target.partition_map.clone(),
        content_type: "delta".into(),
        base_image_id: Some(base.id),
        created_at: now,
    };
    insert_image(&state, &image).await?;
    Ok((StatusCode::CREATED, Json(image)))
}

pub async fn apply_delta_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<ApplyDeltaRequest>,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    let base = load_image(&state, &body.base_image_id).await?;
    let delta = load_image(&state, &body.delta_image_id).await?;
    if base.content_type != "full" {
        return Err((StatusCode::BAD_REQUEST, "base must be full image".into()));
    }
    if delta.content_type != "delta" {
        return Err((StatusCode::BAD_REQUEST, "delta_image_id must be delta".into()));
    }
    if let Some(ref bid) = delta.base_image_id {
        if bid != &base.id {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("delta expects base {bid}, got {}", base.id),
            ));
        }
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "delta missing base_image_id".into(),
        ));
    }

    let base_bytes = read_blob(&base)?;
    let delta_bytes = read_blob(&delta)?;
    let out = apply_delta(&base_bytes, &delta_bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let got = hex_sha256(&out);

    let id = new_id();
    let dest = state.images_dir.join(format!("{id}.bin"));
    tokio::fs::write(&dest, &out).await.map_err(internal)?;
    let now = Utc::now();
    let name = if body.name.trim().is_empty() {
        format!("applied-{}", &id[..8])
    } else {
        body.name.trim().to_string()
    };
    let image = Image {
        id: id.clone(),
        name,
        os_family: base.os_family.clone(),
        version_label: "applied-delta".into(),
        checksum_sha256: got,
        size_bytes: out.len() as i64,
        storage_uri: dest
            .canonicalize()
            .unwrap_or(dest)
            .to_string_lossy()
            .to_string(),
        boot_mode: base.boot_mode.clone(),
        notes: format!("applied delta {} onto {}", delta.id, base.id),
        partition_map: delta.partition_map.clone(),
        content_type: "full".into(),
        base_image_id: None,
        created_at: now,
    };
    insert_image(&state, &image).await?;
    Ok((StatusCode::CREATED, Json(image)))
}

async fn load_image(state: &AppState, id: &str) -> Result<Image, (StatusCode, String)> {
    let sql = format!("SELECT {IMAGE_COLS} FROM images WHERE id = ?");
    sqlx::query_as::<_, Image>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "image not found".into()))
}

fn read_blob(image: &Image) -> Result<Vec<u8>, (StatusCode, String)> {
    let path = resolve_storage_path(&image.storage_uri)?;
    std::fs::read(&path).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("blob missing at {}: {e}", path.display()),
        )
    })
}

fn resolve_storage_path(storage_uri: &str) -> Result<PathBuf, (StatusCode, String)> {
    let raw = storage_uri.trim();
    if raw.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty storage_uri".into()));
    }
    if let Some(rest) = raw.strip_prefix("file:///") {
        return Ok(PathBuf::from(rest));
    }
    if let Some(rest) = raw.strip_prefix("file://") {
        return Ok(PathBuf::from(rest));
    }
    Ok(PathBuf::from(raw))
}

async fn insert_image(state: &AppState, image: &Image) -> Result<(), (StatusCode, String)> {
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
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
