//! Full-image delta (A→B) for rolling delivery.
//!
//! Format (yixue-delta-v1):
//!   MAGIC "YIXDELTA\n"
//!   u32le header_len
//!   JSON DeltaHeader
//!   concatenated hunk payloads
//!
//! Prefer [`build_delta_files`] / [`apply_delta_files`] for large images (streaming).

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8] = b"YIXDELTA\n";
/// Small in-memory / unit-test block size.
pub const DEFAULT_BLOCK: usize = 4096;
/// Disk streaming block size for multi-GB images.
pub const FILE_BLOCK: usize = 4 * 1024 * 1024;

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

/// Stream two files on disk into a delta file (no full-image RAM load).
pub fn build_delta_files(
    base_path: &Path,
    target_path: &Path,
    out_path: &Path,
    block_size: usize,
    mut on_progress: impl FnMut(u64, u64, &str),
) -> anyhow::Result<DeltaHeader> {
    let block_size = block_size.max(1);
    let mut base = File::open(base_path)?;
    let mut target = File::open(target_path)?;
    let base_size = base.seek(SeekFrom::End(0))?;
    let target_size = target.seek(SeekFrom::End(0))?;
    base.seek(SeekFrom::Start(0))?;
    target.seek(SeekFrom::Start(0))?;

    let total = base_size.max(target_size);
    on_progress(0, total, "[delta] build start");

    let hunks_tmp = out_path.with_extension("hunks.tmp");
    let mut hunk_file = File::create(&hunks_tmp)?;
    let mut hunks = Vec::new();
    let mut base_hasher = Sha256::new();
    let mut target_hasher = Sha256::new();
    let mut base_buf = vec![0u8; block_size];
    let mut target_buf = vec![0u8; block_size];
    let mut offset = 0u64;

    while offset < total {
        let end = (offset + block_size as u64).min(total);
        let chunk = (end - offset) as usize;

        let b_real = if offset >= base_size {
            0
        } else {
            ((base_size - offset) as usize).min(chunk)
        };
        let t_real = if offset >= target_size {
            0
        } else {
            ((target_size - offset) as usize).min(chunk)
        };
        read_chunk(&mut base, offset, base_size, &mut base_buf[..chunk])?;
        read_chunk(&mut target, offset, target_size, &mut target_buf[..chunk])?;
        if b_real > 0 {
            base_hasher.update(&base_buf[..b_real]);
        }
        if t_real > 0 {
            target_hasher.update(&target_buf[..t_real]);
        }

        let b_slice = &base_buf[..b_real];
        let t_slice = &target_buf[..t_real];

        if b_slice != t_slice {
            let data = t_slice;
            let digest = hex_sha256(data);
            hunks.push(DeltaHunkMeta {
                offset,
                length: data.len() as u32,
                sha256: digest,
            });
            hunk_file.write_all(data)?;
        }

        offset = end;
        if offset == total || offset % (64 * 1024 * 1024) < block_size as u64 {
            on_progress(
                offset,
                total,
                &format!("[delta] build compared {}/{}", offset, total),
            );
        }
    }

    let header = DeltaHeader {
        format_version: 1,
        base_sha256: hex::encode(base_hasher.finalize()),
        target_sha256: hex::encode(target_hasher.finalize()),
        base_size,
        target_size,
        block_size: block_size as u32,
        hunks,
    };
    drop(hunk_file);

    let header_bytes = serde_json::to_vec(&header)?;
    let mut out = File::create(out_path)?;
    out.write_all(MAGIC)?;
    out.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
    out.write_all(&header_bytes)?;
    let mut hunk_in = File::open(&hunks_tmp)?;
    std::io::copy(&mut hunk_in, &mut out)?;
    out.flush()?;
    drop(out);
    let _ = fs::remove_file(&hunks_tmp);

    on_progress(
        total,
        total,
        &format!(
            "[delta] build done hunks={} out={}",
            header.hunks.len(),
            out_path.display()
        ),
    );
    Ok(header)
}

fn read_chunk(f: &mut File, offset: u64, file_size: u64, buf: &mut [u8]) -> anyhow::Result<usize> {
    if offset >= file_size {
        buf.fill(0);
        return Ok(buf.len());
    }
    f.seek(SeekFrom::Start(offset))?;
    let avail = ((file_size - offset) as usize).min(buf.len());
    f.read_exact(&mut buf[..avail])?;
    if avail < buf.len() {
        buf[avail..].fill(0);
    }
    Ok(buf.len())
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

pub fn unpack_delta_header_from_file(path: &Path) -> anyhow::Result<(DeltaHeader, u64)> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 9];
    f.read_exact(&mut magic)?;
    if &magic != MAGIC {
        anyhow::bail!("bad delta magic");
    }
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as u64;
    let mut header_bytes = vec![0u8; header_len as usize];
    f.read_exact(&mut header_bytes)?;
    let header: DeltaHeader = serde_json::from_slice(&header_bytes)?;
    let data_off = MAGIC.len() as u64 + 4 + header_len;
    Ok((header, data_off))
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

/// Apply delta on disk: copy base → out, patch hunks, verify target sha256.
pub fn apply_delta_files(
    base_path: &Path,
    delta_path: &Path,
    out_path: &Path,
    mut on_progress: impl FnMut(u64, u64, &str),
) -> anyhow::Result<String> {
    let (header, data_off) = unpack_delta_header_from_file(delta_path)?;
    on_progress(0, header.target_size, "[delta] apply start (copy base)");

    if base_path != out_path {
        fs::copy(base_path, out_path)?;
    }
    {
        let f = File::options().write(true).open(out_path)?;
        f.set_len(header.target_size)?;
    }

    // Verify base checksum while streaming.
    let base_sha = hash_file(base_path)?;
    if base_sha != header.base_sha256 {
        anyhow::bail!("base checksum mismatch; cannot apply delta");
    }

    let mut delta = File::open(delta_path)?;
    delta.seek(SeekFrom::Start(data_off))?;
    let mut out = File::options().read(true).write(true).open(out_path)?;
    let hunk_total = header.hunks.len() as u64;
    for (i, hunk) in header.hunks.iter().enumerate() {
        let mut data = vec![0u8; hunk.length as usize];
        delta.read_exact(&mut data)?;
        if hex_sha256(&data) != hunk.sha256 {
            anyhow::bail!("delta hunk checksum failed at offset {}", hunk.offset);
        }
        out.seek(SeekFrom::Start(hunk.offset))?;
        out.write_all(&data)?;
        if i % 64 == 0 || i + 1 == header.hunks.len() {
            on_progress(
                i as u64 + 1,
                hunk_total.max(1),
                &format!("[delta] apply hunk {}/{}", i + 1, hunk_total),
            );
        }
    }
    out.flush()?;
    drop(out);

    on_progress(hunk_total, hunk_total.max(1), "[delta] apply verifying sha256");
    let got = hash_file(out_path)?;
    if got != header.target_sha256 {
        anyhow::bail!("applied delta checksum mismatch");
    }
    on_progress(
        hunk_total,
        hunk_total.max(1),
        &format!("[delta] apply done out={}", out_path.display()),
    );
    Ok(got)
}

pub fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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

    #[test]
    fn file_delta_roundtrip() {
        let dir = std::env::temp_dir().join(format!("idv-delta-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let base_p = dir.join("base.bin");
        let target_p = dir.join("target.bin");
        let delta_p = dir.join("d.delta");
        let out_p = dir.join("out.bin");
        {
            let b = vec![3u8; 50_000];
            let mut t = b.clone();
            t[10_000..12_000].fill(8);
            t.extend_from_slice(&[1u8; 500]);
            File::create(&base_p).unwrap().write_all(&b).unwrap();
            File::create(&target_p).unwrap().write_all(&t).unwrap();
        }
        build_delta_files(&base_p, &target_p, &delta_p, 4096, |_, _, _| {}).unwrap();
        apply_delta_files(&base_p, &delta_p, &out_p, |_, _, _| {}).unwrap();
        let t = fs::read(&target_p).unwrap();
        let o = fs::read(&out_p).unwrap();
        assert_eq!(t, o);
        let _ = fs::remove_dir_all(&dir);
    }
}
