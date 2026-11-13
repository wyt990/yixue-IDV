use std::path::PathBuf;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use idv_common::delta::{FILE_BLOCK, apply_delta_files, build_delta_files};
use tracing::info;

use crate::auth::AuthUser;
use crate::models::{ApplyDeltaRequest, BuildDeltaRequest, Image, new_id};
use crate::state::AppState;

const IMAGE_COLS: &str = "id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at";

pub async fn build_delta_image(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<BuildDeltaRequest>,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    user.require_write()?;
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
    let base_path = resolve_storage_path(&base.storage_uri)?;
    let target_path = resolve_storage_path(&target.storage_uri)?;
    tokio::fs::create_dir_all(&state.images_dir)
        .await
        .map_err(internal)?;
    let id = new_id();
    let dest = state.images_dir.join(format!("{id}.delta"));
    let dest_clone = dest.clone();
    let base_path_c = base_path.clone();
    let target_path_c = target_path.clone();

    info!(
        base = %base.id,
        target = %target.id,
        out = %dest.display(),
        "[delta] build queued"
    );

    let header = tokio::task::spawn_blocking(move || {
        build_delta_files(
            &base_path_c,
            &target_path_c,
            &dest_clone,
            FILE_BLOCK,
            |done, total, msg| {
                if done == 0 || done == total || done % (256 * 1024 * 1024) < FILE_BLOCK as u64 {
                    info!(done, total, "{msg}");
                }
            },
        )
    })
    .await
    .map_err(|e| internal(e))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let size_bytes = tokio::fs::metadata(&dest)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    let checksum = idv_common::delta::hash_file(&dest).map_err(|e| internal(e))?;
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
        checksum_sha256: checksum,
        size_bytes,
        storage_uri: dest
            .canonicalize()
            .unwrap_or(dest)
            .to_string_lossy()
            .replace('\\', "/"),
        boot_mode: target.boot_mode.clone(),
        notes: format!(
            "[delta] hunks={} block={} base={} target={}",
            header.hunks.len(),
            header.block_size,
            base.id,
            target.id
        ),
        partition_map: target.partition_map.clone(),
        content_type: "delta".into(),
        base_image_id: Some(base.id),
        created_at: now,
    };
    insert_image(&state, &image).await?;
    info!(
        image_id = %image.id,
        hunks = header.hunks.len(),
        size_bytes,
        "[delta] build ok"
    );
    Ok((StatusCode::CREATED, Json(image)))
}

pub async fn apply_delta_image(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<ApplyDeltaRequest>,
) -> Result<(StatusCode, Json<Image>), (StatusCode, String)> {
    user.require_write()?;
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

    let base_path = resolve_storage_path(&base.storage_uri)?;
    let delta_path = resolve_storage_path(&delta.storage_uri)?;
    let id = new_id();
    let dest = state.images_dir.join(format!("{id}.bin"));
    let dest_c = dest.clone();
    let base_c = base_path.clone();
    let delta_c = delta_path.clone();

    info!(
        base = %base.id,
        delta = %delta.id,
        out = %dest.display(),
        "[delta] apply queued"
    );

    let got = tokio::task::spawn_blocking(move || {
        apply_delta_files(&base_c, &delta_c, &dest_c, |done, total, msg| {
            if done == 0 || done == total || done % 256 == 0 {
                info!(done, total, "{msg}");
            }
        })
    })
    .await
    .map_err(|e| internal(e))?
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let size_bytes = tokio::fs::metadata(&dest)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);
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
        size_bytes,
        storage_uri: dest
            .canonicalize()
            .unwrap_or(dest)
            .to_string_lossy()
            .replace('\\', "/"),
        boot_mode: base.boot_mode.clone(),
        notes: format!("[delta] applied {} onto {}", delta.id, base.id),
        partition_map: delta.partition_map.clone(),
        content_type: "full".into(),
        base_image_id: None,
        created_at: now,
    };
    insert_image(&state, &image).await?;
    info!(image_id = %image.id, size_bytes, "[delta] apply ok");
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
