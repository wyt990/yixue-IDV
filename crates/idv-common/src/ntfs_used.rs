//! NTFS used-cluster capture (partclone-like) and expand for deploy.
//!
//! Payload layout for `encoding = "ntfs_used_v1"`:
//! ```text
//! u64  logical_size
//! u32  part_count
//! for each part:
//!   u64 part_offset
//!   u64 part_length
//!   u8  kind  (0=raw bytes of part_length, 1=ntfs used clusters)
//!   if kind==0: raw[part_length]
//!   if kind==1:
//!     u32 cluster_size
//!     u64 cluster_count
//!     u64 bitmap_byte_len
//!     bitmap[bitmap_byte_len]
//!     used_cluster_data...
//! ```

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::image_format::{
    ImageHeader, PartitionRole, default_partitions, image_file_prefix, pending_payload_sha256,
};

const GPT_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];
const GPT_MS_RESERVED: [u8; 16] = [
    0x16, 0xe3, 0xc9, 0xe3, 0x5c, 0x0b, 0xb8, 0x4d, 0x81, 0x7d, 0xf9, 0x2d, 0xf0, 0x02, 0x15, 0xae,
];
const GPT_MS_BASIC_DATA: [u8; 16] = [
    0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7,
];

#[derive(Debug, Clone)]
struct GptPart {
    type_guid: [u8; 16],
    first_lba: u64,
    last_lba: u64,
}

enum PartPlan {
    Raw {
        offset: u64,
        length: u64,
    },
    NtfsUsed {
        offset: u64,
        length: u64,
        cluster_size: u32,
        cluster_count: u64,
        bitmap: Vec<u8>,
        used_clusters: u64,
    },
}

fn read_exact_at(f: &mut File, off: u64, buf: &mut [u8]) -> anyhow::Result<()> {
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(buf)?;
    Ok(())
}

/// Read up to `buf.len()` bytes at `off`; zero-fill if past `limit` (EOF / partition end).
fn read_at_clamped(
    f: &mut File,
    off: u64,
    buf: &mut [u8],
    limit: u64,
) -> anyhow::Result<()> {
    if off >= limit {
        buf.fill(0);
        return Ok(());
    }
    let avail = (limit - off) as usize;
    if avail >= buf.len() {
        return read_exact_at(f, off, buf);
    }
    buf.fill(0);
    if avail > 0 {
        read_exact_at(f, off, &mut buf[..avail])?;
    }
    Ok(())
}

fn parse_gpt(f: &mut File, disk_size: u64) -> anyhow::Result<Vec<GptPart>> {
    let mut hdr = [0u8; 92];
    read_exact_at(f, 512, &mut hdr)?;
    if &hdr[0..8] != b"EFI PART" {
        anyhow::bail!("not a GPT disk");
    }
    let part_entry_lba = u64::from_le_bytes(hdr[72..80].try_into()?);
    let num_parts = u32::from_le_bytes(hdr[80..84].try_into()?) as usize;
    let part_entry_size = u32::from_le_bytes(hdr[84..88].try_into()?) as usize;
    if !(128..=4096).contains(&part_entry_size) {
        anyhow::bail!("unsupported GPT entry size {part_entry_size}");
    }
    let mut table = vec![0u8; num_parts.saturating_mul(part_entry_size)];
    read_exact_at(f, part_entry_lba.saturating_mul(512), &mut table)?;
    let mut out = Vec::new();
    for i in 0..num_parts {
        let base = i * part_entry_size;
        if base + 128 > table.len() {
            break;
        }
        let e = &table[base..base + 128];
        if e[0..16].iter().all(|b| *b == 0) {
            continue;
        }
        let mut type_guid = [0u8; 16];
        type_guid.copy_from_slice(&e[0..16]);
        let first_lba = u64::from_le_bytes(e[32..40].try_into()?);
        let last_lba = u64::from_le_bytes(e[40..48].try_into()?);
        if last_lba < first_lba {
            continue;
        }
        let end = last_lba.saturating_add(1).saturating_mul(512);
        if end > disk_size {
            continue;
        }
        out.push(GptPart {
            type_guid,
            first_lba,
            last_lba,
        });
    }
    if out.is_empty() {
        anyhow::bail!("GPT has no partitions");
    }
    Ok(out)
}

fn is_ntfs_partition(f: &mut File, part_off: u64) -> anyhow::Result<bool> {
    let mut boot = [0u8; 512];
    read_exact_at(f, part_off, &mut boot)?;
    Ok(&boot[3..11] == b"NTFS    ")
}

fn read_ntfs_bitmap(f: &mut File, part_off: u64) -> anyhow::Result<(u32, u64, Vec<u8>)> {
    let mut boot = [0u8; 512];
    read_exact_at(f, part_off, &mut boot)?;
    if &boot[3..11] != b"NTFS    " {
        anyhow::bail!("not NTFS");
    }
    let bytes_per_sector = u16::from_le_bytes(boot[11..13].try_into()?) as u64;
    let sectors_per_cluster = boot[13] as u64;
    if bytes_per_sector == 0 || sectors_per_cluster == 0 {
        anyhow::bail!("invalid NTFS geometry");
    }
    let cluster_size = bytes_per_sector * sectors_per_cluster;
    // Boot "total sectors" is the authoritative volume size; $Bitmap is often padded
    // beyond the last real cluster — reading those LCNs hits UnexpectedEof.
    let total_sectors = u64::from_le_bytes(boot[40..48].try_into()?);
    let volume_clusters = if total_sectors > 0 {
        total_sectors / sectors_per_cluster
    } else {
        u64::MAX
    };
    let mft_lcn = u64::from_le_bytes(boot[48..56].try_into()?);
    let file_record_size = i8::from_le_bytes([boot[64]]) as i32;
    let frs = if file_record_size < 0 {
        1u64 << (-file_record_size as u32)
    } else {
        file_record_size as u64
    };
    let bitmap_record_off = part_off + mft_lcn * cluster_size + 6 * frs;
    let mut rec = vec![0u8; frs as usize];
    read_exact_at(f, bitmap_record_off, &mut rec)?;
    if &rec[0..4] != b"FILE" {
        anyhow::bail!("$Bitmap MFT record missing");
    }
    let attrs_off = u16::from_le_bytes(rec[20..22].try_into()?) as usize;
    let mut pos = attrs_off;
    let mut data_runs: Vec<u8> = Vec::new();
    let mut real_size = 0u64;
    while pos + 8 < rec.len() {
        let atype = u32::from_le_bytes(rec[pos..pos + 4].try_into()?);
        if atype == 0xffff_ffff {
            break;
        }
        let alen = u32::from_le_bytes(rec[pos + 4..pos + 8].try_into()?) as usize;
        if alen < 16 || pos + alen > rec.len() {
            break;
        }
        if atype == 0x80 {
            let non_resident = rec[pos + 8];
            if non_resident == 0 {
                anyhow::bail!("$Bitmap is resident (unexpected)");
            }
            real_size = u64::from_le_bytes(rec[pos + 48..pos + 56].try_into()?);
            let run_off = u16::from_le_bytes(rec[pos + 32..pos + 34].try_into()?) as usize;
            if pos + run_off < pos + alen {
                data_runs = rec[pos + run_off..pos + alen].to_vec();
            }
            break;
        }
        pos += alen;
    }
    if data_runs.is_empty() || real_size == 0 {
        anyhow::bail!("$Bitmap data runs not found");
    }
    let mut bitmap = vec![0u8; real_size as usize];
    let mut out_off = 0usize;
    let mut vcn_lcn: i64 = 0;
    let mut i = 0usize;
    while i < data_runs.len() {
        let header = data_runs[i];
        i += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0f) as usize;
        let off_size = ((header >> 4) & 0x0f) as usize;
        if i + len_size + off_size > data_runs.len() {
            break;
        }
        let mut run_len = 0u64;
        for b in 0..len_size {
            run_len |= (data_runs[i + b] as u64) << (8 * b);
        }
        i += len_size;
        let mut run_off = 0i64;
        for b in 0..off_size {
            run_off |= (data_runs[i + b] as i64) << (8 * b);
        }
        if off_size > 0 && data_runs[i + off_size - 1] & 0x80 != 0 {
            for b in off_size..8 {
                run_off |= -1i64 << (8 * b);
            }
        }
        i += off_size;
        if off_size == 0 {
            let hole = (run_len as usize).saturating_mul(cluster_size as usize);
            out_off = out_off.saturating_add(hole).min(bitmap.len());
            continue;
        }
        vcn_lcn += run_off;
        let src = part_off + (vcn_lcn as u64).saturating_mul(cluster_size);
        let bytes = (run_len as usize).saturating_mul(cluster_size as usize);
        let take = bytes.min(bitmap.len().saturating_sub(out_off));
        if take == 0 {
            break;
        }
        read_exact_at(f, src, &mut bitmap[out_off..out_off + take])?;
        out_off += take;
        if out_off >= bitmap.len() {
            break;
        }
    }
    let bitmap_clusters = (bitmap.len() as u64).saturating_mul(8);
    let clusters = bitmap_clusters.min(volume_clusters);
    if clusters < bitmap_clusters {
        eprintln!(
            "ntfs_used: clamp cluster_count {bitmap_clusters} -> {clusters} (volume_sectors={total_sectors})"
        );
    }
    Ok((cluster_size as u32, clusters, bitmap))
}

fn bit_used(bitmap: &[u8], cluster: u64) -> bool {
    let i = cluster as usize;
    let byte = i / 8;
    let bit = i % 8;
    if byte >= bitmap.len() {
        return false;
    }
    bitmap[byte] & (1 << bit) != 0
}

fn role_for_gpt(type_guid: &[u8; 16]) -> (&'static str, &'static str) {
    if type_guid == &GPT_EFI_SYSTEM {
        ("esp", "system")
    } else if type_guid == &GPT_MS_RESERVED {
        ("msr", "system")
    } else if type_guid == &GPT_MS_BASIC_DATA {
        ("basic", "system")
    } else {
        ("part", "system")
    }
}

fn analyze_disk(f: &mut File, disk_size: u64) -> anyhow::Result<(Vec<PartitionRole>, Vec<PartPlan>)> {
    let gpt = parse_gpt(f, disk_size)?;
    let mut roles = Vec::new();
    let mut plans = Vec::new();
    let mut seen_system_ntfs = false;
    for p in gpt {
        let off = p.first_lba * 512;
        let len = (p.last_lba + 1 - p.first_lba) * 512;
        let ntfs = is_ntfs_partition(f, off).unwrap_or(false);
        let (name, base_role) = role_for_gpt(&p.type_guid);
        let role = if ntfs && len > 1024 * 1024 * 1024 {
            if !seen_system_ntfs {
                seen_system_ntfs = true;
                "system"
            } else {
                "persistent"
            }
        } else {
            base_role
        };
        roles.push(PartitionRole {
            name: name.into(),
            role: role.into(),
            offset: off,
            length: len,
        });
        if ntfs && len > 64 * 1024 * 1024 {
            match read_ntfs_bitmap(f, off) {
                Ok((cs, cc, bm)) => {
                    let part_clusters = if cs > 0 { len / cs as u64 } else { 0 };
                    let cluster_count = cc.min((bm.len() as u64) * 8).min(part_clusters);
                    let mut used = 0u64;
                    for c in 0..cluster_count {
                        if bit_used(&bm, c) {
                            used += 1;
                        }
                    }
                    eprintln!(
                        "ntfs_used: part@{:#x} len={}GB used_clusters={used}/{cluster_count} (~{}GB)",
                        off,
                        len / (1024 * 1024 * 1024),
                        used * cs as u64 / (1024 * 1024 * 1024)
                    );
                    plans.push(PartPlan::NtfsUsed {
                        offset: off,
                        length: len,
                        cluster_size: cs,
                        cluster_count,
                        bitmap: bm,
                        used_clusters: used,
                    });
                    continue;
                }
                Err(err) => {
                    eprintln!("ntfs_used: bitmap failed for part@{off:#x}: {err}; raw fallback");
                }
            }
        }
        plans.push(PartPlan::Raw {
            offset: off,
            length: len,
        });
    }
    Ok((roles, plans))
}

fn payload_size_for(plans: &[PartPlan], disk_size: u64) -> u64 {
    let _ = disk_size;
    let mut n = 8u64 + 4;
    for p in plans {
        n += 8 + 8 + 1;
        match p {
            PartPlan::Raw { length, .. } => n += *length,
            PartPlan::NtfsUsed {
                cluster_size,
                bitmap,
                used_clusters,
                ..
            } => {
                n += 4 + 8 + 8 + bitmap.len() as u64 + (*used_clusters) * (*cluster_size as u64);
            }
        }
    }
    n
}

/// Pack disk into yixue image using NTFS used-cluster encoding when possible.
pub fn pack_disk_ntfs_used_to_writer(
    disk: &Path,
    out: &mut dyn Write,
    os_family: &str,
    boot_mode: &str,
) -> anyhow::Result<ImageHeader> {
    pack_disk_ntfs_used_to_writer_ex(disk, out, os_family, boot_mode, &mut |_| {})
}

/// Like [`pack_disk_ntfs_used_to_writer`], but invokes `on_total(total_bytes)` once the
/// packed size is known (after the image header is written).
pub fn pack_disk_ntfs_used_to_writer_ex(
    disk: &Path,
    out: &mut dyn Write,
    os_family: &str,
    boot_mode: &str,
    on_total: &mut dyn FnMut(u64),
) -> anyhow::Result<ImageHeader> {
    let mut f = File::open(disk)?;
    let disk_size = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(0))?;

    let (roles, plans) = match analyze_disk(&mut f, disk_size) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("ntfs_used: GPT/analyze failed ({err}); raw whole-disk fallback");
            return pack_raw_disk_to_writer(disk, out, os_family, boot_mode, disk_size, on_total);
        }
    };

    let payload_size = payload_size_for(&plans, disk_size);
    let header = ImageHeader {
        format_version: 1,
        os_family: os_family.into(),
        boot_mode: boot_mode.into(),
        partitions: if roles.is_empty() {
            default_partitions(disk_size)
        } else {
            roles
        },
        payload_sha256: pending_payload_sha256(),
        payload_size,
        encoding: "ntfs_used_v1".into(),
        logical_size: disk_size,
    };
    let prefix = image_file_prefix(&header)?;
    out.write_all(&prefix)?;
    on_total(prefix.len() as u64 + payload_size);
    out.write_all(&disk_size.to_le_bytes())?;
    out.write_all(&(plans.len() as u32).to_le_bytes())?;

    let mut buf = vec![0u8; 1024 * 1024];
    for plan in &plans {
        match plan {
            PartPlan::Raw { offset, length } => {
                out.write_all(&offset.to_le_bytes())?;
                out.write_all(&length.to_le_bytes())?;
                out.write_all(&[0u8])?;
                let mut left = *length;
                let mut pos = *offset;
                let part_end = offset.saturating_add(*length).min(disk_size);
                while left > 0 {
                    let n = std::cmp::min(left, buf.len() as u64) as usize;
                    read_at_clamped(&mut f, pos, &mut buf[..n], part_end)?;
                    out.write_all(&buf[..n])?;
                    pos += n as u64;
                    left -= n as u64;
                }
            }
            PartPlan::NtfsUsed {
                offset,
                length,
                cluster_size,
                cluster_count,
                bitmap,
                ..
            } => {
                out.write_all(&offset.to_le_bytes())?;
                out.write_all(&length.to_le_bytes())?;
                out.write_all(&[1u8])?;
                out.write_all(&cluster_size.to_le_bytes())?;
                out.write_all(&cluster_count.to_le_bytes())?;
                out.write_all(&(bitmap.len() as u64).to_le_bytes())?;
                out.write_all(bitmap)?;
                let cs = *cluster_size as usize;
                if buf.len() < cs {
                    buf.resize(cs, 0);
                }
                let part_end = offset.saturating_add(*length).min(disk_size);
                for c in 0..*cluster_count {
                    if !bit_used(bitmap, c) {
                        continue;
                    }
                    let src = *offset + c * *cluster_size as u64;
                    read_at_clamped(&mut f, src, &mut buf[..cs], part_end)?;
                    out.write_all(&buf[..cs])?;
                }
            }
        }
    }
    out.flush()?;
    Ok(header)
}

fn pack_raw_disk_to_writer(
    disk: &Path,
    out: &mut dyn Write,
    os_family: &str,
    boot_mode: &str,
    disk_size: u64,
    on_total: &mut dyn FnMut(u64),
) -> anyhow::Result<ImageHeader> {
    let header = ImageHeader {
        format_version: 1,
        os_family: os_family.into(),
        boot_mode: boot_mode.into(),
        partitions: default_partitions(disk_size),
        payload_sha256: pending_payload_sha256(),
        payload_size: disk_size,
        encoding: "raw".into(),
        logical_size: disk_size,
    };
    let prefix = image_file_prefix(&header)?;
    out.write_all(&prefix)?;
    on_total(prefix.len() as u64 + disk_size);
    let mut f = File::open(disk)?;
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    Ok(header)
}

/// Expand `ntfs_used_v1` / `raw` payload into a full logical disk.
pub fn expand_payload_to_logical(header: &ImageHeader, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    if header.encoding != "ntfs_used_v1" {
        return Ok(payload.to_vec());
    }
    let logical = if header.logical_size > 0 {
        header.logical_size
    } else {
        header.payload_size
    };
    let mut disk = vec![0u8; logical as usize];
    if payload.len() < 12 {
        anyhow::bail!("ntfs_used payload too short");
    }
    let mut o = 0usize;
    o += 8; // logical_size
    let part_count = u32::from_le_bytes(payload[o..o + 4].try_into()?) as usize;
    o += 4;
    for _ in 0..part_count {
        if o + 17 > payload.len() {
            anyhow::bail!("truncated part header");
        }
        let part_off = u64::from_le_bytes(payload[o..o + 8].try_into()?) as usize;
        o += 8;
        let part_len = u64::from_le_bytes(payload[o..o + 8].try_into()?) as usize;
        o += 8;
        let kind = payload[o];
        o += 1;
        if kind == 0 {
            if o + part_len > payload.len() || part_off + part_len > disk.len() {
                anyhow::bail!("raw part out of range");
            }
            disk[part_off..part_off + part_len].copy_from_slice(&payload[o..o + part_len]);
            o += part_len;
        } else if kind == 1 {
            let cs = u32::from_le_bytes(payload[o..o + 4].try_into()?) as usize;
            o += 4;
            let cc = u64::from_le_bytes(payload[o..o + 8].try_into()?);
            o += 8;
            let bm_len = u64::from_le_bytes(payload[o..o + 8].try_into()?) as usize;
            o += 8;
            if o + bm_len > payload.len() {
                anyhow::bail!("truncated bitmap");
            }
            let bm = &payload[o..o + bm_len];
            o += bm_len;
            for c in 0..cc {
                if !bit_used(bm, c) {
                    continue;
                }
                if o + cs > payload.len() {
                    anyhow::bail!("truncated cluster data");
                }
                let dst = part_off + c as usize * cs;
                if dst + cs > disk.len() {
                    anyhow::bail!("cluster out of disk");
                }
                disk[dst..dst + cs].copy_from_slice(&payload[o..o + cs]);
                o += cs;
            }
        } else {
            anyhow::bail!("unknown part kind {kind}");
        }
    }
    Ok(disk)
}
