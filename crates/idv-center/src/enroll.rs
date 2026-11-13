//! Auto-enroll unknown clients into the default lab (shared by Agent + PE check-in).

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use tracing::warn;

use crate::models::new_id;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct EnrollOutcome {
    pub client_id: String,
    pub lab_id: String,
    pub client_name: String,
    /// True if a new clients row was inserted this call.
    pub created: bool,
}

/// Ensure MAC exists in `clients`. Uses default lab naming when creating.
/// Returns `None` when no default lab is configured (caller should still succeed).
pub async fn ensure_client_enrolled(
    state: &AppState,
    mac: &str,
    hostname_note: &str,
    now: DateTime<Utc>,
) -> Result<Option<EnrollOutcome>, (StatusCode, String)> {
    let existing: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, lab_id, name FROM clients WHERE mac = ?")
            .bind(mac)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;

    if let Some((client_id, lab_id, client_name)) = existing {
        sqlx::query("UPDATE clients SET last_seen = ? WHERE id = ?")
            .bind(now)
            .bind(&client_id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        return Ok(Some(EnrollOutcome {
            client_id,
            lab_id,
            client_name,
            created: false,
        }));
    }

    let settings = crate::api::ops::load_enrollment(state)
        .await
        .map_err(internal)?;
    if settings.default_lab_id.trim().is_empty() {
        warn!(%mac, "PE/agent enroll skipped: default lab not configured");
        return Ok(None);
    }
    let lab_id = settings.default_lab_id.trim().to_string();
    let _ = crate::lab_naming::load_lab(state, &lab_id).await?;
    let name = crate::lab_naming::allocate_next_name(state, &lab_id).await?;
    let assigned_ip = crate::lab_naming::allocate_next_ip(state, &lab_id)
        .await
        .unwrap_or_default();
    // 备注留给管理员手填，自动登记不写入内容
    let notes = String::new();
    let _ = hostname_note;
    let id = new_id();
    match sqlx::query(
        "INSERT INTO clients (id, lab_id, mac, name, notes, last_ip, assigned_ip, last_seen, created_at) VALUES (?, ?, ?, ?, ?, '', ?, ?, ?)",
    )
    .bind(&id)
    .bind(&lab_id)
    .bind(mac)
    .bind(&name)
    .bind(&notes)
    .bind(&assigned_ip)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    {
        Ok(_) => {
            let _ = crate::api::audit::record(
                &state.pool,
                "system",
                "client.auto_enroll",
                "client",
                &id,
                &format!("mac={mac} lab={lab_id} name={name} ip={assigned_ip}"),
            )
            .await;
            Ok(Some(EnrollOutcome {
                client_id: id,
                lab_id,
                client_name: name,
                created: true,
            }))
        }
        Err(err) if err.to_string().contains("UNIQUE") => {
            let row: Option<(String, String, String)> =
                sqlx::query_as("SELECT id, lab_id, name FROM clients WHERE mac = ?")
                    .bind(mac)
                    .fetch_optional(&state.pool)
                    .await
                    .map_err(internal)?;
            Ok(row.map(|(client_id, lab_id, client_name)| EnrollOutcome {
                client_id,
                lab_id,
                client_name,
                created: false,
            }))
        }
        Err(err) => Err(internal(err)),
    }
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
