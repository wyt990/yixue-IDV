//! Partition-role restore helpers (stage 6 / phase-2).

use crate::image_format::{PartitionRole, PackedImage};

/// Apply mode actions onto a local disk buffer using baseline image payload.
///
/// - system_action `restore`: copy system role bytes from baseline
/// - persistent_action `keep` | `wipe`
/// - volatile_action `keep` | `wipe`
pub fn apply_partition_restore(
    disk: &mut [u8],
    baseline: &PackedImage,
    system_action: &str,
    persistent_action: &str,
    volatile_action: &str,
) -> anyhow::Result<Vec<String>> {
    let logical = crate::ntfs_used::expand_payload_to_logical(&baseline.header, &baseline.payload)?;
    let mut log = Vec::new();
    if disk.len() < logical.len() {
        anyhow::bail!(
            "disk shorter than baseline: disk={} baseline={}",
            disk.len(),
            logical.len()
        );
    }

    for part in &baseline.header.partitions {
        match part.role.as_str() {
            "system" if system_action == "restore" => {
                copy_role(disk, &logical, part)?;
                log.push(format!("restored system partition {}", part.name));
            }
            "persistent" if persistent_action == "wipe" => {
                wipe_role(disk, part)?;
                log.push(format!("wiped persistent partition {}", part.name));
            }
            "volatile" if volatile_action == "wipe" => {
                wipe_role(disk, part)?;
                log.push(format!("wiped volatile partition {}", part.name));
            }
            _ => {}
        }
    }
    Ok(log)
}

fn copy_role(disk: &mut [u8], baseline: &[u8], part: &PartitionRole) -> anyhow::Result<()> {
    let start = part.offset as usize;
    let end = start + part.length as usize;
    if end > baseline.len() || end > disk.len() {
        anyhow::bail!("partition {} out of range", part.name);
    }
    disk[start..end].copy_from_slice(&baseline[start..end]);
    Ok(())
}

fn wipe_role(disk: &mut [u8], part: &PartitionRole) -> anyhow::Result<()> {
    let start = part.offset as usize;
    let end = start + part.length as usize;
    if end > disk.len() {
        anyhow::bail!("partition {} out of range", part.name);
    }
    disk[start..end].fill(0);
    Ok(())
}

/// Whether agent should restore on this boot/event.
pub fn should_restore_now(restore: &str, trigger: &str) -> bool {
    match (restore, trigger) {
        ("off", _) => false,
        ("reboot", "boot" | "heartbeat") => true,
        ("manual", "manual") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_format::{ImageHeader, default_partitions, hex_sha256};

    #[test]
    fn restore_system_keeps_persistent() {
        let mut payload = vec![1u8; 90];
        payload[60..].fill(2);
        let baseline = PackedImage {
            header: ImageHeader {
                format_version: 1,
                os_family: "windows".into(),
                boot_mode: "uefi".into(),
                partitions: default_partitions(90),
                payload_sha256: hex_sha256(&payload),
                payload_size: 90,
                encoding: "raw".into(),
                logical_size: 0,
            },
            payload: payload.clone(),
        };
        let mut disk = vec![9u8; 90];
        disk[60..].fill(7);
        apply_partition_restore(&mut disk, &baseline, "restore", "keep", "wipe").unwrap();
        assert_eq!(&disk[..60], &payload[..60]);
        assert_eq!(&disk[60..], &[7u8; 30]);
    }

    #[test]
    fn office_mode_skips() {
        assert!(!should_restore_now("off", "boot"));
        assert!(should_restore_now("reboot", "boot"));
        assert!(should_restore_now("manual", "manual"));
        assert!(!should_restore_now("manual", "boot"));
    }
}
