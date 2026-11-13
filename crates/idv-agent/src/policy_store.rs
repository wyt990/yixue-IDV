//! Local DesiredState pending / applied revision store.

use std::fs;
use std::path::{Path, PathBuf};

use idv_common::endpoint_policy::{DesiredEndpointState, EndpointAppliedStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalPolicyStore {
    pub applied_revision: u64,
    pub pending_reboot: bool,
    pub pending: Option<DesiredEndpointState>,
    pub last_driver_status: String,
}

impl LocalPolicyStore {
    pub fn path(state_dir: &Path) -> PathBuf {
        state_dir.join("endpoint-policy.json")
    }

    pub fn load(state_dir: &Path) -> Self {
        let path = Self::path(state_dir);
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, state_dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(state_dir)?;
        let path = Self::path(state_dir);
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn applied_status(&self) -> EndpointAppliedStatus {
        EndpointAppliedStatus {
            applied_revision: self.applied_revision,
            pending_reboot: self.pending_reboot,
            driver_status: self.last_driver_status.clone(),
        }
    }

    /// Queue a new DesiredState if revision changed. Returns true if pending was updated.
    pub fn queue_if_newer(&mut self, desired: DesiredEndpointState) -> bool {
        if desired.revision == self.applied_revision && self.pending.is_none() {
            return false;
        }
        if self
            .pending
            .as_ref()
            .map(|p| p.revision == desired.revision)
            .unwrap_or(false)
        {
            return false;
        }
        if desired.revision == self.applied_revision {
            return false;
        }
        self.pending = Some(desired);
        self.pending_reboot = true;
        true
    }

    pub fn clear_pending_applied(&mut self, revision: u64, driver_status: String) {
        self.applied_revision = revision;
        self.pending = None;
        self.pending_reboot = false;
        self.last_driver_status = driver_status;
    }
}

pub fn default_state_dir() -> PathBuf {
    std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("YixueIDV")
}
