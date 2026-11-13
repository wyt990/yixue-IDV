//! Full-image delta (A→B) for stage 6 rolling delivery.
//!
//! Format (yixue-delta-v1):
//!   MAGIC "YIXDELTA\n"
//!   u32le header_len
//!   JSON DeltaHeader
//!   concatenated hunk payloads

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8] = b"YIXDELTA\n";
pub const DEFAULT_BLOCK: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeltaHunkMeta {
    pub offset: u64,
    pub length: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeltaHeader {
    pub format_version: u32,
    pub base_sha256: String,
    pub target_sha256: String,
    pub base_size: u64,
    pub target_size: u64,
    pub block_size: u32,
    pub hunks: Vec<DeltaHunkMeta>,
}

#[derive(Debug, Clone)]
pub struct PackedDelta {
    pub header: DeltaHeader,
    pub hunk_data: Vec<u8>,
}

pub fn build_delta(base: &[u8], target: &[u8], block_size: usize) -> anyhow::Result<Vec<u8>> {
    let block_size = block_size.max(1);
    let mut hunks = Vec::new();
    let mut hunk_data = Vec::new();
    let max_len = base.len().max(target.len());
    let mut off = 0usize;
    while off < max_len {
        let end = (off + block_size).min(max_len);
        let t_slice = if off >= target.len() {
            &[][..]
        } else {
            &target[off..end.min(target.len())]
        };
        let b_slice = if off >= base.len() {
            &[][..]
        } else {
            &base[off..end.min(base.len())]
        };
        if t_slice != b_slice {
            // Store the target block bytes (may be shorter on shrink — pad not needed; length is exact).
            let data = if off < target.len() {
                target[off..end.min(target.len())].to_vec()
            } else {
                Vec::new()
            };
            hunks.push(DeltaHunkMeta {
                offset: off as u64,
                length: data.len() as u32,
                sha256: hex_sha256(&data),
            });
            hunk_data.extend_from_slice(&data);
        }
        off = end;
    }

    // If target shorter than base, record a final truncate marker via target_size.
    let header = DeltaHeader {
        format_version: 1,
        base_sha256: hex_sha256(base),
        target_sha256: hex_sha256(target),
        base_size: base.len() as u64,
        target_size: target.len() as u64,
        block_size: block_size as u32,
        hunks,
    };
    pack_delta(header, hunk_data)
}

pub fn pack_delta(header: DeltaHeader, hunk_data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let header_bytes = serde_json::to_vec(&header)?;
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + header_bytes.len() + hunk_data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&hunk_data);
    Ok(out)
}

pub fn unpack_delta(bytes: &[u8]) -> anyhow::Result<PackedDelta> {
    if bytes.len() < MAGIC.len() + 4 {
        anyhow::bail!("delta too short");
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        anyhow::bail!("bad delta magic");
    }
    let mut off = MAGIC.len();
    let header_len =
        u32::from_le_bytes(bytes[off..off + 4].try_into().expect("4 bytes")) as usize;
    off += 4;
    if off + header_len > bytes.len() {
        anyhow::bail!("truncated delta header");
    }
    let header: DeltaHeader = serde_json::from_slice(&bytes[off..off + header_len])?;
    off += header_len;
    let hunk_data = bytes[off..].to_vec();
    let expected: usize = header.hunks.iter().map(|h| h.length as usize).sum();
    if hunk_data.len() != expected {
        anyhow::bail!(
            "delta hunk payload size mismatch: got {} expected {}",
            hunk_data.len(),
            expected
        );
    }
    Ok(PackedDelta { header, hunk_data })
}

pub fn apply_delta(base: &[u8], delta_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let packed = unpack_delta(delta_bytes)?;
    if hex_sha256(base) != packed.header.base_sha256 {
        anyhow::bail!("base checksum mismatch; cannot apply delta");
    }
    let mut out = base.to_vec();
    if (packed.header.target_size as usize) > out.len() {
        out.resize(packed.header.target_size as usize, 0);
    }
    let mut cursor = 0usize;
    for hunk in &packed.header.hunks {
        let len = hunk.length as usize;
        let data = &packed.hunk_data[cursor..cursor + len];
        if hex_sha256(data) != hunk.sha256 {
            anyhow::bail!("delta hunk checksum failed at offset {}", hunk.offset);
        }
        let start = hunk.offset as usize;
        let end = start + len;
        if end > out.len() {
            out.resize(end, 0);
        }
        out[start..end].copy_from_slice(data);
        cursor += len;
    }
    out.truncate(packed.header.target_size as usize);
    let got = hex_sha256(&out);
    if got != packed.header.target_sha256 {
        anyhow::bail!("applied delta checksum mismatch");
    }
    Ok(out)
}

pub fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_roundtrip_small_change() {
        let base = vec![1u8; 20_000];
        let mut target = base.clone();
        target[1000..1200].fill(9);
        target.extend_from_slice(&[7u8; 100]);
        let delta = build_delta(&base, &target, 4096).unwrap();
        assert!(delta.len() < target.len());
        let out = apply_delta(&base, &delta).unwrap();
        assert_eq!(out, target);
    }

    #[test]
    fn reject_wrong_base() {
        let base = vec![1u8; 4096];
        let target = vec![2u8; 4096];
        let delta = build_delta(&base, &target, 4096).unwrap();
        let wrong = vec![3u8; 4096];
        assert!(apply_delta(&wrong, &delta).is_err());
    }
}
