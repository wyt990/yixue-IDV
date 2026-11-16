//! Edge DHCP IP reservations pulled from Center (`/api/v1/edge/ip-bindings`).
//!
//! Batch-deployed clients get their Center-assigned IPv4 from the Edge DHCP
//! server. Bindings are cached on disk so DHCP keeps working while Center is
//! unreachable.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BindingsDoc {
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub mac: String,
    pub ip: String,
    #[serde(default)]
    pub lab_id: String,
}

#[derive(Debug, Default)]
struct Inner {
    by_mac: HashMap<[u8; 6], Ipv4Addr>,
    by_ip: HashSet<u32>,
}

/// Shared, cheaply-clonable MAC -> reserved IPv4 map.
#[derive(Debug, Default, Clone)]
pub struct IpBindings {
    inner: Arc<RwLock<Inner>>,
}

impl IpBindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a previously cached bindings doc; missing/corrupt file is ignored.
    pub async fn load(path: &Path) -> Self {
        let bindings = Self::new();
        if let Ok(bytes) = tokio::fs::read(path).await
            && let Ok(doc) = serde_json::from_slice::<BindingsDoc>(&bytes)
        {
            bindings.replace(&doc).await;
        }
        bindings
    }

    pub async fn replace(&self, doc: &BindingsDoc) -> usize {
        let mut inner = self.inner.write().await;
        inner.by_mac.clear();
        inner.by_ip.clear();
        for b in &doc.bindings {
            let Some(mac) = parse_mac(&b.mac) else {
                continue;
            };
            let Ok(ip) = b.ip.trim().parse::<Ipv4Addr>() else {
                continue;
            };
            inner.by_mac.insert(mac, ip);
            inner.by_ip.insert(u32::from(ip));
        }
        inner.by_mac.len()
    }

    pub async fn get(&self, mac: &[u8; 6]) -> Option<Ipv4Addr> {
        self.inner.read().await.by_mac.get(mac).copied()
    }

    pub async fn reserved_ips(&self) -> HashSet<u32> {
        self.inner.read().await.by_ip.clone()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.by_mac.is_empty()
    }

    /// Snapshot for persistence.
    pub async fn to_doc(&self) -> BindingsDoc {
        let inner = self.inner.read().await;
        let mut bindings: Vec<Binding> = inner
            .by_mac
            .iter()
            .map(|(mac, ip)| Binding {
                mac: format_mac(mac),
                ip: ip.to_string(),
                lab_id: String::new(),
            })
            .collect();
        bindings.sort_by(|a, b| a.mac.cmp(&b.mac));
        BindingsDoc { bindings }
    }

    /// Persist atomically (tmp + rename); errors are returned for logging.
    pub async fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let doc = self.to_doc().await;
        let bytes =
            serde_json::to_vec_pretty(&doc).map_err(|e| std::io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

/// Accept `AA:BB:CC:DD:EE:FF`, `aa-bb-...`, `aabbccddeeff`.
pub fn parse_mac(input: &str) -> Option<[u8; 6]> {
    let hex: String = input
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return None;
    }
    let mut mac = [0u8; 6];
    for i in 0..6 {
        mac[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(mac)
}

pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// True when `ip` is inside the DHCP server subnet (`server` + `mask`).
pub fn ip_in_subnet(ip: Ipv4Addr, server: Ipv4Addr, mask: Ipv4Addr) -> bool {
    (u32::from(ip) & u32::from(mask)) == (u32::from(server) & u32::from(mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mac_variants() {
        let want = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        assert_eq!(parse_mac("AA:BB:CC:DD:EE:FF"), Some(want));
        assert_eq!(parse_mac("aa-bb-cc-dd-ee-ff"), Some(want));
        assert_eq!(parse_mac("aabbccddeeff"), Some(want));
        assert_eq!(parse_mac("not-a-mac"), None);
    }

    #[test]
    fn formats_mac() {
        assert_eq!(
            format_mac(&[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]),
            "02:11:22:33:44:55"
        );
    }

    #[test]
    fn subnet_check() {
        let server = Ipv4Addr::new(10, 110, 0, 1);
        let mask = Ipv4Addr::new(255, 255, 255, 0);
        assert!(ip_in_subnet(Ipv4Addr::new(10, 110, 0, 50), server, mask));
        assert!(!ip_in_subnet(Ipv4Addr::new(10, 111, 0, 50), server, mask));
    }

    #[tokio::test]
    async fn replace_and_lookup() {
        let bindings = IpBindings::new();
        let doc = BindingsDoc {
            bindings: vec![Binding {
                mac: "AA:BB:CC:DD:EE:01".into(),
                ip: "10.110.0.31".into(),
                lab_id: "lab".into(),
            }],
        };
        assert_eq!(bindings.replace(&doc).await, 1);
        let mac = parse_mac("aa:bb:cc:dd:ee:01").unwrap();
        assert_eq!(
            bindings.get(&mac).await,
            Some(Ipv4Addr::new(10, 110, 0, 31))
        );
        assert!(
            bindings
                .reserved_ips()
                .await
                .contains(&u32::from(Ipv4Addr::new(10, 110, 0, 31)))
        );
        assert!(
            !bindings
                .reserved_ips()
                .await
                .contains(&u32::from(Ipv4Addr::new(10, 110, 0, 32)))
        );
    }
}
