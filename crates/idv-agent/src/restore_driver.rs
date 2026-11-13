//! Volume-filter restore driver control surface.
//!
//! Real IOCTL path targets QHEngineControlDevice (SysRestoreDriver). When the
//! device is missing, a file-backed mock under ProgramData is used so policy
//! loops can be tested without a signed driver.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const MOCK_STATE_REL: &str = "YixueIDV/restore-driver-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverBackendKind {
    /// Talk to `\\.\QHEngineControlDevice` when present.
    Ioctl,
    /// Persist freeze flag to a local JSON file (dev / no driver).
    MockFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverStatus {
    pub backend: DriverBackendKind,
    pub available: bool,
    pub frozen: bool,
    pub detail: String,
}

pub trait RestoreDriver: Send + Sync {
    fn status(&self) -> anyhow::Result<DriverStatus>;
    fn protect_on(&self, volumes: &[String]) -> anyhow::Result<DriverStatus>;
    fn protect_off(&self) -> anyhow::Result<DriverStatus>;
}

/// Prefer real device; fall back to mock file store.
pub fn default_driver() -> Box<dyn RestoreDriver> {
    if QhIoctlDriver::device_present() {
        Box::new(QhIoctlDriver)
    } else {
        Box::new(MockFileDriver::default())
    }
}

#[derive(Debug, Default)]
pub struct MockFileDriver {
    path: Option<PathBuf>,
}

impl MockFileDriver {
    #[allow(dead_code)]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn state_path(&self) -> PathBuf {
        if let Some(p) = &self.path {
            return p.clone();
        }
        std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(MOCK_STATE_REL)
    }

    fn load(&self) -> (bool, String) {
        let path = self.state_path();
        match fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(v) => (
                    v.get("frozen").and_then(|x| x.as_bool()).unwrap_or(false),
                    format!("mock file {}", path.display()),
                ),
                Err(_) => (false, format!("mock corrupt {}", path.display())),
            },
            Err(_) => (false, format!("mock absent {}", path.display())),
        }
    }

    fn save(&self, frozen: bool, volumes: &[String]) -> anyhow::Result<()> {
        let path = self.state_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::json!({
            "frozen": frozen,
            "volumes": volumes,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        fs::write(&path, serde_json::to_vec_pretty(&body)?)?;
        Ok(())
    }
}

impl RestoreDriver for MockFileDriver {
    fn status(&self) -> anyhow::Result<DriverStatus> {
        let (frozen, detail) = self.load();
        Ok(DriverStatus {
            backend: DriverBackendKind::MockFile,
            available: true,
            frozen,
            detail,
        })
    }

    fn protect_on(&self, volumes: &[String]) -> anyhow::Result<DriverStatus> {
        self.save(true, volumes)?;
        info!(volumes = ?volumes, "mock restore-driver: protect_on");
        self.status()
    }

    fn protect_off(&self) -> anyhow::Result<DriverStatus> {
        self.save(false, &[])?;
        info!("mock restore-driver: protect_off");
        self.status()
    }
}

/// Placeholder IOCTL backend — probes device node; protect calls fail clearly until wired.
pub struct QhIoctlDriver;

impl QhIoctlDriver {
    pub fn device_present() -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use std::ptr;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn CreateFileW(
                    name: *const u16,
                    access: u32,
                    share: u32,
                    sec: *mut core::ffi::c_void,
                    disposition: u32,
                    flags: u32,
                    template: *mut core::ffi::c_void,
                ) -> *mut core::ffi::c_void;
                fn CloseHandle(h: *mut core::ffi::c_void) -> i32;
            }
            const GENERIC_READ: u32 = 0x80000000;
            const OPEN_EXISTING: u32 = 3;
            const FILE_SHARE_READ: u32 = 1;
            const FILE_SHARE_WRITE: u32 = 2;
            const INVALID: isize = -1;
            let path: Vec<u16> = std::ffi::OsStr::new(r"\\.\QHEngineControlDevice")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                let h = CreateFileW(
                    path.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    ptr::null_mut(),
                );
                if h as isize == INVALID {
                    return false;
                }
                CloseHandle(h);
                true
            }
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

impl RestoreDriver for QhIoctlDriver {
    fn status(&self) -> anyhow::Result<DriverStatus> {
        let present = Self::device_present();
        Ok(DriverStatus {
            backend: DriverBackendKind::Ioctl,
            available: present,
            // Full QUERY_PROTECT_STATUS IOCTL wiring comes with signed .sys deploy.
            frozen: false,
            detail: if present {
                "QHEngineControlDevice present (IOCTL protect status TBD)".into()
            } else {
                "QHEngineControlDevice missing".into()
            },
        })
    }

    fn protect_on(&self, volumes: &[String]) -> anyhow::Result<DriverStatus> {
        if !Self::device_present() {
            anyhow::bail!("QHEngineControlDevice not available");
        }
        warn!(
            volumes = ?volumes,
            "IOCTL protect_on not fully wired; install SysRestoreDriver UI path first"
        );
        anyhow::bail!("IOCTL protect_on not implemented — use mock backend or QHEngineUI for now")
    }

    fn protect_off(&self) -> anyhow::Result<DriverStatus> {
        if !Self::device_present() {
            anyhow::bail!("QHEngineControlDevice not available");
        }
        anyhow::bail!("IOCTL protect_off not implemented — use mock backend or QHEngineUI for now")
    }
}

/// Apply desired freeze flag using the given driver (used after reboot).
pub fn apply_frozen_state(
    driver: &dyn RestoreDriver,
    frozen: bool,
    volumes: &[String],
) -> anyhow::Result<DriverStatus> {
    if frozen {
        driver.protect_on(volumes)
    } else {
        driver.protect_off()
    }
}

#[allow(dead_code)]
pub fn mock_state_path_for_tests(dir: &Path) -> PathBuf {
    dir.join("restore-driver-state.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_protect_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "idv-restore-driver-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let d = MockFileDriver::with_path(mock_state_path_for_tests(&dir));
        assert!(!d.status().unwrap().frozen);
        d.protect_on(&["system".into()]).unwrap();
        assert!(d.status().unwrap().frozen);
        d.protect_off().unwrap();
        assert!(!d.status().unwrap().frozen);
        let _ = fs::remove_dir_all(&dir);
    }
}
