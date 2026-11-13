//! Allocate sequential display names for clients within a lab.

use axum::http::StatusCode;

use crate::models::Lab;
use crate::state::AppState;

pub const LAB_COLS: &str = "id, campus_id, name, name_prefix, suffix_width, suffix_start, next_suffix, ip_start, ip_end, default_image_id, created_at";
pub const CAMPUS_COLS: &str = "id, name, default_image_id, created_at";

pub fn format_client_name(prefix: &str, width: i64, n: i64) -> String {
    let width = width.clamp(1, 8) as usize;
    format!("{prefix}{n:0width$}")
}

pub async fn load_lab(
    state: &AppState,
    lab_id: &str,
) -> Result<Lab, (StatusCode, String)> {
    let sql = format!("SELECT {LAB_COLS} FROM labs WHERE id = ?");
    sqlx::query_as::<_, Lab>(&sql)
        .bind(lab_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "lab not found".into()))
}

/// Atomically allocate the next name for a lab (updates `next_suffix`).
pub async fn allocate_next_name(
    state: &AppState,
    lab_id: &str,
) -> Result<String, (StatusCode, String)> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let sql = format!("SELECT {LAB_COLS} FROM labs WHERE id = ?");
    let lab = sqlx::query_as::<_, Lab>(&sql)
        .bind(lab_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "lab not found".into()))?;

    let width = lab.suffix_width.clamp(1, 8);
    let mut n = lab.next_suffix.max(lab.suffix_start).max(1);
    let prefix = lab.name_prefix.trim();

    let name = loop {
        let candidate = format_client_name(prefix, width, n);
        let taken: Option<(String,)> =
            sqlx::query_as("SELECT id FROM clients WHERE lab_id = ? AND name = ?")
                .bind(lab_id)
                .bind(&candidate)
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?;
        if taken.is_none() {
            break candidate;
        }
        n += 1;
        if n > 999_999 {
            return Err((
                StatusCode::CONFLICT,
                "unable to allocate client name".into(),
            ));
        }
    };

    sqlx::query("UPDATE labs SET next_suffix = ? WHERE id = ?")
        .bind(n + 1)
        .bind(lab_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(name)
}

/// Re-allocate a fresh name for an existing client after moving labs.
pub async fn rename_for_lab(
    state: &AppState,
    lab_id: &str,
    client_id: &str,
) -> Result<String, (StatusCode, String)> {
    let name = allocate_next_name(state, lab_id).await?;
    sqlx::query("UPDATE clients SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(client_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(name)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
