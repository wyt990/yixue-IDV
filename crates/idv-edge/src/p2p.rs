//! Edge-mediated P2P chunk coordinator (stage 5 pilot).
//! Seeders PUT chunks into a swarm store; leechers prefer swarm (p2p) then authoritative cache (edge).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use idv_common::chunking::{ChunkManifest, build_manifest, verify_chunk};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::worker::EdgeRuntime;

#[derive(Clone)]
pub struct P2pState {
    pub runtime: EdgeRuntime,
    pub swarm: Arc<RwLock<HashMap<String, SwarmImage>>>,
    pub peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    pub chunk_size: usize,
}

#[derive(Debug, Default)]
pub struct SwarmImage {
    /// chunk_index -> bytes donated by peers
    pub chunks: HashMap<u32, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub mac: String,
    pub task_id: String,
    pub image_id: String,
    pub have: Vec<u32>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterPeer {
    pub mac: String,
    pub task_id: String,
    pub image_id: String,
    #[serde(default)]
    pub have: Vec<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PeerQuery {
    pub image_id: String,
    #[serde(default)]
    pub task_id: String,
}

pub async fn register_peer(
    State(state): State<P2pState>,
    Json(body): Json<RegisterPeer>,
) -> Result<Json<PeerInfo>, (StatusCode, String)> {
    let info = PeerInfo {
        mac: body.mac,
        task_id: body.task_id,
        image_id: body.image_id,
        have: body.have,
        updated_at: Utc::now(),
    };
    state
        .peers
        .write()
        .await
        .insert(format!("{}:{}", info.image_id, info.mac), info.clone());
    Ok(Json(info))
}

pub async fn list_peers(
    State(state): State<P2pState>,
    Query(q): Query<PeerQuery>,
) -> Json<Vec<PeerInfo>> {
    let guard = state.peers.read().await;
    let mut rows: Vec<_> = guard
        .values()
        .filter(|p| p.image_id == q.image_id)
        .filter(|p| q.task_id.is_empty() || p.task_id == q.task_id)
        .cloned()
        .collect();
    rows.sort_by_key(|p| std::cmp::Reverse(p.have.len()));
    Json(rows)
}

pub async fn manifest(
    State(state): State<P2pState>,
    Path(image_id): Path<String>,
) -> Result<Json<ChunkManifest>, (StatusCode, String)> {
    let path = state.runtime.cache_file(&image_id);
    let data = tokio::fs::read(&path).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("cache missing for {image_id}: {e}"),
        )
    })?;
    Ok(Json(build_manifest(
        &image_id,
        &data,
        state.chunk_size.max(1),
    )))
}

pub async fn get_chunk(
    State(state): State<P2pState>,
    Path((image_id, index)): Path<(String, u32)>,
    Query(q): Query<ChunkSourceQuery>,
) -> Result<Response, (StatusCode, String)> {
    // Prefer swarm (counts as p2p at client) unless force=edge
    if q.source != "edge" {
        let guard = state.swarm.read().await;
        if let Some(img) = guard.get(&image_id) {
            if let Some(bytes) = img.chunks.get(&index) {
                return chunk_response(bytes.clone(), "p2p");
            }
        }
    }

    let path = state.runtime.cache_file(&image_id);
    let data = tokio::fs::read(&path).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("edge cache missing: {e}"),
        )
    })?;
    let manifest = build_manifest(&image_id, &data, state.chunk_size.max(1));
    let info = manifest
        .chunks
        .iter()
        .find(|c| c.index == index)
        .ok_or((StatusCode::NOT_FOUND, "chunk index out of range".into()))?;
    let start = info.offset as usize;
    let end = start + info.length as usize;
    let slice = data[start..end].to_vec();
    if !verify_chunk(info, &slice) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "chunk corrupt".into()));
    }
    chunk_response(slice, "edge")
}

#[derive(Debug, Deserialize)]
pub struct ChunkSourceQuery {
    #[serde(default = "default_source")]
    pub source: String,
}

fn default_source() -> String {
    "auto".into()
}

pub async fn put_chunk(
    State(state): State<P2pState>,
    Path((image_id, index)): Path<(String, u32)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty chunk".into()));
    }
    // Validate against authoritative cache when present.
    let path = state.runtime.cache_file(&image_id);
    if path.exists() {
        let data = tokio::fs::read(&path).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read cache: {e}"),
            )
        })?;
        let manifest = build_manifest(&image_id, &data, state.chunk_size.max(1));
        let info = manifest
            .chunks
            .iter()
            .find(|c| c.index == index)
            .ok_or((StatusCode::BAD_REQUEST, "chunk index out of range".into()))?;
        if !verify_chunk(info, &body) {
            return Err((StatusCode::BAD_REQUEST, "chunk checksum mismatch".into()));
        }
    }

    let mut guard = state.swarm.write().await;
    let entry = guard.entry(image_id.clone()).or_default();
    entry.chunks.insert(index, body.to_vec());
    let count = entry.chunks.len();
    Ok(Json(serde_json::json!({
        "ok": true,
        "image_id": image_id,
        "index": index,
        "swarm_chunks": count
    })))
}

pub async fn swarm_stats(
    State(state): State<P2pState>,
    Path(image_id): Path<String>,
) -> Json<serde_json::Value> {
    let guard = state.swarm.read().await;
    let n = guard.get(&image_id).map(|s| s.chunks.len()).unwrap_or(0);
    let peers = state.peers.read().await;
    let peer_n = peers.values().filter(|p| p.image_id == image_id).count();
    Json(serde_json::json!({
        "image_id": image_id,
        "swarm_chunks": n,
        "peer_count": peer_n,
        "seeding": n > 0
    }))
}

#[derive(Debug, Deserialize)]
pub struct StopSwarmQuery {
    #[serde(default)]
    pub task_id: String,
}

/// End seeding for an image (and optionally only peers of one task).
pub async fn stop_swarm(
    State(state): State<P2pState>,
    Path(image_id): Path<String>,
    Query(q): Query<StopSwarmQuery>,
) -> Json<serde_json::Value> {
    let mut swarm = state.swarm.write().await;
    let removed_chunks = swarm.remove(&image_id).map(|s| s.chunks.len()).unwrap_or(0);
    let mut peers = state.peers.write().await;
    let before = peers.len();
    peers.retain(|_, p| {
        if p.image_id != image_id {
            return true;
        }
        if q.task_id.is_empty() {
            return false;
        }
        p.task_id != q.task_id
    });
    let removed_peers = before - peers.len();
    Json(serde_json::json!({
        "ok": true,
        "image_id": image_id,
        "removed_chunks": removed_chunks,
        "removed_peers": removed_peers,
        "seeding": false
    }))
}

fn chunk_response(bytes: Vec<u8>, source: &str) -> Result<Response, (StatusCode, String)> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header("X-Idv-Chunk-Source", source)
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[allow(dead_code)]
pub fn swarm_dir(cache_dir: &PathBuf) -> PathBuf {
    cache_dir.join("p2p-swarm")
}
