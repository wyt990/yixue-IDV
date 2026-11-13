//! Endpoint DesiredState shared by Center and Agent (restore + future capabilities).

use serde::{Deserialize, Serialize};

/// When a capability change should take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyWhen {
    Immediate,
    NextReboot,
}

impl Default for ApplyWhen {
    fn default() -> Self {
        Self::NextReboot
    }
}

/// Freeze / volume-filter restore capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreCapability {
    /// Master switch for the restore engine on this endpoint.
    pub enabled: bool,
    #[serde(default)]
    pub apply: ApplyWhen,
    /// Logical volume roles to protect (e.g. `system`).
    #[serde(default)]
    pub protect_volumes: Vec<String>,
    /// Desired freeze state: true = protection on (reboot-to-restore).
    pub frozen: bool,
}

impl RestoreCapability {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            apply: ApplyWhen::NextReboot,
            protect_volumes: vec!["system".into()],
            frozen: false,
        }
    }
}

/// Center → Agent desired configuration document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredEndpointState {
    pub revision: u64,
    pub restore: RestoreCapability,
    /// Capability ids this revision may mention (placeholder for future modules).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl DesiredEndpointState {
    pub fn with_restore(revision: u64, restore: RestoreCapability) -> Self {
        Self {
            revision,
            restore,
            capabilities: vec!["restore".into()],
        }
    }
}

/// Agent → Center status fields (also embeddable in heartbeat body).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointAppliedStatus {
    #[serde(default)]
    pub applied_revision: u64,
    #[serde(default)]
    pub pending_reboot: bool,
    #[serde(default)]
    pub driver_status: String,
}

/// Map ModePolicy.restore (+ optional mode name) to freeze DesiredState semantics.
///
/// - `reboot` → frozen (teaching / exam style)
/// - `off` → thawed (office)
/// - `manual` → thawed by default; admin triggers protect via separate path
pub fn restore_from_mode_policy(restore: &str, _mode_name: &str) -> RestoreCapability {
    let restore = restore.trim().to_ascii_lowercase();
    match restore.as_str() {
        "reboot" => RestoreCapability {
            enabled: true,
            apply: ApplyWhen::NextReboot,
            protect_volumes: vec!["system".into()],
            frozen: true,
        },
        "off" | "manual" => RestoreCapability {
            enabled: true,
            apply: ApplyWhen::NextReboot,
            protect_volumes: vec!["system".into()],
            frozen: false,
        },
        _ => RestoreCapability::disabled(),
    }
}

/// Build revision from RFC3339 / chrono-like timestamp string (binding updated_at).
pub fn revision_from_timestamp(ts: &str) -> u64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return dt.timestamp().max(0) as u64;
    }
    // Fallback: stable hash-ish from bytes
    let mut n = 0u64;
    for (i, b) in ts.as_bytes().iter().take(16).enumerate() {
        n ^= (*b as u64).wrapping_shl((i as u32 % 8) * 8);
    }
    n
}

/// Planned AgentJob kinds (not implemented this slice).
pub mod job_kinds {
    pub const OPEN_URL: &str = "open_url";
    pub const RUN_PROGRAM: &str = "run_program";
    pub const DEPLOY_FILES: &str = "deploy_files";
    pub const COLLECT_FILES: &str = "collect_files";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_desired_state() {
        let s = DesiredEndpointState::with_restore(
            42,
            RestoreCapability {
                enabled: true,
                apply: ApplyWhen::NextReboot,
                protect_volumes: vec!["system".into()],
                frozen: true,
            },
        );
        let json = serde_json::to_string(&s).unwrap();
        let back: DesiredEndpointState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert!(json.contains("next_reboot"));
        assert!(json.contains("\"frozen\":true"));
    }

    #[test]
    fn mode_mapping() {
        assert!(restore_from_mode_policy("reboot", "teaching").frozen);
        assert!(!restore_from_mode_policy("off", "office").frozen);
        assert!(!restore_from_mode_policy("manual", "manual").frozen);
        assert!(!restore_from_mode_policy("unknown", "x").enabled);
    }

    #[test]
    fn revision_from_rfc3339() {
        let r = revision_from_timestamp("2026-11-13T03:00:00+00:00");
        assert!(r > 1_700_000_000);
    }
}
