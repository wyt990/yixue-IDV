//! Allocate sequential display names for clients within a lab.

use axum::http::StatusCode;

use crate::models::Lab;
use crate::state::AppState;

pub const LAB_COLS: &str = "id, campus_id, name, name_prefix, suffix_width, suffix_start, next_suffix, ip_start, ip_end, netmask, gateway, dns, ip_mode, default_image_id, created_at";
pub const CAMPUS_COLS: &str = "id, name, default_image_id, created_at";

pub fn format_client_name(prefix: &str, width: i64, n: i64) -> String {
    let width = width.clamp(1, 8) as usize;
    format!("{prefix}{n:0width$}")
}

pub async fn load_lab(state: &AppState, lab_id: &str) -> Result<Lab, (StatusCode, String)> {
    let sql = format!("SELECT {LAB_COLS} FROM labs WHERE id = ?");
    sqlx::query_as::<_, Lab>(&sql)
        .bind(lab_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "lab not found".into()))
}

/// Atomically allocate the first free name for a lab.
///
/// Always starts at `suffix_start` and scans for the first unused name, so
/// deleting a client (or all clients) reuses the lowest free number instead of
/// continuing from a monotonic counter. `next_suffix` is kept only as a
/// display hint (next candidate after the allocated one).
pub async fn allocate_next_name(
    state: &AppState,
    lab_id: &str,
) -> Result<String, (StatusCode, String)> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let sql = format!("SELECT {LAB_COLS} FROM labs WHERE id = ?");
    let lab = sqlx::query_as::<_, Lab>(&sql)
        .bind(lab_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "lab not found".into()))?;

    let width = lab.suffix_width.clamp(1, 8);
    let mut n = lab.suffix_start.max(1);
    let prefix = lab.name_prefix.trim();

    let name = loop {
        let candidate = format_client_name(prefix, width, n);
        let taken: Option<(String,)> =
            sqlx::query_as("SELECT id FROM clients WHERE lab_id = ? AND name = ?")
                .bind(lab_id)
                .bind(&candidate)
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?;
        if taken.is_none() {
            break candidate;
        }
        n += 1;
        if n > 999_999 {
            return Err((
                StatusCode::CONFLICT,
                "unable to allocate client name".into(),
            ));
        }
    };

    sqlx::query("UPDATE labs SET next_suffix = ? WHERE id = ?")
        .bind(n + 1)
        .bind(lab_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(name)
}

/// Re-allocate a fresh name for an existing client after moving labs.
pub async fn rename_for_lab(
    state: &AppState,
    lab_id: &str,
    client_id: &str,
) -> Result<String, (StatusCode, String)> {
    let name = allocate_next_name(state, lab_id).await?;
    let assigned_ip = allocate_next_ip(state, lab_id).await.unwrap_or_default();
    sqlx::query("UPDATE clients SET name = ?, assigned_ip = ? WHERE id = ?")
        .bind(&name)
        .bind(&assigned_ip)
        .bind(client_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(name)
}

/// Allocate next free IPv4 from lab `ip_start`..=`ip_end` (empty range → empty string).
pub async fn allocate_next_ip(
    state: &AppState,
    lab_id: &str,
) -> Result<String, (StatusCode, String)> {
    let lab = load_lab(state, lab_id).await?;
    let start = match parse_ipv4(lab.ip_start.trim()) {
        Some(v) => v,
        None => return Ok(String::new()),
    };
    let end = match parse_ipv4(lab.ip_end.trim()) {
        Some(v) => v,
        None => return Ok(String::new()),
    };
    if end < start {
        return Err((
            StatusCode::BAD_REQUEST,
            "lab ip_end must be >= ip_start".into(),
        ));
    }
    let used: Vec<(String,)> =
        sqlx::query_as("SELECT assigned_ip FROM clients WHERE lab_id = ? AND assigned_ip != ''")
            .bind(lab_id)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    let used: std::collections::HashSet<u32> =
        used.into_iter().filter_map(|(s,)| parse_ipv4(&s)).collect();
    let mut cur = start;
    loop {
        if !used.contains(&cur) {
            return Ok(format_ipv4(cur));
        }
        if cur == end || cur == u32::MAX {
            break;
        }
        cur += 1;
    }
    Err((
        StatusCode::CONFLICT,
        "unable to allocate client IP in lab range".into(),
    ))
}

/// Validate a manually assigned client IPv4. Manual addresses may intentionally
/// fall outside the automatic `ip_start..ip_end` range, but must be a usable host
/// in the lab subnet, must not be the gateway, and must not duplicate another
/// client in the same lab.
pub async fn validate_manual_ip(
    state: &AppState,
    lab_id: &str,
    value: &str,
    exclude_client_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    let ip = parse_ipv4(value).ok_or((
        StatusCode::BAD_REQUEST,
        format!("IP 地址格式不正确: {value}"),
    ))?;
    let lab = load_lab(state, lab_id).await?;

    if let Some(mask) = parse_ipv4(lab.netmask.trim())
        && mask != 0
    {
        let network = ip & mask;
        let broadcast = network | !mask;
        if ip == network || ip == broadcast {
            return Err((
                StatusCode::BAD_REQUEST,
                "不能使用网络地址或广播地址".into(),
            ));
        }
        if let Some(gateway) = parse_ipv4(lab.gateway.trim())
            && ip == gateway
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "不能使用机房网关地址".into(),
            ));
        }
    }

    let duplicate: Option<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM clients WHERE lab_id = ? AND assigned_ip = ? AND id != ? LIMIT 1",
    )
    .bind(lab_id)
    .bind(value)
    .bind(exclude_client_id.unwrap_or(""))
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if let Some((_, name)) = duplicate {
        return Err((
            StatusCode::CONFLICT,
            format!("IP {value} 已被终端 {name} 使用"),
        ));
    }
    Ok(())
}

fn parse_ipv4(s: &str) -> Option<u32> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut n = 0u32;
    for p in parts {
        let o: u32 = p.parse().ok()?;
        if o > 255 {
            return None;
        }
        n = (n << 8) | o;
    }
    Some(n)
}

fn format_ipv4(n: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (n >> 24) & 0xff,
        (n >> 16) & 0xff,
        (n >> 8) & 0xff,
        n & 0xff
    )
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
