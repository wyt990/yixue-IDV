//! Network identity helpers for batch provisioning.
//!
//! The agent must not require a per-machine `mac` setting. We resolve the NIC
//! that actually routes to Center (via PowerShell `Get-NetIPAddress` on the
//! local address of that route) and use its MAC / alias for heartbeat, static
//! IP configuration and DHCP renewal.

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetInterface {
    /// Windows adapter name (usable with netsh / ipconfig).
    pub alias: String,
    pub mac: String,
    pub ip: String,
}

/// Best-effort local IPv4 used to reach `center_url` (no packets are sent).
pub fn detect_local_ip(center_url: &str) -> String {
    let host = center_url
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(['/', ':'])
        .next()
        .unwrap_or("8.8.8.8");
    let addr = format!("{host}:80");
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect(&addr)?;
            s.local_addr()
        })
        .ok()
        .map(|a| a.ip().to_string())
        .filter(|ip| ip != "0.0.0.0")
        .unwrap_or_default()
}

/// Resolve the NIC that routes to Center. Returns `None` on non-Windows or when
/// PowerShell/adapter lookup fails; callers should report a clear error.
pub fn interface_for_center(center_url: &str) -> Option<NetInterface> {
    #[cfg(windows)]
    {
        let local_ip = detect_local_ip(center_url);
        if let Some(iface) = query_interface(&local_ip) {
            return Some(iface);
        }
        // Fallback: first physical adapter that is up.
        query_first_up_interface()
    }
    #[cfg(not(windows))]
    {
        let _ = center_url;
        None
    }
}

/// Normalize a MAC to `AA:BB:CC:DD:EE:FF`; returns empty string when invalid.
pub fn normalize_mac(input: &str) -> String {
    let hex: String = input
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return String::new();
    }
    hex.as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap_or("00"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Release + renew DHCP on the given adapter (Edge reservation takes effect).
pub fn renew_dhcp(alias: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let alias = alias.trim();
        if alias.is_empty() {
            anyhow::bail!("no adapter alias for DHCP renew");
        }
        let _ = Command::new("ipconfig").args(["/release", alias]).status();
        let st = Command::new("ipconfig")
            .args(["/renew", alias])
            .status()
            .map_err(|e| anyhow::anyhow!("ipconfig /renew failed to start: {e}"))?;
        if !st.success() {
            anyhow::bail!("ipconfig /renew failed: {st}");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = alias;
        anyhow::bail!("DHCP renew only supported on Windows")
    }
}

#[cfg(windows)]
fn query_interface(local_ip: &str) -> Option<NetInterface> {
    let ip = local_ip.trim();
    if ip.is_empty() {
        return None;
    }
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $a = Get-NetIPAddress -IPAddress '{ip}' -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1; \
         if (-not $a) {{ exit 4 }}; \
         $n = Get-NetAdapter -InterfaceIndex $a.InterfaceIndex -ErrorAction SilentlyContinue; \
         if (-not $n) {{ exit 5 }}; \
         Write-Output ('{{0}}|{{1}}|{{2}}' -f $n.Name, $n.MacAddress, '{ip}')"
    );
    let out = run_powershell(&script)?;
    parse_interface_line(&out)
}

#[cfg(windows)]
fn query_first_up_interface() -> Option<NetInterface> {
    let script = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $n = Get-NetAdapter | Where-Object { $_.Status -eq 'Up' -and $_.MacAddress -match '^([0-9A-Fa-f]{2}-){{5}}[0-9A-Fa-f]{{2}}$' } | Select-Object -First 1; \
         if (-not $n) {{ exit 3 }}; \
         $a = Get-NetIPAddress -InterfaceIndex $n.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1; \
         Write-Output ('{{0}}|{{1}}|{{2}}' -f $n.Name, $n.MacAddress, $(if ($a) {{ $a.IPAddress }} else {{ '' }}))";
    let out = run_powershell(script)?;
    parse_interface_line(&out)
}

#[cfg(windows)]
fn run_powershell(script: &str) -> Option<String> {
    let out = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Parse `alias|mac|ip` (unit-testable without spawning PowerShell).
pub fn parse_interface_line(line: &str) -> Option<NetInterface> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 2 {
        return None;
    }
    let alias = parts[0].trim().to_string();
    let mac = normalize_mac(parts[1].trim());
    if alias.is_empty() || mac.is_empty() {
        return None;
    }
    Some(NetInterface {
        alias,
        mac,
        ip: parts
            .get(2)
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mac() {
        assert_eq!(normalize_mac("02-11-22-33-44-55"), "02:11:22:33:44:55");
        assert_eq!(normalize_mac("aabbccddeeff"), "AA:BB:CC:DD:EE:FF");
        assert_eq!(normalize_mac("bad"), "");
    }

    #[test]
    fn parses_interface_output() {
        let iface = parse_interface_line("以太网|02-11-22-33-44-55|10.110.0.31").unwrap();
        assert_eq!(iface.mac, "02:11:22:33:44:55");
        assert_eq!(iface.ip, "10.110.0.31");
        assert!(parse_interface_line("bad-line").is_none());
    }
}
