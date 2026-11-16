use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{CreatePowerCommand, PowerCommand, new_id};
use crate::state::AppState;

/// Queue WOL / shutdown / reboot for a MAC. WOL is best-effort from Center host.
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreatePowerCommand>,
) -> Result<(StatusCode, Json<PowerCommand>), (StatusCode, String)> {
    user.require_write()?;
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "wol" | "shutdown" | "reboot") {
        return Err((
            StatusCode::BAD_REQUEST,
            "kind must be wol|shutdown|reboot".into(),
        ));
    }
    let cmd = enqueue_power(&state.pool, &mac, &kind, &body.detail).await?;
    if matches!(kind.as_str(), "shutdown" | "reboot") {
        let payload = serde_json::json!({ "power_id": cmd.id }).to_string();
        let _ =
            crate::api::agent_commands::enqueue(&state, &mac, &kind, &payload, &user.0.username)
                .await;
    }
    Ok((StatusCode::CREATED, Json(cmd)))
}

#[derive(Debug, serde::Deserialize)]
pub struct BulkPowerCommand {
    pub macs: Vec<String>,
    pub kind: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BulkPowerResult {
    pub created: usize,
    pub errors: Vec<String>,
}

pub async fn create_bulk(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<BulkPowerCommand>,
) -> Result<Json<BulkPowerResult>, (StatusCode, String)> {
    user.require_write()?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "wol" | "shutdown" | "reboot") {
        return Err((
            StatusCode::BAD_REQUEST,
            "kind must be wol|shutdown|reboot".into(),
        ));
    }
    let mut created = 0usize;
    let mut errors = Vec::new();
    for raw in body.macs {
        let mac = match normalize_mac(&raw) {
            Ok(m) => m,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        match enqueue_power(&state.pool, &mac, &kind, &body.detail).await {
            Ok(cmd) => {
                created += 1;
                if matches!(kind.as_str(), "shutdown" | "reboot") {
                    let payload = serde_json::json!({ "power_id": cmd.id }).to_string();
                    let _ = crate::api::agent_commands::enqueue(
                        &state,
                        &mac,
                        &kind,
                        &payload,
                        &user.0.username,
                    )
                    .await;
                }
            }
            Err((_, msg)) => errors.push(format!("{mac}: {msg}")),
        }
    }
    Ok(Json(BulkPowerResult { created, errors }))
}

/// Enqueue reboot for every client in a lab (mode-change path A).
pub async fn enqueue_reboots_for_lab(
    state: &AppState,
    lab_id: &str,
    detail: &str,
) -> Result<usize, (StatusCode, String)> {
    let macs: Vec<(String,)> = sqlx::query_as("SELECT mac FROM clients WHERE lab_id = ?")
        .bind(lab_id)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    let mut n = 0usize;
    for (mac,) in macs {
        let cmd = enqueue_power(&state.pool, &mac, "reboot", detail).await?;
        let payload = serde_json::json!({ "power_id": cmd.id }).to_string();
        let _ =
            crate::api::agent_commands::enqueue(state, &mac, "reboot", &payload, "system").await;
        n += 1;
    }
    Ok(n)
}

pub async fn enqueue_power(
    pool: &SqlitePool,
    mac: &str,
    kind: &str,
    detail_in: &str,
) -> Result<PowerCommand, (StatusCode, String)> {
    let client_id: Option<String> = sqlx::query_scalar("SELECT id FROM clients WHERE mac = ?")
        .bind(mac)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;

    let now = Utc::now();
    let mut detail = detail_in.to_string();
    if kind == "wol" {
        // Prefer the lab's directed broadcast so the packet egresses the lab VLAN
        // interface instead of relying on the default route.
        let subnet: Option<(String, String)> = sqlx::query_as(
            "SELECT l.ip_start, l.netmask FROM clients c JOIN labs l ON l.id = c.lab_id WHERE c.mac = ?",
        )
        .bind(mac)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
        let directed = subnet
            .as_ref()
            .and_then(|(ip, mask)| idv_common::wol::directed_broadcast(ip, mask));
        match idv_common::wol::send_magic_packet_to(mac, directed) {
            Ok(msg) => {
                detail = if detail.is_empty() {
                    msg
                } else {
                    format!("{detail} | {msg}")
                };
            }
            Err(err) => {
                detail = format!("{detail} | wol_send_failed: {err}");
            }
        }
    }

    let cmd = PowerCommand {
        id: new_id(),
        client_id,
        mac: mac.to_string(),
        kind: kind.to_string(),
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
    .execute(pool)
    .await
    .map_err(internal)?;
    Ok(cmd)
}

pub async fn ack(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<PowerCommand>, (StatusCode, String)> {
    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
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

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
