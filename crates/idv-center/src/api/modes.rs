use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;

use crate::auth::AuthUser;
use crate::models::{
    BindLabMode, CreateModePolicy, LabModeBinding, ModePolicy, UpdateModeSchedule, new_id,
};
use crate::state::AppState;

const MODE_COLS: &str = "id, name, restore, system_action, persistent_action, volatile_action, notes, schedule_start, schedule_end, usb_restricted, protect_scope, created_at, updated_at";

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<ModePolicy>>, (StatusCode, String)> {
    let sql = format!("SELECT {MODE_COLS} FROM mode_policies ORDER BY name");
    let rows = sqlx::query_as::<_, ModePolicy>(&sql)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn create(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<CreateModePolicy>,
) -> Result<(StatusCode, Json<ModePolicy>), (StatusCode, String)> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name required".into()));
    }
    validate_mode(
        &body.restore,
        &body.system_action,
        &body.persistent_action,
        &body.volatile_action,
    )?;
    let protect_scope = match body.protect_scope.trim().to_ascii_lowercase().as_str() {
        "all_fixed" | "all" => "all_fixed".to_string(),
        "" | "system" => "system".to_string(),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("protect_scope must be system|all_fixed, got '{other}'"),
            ));
        }
    };
    let now = Utc::now();
    let policy = ModePolicy {
        id: new_id(),
        name: name.to_string(),
        restore: body.restore,
        system_action: body.system_action,
        persistent_action: body.persistent_action,
        volatile_action: body.volatile_action,
        notes: body.notes,
        schedule_start: body.schedule_start.filter(|s| !s.trim().is_empty()),
        schedule_end: body.schedule_end.filter(|s| !s.trim().is_empty()),
        usb_restricted: if body.usb_restricted { 1 } else { 0 },
        protect_scope,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "INSERT INTO mode_policies (id, name, restore, system_action, persistent_action, volatile_action, notes, schedule_start, schedule_end, usb_restricted, protect_scope, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&policy.id)
    .bind(&policy.name)
    .bind(&policy.restore)
    .bind(&policy.system_action)
    .bind(&policy.persistent_action)
    .bind(&policy.volatile_action)
    .bind(&policy.notes)
    .bind(&policy.schedule_start)
    .bind(&policy.schedule_end)
    .bind(policy.usb_restricted)
    .bind(&policy.protect_scope)
    .bind(policy.created_at)
    .bind(policy.updated_at)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "mode name exists".into())
        } else {
            internal(err)
        }
    })?;
    Ok((StatusCode::CREATED, Json(policy)))
}

pub async fn update_schedule(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateModeSchedule>,
) -> Result<Json<ModePolicy>, (StatusCode, String)> {
    let existing = load_mode(&state, &id).await?;
    let start = body
        .schedule_start
        .or(existing.schedule_start.clone())
        .filter(|s| !s.trim().is_empty());
    let end = body
        .schedule_end
        .or(existing.schedule_end.clone())
        .filter(|s| !s.trim().is_empty());
    let usb = body
        .usb_restricted
        .map(|b| if b { 1 } else { 0 })
        .unwrap_or(existing.usb_restricted);
    let protect_scope = match body
        .protect_scope
        .as_deref()
        .unwrap_or(&existing.protect_scope)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "all_fixed" | "all" => "all_fixed",
        "" | "system" => "system",
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("protect_scope must be system|all_fixed, got '{other}'"),
            ));
        }
    };
    let now = Utc::now();
    sqlx::query(
        "UPDATE mode_policies SET schedule_start = ?, schedule_end = ?, usb_restricted = ?, protect_scope = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&start)
    .bind(&end)
    .bind(usb)
    .bind(protect_scope)
    .bind(now)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;

    // Scope change must produce a new DesiredState revision on bound labs, and
    // long-poll clients get an immediate apply_policy (they auto-reboot once).
    if existing.protect_scope.as_str() != protect_scope {
        sqlx::query("UPDATE lab_mode_bindings SET updated_at = ? WHERE mode_policy_id = ?")
            .bind(now)
            .bind(&id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        let macs: Vec<(String,)> = sqlx::query_as(
            "SELECT c.mac FROM clients c JOIN lab_mode_bindings b ON b.lab_id = c.lab_id WHERE b.mode_policy_id = ?",
        )
        .bind(&id)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
        for (mac,) in &macs {
            let _ =
                super::agent_commands::enqueue(&state, mac, "apply_policy", "", &user.0.username)
                    .await;
        }
    }

    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "mode.schedule_update",
        "mode_policy",
        &id,
        &format!(
            "name={} start={:?} end={:?} usb={}",
            existing.name, start, end, usb
        ),
    )
    .await;
    load_mode(&state, &id).await.map(Json)
}

pub async fn delete(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM mode_policies WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "mode not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn bind_lab(
    State(state): State<AppState>,
    user: AuthUser,
    Path(lab_id): Path<String>,
    Json(body): Json<BindLabMode>,
) -> Result<Json<LabModeBinding>, (StatusCode, String)> {
    user.require_write()?;
    let lab: Option<(String,)> = sqlx::query_as("SELECT id FROM labs WHERE id = ?")
        .bind(&lab_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if lab.is_none() {
        return Err((StatusCode::NOT_FOUND, "lab not found".into()));
    }
    let mode = load_mode(&state, &body.mode_policy_id).await?;
    if !matches!(body.effective.as_str(), "next_boot" | "immediate") {
        return Err((
            StatusCode::BAD_REQUEST,
            "effective must be next_boot|immediate".into(),
        ));
    }
    let prev: Option<String> =
        sqlx::query_scalar("SELECT mode_policy_id FROM lab_mode_bindings WHERE lab_id = ?")
            .bind(&lab_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let now = Utc::now();
    sqlx::query(
        r#"
        INSERT INTO lab_mode_bindings (lab_id, mode_policy_id, effective, updated_at)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(lab_id) DO UPDATE SET
          mode_policy_id = excluded.mode_policy_id,
          effective = excluded.effective,
          updated_at = excluded.updated_at
        "#,
    )
    .bind(&lab_id)
    .bind(&body.mode_policy_id)
    .bind(&body.effective)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let mode_changed = prev.as_deref() != Some(body.mode_policy_id.as_str());
    let mut reboot_n = 0usize;
    if mode_changed {
        reboot_n = super::power::enqueue_reboots_for_lab(
            &state,
            &lab_id,
            &format!("mode_bind→{} auto-reboot", mode.name),
        )
        .await
        .unwrap_or(0);
    }
    // Push an immediate policy apply to every client in the lab (persistent
    // agents receive it via long-poll; old agents still get it on heartbeat).
    let macs: Vec<(String,)> = sqlx::query_as("SELECT mac FROM clients WHERE lab_id = ?")
        .bind(&lab_id)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    for (mac,) in &macs {
        let _ =
            super::agent_commands::enqueue(&state, mac, "apply_policy", "", &user.0.username).await;
    }
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "mode.bind_lab",
        "lab",
        &lab_id,
        &format!(
            "mode={} ({}) effective={} prev={:?} reboot_queued={}",
            mode.name, body.mode_policy_id, body.effective, prev, reboot_n
        ),
    )
    .await;
    Ok(Json(LabModeBinding {
        lab_id,
        mode_policy_id: body.mode_policy_id,
        effective: body.effective,
        updated_at: now,
    }))
}

pub async fn list_bindings(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<LabModeBinding>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, LabModeBinding>(
        "SELECT lab_id, mode_policy_id, effective, updated_at FROM lab_mode_bindings ORDER BY updated_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn load_mode(state: &AppState, id: &str) -> Result<ModePolicy, (StatusCode, String)> {
    let sql = format!("SELECT {MODE_COLS} FROM mode_policies WHERE id = ?");
    sqlx::query_as::<_, ModePolicy>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "mode not found".into()))
}

fn validate_mode(
    restore: &str,
    system: &str,
    persistent: &str,
    volatile: &str,
) -> Result<(), (StatusCode, String)> {
    if !matches!(restore, "reboot" | "manual" | "off") {
        return Err((
            StatusCode::BAD_REQUEST,
            "restore must be reboot|manual|off".into(),
        ));
    }
    if !matches!(system, "restore" | "keep") {
        return Err((
            StatusCode::BAD_REQUEST,
            "system_action must be restore|keep".into(),
        ));
    }
    if !matches!(persistent, "keep" | "wipe") {
        return Err((
            StatusCode::BAD_REQUEST,
            "persistent_action must be keep|wipe".into(),
        ));
    }
    if !matches!(volatile, "keep" | "wipe") {
        return Err((
            StatusCode::BAD_REQUEST,
            "volatile_action must be keep|wipe".into(),
        ));
    }
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
