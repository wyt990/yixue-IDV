use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthUser;
use crate::mac::normalize_mac;
use crate::models::{Client, CreateClient, UpdateClient, new_id};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ClientFilter {
    pub lab_id: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetTags {
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkClients {
    pub lab_id: String,
    pub macs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BulkResult {
    pub created: Vec<Client>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkIds {
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkMove {
    pub ids: Vec<String>,
    pub lab_id: String,
    #[serde(default)]
    pub rename: bool,
}

#[derive(Debug, Serialize)]
pub struct BulkMutateResult {
    pub affected: i64,
    pub errors: Vec<String>,
}

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(filter): Query<ClientFilter>,
) -> Result<Json<Vec<Client>>, (StatusCode, String)> {
    let rows = match (filter.lab_id, filter.tag) {
        (Some(lab_id), Some(tag)) => {
            sqlx::query_as::<_, Client>(
                r#"
                SELECT c.id, c.lab_id, c.mac, c.name, c.notes, c.last_seen, c.created_at
                FROM clients c
                JOIN client_tags t ON t.client_id = c.id
                WHERE c.lab_id = ? AND t.tag = ?
                ORDER BY c.mac
                "#,
            )
            .bind(lab_id)
            .bind(tag)
            .fetch_all(&state.pool)
            .await
        }
        (None, Some(tag)) => {
            sqlx::query_as::<_, Client>(
                r#"
                SELECT c.id, c.lab_id, c.mac, c.name, c.notes, c.last_seen, c.created_at
                FROM clients c
                JOIN client_tags t ON t.client_id = c.id
                WHERE t.tag = ?
                ORDER BY c.mac
                "#,
            )
            .bind(tag)
            .fetch_all(&state.pool)
            .await
        }
        (Some(lab_id), None) => {
            sqlx::query_as::<_, Client>(
                "SELECT id, lab_id, mac, name, notes, last_seen, created_at FROM clients WHERE lab_id = ? ORDER BY mac",
            )
            .bind(lab_id)
            .fetch_all(&state.pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, Client>(
                "SELECT id, lab_id, mac, name, notes, last_seen, created_at FROM clients ORDER BY mac",
            )
            .fetch_all(&state.pool)
            .await
        }
    }
    .map_err(internal)?;
    Ok(Json(rows))
}

pub async fn list_tags(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT tag FROM client_tags WHERE client_id = ? ORDER BY tag")
        .bind(id)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(rows.into_iter().map(|r| r.0).collect()))
}

pub async fn set_tags(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<SetTags>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    user.require_write()?;
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM clients WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        return Err((StatusCode::NOT_FOUND, "client not found".into()));
    }
    sqlx::query("DELETE FROM client_tags WHERE client_id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let mut tags = Vec::new();
    for raw in body.tags {
        let tag = raw.trim().to_string();
        if tag.is_empty() {
            continue;
        }
        sqlx::query("INSERT OR IGNORE INTO client_tags (client_id, tag) VALUES (?, ?)")
            .bind(&id)
            .bind(&tag)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        tags.push(tag);
    }
    tags.sort();
    Ok(Json(tags))
}

pub async fn get(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Client>, (StatusCode, String)> {
    sqlx::query_as::<_, Client>(
        "SELECT id, lab_id, mac, name, notes, last_seen, created_at FROM clients WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .map(Json)
    .ok_or((StatusCode::NOT_FOUND, "client not found".into()))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateClient>,
) -> Result<(StatusCode, Json<Client>), (StatusCode, String)> {
    user.require_write()?;
    ensure_lab(&state, &body.lab_id).await?;
    let mac = normalize_mac(&body.mac).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let name = if body.name.trim().is_empty() {
        crate::lab_naming::allocate_next_name(&state, &body.lab_id).await?
    } else {
        body.name.trim().to_string()
    };
    let client = Client {
        id: new_id(),
        lab_id: body.lab_id,
        mac,
        name,
        notes: body.notes,
        last_seen: None,
        created_at: Utc::now(),
    };
    insert_client(&state, &client).await?;
    Ok((StatusCode::CREATED, Json(client)))
}

pub async fn bulk_create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<BulkClients>,
) -> Result<Json<BulkResult>, (StatusCode, String)> {
    user.require_write()?;
    ensure_lab(&state, &body.lab_id).await?;
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();

    for raw in body.macs {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let mac = match normalize_mac(raw) {
            Ok(m) => m,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let name = match crate::lab_naming::allocate_next_name(&state, &body.lab_id).await {
            Ok(n) => n,
            Err((_, msg)) => {
                errors.push(msg);
                continue;
            }
        };
        let client = Client {
            id: new_id(),
            lab_id: body.lab_id.clone(),
            mac: mac.clone(),
            name,
            notes: String::new(),
            last_seen: None,
            created_at: Utc::now(),
        };
        match insert_client(&state, &client).await {
            Ok(()) => created.push(client),
            Err((StatusCode::CONFLICT, _)) => skipped.push(mac),
            Err((_, msg)) => errors.push(msg),
        }
    }

    Ok(Json(BulkResult {
        created,
        skipped,
        errors,
    }))
}

pub async fn bulk_delete(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<BulkIds>,
) -> Result<Json<BulkMutateResult>, (StatusCode, String)> {
    user.require_write()?;
    let mut affected = 0i64;
    let mut errors = Vec::new();
    for id in body.ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        match sqlx::query("DELETE FROM clients WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await
        {
            Ok(r) => {
                if r.rows_affected() == 0 {
                    errors.push(format!("not found: {id}"));
                } else {
                    affected += 1;
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    Ok(Json(BulkMutateResult { affected, errors }))
}

pub async fn bulk_move(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<BulkMove>,
) -> Result<Json<BulkMutateResult>, (StatusCode, String)> {
    user.require_write()?;
    ensure_lab(&state, &body.lab_id).await?;
    let mut affected = 0i64;
    let mut errors = Vec::new();
    for id in body.ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM clients WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
        if exists.is_none() {
            errors.push(format!("not found: {id}"));
            continue;
        }
        if let Err(e) = sqlx::query("UPDATE clients SET lab_id = ? WHERE id = ?")
            .bind(&body.lab_id)
            .bind(id)
            .execute(&state.pool)
            .await
        {
            errors.push(e.to_string());
            continue;
        }
        if body.rename {
            if let Err((_, msg)) =
                crate::lab_naming::rename_for_lab(&state, &body.lab_id, id).await
            {
                errors.push(format!("{id}: {msg}"));
                continue;
            }
        }
        affected += 1;
    }
    Ok(Json(BulkMutateResult { affected, errors }))
}

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateClient>,
) -> Result<Json<Client>, (StatusCode, String)> {
    user.require_write()?;
    let existing = sqlx::query_as::<_, Client>(
        "SELECT id, lab_id, mac, name, notes, last_seen, created_at FROM clients WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "client not found".into()))?;

    let lab_id = body
        .lab_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(existing.lab_id);
    ensure_lab(&state, &lab_id).await?;
    let name = body.name.unwrap_or(existing.name);
    let notes = body.notes.unwrap_or(existing.notes);

    sqlx::query("UPDATE clients SET lab_id = ?, name = ?, notes = ? WHERE id = ?")
        .bind(&lab_id)
        .bind(&name)
        .bind(&notes)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;

    sqlx::query_as::<_, Client>(
        "SELECT id, lab_id, mac, name, notes, last_seen, created_at FROM clients WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .map(Json)
    .ok_or((StatusCode::NOT_FOUND, "client not found".into()))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    user.require_write()?;
    let result = sqlx::query("DELETE FROM clients WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "client not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn ensure_lab(state: &AppState, lab_id: &str) -> Result<(), (StatusCode, String)> {
    let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM labs WHERE id = ?")
        .bind(lab_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        return Err((StatusCode::BAD_REQUEST, "lab not found".into()));
    }
    Ok(())
}

async fn insert_client(state: &AppState, client: &Client) -> Result<(), (StatusCode, String)> {
    sqlx::query(
        "INSERT INTO clients (id, lab_id, mac, name, notes, last_seen, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&client.id)
    .bind(&client.lab_id)
    .bind(&client.mac)
    .bind(&client.name)
    .bind(&client.notes)
    .bind(client.last_seen)
    .bind(client.created_at)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        if err.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, format!("MAC already exists: {}", client.mac))
        } else {
            internal(err)
        }
    })?;
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
