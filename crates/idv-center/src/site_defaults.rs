//! Resolve lab/campus default baseline image and preferred Edge list.

use axum::http::StatusCode;
use sqlx::SqlitePool;

use crate::models::edge_online;

/// An online Edge that has a ready replica of the deploy image.
#[derive(Debug, Clone)]
pub struct DeployEdgeCandidate {
    pub id: String,
    pub name: String,
    pub listen_url: String,
    pub active_targets: i64,
}

/// Edges attached to the lab (or campus fallback) with a ready image replica.
/// Ordered by current deploy load ascending; only online Edges are returned.
pub async fn ready_deploy_edges(
    pool: &SqlitePool,
    lab_id: &str,
    image_id: &str,
) -> Result<Vec<DeployEdgeCandidate>, (StatusCode, String)> {
    let mut edge_ids = resolve_preferred_edge_ids(pool, lab_id).await?;
    if edge_ids.is_empty() {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM edges")
            .fetch_all(pool)
            .await
            .map_err(internal)?;
        edge_ids = rows.into_iter().map(|(id,)| id).collect();
    }
    if edge_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; edge_ids.len()].join(",");
    let sql = format!(
        "SELECT e.id, e.name, e.listen_url, e.last_seen, \
         (SELECT COUNT(*) FROM maintenance_task_targets t \
          WHERE t.edge_id = e.id AND t.state IN ('pending','running')) AS active \
         FROM image_replicas r JOIN edges e ON e.id = r.edge_id \
         WHERE r.image_id = ? AND r.state = 'ready' \
           AND e.id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (String, String, String, Option<chrono::DateTime<chrono::Utc>>, i64)>(&sql);
    q = q.bind(image_id);
    for id in &edge_ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await.map_err(internal)?;
    let mut out: Vec<DeployEdgeCandidate> = rows
        .into_iter()
        .filter(|(_, _, _, seen, _)| edge_online(*seen))
        .map(|(id, name, listen_url, _, active_targets)| DeployEdgeCandidate {
            id,
            name,
            listen_url,
            active_targets,
        })
        .filter(|c| {
            let url = c.listen_url.trim();
            !url.is_empty() && !url.contains("0.0.0.0")
        })
        .collect();
    out.sort_by(|a, b| {
        a.active_targets
            .cmp(&b.active_targets)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}


/// Lab default image → campus default → none.
pub async fn resolve_baseline_image_id(
    pool: &SqlitePool,
    lab_id: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT default_image_id, campus_id FROM labs WHERE id = ?")
            .bind(lab_id)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
    let Some((lab_img, campus_id)) = row else {
        return Ok(None);
    };
    if let Some(id) = lab_img.filter(|s| !s.is_empty()) {
        return Ok(Some(id));
    }
    let campus_img: Option<Option<String>> =
        sqlx::query_scalar("SELECT default_image_id FROM campuses WHERE id = ?")
            .bind(campus_id)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
    Ok(campus_img.flatten().filter(|s| !s.is_empty()))
}

/// Lab edge bindings if any; else campus bindings. Ordered by priority ascending.
pub async fn resolve_preferred_edge_ids(
    pool: &SqlitePool,
    lab_id: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let campus_id: Option<String> = sqlx::query_scalar("SELECT campus_id FROM labs WHERE id = ?")
        .bind(lab_id)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
    let Some(campus_id) = campus_id else {
        return Ok(Vec::new());
    };
    let lab_edges: Vec<(String,)> = sqlx::query_as(
        "SELECT edge_id FROM lab_edges WHERE lab_id = ? ORDER BY priority ASC, edge_id ASC",
    )
    .bind(lab_id)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    if !lab_edges.is_empty() {
        return Ok(lab_edges.into_iter().map(|(id,)| id).collect());
    }
    let campus_edges: Vec<(String,)> = sqlx::query_as(
        "SELECT edge_id FROM campus_edges WHERE campus_id = ? ORDER BY priority ASC, edge_id ASC",
    )
    .bind(campus_id)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    Ok(campus_edges.into_iter().map(|(id,)| id).collect())
}

pub async fn resolve_preferred_edge_id(
    pool: &SqlitePool,
    lab_id: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    Ok(resolve_preferred_edge_ids(pool, lab_id)
        .await?
        .into_iter()
        .next())
}

pub async fn lab_id_for_mac(
    pool: &SqlitePool,
    mac: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    sqlx::query_scalar("SELECT lab_id FROM clients WHERE mac = ?")
        .bind(mac)
        .fetch_optional(pool)
        .await
        .map_err(internal)
}

pub async fn ensure_image_exists(
    pool: &SqlitePool,
    image_id: &str,
) -> Result<(), (StatusCode, String)> {
    let ok: Option<(String,)> = sqlx::query_as("SELECT id FROM images WHERE id = ?")
        .bind(image_id)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
    if ok.is_none() {
        return Err((StatusCode::BAD_REQUEST, "default_image_id not found".into()));
    }
    Ok(())
}

pub async fn ensure_edges_exist(
    pool: &SqlitePool,
    edge_ids: &[String],
) -> Result<(), (StatusCode, String)> {
    for id in edge_ids {
        let ok: Option<(String,)> = sqlx::query_as("SELECT id FROM edges WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
        if ok.is_none() {
            return Err((StatusCode::BAD_REQUEST, format!("edge not found: {id}")));
        }
    }
    Ok(())
}

pub async fn replace_campus_edges(
    pool: &SqlitePool,
    campus_id: &str,
    edge_ids: &[String],
) -> Result<(), (StatusCode, String)> {
    ensure_edges_exist(pool, edge_ids).await?;
    sqlx::query("DELETE FROM campus_edges WHERE campus_id = ?")
        .bind(campus_id)
        .execute(pool)
        .await
        .map_err(internal)?;
    for (i, edge_id) in edge_ids.iter().enumerate() {
        sqlx::query("INSERT INTO campus_edges (campus_id, edge_id, priority) VALUES (?, ?, ?)")
            .bind(campus_id)
            .bind(edge_id)
            .bind(i as i64)
            .execute(pool)
            .await
            .map_err(internal)?;
    }
    Ok(())
}

pub async fn replace_lab_edges(
    pool: &SqlitePool,
    lab_id: &str,
    edge_ids: &[String],
) -> Result<(), (StatusCode, String)> {
    ensure_edges_exist(pool, edge_ids).await?;
    sqlx::query("DELETE FROM lab_edges WHERE lab_id = ?")
        .bind(lab_id)
        .execute(pool)
        .await
        .map_err(internal)?;
    for (i, edge_id) in edge_ids.iter().enumerate() {
        sqlx::query("INSERT INTO lab_edges (lab_id, edge_id, priority) VALUES (?, ?, ?)")
            .bind(lab_id)
            .bind(edge_id)
            .bind(i as i64)
            .execute(pool)
            .await
            .map_err(internal)?;
    }
    Ok(())
}

pub async fn load_campus_edge_ids(
    pool: &SqlitePool,
    campus_id: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT edge_id FROM campus_edges WHERE campus_id = ? ORDER BY priority ASC, edge_id ASC",
    )
    .bind(campus_id)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn load_lab_edge_ids(
    pool: &SqlitePool,
    lab_id: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT edge_id FROM lab_edges WHERE lab_id = ? ORDER BY priority ASC, edge_id ASC",
    )
    .bind(lab_id)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn attach_campus_edges(
    pool: &SqlitePool,
    campuses: &mut [crate::models::Campus],
) -> Result<(), (StatusCode, String)> {
    for c in campuses.iter_mut() {
        c.edge_ids = load_campus_edge_ids(pool, &c.id).await?;
    }
    Ok(())
}

pub async fn attach_lab_edges(
    pool: &SqlitePool,
    labs: &mut [crate::models::Lab],
) -> Result<(), (StatusCode, String)> {
    for lab in labs.iter_mut() {
        lab.edge_ids = load_lab_edge_ids(pool, &lab.id).await?;
    }
    Ok(())
}

/// Normalize optional image id from API: empty → None; Some(id) must exist.
pub async fn normalize_default_image_id(
    pool: &SqlitePool,
    raw: Option<String>,
) -> Result<Option<String>, (StatusCode, String)> {
    let Some(id) = raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    ensure_image_exists(pool, &id).await?;
    Ok(Some(id))
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
