//! Pack-time NTFS path exclusion (no mount / ntfs-3g required).
//!
//! Clears used-cluster bitmap bits for pagefile / hiberfil / Temp / etc. so
//! `ntfs_used_v1` payloads shrink without mutating the gold disk.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Default relative paths / prefixes excluded from used-cluster packs.
pub const DEFAULT_EXCLUDE_PATHS: &[&str] = &[
    "pagefile.sys",
    "swapfile.sys",
    "hiberfil.sys",
    "$recycle.bin",
    "windows\\temp",
    "windows\\prefetch",
    "windows\\softwaredistribution\\download",
];

#[derive(Debug, Default, Clone)]
pub struct ExcludePackReport {
    pub files: u32,
    pub clusters: u64,
    pub bytes: u64,
    pub lines: Vec<String>,
}

impl ExcludePackReport {
    fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("{line}");
        self.lines.push(line);
    }

    pub fn summary(&self) -> String {
        format!(
            "[exclude] pack-skip files={} clusters={} (~{}MB)",
            self.files,
            self.clusters,
            self.bytes / (1024 * 1024)
        )
    }
}

/// Returns true when a volume-relative NTFS path should be omitted from the image.
pub fn path_should_exclude(path: &str, rules: &[&str]) -> bool {
    let p = normalize_path(path);
    if p.is_empty() {
        return false;
    }
    for rule in rules {
        let r = normalize_path(rule);
        if r.is_empty() {
            continue;
        }
        if p == r || p.starts_with(&(r.clone() + "\\")) {
            return true;
        }
        // Also match "Users\\*\\AppData\\Local\\Temp\\..."
        if r.contains("\\*\\") {
            continue;
        }
    }
    // User profile temp dirs (any depth-1 profile name).
    if let Some(rest) = p.strip_prefix("users\\") {
        if let Some((_, after)) = rest.split_once('\\') {
            if after == "appdata\\local\\temp"
                || after.starts_with("appdata\\local\\temp\\")
            {
                return true;
            }
        }
    }
    false
}

fn normalize_path(path: &str) -> String {
    path.replace('/', "\\")
        .trim_matches('\\')
        .to_ascii_lowercase()
}

struct NtfsGeo {
    part_off: u64,
    sector_size: usize,
    cluster_size: u64,
    frs: u64,
    mft_lcn: u64,
}

#[derive(Clone, Default)]
struct NameEntry {
    parent: u64,
    name: String,
}

/// Clear bitmap bits for clusters belonging to excluded paths. Disk is read-only.
pub fn clear_excluded_clusters(
    f: &mut File,
    part_off: u64,
    cluster_size: u32,
    cluster_count: u64,
    bitmap: &mut [u8],
    rules: &[&str],
) -> ExcludePackReport {
    let mut report = ExcludePackReport::default();
    if rules.is_empty() || cluster_size == 0 {
        return report;
    }
    let geo = match read_geo(f, part_off) {
        Ok(g) => g,
        Err(err) => {
            report.log(format!("[exclude] pack-skip abort: geo {err}"));
            return report;
        }
    };
    if geo.cluster_size == 0 || geo.frs == 0 {
        report.log("[exclude] pack-skip abort: invalid geometry");
        return report;
    }

    let mft_runs = match read_mft_data_runs(f, &geo) {
        Ok(r) => r,
        Err(err) => {
            report.log(format!("[exclude] pack-skip abort: $MFT {err}"));
            return report;
        }
    };

    let mft_bytes = match read_runs_data(f, &geo, &mft_runs, None) {
        Ok(b) => b,
        Err(err) => {
            report.log(format!("[exclude] pack-skip abort: read $MFT {err}"));
            return report;
        }
    };

    let frs = geo.frs as usize;
    if frs == 0 || mft_bytes.len() < frs {
        report.log("[exclude] pack-skip abort: empty $MFT");
        return report;
    }
    let record_count = mft_bytes.len() / frs;
    let mut names: Vec<Option<NameEntry>> = vec![None; record_count];
    let mut data_runs: Vec<Option<Vec<u8>>> = vec![None; record_count];

    for i in 0..record_count {
        let start = i * frs;
        let mut rec = mft_bytes[start..start + frs].to_vec();
        if !apply_fixups(&mut rec, geo.sector_size) {
            continue;
        }
        if &rec[0..4] != b"FILE" {
            continue;
        }
        let flags = u16::from_le_bytes(rec[22..24].try_into().unwrap_or([0, 0]));
        if flags & 0x1 == 0 {
            continue; // not in use
        }
        let attrs_off = u16::from_le_bytes(rec[20..22].try_into().unwrap_or([0, 0])) as usize;
        let mut pos = attrs_off;
        let mut best_name: Option<NameEntry> = None;
        let mut unnamed_runs: Option<Vec<u8>> = None;
        while pos + 8 < rec.len() {
            let atype = u32::from_le_bytes(rec[pos..pos + 4].try_into().unwrap_or([0; 4]));
            if atype == 0xffff_ffff {
                break;
            }
            let alen = u32::from_le_bytes(rec[pos + 4..pos + 8].try_into().unwrap_or([0; 4])) as usize;
            if alen < 16 || pos + alen > rec.len() {
                break;
            }
            if atype == 0x30 {
                // FILE_NAME
                let non_res = rec[pos + 8];
                if non_res == 0 {
                    let content_off =
                        u16::from_le_bytes(rec[pos + 20..pos + 22].try_into().unwrap_or([0, 0]))
                            as usize;
                    let content_len =
                        u32::from_le_bytes(rec[pos + 16..pos + 20].try_into().unwrap_or([0; 4]))
                            as usize;
                    let c0 = pos + content_off;
                    if c0 + 66 <= pos + alen && content_len >= 66 {
                        let body = &rec[c0..c0 + content_len.min(pos + alen - c0)];
                        if let Some(entry) = parse_file_name(body) {
                            // Prefer Win32 namespace (1) over DOS (2).
                            let ns = body.get(65).copied().unwrap_or(0);
                            let replace = match &best_name {
                                None => true,
                                Some(_) => ns == 1 || ns == 3,
                            };
                            if replace {
                                best_name = Some(entry);
                            }
                        }
                    }
                }
            } else if atype == 0x80 {
                let name_len = rec[pos + 9] as usize;
                let non_res = rec[pos + 8];
                if name_len == 0 && non_res != 0 {
                    let run_off =
                        u16::from_le_bytes(rec[pos + 32..pos + 34].try_into().unwrap_or([0, 0]))
                            as usize;
                    if pos + run_off < pos + alen {
                        unnamed_runs = Some(rec[pos + run_off..pos + alen].to_vec());
                    }
                }
            }
            pos += alen;
        }
        if let Some(n) = best_name {
            names[i] = Some(n);
        }
        if let Some(r) = unnamed_runs {
            data_runs[i] = Some(r);
        }
    }

    let mut cleared_files = 0u32;
    let mut cleared_clusters = 0u64;
    for i in 0..record_count {
        let Some(path) = resolve_path(&names, i as u64) else {
            continue;
        };
        if !path_should_exclude(&path, rules) {
            continue;
        }
        let Some(runs) = data_runs[i].as_ref() else {
            continue;
        };
        let before = cleared_clusters;
        cleared_clusters += clear_runs_in_bitmap(runs, cluster_count, bitmap);
        let delta = cleared_clusters - before;
        if delta > 0 {
            cleared_files += 1;
            report.log(format!(
                "[exclude] pack-skip {path} clusters={delta} (~{}MB)",
                delta * cluster_size as u64 / (1024 * 1024)
            ));
        }
    }

    report.files = cleared_files;
    report.clusters = cleared_clusters;
    report.bytes = cleared_clusters.saturating_mul(cluster_size as u64);
    report.log(report.summary());
    report
}

fn read_geo(f: &mut File, part_off: u64) -> anyhow::Result<NtfsGeo> {
    let mut boot = [0u8; 512];
    read_exact_at(f, part_off, &mut boot)?;
    if &boot[3..11] != b"NTFS    " {
        anyhow::bail!("not NTFS");
    }
    let bytes_per_sector = u16::from_le_bytes(boot[11..13].try_into()?) as u64;
    let sectors_per_cluster = boot[13] as u64;
    if bytes_per_sector == 0 || sectors_per_cluster == 0 {
        anyhow::bail!("bad geometry");
    }
    let cluster_size = bytes_per_sector * sectors_per_cluster;
    let mft_lcn = u64::from_le_bytes(boot[48..56].try_into()?);
    let file_record_size = i8::from_le_bytes([boot[64]]) as i32;
    let frs = if file_record_size < 0 {
        1u64 << (-file_record_size as u32)
    } else {
        file_record_size as u64
    };
    Ok(NtfsGeo {
        part_off,
        sector_size: bytes_per_sector as usize,
        cluster_size,
        frs,
        mft_lcn,
    })
}

fn read_mft_data_runs(f: &mut File, geo: &NtfsGeo) -> anyhow::Result<Vec<u8>> {
    let mut rec = vec![0u8; geo.frs as usize];
    let off = geo.part_off + geo.mft_lcn * geo.cluster_size;
    read_exact_at(f, off, &mut rec)?;
    if !apply_fixups(&mut rec, geo.sector_size) || &rec[0..4] != b"FILE" {
        anyhow::bail!("$MFT record invalid");
    }
    let attrs_off = u16::from_le_bytes(rec[20..22].try_into()?) as usize;
    let mut pos = attrs_off;
    while pos + 8 < rec.len() {
        let atype = u32::from_le_bytes(rec[pos..pos + 4].try_into()?);
        if atype == 0xffff_ffff {
            break;
        }
        let alen = u32::from_le_bytes(rec[pos + 4..pos + 8].try_into()?) as usize;
        if alen < 16 || pos + alen > rec.len() {
            break;
        }
        if atype == 0x80 && rec[pos + 9] == 0 && rec[pos + 8] != 0 {
            let run_off = u16::from_le_bytes(rec[pos + 32..pos + 34].try_into()?) as usize;
            if pos + run_off < pos + alen {
                return Ok(rec[pos + run_off..pos + alen].to_vec());
            }
        }
        pos += alen;
    }
    // Fallback: contiguous MFT at mft_lcn (first few records only) — not enough.
    anyhow::bail!("$MFT DATA runs missing");
}

fn read_runs_data(
    f: &mut File,
    geo: &NtfsGeo,
    runs: &[u8],
    max_bytes: Option<usize>,
) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut vcn_lcn: i64 = 0;
    let mut i = 0usize;
    while i < runs.len() {
        let header = runs[i];
        i += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0f) as usize;
        let off_size = ((header >> 4) & 0x0f) as usize;
        if i + len_size + off_size > runs.len() {
            break;
        }
        let mut run_len = 0u64;
        for b in 0..len_size {
            run_len |= (runs[i + b] as u64) << (8 * b);
        }
        i += len_size;
        let mut run_off = 0i64;
        for b in 0..off_size {
            run_off |= (runs[i + b] as i64) << (8 * b);
        }
        if off_size > 0 && runs[i + off_size - 1] & 0x80 != 0 {
            for b in off_size..8 {
                run_off |= -1i64 << (8 * b);
            }
        }
        i += off_size;
        let bytes = (run_len as usize).saturating_mul(geo.cluster_size as usize);
        if off_size == 0 {
            out.resize(out.len().saturating_add(bytes), 0);
        } else {
            vcn_lcn += run_off;
            let src = geo.part_off + (vcn_lcn as u64).saturating_mul(geo.cluster_size);
            let mut chunk = vec![0u8; bytes];
            read_exact_at(f, src, &mut chunk)?;
            out.extend_from_slice(&chunk);
        }
        if let Some(max) = max_bytes {
            if out.len() >= max {
                out.truncate(max);
                break;
            }
        }
        // Soft cap: 2 GiB MFT read (extremely large volumes).
        if out.len() > 2 * 1024 * 1024 * 1024 {
            anyhow::bail!("$MFT too large");
        }
    }
    Ok(out)
}

fn clear_runs_in_bitmap(runs: &[u8], cluster_count: u64, bitmap: &mut [u8]) -> u64 {
    let mut cleared = 0u64;
    let mut vcn_lcn: i64 = 0;
    let mut i = 0usize;
    while i < runs.len() {
        let header = runs[i];
        i += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0f) as usize;
        let off_size = ((header >> 4) & 0x0f) as usize;
        if i + len_size + off_size > runs.len() {
            break;
        }
        let mut run_len = 0u64;
        for b in 0..len_size {
            run_len |= (runs[i + b] as u64) << (8 * b);
        }
        i += len_size;
        let mut run_off = 0i64;
        for b in 0..off_size {
            run_off |= (runs[i + b] as i64) << (8 * b);
        }
        if off_size > 0 && runs[i + off_size - 1] & 0x80 != 0 {
            for b in off_size..8 {
                run_off |= -1i64 << (8 * b);
            }
        }
        i += off_size;
        if off_size == 0 {
            continue; // sparse hole
        }
        vcn_lcn += run_off;
        let start = vcn_lcn as u64;
        for c in start..start.saturating_add(run_len) {
            if c >= cluster_count {
                break;
            }
            let byte = (c as usize) / 8;
            let bit = (c as usize) % 8;
            if byte >= bitmap.len() {
                break;
            }
            if bitmap[byte] & (1 << bit) != 0 {
                bitmap[byte] &= !(1 << bit);
                cleared += 1;
            }
        }
    }
    cleared
}

fn parse_file_name(body: &[u8]) -> Option<NameEntry> {
    if body.len() < 66 {
        return None;
    }
    let parent = u64::from_le_bytes(body[0..8].try_into().ok()?) & 0x0000_ffff_ffff_ffff;
    let name_len = body[64] as usize;
    if name_len == 0 || 66 + name_len * 2 > body.len() {
        return None;
    }
    let mut u16s = Vec::with_capacity(name_len);
    for i in 0..name_len {
        let o = 66 + i * 2;
        u16s.push(u16::from_le_bytes([body[o], body[o + 1]]));
    }
    let name = String::from_utf16_lossy(&u16s);
    if name.is_empty() {
        return None;
    }
    Some(NameEntry { parent, name })
}

fn resolve_path(names: &[Option<NameEntry>], mut mft: u64) -> Option<String> {
    let mut parts = Vec::new();
    let mut guard = 0;
    while mft != 5 && guard < 64 {
        guard += 1;
        let idx = mft as usize;
        if idx >= names.len() {
            return None;
        }
        let ent = names[idx].as_ref()?;
        parts.push(ent.name.clone());
        if ent.parent == mft {
            return None;
        }
        mft = ent.parent;
    }
    if mft != 5 && mft != 0 {
        // Not under root — still allow matching by leaf name for pagefile etc.
        if parts.len() == 1 {
            return Some(parts[0].clone());
        }
        return None;
    }
    parts.reverse();
    Some(parts.join("\\"))
}

fn apply_fixups(buf: &mut [u8], sector_size: usize) -> bool {
    if buf.len() < 8 || sector_size == 0 {
        return false;
    }
    let sig = &buf[0..4];
    if sig != b"FILE" && sig != b"INDX" {
        return false;
    }
    let usa_off = u16::from_le_bytes([buf[4], buf[5]]) as usize;
    let usa_count = u16::from_le_bytes([buf[6], buf[7]]) as usize;
    if usa_count == 0 || usa_off == 0 {
        return true;
    }
    if usa_off + usa_count * 2 > buf.len() {
        return false;
    }
    let mut usa = vec![0u8; usa_count * 2];
    usa.copy_from_slice(&buf[usa_off..usa_off + usa_count * 2]);
    for i in 1..usa_count {
        let sector_end = i * sector_size;
        if sector_end < 2 || sector_end > buf.len() {
            return false;
        }
        let dst = sector_end - 2;
        buf[dst] = usa[i * 2];
        buf[dst + 1] = usa[i * 2 + 1];
    }
    true
}

fn read_exact_at(f: &mut File, off: u64, buf: &mut [u8]) -> anyhow::Result<()> {
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(buf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_sys_and_temp_paths() {
        let rules = DEFAULT_EXCLUDE_PATHS;
        assert!(path_should_exclude("pagefile.sys", rules));
        assert!(path_should_exclude("PageFile.SYS", rules));
        assert!(path_should_exclude("Windows\\Temp\\a.txt", rules));
        assert!(path_should_exclude("Windows\\Prefetch\\x.pf", rules));
        assert!(path_should_exclude("$Recycle.Bin\\S-1-5\\x", rules));
        assert!(path_should_exclude(
            "Users\\Admin\\AppData\\Local\\Temp\\x",
            rules
        ));
        assert!(!path_should_exclude("Windows\\System32\\kernel32.dll", rules));
        assert!(!path_should_exclude("Users\\Admin\\Documents\\a.docx", rules));
    }
}
