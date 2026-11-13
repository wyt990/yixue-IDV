//! Simulated USB restriction policy for lab / agent acceptance.
//! Real Windows device-class disable is out of scope; this persists intent locally.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsbPolicyState {
    pub restricted: bool,
    pub applied_at: String,
    pub detail: String,
}

pub fn apply_usb_policy(path: &Path, restricted: bool) -> Result<UsbPolicyState> {
    let state = UsbPolicyState {
        restricted,
        applied_at: chrono::Utc::now().to_rfc3339(),
        detail: if restricted {
            "usb storage class blocked (simulated)".into()
        } else {
            "usb unrestricted".into()
        },
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    let json = serde_json::to_vec_pretty(&state).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    std::fs::write(path, json).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok(state)
}

pub fn read_usb_policy(path: &Path) -> Result<Option<UsbPolicyState>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    let state: UsbPolicyState =
        serde_json::from_slice(&bytes).map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("idv-usb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("usb_policy.json");
        let applied = apply_usb_policy(&path, true).unwrap();
        assert!(applied.restricted);
        let got = read_usb_policy(&path).unwrap().unwrap();
        assert_eq!(got.restricted, true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
