use std::collections::HashMap;
use std::net::Ipv4Addr;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{
    CreatePowerCommand, PowerCommand, edge_online, new_id,
};
use crate::state::AppState;

/// Queue WOL / shutdown / reboot for a MAC.
///
/// WOL prefers the lab's Edge relay: Center -> Edge command -> Edge sends the
/// magic packet on the classroom VLAN. If no online Edge is bound, it falls
/// back to sending from the Center host.
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
    if kind == "wol" {
        let mut cmds = enqueue_wol_batch(&state, &[mac], &body.detail).await?;
        let cmd = cmds
            .pop()
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "wol enqueue failed".into()))?;
        return Ok((StatusCode::CREATED, Json(cmd)));
    }
    let cmd = enqueue_power(&state.pool, &mac, &kind, &body.detail).await?;
    let payload = serde_json::json!({ "power_id": cmd.id }).to_string();
    let _ = crate::api::agent_commands::enqueue(&state, &mac, &kind, &payload, &user.0.username)
        .await;
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
    let mut errors = Vec::new();
    let mut macs = Vec::new();
    for raw in body.macs {
        match normalize_mac(&raw) {
            Ok(mac) => macs.push(mac),
            Err(e) => errors.push(e),
        }
    }

    if kind == "wol" {
        let cmds = enqueue_wol_batch(&state, &macs, &body.detail).await?;
        return Ok(Json(BulkPowerResult {
            created: cmds.len(),
            errors,
        }));
    }

    let mut created = 0usize;
    for mac in macs {
        match enqueue_power(&state.pool, &mac, &kind, &body.detail).await {
            Ok(cmd) => {
                created += 1;
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

/// Insert a power command row in `pending` state. WOL rows are finalized by
/// the Edge report or the local fallback; reboot/shutdown rows are finalized by
/// the Agent power ack.
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
    insert_power_command(pool, mac, client_id, kind, "pending", detail_in).await
}

async fn insert_power_command(
    pool: &SqlitePool,
    mac: &str,
    client_id: Option<String>,
    kind: &str,
    state: &str,
    detail: &str,
) -> Result<PowerCommand, (StatusCode, String)> {
    let now = Utc::now();
    let cmd = PowerCommand {
        id: new_id(),
        client_id,
        mac: mac.to_string(),
        kind: kind.to_string(),
        state: state.to_string(),
        detail: detail.to_string(),
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

#[derive(Debug)]
struct WolTarget {
    client_id: Option<String>,
    lab_id: Option<String>,
    broadcast: Option<String>,
}

async fn resolve_wol_target(
    state: &AppState,
    mac: &str,
) -> Result<WolTarget, (StatusCode, String)> {
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.lab_id, l.ip_start, l.netmask \
         FROM clients c LEFT JOIN labs l ON l.id = c.lab_id WHERE c.mac = ?",
    )
    .bind(mac)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    Ok(match row {
        Some((client_id, lab_id, ip_start, netmask)) => {
            let broadcast = match (ip_start.as_deref(), netmask.as_deref()) {
                (Some(ip), Some(mask)) => {
                    idv_common::wol::directed_broadcast(ip, mask).map(|ip| ip.to_string())
                }
                _ => None,
            };
            WolTarget {
                client_id: Some(client_id),
                broadcast,
                lab_id: lab_id.filter(|id| !id.is_empty()),
            }
        }
        None => WolTarget {
            client_id: None,
            lab_id: None,
            broadcast: None,
        },
    })
}

/// Pick an online Edge bound to the lab. A lab-level binding wins over a
/// campus-level binding; priority is ascending (lower value first).
async fn find_lab_edge(
    state: &AppState,
    lab_id: &str,
) -> Result<Option<(String, String)>, (StatusCode, String)> {
    let rows: Vec<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT e.id, e.name, e.last_seen \
         FROM lab_edges le JOIN edges e ON e.id = le.edge_id \
         WHERE le.lab_id = ? ORDER BY le.priority ASC, e.name",
    )
    .bind(lab_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    if let Some((id, name, _)) = rows.into_iter().find(|(_, _, seen)| edge_online(*seen)) {
        return Ok(Some((id, name)));
    }

    let rows: Vec<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT e.id, e.name, e.last_seen \
         FROM labs l JOIN campus_edges ce ON ce.campus_id = l.campus_id \
         JOIN edges e ON e.id = ce.edge_id \
         WHERE l.id = ? ORDER BY ce.priority ASC, e.name",
    )
    .bind(lab_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(rows
        .into_iter()
        .find(|(_, _, seen)| edge_online(*seen))
        .map(|(id, name, _)| (id, name)))
}

struct PendingWol {
    cmd: PowerCommand,
    broadcast: Option<String>,
}

/// Enqueue WOL for a batch, grouping Edge-relayed targets into one command per
/// Edge. Targets without an online Edge are sent from Center as a fallback.
async fn enqueue_wol_batch(
    state: &AppState,
    macs: &[String],
    detail: &str,
) -> Result<Vec<PowerCommand>, (StatusCode, String)> {
    let mut by_edge: HashMap<String, (String, Vec<PendingWol>)> = HashMap::new();
    let mut local: Vec<PendingWol> = Vec::new();
    let mut result = Vec::with_capacity(macs.len());

    for mac in macs {
        let target = resolve_wol_target(state, mac).await?;
        let edge = match &target.lab_id {
            Some(lab_id) => find_lab_edge(state, lab_id).await?,
            None => None,
        };
        let initial_detail = match &edge {
            Some((_, name)) => {
                if detail.is_empty() {
                    format!("wol queued via edge:{name}")
                } else {
                    format!("{detail} | wol queued via edge:{name}")
                }
            }
            None => {
                if detail.is_empty() {
                    "wol local fallback (no online edge)".to_string()
                } else {
                    format!("{detail} | wol local fallback (no online edge)")
                }
            }
        };
        let cmd = insert_power_command(
            &state.pool,
            mac,
            target.client_id.clone(),
            "wol",
            "pending",
            &initial_detail,
        )
        .await?;
        let pending = PendingWol {
            cmd,
            broadcast: target.broadcast,
        };
        match edge {
            Some((edge_id, edge_name)) => {
                by_edge
                    .entry(edge_id)
                    .or_insert_with(|| (edge_name, Vec::new()))
                    .1
                    .push(pending);
            }
            None => local.push(pending),
        }
    }

    // One Edge command per Edge, containing all target MACs.
    for (edge_id, (edge_name, items)) in by_edge {
        let payload = serde_json::json!({
            "items": items.iter().map(|p| serde_json::json!({
                "power_id": p.cmd.id,
                "mac": p.cmd.mac,
                "broadcast": p.broadcast,
            })).collect::<Vec<_>>(),
        })
        .to_string();
        let now = Utc::now();
        let insert = sqlx::query(
            "INSERT INTO edge_commands \
             (id, edge_id, kind, image_id, state, progress, error_message, payload, created_at, updated_at) \
             VALUES (?, ?, 'wol', NULL, 'pending', 0, '', ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&edge_id)
        .bind(&payload)
        .bind(now)
        .bind(now)
        .execute(&state.pool)
        .await;
        if let Err(err) = insert {
            for p in &items {
                let detail = format!("wol failed: enqueue edge:{edge_name}: {err}");
                update_power(&state.pool, &p.cmd.id, "failed", &detail).await?;
            }
            continue;
        }
        for mut p in items {
            p.cmd.detail = format!("wol queued via edge:{edge_name}");
            result.push(p.cmd);
        }
    }

    // Local fallback: one socket/batch for all remaining targets.
    if !local.is_empty() {
        let batch: Vec<(String, Option<Ipv4Addr>)> = local
            .iter()
            .map(|p| {
                (
                    p.cmd.mac.clone(),
                    p.broadcast.as_ref().and_then(|s| s.parse().ok()),
                )
            })
            .collect();
        let sent = tokio::task::spawn_blocking(move || {
            idv_common::wol::send_magic_packets_to(&batch)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for mut p in local {
            let (state_name, detail) = match &sent {
                Ok(msg) => (
                    "done",
                    if detail.is_empty() {
                        format!("wol local_fallback | {msg}")
                    } else {
                        format!("{detail} | wol local_fallback | {msg}")
                    },
                ),
                Err(err) => ("failed", format!("wol local_fallback failed: {err}")),
            };
            update_power(&state.pool, &p.cmd.id, state_name, &detail).await?;
            p.cmd.state = state_name.into();
            p.cmd.detail = detail;
            result.push(p.cmd);
        }
    }

    Ok(result)
}

async fn update_power(
    pool: &SqlitePool,
    id: &str,
    state: &str,
    detail: &str,
) -> Result<(), (StatusCode, String)> {
    let now = Utc::now();
    sqlx::query("UPDATE power_commands SET state = ?, detail = ?, updated_at = ? WHERE id = ?")
        .bind(state)
        .bind(detail)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(internal)?;
    Ok(())
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
