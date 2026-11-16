use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use chrono::{Duration, Utc};
use serde::Serialize;

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{
    CreateCaptureTask, CreateDeployTask, MaintenanceTask, MaintenanceTaskTarget, new_id,
};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: MaintenanceTask,
    pub targets: Vec<MaintenanceTaskTarget>,
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<TaskView>>, (StatusCode, String)> {
    let tasks = sqlx::query_as::<_, MaintenanceTask>(
        "SELECT id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at FROM maintenance_tasks ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut out = Vec::new();
    for task in tasks {
        let targets = load_targets(&state, &task.id).await?;
        out.push(TaskView { task, targets });
    }
    Ok(Json(out))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<TaskView>, (StatusCode, String)> {
    let task = load_task(&state, &id).await?;
    let targets = load_targets(&state, &id).await?;
    Ok(Json(TaskView { task, targets }))
}

pub async fn export_csv(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Response, (StatusCode, String)> {
    let tasks = sqlx::query_as::<_, MaintenanceTask>(
        "SELECT id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at FROM maintenance_tasks ORDER BY created_at DESC LIMIT 500",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;

    let mut csv = String::from(
        "task_id,name,kind,task_state,disk_plan,image_id,mac,target_state,progress,error,updated_at\n",
    );
    for task in tasks {
        let targets = load_targets(&state, &task.id).await?;
        let name = task.name.replace(',', ";");
        if targets.is_empty() {
            csv.push_str(&format!(
                "{},{},{},{},{},{},,,0,,{}\n",
                task.id,
                name,
                task.kind,
                task.state,
                task.disk_plan,
                task.image_id.clone().unwrap_or_default(),
                task.updated_at.to_rfc3339()
            ));
        } else {
            for target in targets {
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{}\n",
                    task.id,
                    name,
                    task.kind,
                    task.state,
                    task.disk_plan,
                    task.image_id.clone().unwrap_or_default(),
                    target.mac,
                    target.state,
                    target.progress,
                    target.error_message.replace(',', ";"),
                    target.updated_at.to_rfc3339()
                ));
            }
        }
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"idv-tasks.csv\"",
        )
        .body(Body::from(csv))
        .map_err(internal)
}

pub async fn create_capture(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateCaptureTask>,
) -> Result<(StatusCode, Json<TaskView>), (StatusCode, String)> {
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();
    let edge_id = match body.edge_id.filter(|s| !s.trim().is_empty()) {
        Some(id) => Some(id),
        None => {
            if let Some(lab_id) = crate::site_defaults::lab_id_for_mac(&state.pool, &mac).await? {
                crate::site_defaults::resolve_preferred_edge_id(&state.pool, &lab_id).await?
            } else {
                None
            }
        }
    };
    let task = MaintenanceTask {
        id: new_id(),
        kind: "capture".into(),
        state: "pending".into(),
        image_id: None,
        edge_id,
        disk_plan: "wipe_whole_disk".into(),
        name: if body.name.trim().is_empty() {
            format!("capture-{mac}")
        } else {
            body.name
        },
        error_message: String::new(),
        created_by: user.0.username,
        created_at: now,
        updated_at: now,
    };
    insert_task(&state, &task).await?;
    let detail = serde_json::json!({
        "os_family": body.os_family,
        "exclude_temp": body.exclude_temp,
    })
    .to_string();
    let target = MaintenanceTaskTarget {
        id: new_id(),
        task_id: task.id.clone(),
        mac,
        state: "pending".into(),
        progress: 0.0,
        error_message: String::new(),
        detail,
        p2p_bytes: 0,
        edge_bytes: 0,
        updated_at: now,
    };
    insert_target(&state, &target).await?;
    tracing::info!(
        task_id = %task.id,
        exclude_temp = body.exclude_temp,
        "[capture] task created"
    );
    Ok((
        StatusCode::CREATED,
        Json(TaskView {
            task,
            targets: vec![target],
        }),
    ))
}

pub async fn create_deploy(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateDeployTask>,
) -> Result<(StatusCode, Json<TaskView>), (StatusCode, String)> {
    user.require_write()?;
    let locked = super::ops::class_lock_active(&state)
        .await
        .map_err(internal)?;
    if locked {
        if body.force_class_lock {
            user.require_super()?;
            let _ = super::audit::record(
                &state.pool,
                &user.0.username,
                "deploy.force_class_lock",
                "task",
                &body.image_id,
                "deploy during class lock",
            )
            .await;
        } else {
            return Err((
                StatusCode::FORBIDDEN,
                "class hour lock active; set force_class_lock=true as super to override".into(),
            ));
        }
    }
    if !body.confirm_destructive {
        return Err((
            StatusCode::BAD_REQUEST,
            "confirm_destructive=true required after reviewing overwrite scope".into(),
        ));
    }
    if !matches!(
        body.disk_plan.as_str(),
        "wipe_whole_disk" | "wipe_target_partitions" | "write_system_only"
    ) {
        return Err((StatusCode::BAD_REQUEST, "invalid disk_plan".into()));
    }
    let image_meta: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT content_type, base_image_id FROM images WHERE id = ?")
            .bind(&body.image_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some((content_type, base_image_id)) = image_meta else {
        return Err((StatusCode::BAD_REQUEST, "image not found".into()));
    };
    if content_type == "delta" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "delta image cannot deploy directly (base={}); apply-delta to produce full first",
                base_image_id.unwrap_or_default()
            ),
        ));
    }
    if body.macs.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "macs required".into()));
    }

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM maintenance_task_targets WHERE state IN ('pending', 'running')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let incoming = body.macs.len() as i64;
    if active + incoming > state.max_concurrent_deploy_targets {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "deploy concurrency limit: active={active} + new={incoming} > max={}",
                state.max_concurrent_deploy_targets
            ),
        ));
    }

    let now = Utc::now();
    let mut normalized_macs = Vec::with_capacity(body.macs.len());
    for raw in &body.macs {
        normalized_macs.push(normalize_mac(raw).map_err(|e| (StatusCode::BAD_REQUEST, e))?);
    }
    let edge_id = match body.edge_id.filter(|s| !s.trim().is_empty()) {
        Some(id) => Some(id),
        None => {
            if let Some(mac0) = normalized_macs.first() {
                if let Some(lab_id) =
                    crate::site_defaults::lab_id_for_mac(&state.pool, mac0).await?
                {
                    crate::site_defaults::resolve_preferred_edge_id(&state.pool, &lab_id).await?
                } else {
                    None
                }
            } else {
                None
            }
        }
    };
    let task = MaintenanceTask {
        id: new_id(),
        kind: "deploy".into(),
        state: "pending".into(),
        image_id: Some(body.image_id),
        edge_id,
        disk_plan: body.disk_plan,
        name: if body.name.trim().is_empty() {
            format!("deploy-{}", now.timestamp())
        } else {
            body.name
        },
        error_message: String::new(),
        created_by: user.0.username,
        created_at: now,
        updated_at: now,
    };
    insert_task(&state, &task).await?;
    let mut targets = Vec::new();
    for mac in normalized_macs {
        let target = MaintenanceTaskTarget {
            id: new_id(),
            task_id: task.id.clone(),
            mac,
            state: "pending".into(),
            progress: 0.0,
            error_message: String::new(),
            detail: String::new(),
            p2p_bytes: 0,
            edge_bytes: 0,
            updated_at: now,
        };
        insert_target(&state, &target).await?;
        targets.push(target);
    }
    Ok((StatusCode::CREATED, Json(TaskView { task, targets })))
}

pub async fn create_agent_update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<crate::models::CreateAgentUpdateTask>,
) -> Result<(StatusCode, Json<TaskView>), (StatusCode, String)> {
    user.require_write()?;
    let version = body.version.trim().to_string();
    if !super::agent_packages::valid_version(&version) {
        return Err((StatusCode::BAD_REQUEST, "invalid agent version".into()));
    }
    let meta = super::agent_packages::package_meta(&state, &version)
        .await
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("agent package {version} not uploaded"),
            )
        })?;
    if body.macs.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "macs required".into()));
    }
    let mut macs = Vec::with_capacity(body.macs.len());
    for raw in &body.macs {
        macs.push(normalize_mac(raw).map_err(|e| (StatusCode::BAD_REQUEST, e))?);
    }
    macs.sort();
    macs.dedup();

    let now = Utc::now();
    let task = MaintenanceTask {
        id: new_id(),
        kind: "agent_update".into(),
        state: "running".into(),
        image_id: Some(version.clone()),
        edge_id: None,
        disk_plan: "none".into(),
        name: if body.name.trim().is_empty() {
            format!("agent-update-{version}")
        } else {
            body.name
        },
        error_message: String::new(),
        created_by: user.0.username.clone(),
        created_at: now,
        updated_at: now,
    };
    insert_task(&state, &task).await?;
    let payload = serde_json::json!({
        "version": version,
        "download_url": format!("/api/v1/agent/packages/{version}/download"),
        "sha256": meta.sha256,
    })
    .to_string();
    let mut targets = Vec::new();
    for mac in macs {
        let target = MaintenanceTaskTarget {
            id: new_id(),
            task_id: task.id.clone(),
            mac: mac.clone(),
            state: "pending".into(),
            progress: 0.0,
            error_message: String::new(),
            detail: format!("waiting for client {version}"),
            p2p_bytes: 0,
            edge_bytes: 0,
            updated_at: now,
        };
        insert_target(&state, &target).await?;
        let _ = super::agent_commands::enqueue(
            &state,
            &mac,
            "update_agent",
            &payload,
            &user.0.username,
        )
        .await;
        targets.push(target);
    }
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "task.agent_update",
        "maintenance_task",
        &task.id,
        &format!("version={version} targets={}", targets.len()),
    )
    .await;
    Ok((StatusCode::CREATED, Json(TaskView { task, targets })))
}

/// Enqueue one update_agent command for the newest active agent_update target.
/// Used right after the client reports `reboot_for_update`.
pub async fn requeue_agent_update(state: &AppState, mac: &str) -> anyhow::Result<()> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT COALESCE(t.image_id, '') FROM maintenance_tasks t \
         JOIN maintenance_task_targets mt ON mt.task_id = t.id \
         WHERE t.kind = 'agent_update' AND mt.mac = ? \
           AND mt.state IN ('pending', 'running') \
         ORDER BY t.created_at DESC LIMIT 1",
    )
    .bind(mac)
    .fetch_optional(&state.pool)
    .await?;
    let Some((version,)) = row else {
        return Ok(());
    };
    let version = version.trim().to_string();
    if version.is_empty() {
        return Ok(());
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_commands WHERE mac = ? AND kind = 'update_agent' \
         AND state IN ('pending', 'delivered')",
    )
    .bind(mac)
    .fetch_one(&state.pool)
    .await?;
    if pending > 0 {
        return Ok(());
    }
    let sha = super::agent_packages::package_meta(state, &version)
        .await
        .map(|p| p.sha256)
        .unwrap_or_default();
    let payload = serde_json::json!({
        "version": version,
        "download_url": format!("/api/v1/agent/packages/{version}/download"),
        "sha256": sha,
    })
    .to_string();
    let _ = super::agent_commands::enqueue(state, mac, "update_agent", &payload, "system").await;
    Ok(())
}

/// Called from Agent heartbeat: advances `agent_update` tasks and retries the
/// update command after the client rebooted to disarm restore (old version).
pub async fn on_agent_heartbeat(
    state: &AppState,
    mac: &str,
    agent_version: &str,
) -> anyhow::Result<()> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.id, COALESCE(t.image_id, ''), mt.id \
         FROM maintenance_tasks t \
         JOIN maintenance_task_targets mt ON mt.task_id = t.id \
         WHERE t.kind = 'agent_update' AND mt.mac = ? \
           AND mt.state IN ('pending', 'running') \
         ORDER BY t.created_at DESC",
    )
    .bind(mac)
    .fetch_all(&state.pool)
    .await?;

    for (task_id, want_version, target_id) in rows {
        let version = want_version.trim();
        if version.is_empty() {
            continue;
        }
        if version == agent_version.trim() {
            sqlx::query(
                "UPDATE maintenance_task_targets SET state = 'done', progress = 1.0, \
                 detail = ?, updated_at = ? WHERE id = ?",
            )
            .bind(format!("updated to {version}"))
            .bind(Utc::now())
            .bind(&target_id)
            .execute(&state.pool)
            .await?;
            refresh_task_aggregate(state, &task_id).await?;
            // Let the new agent re-apply its restore policy (it will arm + reboot).
            let _ = super::agent_commands::enqueue(
                state,
                mac,
                "apply_policy",
                "{\"force\":true}",
                "system",
            )
            .await;
            continue;
        }

        sqlx::query(
            "UPDATE maintenance_task_targets SET state = 'running', detail = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(format!("current={agent_version} target={version}"))
        .bind(Utc::now())
        .bind(&target_id)
        .execute(&state.pool)
        .await?;

        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_commands WHERE mac = ? AND kind = 'update_agent' \
             AND state IN ('pending', 'delivered')",
        )
        .bind(mac)
        .fetch_one(&state.pool)
        .await?;
        let last_attempt: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM agent_commands WHERE mac = ? AND kind = 'update_agent' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(mac)
        .fetch_optional(&state.pool)
        .await?;
        let recently_attempted = last_attempt
            .map(|t| Utc::now() - t < Duration::seconds(15))
            .unwrap_or(false);
        if pending == 0 && !recently_attempted {
            let sha = super::agent_packages::package_meta(state, version)
                .await
                .map(|p| p.sha256)
                .unwrap_or_default();
            let payload = serde_json::json!({
                "version": version,
                "download_url": format!("/api/v1/agent/packages/{version}/download"),
                "sha256": sha,
            })
            .to_string();
            let _ = super::agent_commands::enqueue(state, mac, "update_agent", &payload, "system")
                .await;
        }
    }
    Ok(())
}

pub async fn preview_deploy(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<CreateDeployTask>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let image = sqlx::query_as::<_, crate::models::Image>(
        "SELECT id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at FROM images WHERE id = ?",
    )
    .bind(&body.image_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::BAD_REQUEST, "image not found".into()))?;

    let parts: serde_json::Value =
        serde_json::from_str(&image.partition_map).unwrap_or_else(|_| serde_json::json!([]));
    let (overwrite, keep) = match body.disk_plan.as_str() {
        "wipe_whole_disk" => (parts.clone(), serde_json::json!([])),
        "wipe_target_partitions" | "write_system_only" => {
            let arr = parts.as_array().cloned().unwrap_or_default();
            let overwrite: Vec<_> = arr
                .iter()
                .filter(|p| p.get("role").and_then(|r| r.as_str()) == Some("system"))
                .cloned()
                .collect();
            let keep: Vec<_> = arr
                .iter()
                .filter(|p| p.get("role").and_then(|r| r.as_str()) != Some("system"))
                .cloned()
                .collect();
            (serde_json::json!(overwrite), serde_json::json!(keep))
        }
        _ => {
            return Err((StatusCode::BAD_REQUEST, "invalid disk_plan".into()));
        }
    };

    Ok(Json(serde_json::json!({
        "disk_plan": body.disk_plan,
        "image_id": image.id,
        "image_name": image.name,
        "warning": "被覆盖分区上的原系统与数据不可恢复",
        "overwrite_partitions": overwrite,
        "keep_partitions": keep,
        "target_count": body.macs.len()
    })))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<TaskView>, (StatusCode, String)> {
    user.require_write()?;
    let task = load_task(&state, &id).await?;
    if matches!(task.state.as_str(), "done" | "failed") {
        let targets = load_targets(&state, &id).await?;
        return Ok(Json(TaskView { task, targets }));
    }
    fail_active_targets(&state, &id, "管理员结束")
        .await
        .map_err(internal)?;
    let task = load_task(&state, &id).await?;
    let targets = load_targets(&state, &id).await?;
    Ok(Json(TaskView { task, targets }))
}

/// Mark stale pending/running targets as failed. Returns number of targets updated.
pub async fn sweep_stale_tasks(state: &AppState) -> anyhow::Result<u64> {
    let cfg = crate::api::ops::load_task_lifecycle(state).await?;
    if cfg.stale_after_secs <= 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - Duration::seconds(cfg.stale_after_secs);
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, task_id FROM maintenance_task_targets WHERE state IN ('pending', 'running') AND updated_at < ?",
    )
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let now = Utc::now();
    let msg = format!("超时无进度（>{}s）", cfg.stale_after_secs);
    let mut task_ids = std::collections::BTreeSet::new();
    for (target_id, task_id) in &rows {
        sqlx::query(
            "UPDATE maintenance_task_targets SET state = 'failed', error_message = ?, updated_at = ? WHERE id = ? AND state IN ('pending', 'running')",
        )
        .bind(&msg)
        .bind(now)
        .bind(target_id)
        .execute(&state.pool)
        .await?;
        task_ids.insert(task_id.clone());
    }
    for task_id in task_ids {
        refresh_task_aggregate(state, &task_id).await?;
    }
    Ok(rows.len() as u64)
}

async fn fail_active_targets(state: &AppState, task_id: &str, message: &str) -> anyhow::Result<()> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE maintenance_task_targets SET state = 'failed', error_message = ?, updated_at = ? WHERE task_id = ? AND state IN ('pending', 'running')",
    )
    .bind(message)
    .bind(now)
    .bind(task_id)
    .execute(&state.pool)
    .await?;
    refresh_task_aggregate(state, task_id).await?;
    Ok(())
}

async fn refresh_task_aggregate(state: &AppState, task_id: &str) -> anyhow::Result<()> {
    let states: Vec<(String,)> =
        sqlx::query_as("SELECT state FROM maintenance_task_targets WHERE task_id = ?")
            .bind(task_id)
            .fetch_all(&state.pool)
            .await?;
    if states.is_empty() {
        return Ok(());
    }
    let all_done = states.iter().all(|(s,)| s == "done");
    let any_failed = states.iter().any(|(s,)| s == "failed");
    let any_active = states.iter().any(|(s,)| s == "pending" || s == "running");
    let state_name = if all_done {
        "done"
    } else if any_failed && !any_active {
        "failed"
    } else {
        "running"
    };
    let err = if state_name == "failed" {
        let msgs: Vec<(String,)> = sqlx::query_as(
            "SELECT error_message FROM maintenance_task_targets WHERE task_id = ? AND error_message != '' LIMIT 1",
        )
        .bind(task_id)
        .fetch_all(&state.pool)
        .await?;
        msgs.first().map(|(m,)| m.clone()).unwrap_or_default()
    } else {
        String::new()
    };
    let now = Utc::now();
    sqlx::query(
        "UPDATE maintenance_tasks SET state = ?, error_message = ?, updated_at = ? WHERE id = ?",
    )
    .bind(state_name)
    .bind(err)
    .bind(now)
    .bind(task_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn load_task(state: &AppState, id: &str) -> Result<MaintenanceTask, (StatusCode, String)> {
    sqlx::query_as::<_, MaintenanceTask>(
        "SELECT id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at FROM maintenance_tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "task not found".into()))
}

async fn load_targets(
    state: &AppState,
    task_id: &str,
) -> Result<Vec<MaintenanceTaskTarget>, (StatusCode, String)> {
    sqlx::query_as::<_, MaintenanceTaskTarget>(
        "SELECT id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at FROM maintenance_task_targets WHERE task_id = ? ORDER BY mac",
    )
    .bind(task_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)
}

async fn insert_task(state: &AppState, task: &MaintenanceTask) -> Result<(), (StatusCode, String)> {
    sqlx::query(
        "INSERT INTO maintenance_tasks (id, kind, state, image_id, edge_id, disk_plan, name, error_message, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&task.id)
    .bind(&task.kind)
    .bind(&task.state)
    .bind(&task.image_id)
    .bind(&task.edge_id)
    .bind(&task.disk_plan)
    .bind(&task.name)
    .bind(&task.error_message)
    .bind(&task.created_by)
    .bind(task.created_at)
    .bind(task.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

async fn insert_target(
    state: &AppState,
    target: &MaintenanceTaskTarget,
) -> Result<(), (StatusCode, String)> {
    sqlx::query(
        "INSERT INTO maintenance_task_targets (id, task_id, mac, state, progress, error_message, detail, p2p_bytes, edge_bytes, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&target.id)
    .bind(&target.task_id)
    .bind(&target.mac)
    .bind(&target.state)
    .bind(target.progress)
    .bind(&target.error_message)
    .bind(&target.detail)
    .bind(target.p2p_bytes)
    .bind(target.edge_bytes)
    .bind(target.updated_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
