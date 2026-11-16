//! Shared building blocks for 易学 IDV 机房管理系统.

pub mod chunking;
pub mod config;
pub mod delta;
pub mod endpoint_policy;
pub mod error;
pub mod fs_util;
pub mod image_format;
pub mod logging;
pub mod ntfs_exclude;
pub mod ntfs_used;
pub mod restore;
pub mod usb_policy;
pub mod volume;
pub mod wol;

pub use config::load_toml_file;
pub use error::{Error, Result};
pub use logging::init_tracing;

/// Test-only writable scratch directory.
///
/// Sandboxed/CI environments sometimes expose a non-writable `%TEMP%`; fall back
/// to `<cwd>/target/tmp` (always inside the workspace).
#[cfg(test)]
pub mod test_paths {
    use std::path::PathBuf;

    pub fn temp_dir(name: &str) -> PathBuf {
        let primary = std::env::temp_dir().join(name);
        if std::fs::create_dir_all(&primary).is_ok() {
            return primary;
        }
        let fallback = std::env::current_dir()
            .unwrap_or_default()
            .join("target")
            .join("tmp")
            .join(name);
        std::fs::create_dir_all(&fallback).expect("create fallback test dir");
        fallback
    }
}
