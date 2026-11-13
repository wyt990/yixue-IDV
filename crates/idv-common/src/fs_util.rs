//! Filesystem helpers for colocated Edge/Center handoff.

use std::fs;
use std::path::Path;

/// Move `src` to `dst` preferring rename, then hardlink+remove, then copy+remove.
/// Returns which strategy succeeded.
pub fn relocate_file(src: &Path, dst: &Path) -> anyhow::Result<&'static str> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => return Ok("rename"),
        Err(err) => {
            tracing::debug!(error = %err, "rename failed; trying hardlink/copy");
        }
    }
    match fs::hard_link(src, dst) {
        Ok(()) => {
            let _ = fs::remove_file(src);
            return Ok("hardlink");
        }
        Err(err) => {
            tracing::debug!(error = %err, "hardlink failed; falling back to copy");
        }
    }
    fs::copy(src, dst)?;
    let _ = fs::remove_file(src);
    Ok("copy")
}

pub fn center_url_is_loopback(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.contains("://127.0.0.1")
        || lower.contains("://localhost")
        || lower.contains("://[::1]")
}
