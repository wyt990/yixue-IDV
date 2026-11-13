use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{CreatePowerCommand, PowerCommand, new_id};
use crate::state::AppState;

/// Queue WOL or shutdown for a MAC. WOL is best-effort magic packet from Center host.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreatePowerCommand>,
) -> Result<(StatusCode, Json<PowerCommand>), (StatusCode, String)> {
    user.require_write()?;
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "wol" | "shutdown") {
        return Err((StatusCode::BAD_REQUEST, "kind must be wol|shutdown".into()));
    }
    let client_id: Option<String> = sqlx::query_scalar("SELECT id FROM clients WHERE mac = ?")
        .bind(&mac)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;

    let now = Utc::now();
    let mut detail = body.detail;
    if kind == "wol" {
        match send_wol(&mac) {
            Ok(msg) => {
                if detail.is_empty() {
                    detail = msg;
                }
            }
            Err(err) => {
                detail = format!("wol_send_failed: {err}");
            }
        }
    }

    let cmd = PowerCommand {
        id: new_id(),
        client_id,
        mac: mac.clone(),
        kind: kind.clone(),
        state: if kind == "wol" {
            "done".into()
        } else {
            "pending".into()
        },
        detail,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO power_commands (id, client_id, mac, kind, state, detail, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&cmd.id)
    .bind(&cmd.client_id)
    .bind(&cmd.mac)
    .bind(&cmd.kind)
    .bind(&cmd.state)
    .bind(&cmd.detail)
    .bind(cmd.created_at)
    .bind(cmd.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(cmd)))
}

pub async fn ack(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<PowerCommand>, (StatusCode, String)> {
    let ok = body
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let detail = body
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now = Utc::now();
    let state_name = if ok { "done" } else { "failed" };
    sqlx::query("UPDATE power_commands SET state = ?, detail = ?, updated_at = ? WHERE id = ?")
        .bind(state_name)
        .bind(&detail)
        .bind(now)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query_as::<_, PowerCommand>(
        "SELECT id, client_id, mac, kind, state, detail, created_at, updated_at FROM power_commands WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .map(Json)
    .ok_or((StatusCode::NOT_FOUND, "power command not found".into()))
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<PowerCommand>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, PowerCommand>(
        "SELECT id, client_id, mac, kind, state, detail, created_at, updated_at FROM power_commands ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

fn send_wol(mac: &str) -> anyhow::Result<String> {
    idv_common::wol::send_magic_packet(mac).map_err(|e| anyhow::anyhow!(e))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
