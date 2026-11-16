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

/// Map ModePolicy.restore (+ optional mode name/scope) to freeze DesiredState semantics.
///
/// - `reboot` → frozen (teaching / exam style)
/// - `off` → thawed (office)
/// - `manual` → thawed by default; admin triggers protect via separate path
/// - `protect_scope = all_fixed` → protect every fixed NTFS volume (teaching)
/// - `protect_scope = system` (default) → protect only the system volume (exam/manual)
pub fn restore_from_mode_policy(
    restore: &str,
    _mode_name: &str,
    protect_scope: &str,
) -> RestoreCapability {
    let restore = restore.trim().to_ascii_lowercase();
    let volumes = if protect_scope.trim().eq_ignore_ascii_case("all_fixed") {
        vec!["all_fixed".into()]
    } else {
        vec!["system".into()]
    };
    match restore.as_str() {
        "reboot" => RestoreCapability {
            enabled: true,
            apply: ApplyWhen::NextReboot,
            protect_volumes: volumes,
            frozen: true,
        },
        "off" | "manual" => RestoreCapability {
            enabled: true,
            apply: ApplyWhen::NextReboot,
            protect_volumes: volumes,
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
        assert!(restore_from_mode_policy("reboot", "teaching", "all_fixed").frozen);
        assert!(!restore_from_mode_policy("off", "office", "system").frozen);
        assert!(!restore_from_mode_policy("manual", "manual", "system").frozen);
        assert!(!restore_from_mode_policy("unknown", "x", "system").enabled);
    }

    #[test]
    fn scope_maps_to_protect_volumes() {
        let teaching = restore_from_mode_policy("reboot", "teaching", "all_fixed");
        assert_eq!(teaching.protect_volumes, vec!["all_fixed".to_string()]);
        let exam = restore_from_mode_policy("reboot", "exam", "system");
        assert_eq!(exam.protect_volumes, vec!["system".to_string()]);
        // Unknown/empty scope falls back to system for safety.
        let custom = restore_from_mode_policy("reboot", "custom", "");
        assert_eq!(custom.protect_volumes, vec!["system".to_string()]);
    }

    #[test]
    fn revision_from_rfc3339() {
        let r = revision_from_timestamp("2026-11-13T03:00:00+00:00");
        assert!(r > 1_700_000_000);
    }
}
