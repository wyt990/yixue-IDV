use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use idv_common::restore::should_restore_now;

use crate::mac::normalize_mac;
use crate::models::{
    AgentHeartbeat, AgentPolicyResponse, AgentRestoreReport, AgentUsbReport,
    DeltaRecommendRequest, DeltaRecommendResponse, Image, ModePolicy, PowerCommand, new_id,
};
use crate::state::AppState;

const MODE_COLS: &str = "id, name, restore, system_action, persistent_action, volatile_action, notes, schedule_start, schedule_end, usb_restricted, created_at, updated_at";

pub async fn heartbeat(
    State(state): State<AppState>,
    Json(body): Json<AgentHeartbeat>,
) -> Result<Json<AgentPolicyResponse>, (StatusCode, String)> {
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();

    let client: Option<(String, String)> =
        sqlx::query_as("SELECT id, lab_id FROM clients WHERE mac = ?")
            .bind(&mac)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let (client_id, lab_id) = match client {
        Some((cid, lid)) => {
            sqlx::query("UPDATE clients SET last_seen = ? WHERE id = ?")
                .bind(now)
                .bind(&cid)
                .execute(&state.pool)
                .await
                .map_err(internal)?;
            (Some(cid), Some(lid))
        }
        None => match crate::enroll::ensure_client_enrolled(
            &state,
            &mac,
            &body.hostname,
            now,
        )
        .await?
        {
            Some(e) => (Some(e.client_id), Some(e.lab_id)),
            None => (None, None),
        },
    };

    let mut mode = if let Some(ref lid) = lab_id {
        let binding: Option<(String,)> =
            sqlx::query_as("SELECT mode_policy_id FROM lab_mode_bindings WHERE lab_id = ?")
                .bind(lid)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?;
        if let Some((mid,)) = binding {
            let sql = format!("SELECT {MODE_COLS} FROM mode_policies WHERE id = ?");
            sqlx::query_as::<_, ModePolicy>(&sql)
                .bind(mid)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?
        } else {
            None
        }
    } else {
        None
    };

    let baseline_image_id = if let Some(ref lid) = lab_id {
        crate::site_defaults::resolve_baseline_image_id(&state.pool, lid).await?
    } else {
        None
    };

    let mut schedule_active = mode
        .as_ref()
        .map(|m| schedule_is_active(m, now))
        .unwrap_or(true);
    let mut fallback_note: Option<String> = None;

    // Exam window ended → auto rebind lab to teaching and audit.
    if let (Some(m), Some(lid)) = (&mode, &lab_id) {
        if m.name == "exam" && !schedule_active {
            let teach_id: Option<String> =
                sqlx::query_scalar("SELECT id FROM mode_policies WHERE name = 'teaching' LIMIT 1")
                    .fetch_optional(&state.pool)
                    .await
                    .map_err(internal)?;
            if let Some(tid) = teach_id {
                sqlx::query(
                    "UPDATE lab_mode_bindings SET mode_policy_id = ?, effective = 'next_boot', updated_at = ? WHERE lab_id = ?",
                )
                .bind(&tid)
                .bind(now)
                .bind(lid)
                .execute(&state.pool)
                .await
                .map_err(internal)?;
                let _ = super::audit::record(
                    &state.pool,
                    "system",
                    "mode.exam_fallback",
                    "lab",
                    lid,
                    &format!("from_exam={} to_teaching={}", m.id, tid),
                )
                .await;
                let sql = format!("SELECT {MODE_COLS} FROM mode_policies WHERE id = ?");
                mode = sqlx::query_as::<_, ModePolicy>(&sql)
                    .bind(tid)
                    .fetch_optional(&state.pool)
                    .await
                    .map_err(internal)?;
                schedule_active = true;
                fallback_note = Some("考试时间窗已结束，已自动回落教学模式".into());
            }
        }
    }

    let usb_restricted = mode
        .as_ref()
        .map(|m| m.usb_restricted != 0 && schedule_active)
        .unwrap_or(false);

    let should = mode
        .as_ref()
        .map(|m| should_restore_now(&m.restore, &body.trigger))
        .unwrap_or(false);

    let pending_power = sqlx::query_as::<_, PowerCommand>(
        "SELECT id, client_id, mac, kind, state, detail, created_at, updated_at FROM power_commands WHERE mac = ? AND state = 'pending' AND kind = 'shutdown' ORDER BY created_at ASC LIMIT 1",
    )
    .bind(&mac)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;

    let pending_software = super::software::pending_for_mac(&state, &mac)
        .await
        .map_err(internal)?;
    let pending_assist = super::assist::pending_for_mac(&state, &mac)
        .await
        .map_err(internal)?;

    let mode_id = mode.as_ref().map(|m| m.id.clone());
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM agent_sessions WHERE mac = ?")
            .bind(&mac)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE agent_sessions SET client_id = ?, hostname = ?, agent_version = ?, mode_policy_id = ?, last_seen = ?, status = 'online' WHERE id = ?",
        )
        .bind(&client_id)
        .bind(&body.hostname)
        .bind(&body.agent_version)
        .bind(&mode_id)
        .bind(now)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    } else {
        sqlx::query(
            "INSERT INTO agent_sessions (id, client_id, mac, hostname, agent_version, mode_policy_id, last_seen, status, detail) VALUES (?, ?, ?, ?, ?, ?, ?, 'online', '')",
        )
        .bind(new_id())
        .bind(&client_id)
        .bind(&mac)
        .bind(&body.hostname)
        .bind(&body.agent_version)
        .bind(&mode_id)
        .bind(now)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }

    let message = if let Some(note) = fallback_note {
        note
    } else if client_id.is_none() {
        "未登记终端且未配置默认机房；仅记录会话，请在管理端设置自动登记默认机房".into()
    } else if mode.is_none() {
        "未绑定工作模式；默认不还原".into()
    } else if should {
        format!(
            "应按策略还原（restore={}, trigger={}）",
            mode.as_ref().unwrap().restore,
            body.trigger
        )
    } else {
        format!(
            "当前模式「{}」本次不还原",
            mode.as_ref().unwrap().name
        )
    };

    Ok(Json(AgentPolicyResponse {
        mac,
        client_id,
        lab_id,
        mode,
        baseline_image_id,
        should_restore: should,
        schedule_active,
        usb_restricted,
        pending_power,
        pending_software: pending_software,
        pending_assist: pending_assist,
        message,
    }))
}

fn schedule_is_active(mode: &ModePolicy, now: chrono::DateTime<Utc>) -> bool {
    match (&mode.schedule_start, &mode.schedule_end) {
        (None, None) => true,
        (Some(start), Some(end)) => {
            let Ok(s) = chrono::DateTime::parse_from_rfc3339(start) else {
                return true;
            };
            let Ok(e) = chrono::DateTime::parse_from_rfc3339(end) else {
                return true;
            };
            let s = s.with_timezone(&Utc);
            let e = e.with_timezone(&Utc);
            now >= s && now <= e
        }
        _ => true,
    }
}

pub async fn report_restore(
    State(state): State<AppState>,
    Json(body): Json<AgentRestoreReport>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();
    let status = if body.ok { "restored" } else { "restore_failed" };
    let result = sqlx::query(
        "UPDATE agent_sessions SET last_restore_at = ?, status = ?, detail = ?, last_seen = ? WHERE mac = ?",
    )
    .bind(now)
    .bind(status)
    .bind(&body.detail)
    .bind(now)
    .bind(&mac)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "agent session not found; heartbeat first".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true, "status": status })))
}

pub async fn report_usb(
    State(state): State<AppState>,
    Json(body): Json<AgentUsbReport>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let now = Utc::now();
    let enforced = if body.enforced { 1 } else { 0 };
    let result = sqlx::query(
        "UPDATE agent_sessions SET usb_enforced = ?, detail = ?, last_seen = ? WHERE mac = ?",
    )
    .bind(enforced)
    .bind(&body.detail)
    .bind(now)
    .bind(&mac)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "agent session not found; heartbeat first".into()));
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "usb_enforced": body.enforced
    })))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    _user: crate::auth::AuthUser,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, (String, Option<String>, String, String, String, Option<String>, String, Option<String>, String, String, i64)>(
        "SELECT id, client_id, mac, hostname, agent_version, mode_policy_id, last_seen, last_restore_at, status, detail, usb_enforced FROM agent_sessions ORDER BY last_seen DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let out = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.0,
                "client_id": r.1,
                "mac": r.2,
                "hostname": r.3,
                "agent_version": r.4,
                "mode_policy_id": r.5,
                "last_seen": r.6,
                "last_restore_at": r.7,
                "status": r.8,
                "detail": r.9,
                "usb_enforced": r.10 != 0,
            })
        })
        .collect();
    Ok(Json(out))
}

pub async fn recommend_delta(
    State(state): State<AppState>,
    _user: crate::auth::AuthUser,
    Json(body): Json<DeltaRecommendRequest>,
) -> Result<Json<DeltaRecommendResponse>, (StatusCode, String)> {
    if body.force_full {
        return Ok(Json(DeltaRecommendResponse {
            recommendation: "full".into(),
            reason: "管理员强制全量".into(),
            content_type: "full".into(),
        }));
    }
    let cols = "id, name, os_family, version_label, checksum_sha256, size_bytes, storage_uri, boot_mode, notes, partition_map, content_type, base_image_id, created_at";
    let base = sqlx::query_as::<_, Image>(&format!("SELECT {cols} FROM images WHERE id = ?"))
        .bind(&body.base_image_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::BAD_REQUEST, "base image not found".into()))?;
    let target = sqlx::query_as::<_, Image>(&format!("SELECT {cols} FROM images WHERE id = ?"))
        .bind(&body.target_image_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::BAD_REQUEST, "target image not found".into()))?;

    if base.os_family != target.os_family {
        return Ok(Json(DeltaRecommendResponse {
            recommendation: "full".into(),
            reason: "os_family 不同，必须全量".into(),
            content_type: "full".into(),
        }));
    }
    if base.partition_map != target.partition_map {
        return Ok(Json(DeltaRecommendResponse {
            recommendation: "full".into(),
            reason: "分区布局不一致，必须全量".into(),
            content_type: "full".into(),
        }));
    }
    let ratio = body.estimated_delta_ratio.unwrap_or_else(|| {
        if target.size_bytes <= 0 || base.size_bytes <= 0 {
            1.0
        } else {
            (target.size_bytes as f64 - base.size_bytes as f64).abs() / target.size_bytes as f64
        }
    });
    if ratio >= 0.65 {
        return Ok(Json(DeltaRecommendResponse {
            recommendation: "full".into(),
            reason: format!("估算差量比例 {ratio:.2} ≥ 0.65，建议全量"),
            content_type: "full".into(),
        }));
    }
    Ok(Json(DeltaRecommendResponse {
        recommendation: "delta".into(),
        reason: format!("布局兼容且估算差量比例 {ratio:.2} < 0.65，推荐增量"),
        content_type: "delta".into(),
    }))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
