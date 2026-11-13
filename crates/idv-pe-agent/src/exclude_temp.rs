//! Pre-capture cleanup of pagefile / temp dirs on NTFS volumes (PE Linux).
//!
//! Logs every action with `[exclude]` prefix for task detail / stderr.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default exclusion relative paths (Windows volume root).
pub const DEFAULT_EXCLUDE_REL: &[&str] = &[
    "pagefile.sys",
    "swapfile.sys",
    "hiberfil.sys",
    "$Recycle.Bin",
    "Windows\\Temp",
    "Windows\\Prefetch",
    "Windows\\SoftwareDistribution\\Download",
];

#[derive(Debug, Default)]
pub struct ExcludeReport {
    pub lines: Vec<String>,
    pub removed: u32,
    pub zeroed: u32,
    pub skipped: u32,
    pub errors: u32,
}

impl ExcludeReport {
    fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("{line}");
        let _ = std::io::stderr().flush();
        self.lines.push(line);
    }

    pub fn summary(&self) -> String {
        format!(
            "[exclude] done removed={} zeroed={} skipped={} errors={}",
            self.removed, self.zeroed, self.skipped, self.errors
        )
    }
}

/// Run exclusion template against NTFS partitions of `disk` (block device).
/// On non-Linux / file-backed disks: skip with log (capture still proceeds).
pub fn run_exclude_on_disk(disk: &Path) -> ExcludeReport {
    let mut report = ExcludeReport::default();
    report.log(format!(
        "[exclude] start disk={} template_entries={}",
        disk.display(),
        DEFAULT_EXCLUDE_REL.len()
    ));

    if cfg!(not(target_os = "linux")) {
        report.log("[exclude] skip: not Linux PE (host OS)");
        report.skipped += 1;
        report.log(report.summary());
        return report;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        match fs::metadata(disk) {
            Ok(meta) if !meta.file_type().is_block_device() => {
                report.log("[exclude] skip: disk is not a block device (file-backed test?)");
                report.skipped += 1;
                report.log(report.summary());
                return report;
            }
            Err(err) => {
                report.log(format!("[exclude] fail: cannot stat disk: {err}"));
                report.errors += 1;
                report.log(report.summary());
                return report;
            }
            _ => {}
        }
    }

    let parts = list_ntfs_partitions(disk, &mut report);
    if parts.is_empty() {
        report.log("[exclude] skip: no NTFS partitions found");
        report.skipped += 1;
        report.log(report.summary());
        return report;
    }

    for (i, part) in parts.iter().enumerate() {
        let mnt = PathBuf::from(format!("/tmp/idv-exclude-{i}"));
        let _ = fs::create_dir_all(&mnt);
        report.log(format!(
            "[exclude] mount {} -> {}",
            part.display(),
            mnt.display()
        ));
        let mount_ok = Command::new("mount")
            .args([
                "-t",
                "ntfs-3g",
                "-o",
                "rw,remove_hiberfile",
                &part.to_string_lossy(),
                &mnt.to_string_lossy(),
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !mount_ok {
            // fallback without remove_hiberfile
            let mount_ok2 = Command::new("mount")
                .args([
                    "-t",
                    "ntfs-3g",
                    "-o",
                    "rw",
                    &part.to_string_lossy(),
                    &mnt.to_string_lossy(),
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !mount_ok2 {
                report.log(format!("[exclude] fail: mount {} failed", part.display()));
                report.errors += 1;
                let _ = fs::remove_dir(&mnt);
                continue;
            }
        }

        for rel in DEFAULT_EXCLUDE_REL {
            let rel_path = rel.replace('\\', "/");
            let target = mnt.join(&rel_path);
            clean_path(&target, rel, &mut report);
        }
        // User profile Temp dirs
        let users = mnt.join("Users");
        if users.is_dir() {
            if let Ok(rd) = fs::read_dir(&users) {
                for ent in rd.flatten() {
                    let temp = ent.path().join("AppData/Local/Temp");
                    if temp.exists() {
                        clean_path(&temp, "Users/*/AppData/Local/Temp", &mut report);
                    }
                }
            }
        }

        let _ = Command::new("umount").arg(&mnt).status();
        let _ = fs::remove_dir(&mnt);
        report.log(format!("[exclude] umount {}", part.display()));
    }

    report.log(report.summary());
    report
}

fn list_ntfs_partitions(disk: &Path, report: &mut ExcludeReport) -> Vec<PathBuf> {
    let out = Command::new("lsblk")
        .args(["-ln", "-o", "PATH,FSTYPE", &disk.to_string_lossy()])
        .output();
    let Ok(out) = out else {
        report.log("[exclude] warn: lsblk failed");
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = Vec::new();
    for line in text.lines() {
        let mut cols = line.split_whitespace();
        let Some(path) = cols.next() else { continue };
        let fstype = cols.next().unwrap_or("").to_ascii_lowercase();
        if fstype == "ntfs" || fstype.contains("ntfs") {
            parts.push(PathBuf::from(path));
        }
    }
    // Fallback: common partition suffixes when lsblk has empty FSTYPE
    if parts.is_empty() {
        for n in 1..=8 {
            let p = PathBuf::from(format!("{}{n}", disk.display()));
            if p.exists() {
                // try blkid
                let ft = Command::new("blkid")
                    .args(["-o", "value", "-s", "TYPE", &p.to_string_lossy()])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_ascii_lowercase())
                    .unwrap_or_default();
                if ft.contains("ntfs") {
                    parts.push(p);
                }
            }
            let p2 = PathBuf::from(format!("{}p{n}", disk.display()));
            if p2.exists() {
                let ft = Command::new("blkid")
                    .args(["-o", "value", "-s", "TYPE", &p2.to_string_lossy()])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_ascii_lowercase())
                    .unwrap_or_default();
                if ft.contains("ntfs") {
                    parts.push(p2);
                }
            }
        }
    }
    report.log(format!("[exclude] ntfs_partitions={}", parts.len()));
    parts
}

fn clean_path(path: &Path, label: &str, report: &mut ExcludeReport) {
    if !path.exists() {
        report.log(format!("[exclude] skip missing {label}"));
        report.skipped += 1;
        return;
    }
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(err) => {
            report.log(format!("[exclude] fail stat {label}: {err}"));
            report.errors += 1;
            return;
        }
    };
    if meta.is_dir() {
        match fs::remove_dir_all(path) {
            Ok(()) => {
                report.log(format!("[exclude] removed dir {label}"));
                report.removed += 1;
                let _ = fs::create_dir_all(path);
            }
            Err(err) => {
                report.log(format!("[exclude] fail rm dir {label}: {err}"));
                report.errors += 1;
            }
        }
        return;
    }
    match fs::remove_file(path) {
        Ok(()) => {
            report.log(format!(
                "[exclude] removed file {label} size={}",
                meta.len()
            ));
            report.removed += 1;
        }
        Err(err) => {
            // Try truncate / zero
            match fs::OpenOptions::new().write(true).open(path) {
                Ok(f) => match f.set_len(0) {
                    Ok(()) => {
                        report.log(format!(
                            "[exclude] zeroed file {label} (rm failed: {err})"
                        ));
                        report.zeroed += 1;
                    }
                    Err(err2) => {
                        report.log(format!(
                            "[exclude] fail {label}: rm={err} truncate={err2}"
                        ));
                        report.errors += 1;
                    }
                },
                Err(err2) => {
                    report.log(format!("[exclude] fail {label}: rm={err} open={err2}"));
                    report.errors += 1;
                }
            }
        }
    }
}
