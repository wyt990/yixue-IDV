//! Volume backends for restore (stage 6d).
//!
//! - `file`: single disk image buffer (existing partition-offset model)
//! - `windows-lab`: directory-per-role layout for Windows agent dry-run / lab CI
//!   (not raw sector I/O; production machines use boot-hook + this layout until
//!    a signed volume filter is available)

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::image_format::PackedImage;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeBackend {
    File,
    WindowsLab,
}

impl VolumeBackend {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "file" => Ok(Self::File),
            "windows-lab" | "windows" | "win" => Ok(Self::WindowsLab),
            other => Err(Error::Other(anyhow::anyhow!(
                "unknown volume backend: {other} (file|windows-lab)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeLayout {
    pub backend: VolumeBackend,
    pub root: PathBuf,
    pub system_dir: String,
    pub persistent_dir: String,
    pub volatile_dir: String,
}

impl VolumeLayout {
    pub fn windows_lab(root: impl Into<PathBuf>) -> Self {
        Self {
            backend: VolumeBackend::WindowsLab,
            root: root.into(),
            system_dir: "system".into(),
            persistent_dir: "persistent".into(),
            volatile_dir: "volatile".into(),
        }
    }

    pub fn system_path(&self) -> PathBuf {
        self.root.join(&self.system_dir)
    }
    pub fn persistent_path(&self) -> PathBuf {
        self.root.join(&self.persistent_dir)
    }
    pub fn volatile_path(&self) -> PathBuf {
        self.root.join(&self.volatile_dir)
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for p in [
            self.system_path(),
            self.persistent_path(),
            self.volatile_path(),
        ] {
            std::fs::create_dir_all(&p).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
        Ok(())
    }
}

/// Marker written by baseline pack for windows-lab (role file snapshots).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabVolumeSnapshot {
    pub format_version: u32,
    pub system_marker: String,
    pub persistent_marker: String,
    pub volatile_marker: String,
}

pub fn write_lab_baseline(baseline_root: &Path, snap: &LabVolumeSnapshot) -> Result<()> {
    let layout = VolumeLayout::windows_lab(baseline_root);
    layout.ensure_dirs()?;
    std::fs::write(layout.system_path().join("BASELINE.marker"), &snap.system_marker)
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    std::fs::write(
        layout.persistent_path().join("BASELINE.marker"),
        &snap.persistent_marker,
    )
    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    std::fs::write(
        layout.volatile_path().join("BASELINE.marker"),
        &snap.volatile_marker,
    )
    .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let meta = baseline_root.join("snapshot.json");
    let bytes = serde_json::to_vec_pretty(snap).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    std::fs::write(meta, bytes).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok(())
}

/// Apply mode actions on a windows-lab volume root using a baseline volume root.
pub fn apply_windows_lab_restore(
    live: &VolumeLayout,
    baseline: &VolumeLayout,
    system_action: &str,
    persistent_action: &str,
    volatile_action: &str,
) -> Result<Vec<String>> {
    live.ensure_dirs()?;
    let mut log = Vec::new();

    if system_action == "restore" {
        copy_dir_file(
            &baseline.system_path().join("BASELINE.marker"),
            &live.system_path().join("BASELINE.marker"),
        )?;
        // Drop drift files in system except marker
        clear_dir_except(&live.system_path(), "BASELINE.marker")?;
        log.push("restored system volume (windows-lab)".into());
    }

    if persistent_action == "wipe" {
        clear_dir(&live.persistent_path())?;
        log.push("wiped persistent volume (windows-lab)".into());
    }

    if volatile_action == "wipe" {
        clear_dir(&live.volatile_path())?;
        log.push("wiped volatile volume (windows-lab)".into());
    }

    Ok(log)
}

fn copy_dir_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    std::fs::copy(from, to).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok(())
}

fn clear_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| Error::Other(anyhow::anyhow!(e)))? {
        let entry = entry.map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        } else {
            std::fs::remove_file(&path).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
    }
    Ok(())
}

fn clear_dir_except(dir: &Path, keep_name: &str) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| Error::Other(anyhow::anyhow!(e)))? {
        let entry = entry.map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        if entry.file_name() == keep_name {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        } else {
            std::fs::remove_file(&path).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
        }
    }
    Ok(())
}

/// Boot-hook install plan (scripts + registry marker). Real schtasks optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootHookInstall {
    pub installed_at: String,
    pub agent_bin: String,
    pub config_path: String,
    pub volume_backend: String,
    pub volume_root: String,
    pub baseline_root: String,
    pub hook_script: String,
    pub trigger: String,
}

pub fn render_boot_hook_cmd(install: &BootHookInstall) -> String {
    format!(
        "@echo off\r\nREM Yixue IDV agent boot hook\r\n\"{bin}\" boot --config \"{cfg}\" --backend {backend} --volume-root \"{vol}\" --baseline-root \"{base}\"\r\n",
        bin = install.agent_bin,
        cfg = install.config_path,
        backend = install.volume_backend,
        vol = install.volume_root,
        base = install.baseline_root,
    )
}

pub fn write_boot_hook(
    state_dir: &Path,
    install: &BootHookInstall,
) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(state_dir).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let script = state_dir.join("idv-boot-hook.cmd");
    let marker = state_dir.join("boot_hook_installed.json");
    let cmd = render_boot_hook_cmd(install);
    std::fs::write(&script, cmd).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let mut recorded = install.clone();
    recorded.hook_script = script.display().to_string();
    let bytes =
        serde_json::to_vec_pretty(&recorded).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    std::fs::write(&marker, bytes).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok((script, marker))
}

/// Helper retained for file backend callers that still pack images.
#[allow(dead_code)]
pub fn baseline_payload_len(img: &PackedImage) -> usize {
    img.payload.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_lab_restore_keeps_persistent() {
        let root = std::env::temp_dir().join(format!("idv-vol-{}", std::process::id()));
        let base = root.join("baseline");
        let live = root.join("live");
        let _ = std::fs::remove_dir_all(&root);
        write_lab_baseline(
            &base,
            &LabVolumeSnapshot {
                format_version: 1,
                system_marker: "SYS-A".into(),
                persistent_marker: "PER-A".into(),
                volatile_marker: "VOL-A".into(),
            },
        )
        .unwrap();
        let live_layout = VolumeLayout::windows_lab(&live);
        live_layout.ensure_dirs().unwrap();
        std::fs::write(live_layout.system_path().join("drift.txt"), b"x").unwrap();
        std::fs::write(live_layout.persistent_path().join("student.doc"), b"keep").unwrap();
        std::fs::write(live_layout.volatile_path().join("tmp.bin"), b"t").unwrap();
        // seed system marker so restore has something to overwrite from baseline
        let base_layout = VolumeLayout::windows_lab(&base);
        apply_windows_lab_restore(
            &live_layout,
            &base_layout,
            "restore",
            "keep",
            "wipe",
        )
        .unwrap();
        assert!(!live_layout.system_path().join("drift.txt").exists());
        assert_eq!(
            std::fs::read_to_string(live_layout.system_path().join("BASELINE.marker")).unwrap(),
            "SYS-A"
        );
        assert!(live_layout.persistent_path().join("student.doc").exists());
        assert!(!live_layout.volatile_path().join("tmp.bin").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
