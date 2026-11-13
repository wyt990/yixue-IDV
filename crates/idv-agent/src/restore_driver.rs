//! Volume-filter restore driver control surface.
//!
//! SysRestoreDriver enables/disables protection via `_qh_protect_state.data`
//! on each volume root (1 MiB, first byte 1/0). IOCTL only queries live status.
//! When the real path is unavailable, a ProgramData mock is used for policy tests.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const MOCK_STATE_REL: &str = "YixueIDV/restore-driver-state.json";
/// Must match `QH_PROTECT_STATE_FILE_SIZE` in SysRestoreDriver.
const PROTECT_STATE_SIZE: u64 = 1024 * 1024;
const PROTECT_STATE_NAME: &str = "_qh_protect_state.data";
/// CTL_CODE(0x8000, 0x801, METHOD_BUFFERED, FILE_ANY_ACCESS)
#[cfg(windows)]
const IOCTL_QH_QUERY_PROTECT_STATUS: u32 = 0x80002004;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverBackendKind {
    /// Write `_qh_protect_state.data` (+ optional IOCTL query).
    StateFile,
    /// Persist freeze flag to a local JSON file (dev / forced mock).
    MockFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverStatus {
    pub backend: DriverBackendKind,
    pub available: bool,
    /// Live protecting according to driver IOCTL (after reboot), if known.
    pub frozen: bool,
    /// Armed in state file for next boot (byte0==1), if known.
    #[serde(default)]
    pub armed: bool,
    pub detail: String,
}

pub trait RestoreDriver: Send + Sync {
    fn status(&self) -> anyhow::Result<DriverStatus>;
    fn protect_on(&self, volumes: &[String]) -> anyhow::Result<DriverStatus>;
    fn protect_off(&self) -> anyhow::Result<DriverStatus>;
}

/// Prefer real state-file path on Windows; mock when forced or non-Windows.
pub fn default_driver() -> Box<dyn RestoreDriver> {
    if std::env::var_os("IDV_RESTORE_MOCK").is_some() {
        return Box::new(MockFileDriver::default());
    }
    #[cfg(windows)]
    {
        Box::new(StateFileDriver)
    }
    #[cfg(not(windows))]
    {
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
            armed: frozen,
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

/// Real SysRestoreDriver control: arm/disarm via state files on volume roots.
pub struct StateFileDriver;

impl StateFileDriver {
    fn volume_roots(volumes: &[String]) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for v in volumes {
            let t = v.trim();
            if t.eq_ignore_ascii_case("system") || t.is_empty() {
                if let Some(r) = system_drive_root() {
                    roots.push(r);
                }
                continue;
            }
            // Accept "C", "C:", "C:\"
            let letter = t.chars().next().unwrap_or('C');
            if letter.is_ascii_alphabetic() {
                roots.push(PathBuf::from(format!("{}:\\", letter.to_ascii_uppercase())));
            }
        }
        if roots.is_empty() {
            if let Some(r) = system_drive_root() {
                roots.push(r);
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    fn state_path(root: &Path) -> PathBuf {
        root.join(PROTECT_STATE_NAME)
    }

    fn read_armed(root: &Path) -> Option<bool> {
        let path = Self::state_path(root);
        let mut buf = [0u8; 1];
        match fs::File::open(&path).and_then(|mut f| {
            use std::io::Read;
            f.read_exact(&mut buf)
        }) {
            Ok(()) => Some(buf[0] == 1),
            Err(_) => None,
        }
    }

    fn arm_volume(root: &Path) -> anyhow::Result<()> {
        let path = Self::state_path(root);
        if path.exists() {
            let _ = fs::set_permissions(
                &path,
                {
                    let mut p = fs::metadata(&path)?.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    {
                        p.set_readonly(false);
                    }
                    p
                },
            );
            #[cfg(windows)]
            clear_hidden_system(&path);
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        // 1 MiB zero pad (force non-resident NTFS attribute)
        let chunk = vec![0u8; 64 * 1024];
        let mut written = 0u64;
        while written < PROTECT_STATE_SIZE {
            let n = ((PROTECT_STATE_SIZE - written) as usize).min(chunk.len());
            file.write_all(&chunk[..n])?;
            written += n as u64;
        }
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&[1u8])?;
        file.flush()?;
        drop(file);
        #[cfg(windows)]
        set_hidden_system(&path);
        info!(path = %path.display(), "armed protect state file (reboot required)");
        Ok(())
    }

    fn disarm_volume(root: &Path) -> anyhow::Result<bool> {
        let path = Self::state_path(root);
        if !path.exists() {
            return Ok(false);
        }
        #[cfg(windows)]
        clear_hidden_system(&path);
        let mut file = OpenOptions::new().write(true).open(&path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&[0u8])?;
        file.flush()?;
        drop(file);
        #[cfg(windows)]
        set_hidden_system(&path);
        info!(path = %path.display(), "disarmed protect state file (reboot required)");
        Ok(true)
    }

    #[cfg(windows)]
    fn ioctl_live_protecting() -> Option<bool> {
        query_protect_status_ioctl()
    }

    #[cfg(not(windows))]
    fn ioctl_live_protecting() -> Option<bool> {
        None
    }
}

impl RestoreDriver for StateFileDriver {
    fn status(&self) -> anyhow::Result<DriverStatus> {
        let roots = Self::volume_roots(&["system".into()]);
        let armed = roots
            .iter()
            .filter_map(|r| Self::read_armed(r))
            .any(|a| a);
        let live = Self::ioctl_live_protecting();
        let frozen = live.unwrap_or(armed);
        let detail = format!(
            "state_file armed={} live_ioctl={} roots={}",
            armed,
            live.map(|b| b.to_string()).unwrap_or_else(|| "n/a".into()),
            roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        Ok(DriverStatus {
            backend: DriverBackendKind::StateFile,
            available: true,
            frozen,
            armed,
            detail,
        })
    }

    fn protect_on(&self, volumes: &[String]) -> anyhow::Result<DriverStatus> {
        let roots = Self::volume_roots(volumes);
        if roots.is_empty() {
            anyhow::bail!("no volume roots resolved for {volumes:?}");
        }
        for root in &roots {
            Self::arm_volume(root)?;
        }
        self.status()
    }

    fn protect_off(&self) -> anyhow::Result<DriverStatus> {
        // Disarm known system root + scan common fixed letters that have the file.
        let mut roots = Self::volume_roots(&["system".into()]);
        for letter in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if Self::state_path(&root).exists() {
                roots.push(root);
            }
        }
        roots.sort();
        roots.dedup();
        let mut any = false;
        for root in &roots {
            if Self::disarm_volume(root)? {
                any = true;
            }
        }
        if !any {
            warn!("no protect state files found to disarm");
        }
        self.status()
    }
}

fn system_drive_root() -> Option<PathBuf> {
    std::env::var_os("SystemDrive")
        .map(|d| {
            let s = d.to_string_lossy();
            if s.ends_with(':') {
                PathBuf::from(format!("{}\\", s))
            } else if s.ends_with(":\\") || s.ends_with(":/") {
                PathBuf::from(s.as_ref())
            } else {
                PathBuf::from(format!("{}:\\", s.trim_end_matches('\\')))
            }
        })
        .or_else(|| Some(PathBuf::from("C:\\")))
}

#[cfg(windows)]
fn clear_hidden_system(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attr: u32) -> i32;
    }
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_NORMAL);
    }
}

#[cfg(windows)]
fn set_hidden_system(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attr: u32) -> i32;
    }
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(
            wide.as_ptr(),
            FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM,
        );
    }
}

#[cfg(windows)]
fn query_protect_status_ioctl() -> Option<bool> {
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
        fn DeviceIoControl(
            h: *mut core::ffi::c_void,
            code: u32,
            in_buf: *mut core::ffi::c_void,
            in_len: u32,
            out_buf: *mut core::ffi::c_void,
            out_len: u32,
            returned: *mut u32,
            overlapped: *mut core::ffi::c_void,
        ) -> i32;
    }
    const GENERIC_READ: u32 = 0x8000_0000;
    const OPEN_EXISTING: u32 = 3;
    const INVALID: isize = -1;
    let path: Vec<u16> = std::ffi::OsStr::new(r"\\.\QHEngineControlDevice")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let h = CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            0,
            ptr::null_mut(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if h as isize == INVALID {
            return None;
        }
        let mut result: u8 = 0;
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            h,
            IOCTL_QH_QUERY_PROTECT_STATUS,
            ptr::null_mut(),
            0,
            &mut result as *mut u8 as *mut _,
            1,
            &mut returned,
            ptr::null_mut(),
        );
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(result != 0)
    }
}

/// Apply desired freeze flag using the given driver (used after reboot / --apply).
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

    #[test]
    fn volume_roots_system() {
        let roots = StateFileDriver::volume_roots(&["system".into()]);
        assert!(!roots.is_empty());
    }
}
