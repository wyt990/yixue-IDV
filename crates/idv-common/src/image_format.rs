//! Yixue image v1 helpers used by PE capture/deploy (stage 4).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8] = b"YIXUE1\n";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartitionRole {
    pub name: String,
    /// system | persistent | volatile
    pub role: String,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageHeader {
    pub format_version: u32,
    pub os_family: String,
    pub boot_mode: String,
    pub partitions: Vec<PartitionRole>,
    pub payload_sha256: String,
    pub payload_size: u64,
    /// `raw` = payload is full logical disk; `ntfs_used_v1` = used NTFS clusters (+ raw small parts).
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Full disk size when encoding packs sparsely; 0 means same as payload_size.
    #[serde(default)]
    pub logical_size: u64,
}

impl Default for ImageHeader {
    fn default() -> Self {
        Self {
            format_version: 1,
            os_family: String::new(),
            boot_mode: String::new(),
            partitions: Vec::new(),
            payload_sha256: String::new(),
            payload_size: 0,
            encoding: default_encoding(),
            logical_size: 0,
        }
    }
}

fn default_encoding() -> String {
    "raw".into()
}

/// Human progress like `32.15/238.37 GB` (GiB).
pub fn format_bytes_progress(done: u64, total: u64) -> String {
    fn gib(n: u64) -> f64 {
        n as f64 / (1024.0 * 1024.0 * 1024.0)
    }
    if total == 0 {
        format!("{:.2}/? GB", gib(done))
    } else {
        format!("{:.2}/{:.2} GB", gib(done), gib(total))
    }
}

/// Fixed-width placeholder so Center can patch SHA in-place after streaming ingest.
pub fn pending_payload_sha256() -> String {
    "0".repeat(64)
}

pub fn is_pending_payload_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b == b'0')
}

#[derive(Debug, Clone)]
pub struct PackedImage {
    pub header: ImageHeader,
    pub payload: Vec<u8>,
}

pub fn pack_image(header: ImageHeader, payload: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let mut header = header;
    header.payload_size = payload.len() as u64;
    header.payload_sha256 = hex_sha256(&payload);
    let header_bytes = serde_json::to_vec(&header)?;
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + header_bytes.len() + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Stream payload from `payload_path` into packed image at `out_path` (no full-disk RAM).
pub fn pack_image_to_path(
    mut header: ImageHeader,
    payload_path: &Path,
    out_path: &Path,
) -> anyhow::Result<ImageHeader> {
    let meta = std::fs::metadata(payload_path)?;
    let payload_size = meta.len();
    header.payload_size = payload_size;
    header.payload_sha256 = hex_sha256_file(payload_path)?;
    let header_bytes = serde_json::to_vec(&header)?;

    let mut out = File::create(out_path)?;
    out.write_all(MAGIC)?;
    out.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
    out.write_all(&header_bytes)?;
    let mut input = File::open(payload_path)?;
    std::io::copy(&mut input, &mut out)?;
    out.flush()?;
    Ok(header)
}

/// Read header and verify payload checksum by streaming (payload not loaded into RAM).
pub fn inspect_image_file(path: &Path) -> anyhow::Result<ImageHeader> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 7];
    f.read_exact(&mut magic)?;
    if magic.as_slice() != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as usize;
    let mut header_bytes = vec![0u8; header_len];
    f.read_exact(&mut header_bytes)?;
    let header: ImageHeader = serde_json::from_slice(&header_bytes)?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut read_total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read_total += n as u64;
    }
    let got = hex::encode(hasher.finalize());
    if !is_pending_payload_sha256(&header.payload_sha256)
        && got != header.payload_sha256.to_ascii_lowercase()
    {
        anyhow::bail!("payload checksum mismatch");
    }
    if read_total != header.payload_size {
        anyhow::bail!(
            "payload size mismatch: file={read_total} header={}",
            header.payload_size
        );
    }
    Ok(header)
}

/// Hash payload only (skip magic+header). Returns (sha256_hex, payload_bytes).
pub fn hash_image_payload(path: &Path) -> anyhow::Result<(String, u64)> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 7];
    f.read_exact(&mut magic)?;
    if magic.as_slice() != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as u64;
    f.seek(SeekFrom::Current(header_len as i64))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

/// Replace a pending (64×`0`) `payload_sha256` value inside the packed image header JSON.
/// Only searches the header region (does not load the whole image into RAM).
pub fn patch_pending_payload_sha256(path: &Path, sha_hex: &str) -> anyhow::Result<()> {
    use std::fs::OpenOptions;
    let sha = sha_hex.trim().to_ascii_lowercase();
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("sha must be 64 hex chars");
    }
    let mut f = OpenOptions::new().read(true).write(true).open(path)?;
    let mut magic = [0u8; 7];
    f.read_exact(&mut magic)?;
    if magic.as_slice() != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as usize;
    let mut header_bytes = vec![0u8; header_len];
    f.read_exact(&mut header_bytes)?;
    let needle = pending_payload_sha256();
    let Some(rel) = header_bytes.windows(64).position(|w| w == needle.as_bytes()) else {
        // Already patched or not pending.
        return Ok(());
    };
    let abs = (MAGIC.len() + 4 + rel) as u64;
    f.seek(SeekFrom::Start(abs))?;
    f.write_all(sha.as_bytes())?;
    f.flush()?;
    Ok(())
}

/// Read header, hash payload, patch pending SHA in-place, return header + whole-file checksum.
///
/// For pending-SHA captures this is one sequential pass over the payload (hash + optional
/// in-memory header patch bookkeeping) plus one full-file pass for `checksum_sha256`.
/// Uses an 8 MiB buffer to cut syscall overhead on multi‑GB images.
pub fn finalize_captured_image(path: &Path) -> anyhow::Result<(ImageHeader, String)> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 7];
    f.read_exact(&mut magic)?;
    if magic.as_slice() != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as usize;
    let mut header_bytes = vec![0u8; header_len];
    f.read_exact(&mut header_bytes)?;
    let mut header: ImageHeader = serde_json::from_slice(&header_bytes)?;

    let mut payload_hasher = Sha256::new();
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut payload_len = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        payload_hasher.update(&buf[..n]);
        payload_len += n as u64;
    }
    drop(f);
    let payload_sha = hex::encode(payload_hasher.finalize());
    if payload_len != header.payload_size {
        anyhow::bail!(
            "payload size mismatch: file={payload_len} header={}",
            header.payload_size
        );
    }
    if is_pending_payload_sha256(&header.payload_sha256) {
        patch_pending_payload_sha256(path, &payload_sha)?;
        header.payload_sha256 = payload_sha.clone();
    } else if header.payload_sha256.to_ascii_lowercase() != payload_sha {
        anyhow::bail!("payload checksum mismatch");
    }
    // Use payload SHA as the image checksum. Avoids a second full-file read of
    // multi‑GB captures (Edge/PE verify via hash_image_payload / header).
    Ok((header, payload_sha))
}

pub fn hex_sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash a disk/file without loading it into RAM (1MiB chunks).
/// Progress lines go to stderr so PE consoles show activity while hashing large disks.
pub fn hash_disk_path(src: &Path) -> anyhow::Result<(u64, String)> {
    use std::io::Write;
    let mut input = File::open(src)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    let mut last_report = 0u64;
    let report_every = 256 * 1024 * 1024u64;
    eprintln!("hashing {} ...", src.display());
    let _ = std::io::stderr().flush();
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
        if total - last_report >= report_every {
            eprintln!(
                "hashing {}: {:.2} GiB",
                src.display(),
                total as f64 / (1024.0 * 1024.0 * 1024.0)
            );
            let _ = std::io::stderr().flush();
            last_report = total;
        }
    }
    eprintln!(
        "hash done {}: {:.2} GiB",
        src.display(),
        total as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    let _ = std::io::stderr().flush();
    Ok((total, hex::encode(hasher.finalize())))
}

/// Copy `src` device/file to `dst` while hashing (1MiB chunks).
pub fn copy_disk_to_file(src: &Path, dst: &Path) -> anyhow::Result<(u64, String)> {
    let mut input = File::open(src)?;
    let mut output = File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    output.flush()?;
    Ok((total, hex::encode(hasher.finalize())))
}

/// Serialized yixue-image-v1 prefix (magic + header length + header JSON).
pub fn image_file_prefix(header: &ImageHeader) -> anyhow::Result<Vec<u8>> {
    let header_bytes = serde_json::to_vec(header)?;
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + header_bytes.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    Ok(out)
}

/// Byte offset where payload begins in a packed image file.
pub fn payload_offset_in_image(path: &Path) -> anyhow::Result<u64> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(MAGIC.len() as u64))?;
    let mut len_buf = [0u8; 4];
    f.read_exact(&mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as u64;
    Ok(MAGIC.len() as u64 + 4 + header_len)
}

pub fn unpack_image(bytes: &[u8]) -> anyhow::Result<PackedImage> {
    if bytes.len() < MAGIC.len() + 4 {
        anyhow::bail!("image too short");
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut off = MAGIC.len();
    let header_len =
        u32::from_le_bytes(bytes[off..off + 4].try_into().expect("4 bytes")) as usize;
    off += 4;
    if off + header_len > bytes.len() {
        anyhow::bail!("truncated header");
    }
    let header: ImageHeader = serde_json::from_slice(&bytes[off..off + header_len])?;
    off += header_len;
    let payload = bytes[off..].to_vec();
    let got = hex_sha256(&payload);
    if !is_pending_payload_sha256(&header.payload_sha256)
        && got != header.payload_sha256.to_ascii_lowercase()
    {
        anyhow::bail!("payload checksum mismatch");
    }
    if payload.len() as u64 != header.payload_size {
        anyhow::bail!("payload size mismatch");
    }
    Ok(PackedImage { header, payload })
}

pub fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Build default system/persistent layout for a raw disk capture.
pub fn default_partitions(total: u64) -> Vec<PartitionRole> {
    let system_len = (total * 2 / 3).max(1).min(total);
    let persistent_off = system_len;
    let persistent_len = total.saturating_sub(system_len);
    let mut parts = vec![PartitionRole {
        name: "system".into(),
        role: "system".into(),
        offset: 0,
        length: system_len,
    }];
    if persistent_len > 0 {
        parts.push(PartitionRole {
            name: "data".into(),
            role: "persistent".into(),
            offset: persistent_off,
            length: persistent_len,
        });
    }
    parts
}

/// Apply deploy according to disk plan onto an existing disk image buffer.
pub fn apply_deploy(
    disk_plan: &str,
    current_disk: Option<&[u8]>,
    image: &PackedImage,
) -> anyhow::Result<Vec<u8>> {
    let logical = crate::ntfs_used::expand_payload_to_logical(&image.header, &image.payload)?;
    match disk_plan {
        "wipe_whole_disk" => Ok(logical),
        "wipe_target_partitions" | "write_system_only" => {
            let mut disk = match current_disk {
                Some(d) if !d.is_empty() => d.to_vec(),
                _ => vec![0u8; logical.len()],
            };
            if disk.len() < logical.len() {
                disk.resize(logical.len(), 0);
            }
            for part in &image.header.partitions {
                if part.role != "system" {
                    continue;
                }
                let start = part.offset as usize;
                let end = start + part.length as usize;
                if end > logical.len() || end > disk.len() {
                    anyhow::bail!("system partition out of range");
                }
                disk[start..end].copy_from_slice(&logical[start..end]);
            }
            Ok(disk)
        }
        other => anyhow::bail!("unsupported disk_plan: {other}"),
    }
}

fn read_exact_reader(r: &mut dyn Read, buf: &mut [u8]) -> anyhow::Result<()> {
    r.read_exact(buf)?;
    Ok(())
}

fn write_at(disk: &mut File, off: u64, data: &[u8]) -> anyhow::Result<()> {
    disk.seek(SeekFrom::Start(off))?;
    disk.write_all(data)?;
    Ok(())
}

fn range_is_system(header: &ImageHeader, off: u64, len: u64) -> bool {
    let end = off.saturating_add(len);
    header.partitions.iter().any(|p| {
        if p.role != "system" {
            return false;
        }
        let p_end = p.offset.saturating_add(p.length);
        off < p_end && end > p.offset
    })
}

fn should_write_range(disk_plan: &str, header: &ImageHeader, off: u64, len: u64) -> bool {
    match disk_plan {
        "wipe_whole_disk" => true,
        "wipe_target_partitions" | "write_system_only" => range_is_system(header, off, len),
        _ => false,
    }
}

/// Stream a packed yixue image from `input` onto a block device / disk file without
/// buffering the whole image or logical disk in RAM (PE-safe).
pub fn deploy_reader_to_disk(
    input: &mut dyn Read,
    disk_path: &Path,
    disk_plan: &str,
    mut on_progress: Option<&mut dyn FnMut(u64, u64)>,
) -> anyhow::Result<ImageHeader> {
    if !matches!(
        disk_plan,
        "wipe_whole_disk" | "wipe_target_partitions" | "write_system_only"
    ) {
        anyhow::bail!("unsupported disk_plan: {disk_plan}");
    }

    let mut magic = [0u8; 7];
    read_exact_reader(input, &mut magic)?;
    if magic.as_slice() != MAGIC {
        anyhow::bail!("bad image magic");
    }
    let mut len_buf = [0u8; 4];
    read_exact_reader(input, &mut len_buf)?;
    let header_len = u32::from_le_bytes(len_buf) as usize;
    if header_len > 16 * 1024 * 1024 {
        anyhow::bail!("header too large: {header_len}");
    }
    let mut header_bytes = vec![0u8; header_len];
    read_exact_reader(input, &mut header_bytes)?;
    let header: ImageHeader = serde_json::from_slice(&header_bytes)?;
    let payload_total = header.payload_size;
    let mut payload_hasher = Sha256::new();
    let mut payload_read = 0u64;

    let mut disk = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk_path)?;

    let mut pull = |buf: &mut [u8]| -> anyhow::Result<()> {
        read_exact_reader(input, buf)?;
        payload_hasher.update(&*buf);
        payload_read += buf.len() as u64;
        if let Some(cb) = on_progress.as_mut() {
            cb(payload_read, payload_total);
        }
        Ok(())
    };

    if header.encoding == "ntfs_used_v1" {
        let mut u64buf = [0u8; 8];
        pull(&mut u64buf)?; // logical_size
        let mut u32buf = [0u8; 4];
        pull(&mut u32buf)?;
        let part_count = u32::from_le_bytes(u32buf) as usize;
        let mut chunk = vec![0u8; 1024 * 1024];
        for _ in 0..part_count {
            let mut part_off_b = [0u8; 8];
            let mut part_len_b = [0u8; 8];
            let mut kind_b = [0u8; 1];
            pull(&mut part_off_b)?;
            pull(&mut part_len_b)?;
            pull(&mut kind_b)?;
            let part_off = u64::from_le_bytes(part_off_b);
            let part_len = u64::from_le_bytes(part_len_b);
            let kind = kind_b[0];
            if kind == 0 {
                let mut left = part_len;
                let mut pos = part_off;
                while left > 0 {
                    let n = std::cmp::min(left, chunk.len() as u64) as usize;
                    pull(&mut chunk[..n])?;
                    if should_write_range(disk_plan, &header, pos, n as u64) {
                        write_at(&mut disk, pos, &chunk[..n])?;
                    }
                    pos += n as u64;
                    left -= n as u64;
                }
            } else if kind == 1 {
                let mut cs_b = [0u8; 4];
                let mut cc_b = [0u8; 8];
                let mut bm_len_b = [0u8; 8];
                pull(&mut cs_b)?;
                pull(&mut cc_b)?;
                pull(&mut bm_len_b)?;
                let cs = u32::from_le_bytes(cs_b) as usize;
                let cc = u64::from_le_bytes(cc_b);
                let bm_len = u64::from_le_bytes(bm_len_b) as usize;
                if cs == 0 || cs > 16 * 1024 * 1024 {
                    anyhow::bail!("invalid cluster size {cs}");
                }
                if bm_len > 64 * 1024 * 1024 {
                    anyhow::bail!("bitmap too large: {bm_len}");
                }
                let mut bitmap = vec![0u8; bm_len];
                pull(&mut bitmap)?;
                if chunk.len() < cs {
                    chunk.resize(cs, 0);
                }
                for c in 0..cc {
                    let used = {
                        let i = c as usize;
                        let byte = i / 8;
                        let bit = i % 8;
                        byte < bitmap.len() && bitmap[byte] & (1 << bit) != 0
                    };
                    if !used {
                        continue;
                    }
                    pull(&mut chunk[..cs])?;
                    let dst = part_off.saturating_add(c.saturating_mul(cs as u64));
                    if should_write_range(disk_plan, &header, dst, cs as u64) {
                        write_at(&mut disk, dst, &chunk[..cs])?;
                    }
                }
                let _ = part_len;
            } else {
                anyhow::bail!("unknown part kind {kind}");
            }
        }
    } else {
        // raw payload == logical disk bytes
        let mut left = payload_total;
        let mut pos = 0u64;
        let mut chunk = vec![0u8; 1024 * 1024];
        while left > 0 {
            let n = std::cmp::min(left, chunk.len() as u64) as usize;
            pull(&mut chunk[..n])?;
            if should_write_range(disk_plan, &header, pos, n as u64) {
                write_at(&mut disk, pos, &chunk[..n])?;
            }
            pos += n as u64;
            left -= n as u64;
        }
    }

    if payload_read != payload_total {
        anyhow::bail!("payload size mismatch: read={payload_read} header={payload_total}");
    }
    let got = hex::encode(payload_hasher.finalize());
    if !is_pending_payload_sha256(&header.payload_sha256)
        && got != header.payload_sha256.to_ascii_lowercase()
    {
        anyhow::bail!(
            "payload checksum mismatch: got {got}, expected {}",
            header.payload_sha256
        );
    }
    disk.flush()?;
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let payload = b"hello-disk-bytes".to_vec();
        let header = ImageHeader {
            format_version: 1,
            os_family: "windows".into(),
            boot_mode: "uefi".into(),
            partitions: default_partitions(payload.len() as u64),
            payload_sha256: String::new(),
            payload_size: 0,
            encoding: "raw".into(),
            logical_size: 0,
        };
        let packed = pack_image(header, payload.clone()).unwrap();
        let unpacked = unpack_image(&packed).unwrap();
        assert_eq!(unpacked.payload, payload);
        assert_eq!(unpacked.header.format_version, 1);
    }

    #[test]
    fn pack_to_path_and_inspect() {
        let dir = std::env::temp_dir().join(format!("idv-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let payload_path = dir.join("payload.bin");
        let out_path = dir.join("out.bin");
        let payload = b"stream-disk-bytes";
        std::fs::write(&payload_path, payload).unwrap();
        let header = ImageHeader {
            format_version: 1,
            os_family: "windows".into(),
            boot_mode: "uefi".into(),
            partitions: default_partitions(payload.len() as u64),
            payload_sha256: String::new(),
            payload_size: 0,
            encoding: "raw".into(),
            logical_size: 0,
        };
        let packed_header = pack_image_to_path(header, &payload_path, &out_path).unwrap();
        let inspected = inspect_image_file(&out_path).unwrap();
        assert_eq!(inspected.payload_sha256, packed_header.payload_sha256);
        assert_eq!(inspected.payload_size, payload.len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finalize_patches_pending_sha() {
        let dir = std::env::temp_dir().join(format!("idv-fin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("img.bin");
        let payload = b"pending-sha-payload";
        let header = ImageHeader {
            format_version: 1,
            os_family: "windows".into(),
            boot_mode: "uefi".into(),
            partitions: default_partitions(payload.len() as u64),
            payload_sha256: pending_payload_sha256(),
            payload_size: payload.len() as u64,
            encoding: "raw".into(),
            logical_size: 0,
        };
        let mut bytes = image_file_prefix(&header).unwrap();
        bytes.extend_from_slice(payload);
        std::fs::write(&out, &bytes).unwrap();
        let (h, _file_sha) = finalize_captured_image(&out).unwrap();
        assert!(!is_pending_payload_sha256(&h.payload_sha256));
        assert_eq!(h.payload_sha256, hex_sha256(payload));
        let again = inspect_image_file(&out).unwrap();
        assert_eq!(again.payload_sha256, h.payload_sha256);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_system_only_preserves_data() {
        let mut payload = vec![1u8; 90];
        payload[60..].fill(2);
        let header = ImageHeader {
            format_version: 1,
            os_family: "linux".into(),
            boot_mode: "uefi".into(),
            partitions: default_partitions(90),
            payload_sha256: String::new(),
            payload_size: 0,
            encoding: "raw".into(),
            logical_size: 0,
        };
        let image = PackedImage {
            header: {
                let mut h = header;
                h.payload_size = 90;
                h.payload_sha256 = hex_sha256(&payload);
                h
            },
            payload: payload.clone(),
        };
        let mut old = vec![9u8; 90];
        old[60..].fill(7);
        let out = apply_deploy("write_system_only", Some(&old), &image).unwrap();
        assert_eq!(&out[..60], &payload[..60]);
        assert_eq!(&out[60..], &[7u8; 30]);
    }
}
