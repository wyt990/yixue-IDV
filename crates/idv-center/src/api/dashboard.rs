use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::models::{
    DashboardEdgeRow, DashboardLabRow, DashboardOverview, DashboardStats, DashboardTaskRow,
    Edge, edge_online,
};
use crate::state::AppState;

const EDGE_COLS: &str = "id, name, node_token, listen_url, last_seen, cache_used_bytes, cache_quota_bytes, cache_warn_pct, cache_level, capture_tmp_dir, cpu_pct, mem_used_bytes, mem_total_bytes, net_rx_bps, net_tx_bps, created_at";

pub async fn stats(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<DashboardOverview>, (StatusCode, String)> {
    let campuses = count(&state, "SELECT COUNT(*) FROM campuses").await?;
    let labs = count(&state, "SELECT COUNT(*) FROM labs").await?;
    let clients = count(&state, "SELECT COUNT(*) FROM clients").await?;
    let images = count(&state, "SELECT COUNT(*) FROM images").await?;
    let edges_total = count(&state, "SELECT COUNT(*) FROM edges").await?;

    let edge_rows = sqlx::query_as::<_, Edge>(&format!(
        "SELECT {EDGE_COLS} FROM edges ORDER BY name"
    ))
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let edges: Vec<DashboardEdgeRow> = edge_rows
        .into_iter()
        .map(|e| {
            let online = edge_online(e.last_seen);
            DashboardEdgeRow {
                id: e.id,
                name: e.name,
                online,
                cpu_pct: e.cpu_pct,
                mem_used_bytes: e.mem_used_bytes,
                mem_total_bytes: e.mem_total_bytes,
                net_rx_bps: e.net_rx_bps,
                net_tx_bps: e.net_tx_bps,
                last_seen: e.last_seen,
            }
        })
        .collect();
    let edges_online = edges.iter().filter(|e| e.online).count() as i64;

    let sync_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM edge_commands WHERE state IN ('pending', 'running')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let maint_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maintenance_tasks WHERE state IN ('pending', 'running')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let tasks_running = sync_running + maint_running;

    let tasks_active = load_tasks(
        &state,
        "SELECT id, kind, state, name, error_message, updated_at FROM maintenance_tasks WHERE state IN ('pending', 'running') ORDER BY updated_at DESC LIMIT 20",
    )
    .await?;
    let tasks_failed = load_tasks(
        &state,
        "SELECT id, kind, state, name, error_message, updated_at FROM maintenance_tasks WHERE state = 'failed' ORDER BY updated_at DESC LIMIT 5",
    )
    .await?;

    let lab_rows: Vec<(String, String, String, Option<String>, Option<String>, i64)> =
        sqlx::query_as(
            r#"
            SELECT
              l.id,
              l.name,
              c.name,
              l.default_image_id,
              c.default_image_id,
              (SELECT COUNT(*) FROM clients cl WHERE cl.lab_id = l.id)
            FROM labs l
            JOIN campuses c ON c.id = l.campus_id
            ORDER BY c.name, l.name
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;

    let mut labs_summary = Vec::with_capacity(lab_rows.len());
    for (id, name, campus_name, lab_img, campus_img, client_count) in lab_rows {
        let (image_id, from_campus) = match lab_img.filter(|s| !s.is_empty()) {
            Some(id) => (Some(id), false),
            None => (
                campus_img.filter(|s| !s.is_empty()),
                true,
            ),
        };
        let default_image_name = if let Some(iid) = image_id {
            let n: Option<String> = sqlx::query_scalar("SELECT name FROM images WHERE id = ?")
                .bind(&iid)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?;
            match n {
                Some(n) if from_campus => format!("{n}（校区）"),
                Some(n) => n,
                None => "—".into(),
            }
        } else {
            "—".into()
        };
        let image_from_campus = from_campus && default_image_name != "—";
        labs_summary.push(DashboardLabRow {
            id,
            name,
            campus_name,
            client_count,
            default_image_name,
            image_from_campus,
        });
    }

    Ok(Json(DashboardOverview {
        stats: DashboardStats {
            campuses,
            labs,
            clients,
            images,
            edges_online,
            edges_total,
            tasks_running,
        },
        edges,
        tasks_active,
        tasks_failed,
        labs: labs_summary,
    }))
}

async fn load_tasks(
    state: &AppState,
    sql: &str,
) -> Result<Vec<DashboardTaskRow>, (StatusCode, String)> {
    let rows: Vec<(String, String, String, String, String, chrono::DateTime<Utc>)> =
        sqlx::query_as(sql)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, kind, state_s, name, error_message, updated_at) in rows {
        let progress_text = task_progress_text(state, &id).await?;
        let err = if !error_message.is_empty() {
            error_message
        } else {
            // fall back to first target error
            sqlx::query_scalar::<_, String>(
                "SELECT error_message FROM maintenance_task_targets WHERE task_id = ? AND error_message != '' LIMIT 1",
            )
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?
            .unwrap_or_default()
        };
        let display_name = if name.is_empty() {
            let short = if id.len() >= 8 { &id[..8] } else { &id };
            format!("{kind}-{short}")
        } else {
            name
        };
        out.push(DashboardTaskRow {
            id,
            name: display_name,
            kind,
            state: state_s,
            progress_text,
            error_message: err,
            updated_at,
        });
    }
    Ok(out)
}

async fn task_progress_text(
    state: &AppState,
    task_id: &str,
) -> Result<String, (StatusCode, String)> {
    let rows: Vec<(String, f64, String)> = sqlx::query_as(
        "SELECT state, progress, detail FROM maintenance_task_targets WHERE task_id = ? ORDER BY updated_at DESC",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    if rows.is_empty() {
        return Ok("—".into());
    }
    let preferred = rows
        .iter()
        .find(|(s, _, d)| (s == "running" || s == "pending") && !d.is_empty())
        .or_else(|| rows.iter().find(|(_, _, d)| !d.is_empty()))
        .or_else(|| rows.first());
    let Some((_, progress, detail)) = preferred else {
        return Ok("—".into());
    };
    if !detail.is_empty()
        && regex_like_gb(detail)
    {
        return Ok(detail.split('·').next().unwrap_or(detail).trim().to_string());
    }
    if *progress > 0.0 {
        return Ok(format!("{}%", (*progress * 100.0).round() as i64));
    }
    Ok("—".into())
}

fn regex_like_gb(s: &str) -> bool {
    // light check for "x / y GB" progress strings without pulling regex crate
    s.to_ascii_lowercase().contains("gb")
}

async fn count(state: &AppState, sql: &str) -> Result<i64, (StatusCode, String)> {
    sqlx::query_scalar(sql)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
