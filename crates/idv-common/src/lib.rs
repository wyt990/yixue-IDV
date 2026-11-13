//! Shared building blocks for 易学 IDV 机房管理系统.

pub mod chunking;
pub mod config;
pub mod delta;
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
