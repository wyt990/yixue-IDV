//! Fixed-size image chunking for Edge P2P pilot (stage 5).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default chunk size for lab/pilot (small enough for accept tests).
pub const DEFAULT_CHUNK_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkInfo {
    pub index: u32,
    pub offset: u64,
    pub length: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkManifest {
    pub image_id: String,
    pub chunk_size: u32,
    pub total_size: u64,
    pub file_sha256: String,
    pub chunks: Vec<ChunkInfo>,
}

pub fn build_manifest(image_id: &str, data: &[u8], chunk_size: usize) -> ChunkManifest {
    let chunk_size = chunk_size.max(1);
    let mut chunks = Vec::new();
    let mut offset = 0u64;
    let mut index = 0u32;
    while (offset as usize) < data.len() {
        let start = offset as usize;
        let end = (start + chunk_size).min(data.len());
        let slice = &data[start..end];
        chunks.push(ChunkInfo {
            index,
            offset,
            length: slice.len() as u32,
            sha256: hex_sha256(slice),
        });
        offset = end as u64;
        index += 1;
    }
    ChunkManifest {
        image_id: image_id.to_string(),
        chunk_size: chunk_size as u32,
        total_size: data.len() as u64,
        file_sha256: hex_sha256(data),
        chunks,
    }
}

pub fn verify_chunk(expected: &ChunkInfo, data: &[u8]) -> bool {
    data.len() == expected.length as usize && hex_sha256(data) == expected.sha256
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_covers_all_bytes() {
        let data = vec![7u8; 40_000];
        let m = build_manifest("img", &data, 16_384);
        assert_eq!(m.chunks.len(), 3);
        assert_eq!(m.total_size, 40_000);
        let mut rebuilt = Vec::new();
        for c in &m.chunks {
            let start = c.offset as usize;
            let end = start + c.length as usize;
            assert!(verify_chunk(c, &data[start..end]));
            rebuilt.extend_from_slice(&data[start..end]);
        }
        assert_eq!(rebuilt, data);
    }
}
