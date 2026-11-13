//! Phase-3 ops: class lock + power schedules.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{EnrollmentSettings, PowerCommand, new_id};
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct ClassLockSettings {
    pub enabled: bool,
    /// Inclusive UTC hour start (0-23)
    pub start_hour_utc: u32,
    /// Exclusive UTC hour end (1-24)
    pub end_hour_utc: u32,
}

impl Default for ClassLockSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start_hour_utc: 1,
            end_hour_utc: 10,
        }
    }
}

pub async fn get_class_lock(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<ClassLockSettings>, (StatusCode, String)> {
    Ok(Json(load_class_lock(&state).await.map_err(internal)?))
}

pub async fn set_class_lock(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<ClassLockSettings>,
) -> Result<Json<ClassLockSettings>, (StatusCode, String)> {
    user.require_write()?;
    if body.start_hour_utc > 23 || body.end_hour_utc > 24 || body.start_hour_utc >= body.end_hour_utc
    {
        return Err((StatusCode::BAD_REQUEST, "invalid hour range".into()));
    }
    let value = serde_json::to_string(&body).map_err(internal)?;
    sqlx::query(
        r#"
        INSERT INTO ops_settings (key, value, updated_at) VALUES ('class_lock', ?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(&value)
    .bind(Utc::now())
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "ops.class_lock",
        "ops_settings",
        "class_lock",
        &value,
    )
    .await;
    Ok(Json(body))
}

pub async fn load_class_lock(state: &AppState) -> anyhow::Result<ClassLockSettings> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value FROM ops_settings WHERE key = 'class_lock'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(row
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

pub async fn class_lock_active(state: &AppState) -> anyhow::Result<bool> {
    let cfg = load_class_lock(state).await?;
    if !cfg.enabled {
        return Ok(false);
    }
    let hour = Utc::now().hour();
    Ok(hour >= cfg.start_hour_utc && hour < cfg.end_hour_utc)
}

pub async fn get_enrollment(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<EnrollmentSettings>, (StatusCode, String)> {
    Ok(Json(load_enrollment(&state).await.map_err(internal)?))
}

pub async fn set_enrollment(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<EnrollmentSettings>,
) -> Result<Json<EnrollmentSettings>, (StatusCode, String)> {
    user.require_write()?;
    let mut body = body;
    body.default_lab_id = body.default_lab_id.trim().to_string();
    body.default_campus_id = body.default_campus_id.trim().to_string();
    if !body.default_lab_id.is_empty() {
        let lab = crate::lab_naming::load_lab(&state, &body.default_lab_id).await?;
        if !body.default_campus_id.is_empty() && body.default_campus_id != lab.campus_id {
            return Err((
                StatusCode::BAD_REQUEST,
                "default_campus_id does not match lab".into(),
            ));
        }
        body.default_campus_id = lab.campus_id;
    } else {
        body.default_campus_id.clear();
    }
    let value = serde_json::to_string(&body).map_err(internal)?;
    sqlx::query(
        r#"
        INSERT INTO ops_settings (key, value, updated_at) VALUES ('enrollment', ?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(&value)
    .bind(Utc::now())
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "ops.enrollment",
        "ops_settings",
        "enrollment",
        &value,
    )
    .await;
    Ok(Json(body))
}

pub async fn load_enrollment(state: &AppState) -> anyhow::Result<EnrollmentSettings> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value FROM ops_settings WHERE key = 'enrollment'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(row
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

/// Auto-fail stale maintenance targets; `stale_after_secs = 0` disables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLifecycleSettings {
    /// Seconds without target `updated_at` progress before auto-fail. 0 = off.
    pub stale_after_secs: i64,
}

impl Default for TaskLifecycleSettings {
    fn default() -> Self {
        Self {
            stale_after_secs: 12 * 3600,
        }
    }
}

pub async fn get_task_lifecycle(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<TaskLifecycleSettings>, (StatusCode, String)> {
    Ok(Json(load_task_lifecycle(&state).await.map_err(internal)?))
}

pub async fn set_task_lifecycle(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<TaskLifecycleSettings>,
) -> Result<Json<TaskLifecycleSettings>, (StatusCode, String)> {
    user.require_write()?;
    if body.stale_after_secs < 0 {
        return Err((StatusCode::BAD_REQUEST, "stale_after_secs must be >= 0".into()));
    }
    let value = serde_json::to_string(&body).map_err(internal)?;
    sqlx::query(
        r#"
        INSERT INTO ops_settings (key, value, updated_at) VALUES ('task_lifecycle', ?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(&value)
    .bind(Utc::now())
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = super::audit::record(
        &state.pool,
        &user.0.username,
        "ops.task_lifecycle",
        "ops_settings",
        "task_lifecycle",
        &value,
    )
    .await;
    Ok(Json(body))
}

pub async fn load_task_lifecycle(state: &AppState) -> anyhow::Result<TaskLifecycleSettings> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value FROM ops_settings WHERE key = 'task_lifecycle'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(row
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PowerSchedule {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mac_or_tag: String,
    pub target_type: String,
    pub minute_utc: i64,
    pub hour_utc: i64,
    pub enabled: i64,
    pub last_fired_date: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePowerSchedule {
    pub name: String,
    pub kind: String,
    pub mac_or_tag: String,
    #[serde(default = "default_target_type")]
    pub target_type: String,
    pub minute_utc: i64,
    pub hour_utc: i64,
}

fn default_target_type() -> String {
    "mac".into()
}

pub async fn list_schedules(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<PowerSchedule>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, PowerSchedule>(
        "SELECT id, name, kind, mac_or_tag, target_type, minute_utc, hour_utc, enabled, last_fired_date, created_at FROM power_schedules ORDER BY hour_utc, minute_utc",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn force_tick(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    user.require_write()?;
    tick_power_schedules(&state).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn create_schedule(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreatePowerSchedule>,
) -> Result<(StatusCode, Json<PowerSchedule>), (StatusCode, String)> {
    user.require_write()?;
    let kind = body.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "wol" | "shutdown") {
        return Err((StatusCode::BAD_REQUEST, "kind must be wol|shutdown".into()));
    }
    let target_type = body.target_type.trim().to_ascii_lowercase();
    if !matches!(target_type.as_str(), "mac" | "tag") {
        return Err((StatusCode::BAD_REQUEST, "target_type must be mac|tag".into()));
    }
    if !(0..60).contains(&body.minute_utc) || !(0..24).contains(&body.hour_utc) {
        return Err((StatusCode::BAD_REQUEST, "invalid hour/minute".into()));
    }
    let now = Utc::now();
    let row = PowerSchedule {
        id: new_id(),
        name: body.name,
        kind,
        mac_or_tag: body.mac_or_tag,
        target_type,
        minute_utc: body.minute_utc,
        hour_utc: body.hour_utc,
        enabled: 1,
        last_fired_date: String::new(),
        created_at: now.to_rfc3339(),
    };
    sqlx::query(
        "INSERT INTO power_schedules (id, name, kind, mac_or_tag, target_type, minute_utc, hour_utc, enabled, last_fired_date, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.name)
    .bind(&row.kind)
    .bind(&row.mac_or_tag)
    .bind(&row.target_type)
    .bind(row.minute_utc)
    .bind(row.hour_utc)
    .bind(row.enabled)
    .bind(&row.last_fired_date)
    .bind(&row.created_at)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// Fire due schedules once per UTC calendar day.
pub async fn tick_power_schedules(state: &AppState) -> anyhow::Result<()> {
    let now = Utc::now();
    let today = now.date_naive().to_string();
    let rows = sqlx::query_as::<_, PowerSchedule>(
        "SELECT id, name, kind, mac_or_tag, target_type, minute_utc, hour_utc, enabled, last_fired_date, created_at FROM power_schedules WHERE enabled = 1",
    )
    .fetch_all(&state.pool)
    .await?;
    for row in rows {
        if row.last_fired_date == today {
            continue;
        }
        if row.hour_utc != now.hour() as i64 || row.minute_utc != now.minute() as i64 {
            continue;
        }
        let macs = resolve_targets(state, &row).await?;
        for mac in macs {
            enqueue_power(state, &mac, &row.kind, &format!("schedule:{}", row.name)).await?;
        }
        sqlx::query("UPDATE power_schedules SET last_fired_date = ? WHERE id = ?")
            .bind(&today)
            .bind(&row.id)
            .execute(&state.pool)
            .await?;
        let _ = super::audit::record(
            &state.pool,
            "system",
            "power.schedule_fire",
            "power_schedule",
            &row.id,
            &format!("kind={} targets_via={}", row.kind, row.mac_or_tag),
        )
        .await;
    }
    Ok(())
}

async fn resolve_targets(state: &AppState, row: &PowerSchedule) -> anyhow::Result<Vec<String>> {
    if row.target_type == "tag" {
        let macs: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT c.mac FROM clients c
            JOIN client_tags t ON t.client_id = c.id
            WHERE t.tag = ?
            "#,
        )
        .bind(&row.mac_or_tag)
        .fetch_all(&state.pool)
        .await?;
        return Ok(macs.into_iter().map(|m| m.0).collect());
    }
    let mac = normalize_mac(&row.mac_or_tag).unwrap_or_else(|_| row.mac_or_tag.clone());
    Ok(vec![mac])
}

async fn enqueue_power(
    state: &AppState,
    mac: &str,
    kind: &str,
    detail: &str,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let mut detail = detail.to_string();
    let state_name = if kind == "wol" {
        match idv_common::wol::send_magic_packet(mac) {
            Ok(msg) => {
                detail = format!("{detail}; {msg}");
                "done"
            }
            Err(err) => {
                detail = format!("{detail}; wol_send_failed: {err}");
                "done"
            }
        }
    } else {
        "pending"
    };
    let cmd = PowerCommand {
        id: new_id(),
        client_id: None,
        mac: mac.to_string(),
        kind: kind.to_string(),
        state: state_name.into(),
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
    .await?;
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
