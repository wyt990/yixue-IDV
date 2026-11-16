//! Edge-facing DHCP IP bindings (MAC -> assigned IPv4) for labs attached to the Edge.
//!
//! Edge DHCP uses these reservations so batch-deployed clients get the IP that
//! Center assigned, instead of a random pool address. Only `ip_mode = 'dhcp'`
//! labs are returned.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::auth::AuthEdge;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct IpBinding {
    pub mac: String,
    pub ip: String,
    pub lab_id: String,
}

#[derive(Debug, Serialize)]
pub struct IpBindingsResponse {
    pub bindings: Vec<IpBinding>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthEdge(edge): AuthEdge,
) -> Result<Json<IpBindingsResponse>, (StatusCode, String)> {
    // Labs directly attached to the Edge; fall back to campus-level attachment.
    let mut lab_ids: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT lab_id FROM lab_edges WHERE edge_id = ? ORDER BY priority ASC",
    )
    .bind(&edge.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?
    .into_iter()
    .map(|(id,)| id)
    .collect();

    if lab_ids.is_empty() {
        lab_ids = sqlx::query_as::<_, (String,)>(
            "SELECT l.id FROM labs l JOIN campus_edges ce ON ce.campus_id = l.campus_id WHERE ce.edge_id = ?",
        )
        .bind(&edge.id)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(id,)| id)
        .collect();
    }

    if lab_ids.is_empty() {
        return Ok(Json(IpBindingsResponse {
            bindings: Vec::new(),
        }));
    }

    // Dynamic IN list; all values are bound, no user input is interpolated.
    let placeholders = vec!["?"; lab_ids.len()].join(",");
    let sql = format!(
        "SELECT c.mac, c.assigned_ip, c.lab_id \
         FROM clients c JOIN labs l ON l.id = c.lab_id \
         WHERE c.assigned_ip != '' AND l.ip_mode = 'dhcp' AND c.lab_id IN ({placeholders}) \
         ORDER BY c.mac"
    );
    let mut query = sqlx::query_as::<_, (String, String, String)>(&sql);
    for id in &lab_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(&state.pool).await.map_err(internal)?;

    let bindings = rows
        .into_iter()
        .map(|(mac, ip, lab_id)| IpBinding { mac, ip, lab_id })
        .collect();
    Ok(Json(IpBindingsResponse { bindings }))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
