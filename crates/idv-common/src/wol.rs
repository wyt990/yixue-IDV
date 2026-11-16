//! Wake-on-LAN magic packet builder / sender.

use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

use crate::{Error, Result};

/// Build a standard WOL magic packet (6×FF + 16×MAC).
pub fn build_magic_packet(mac: &str) -> Result<Vec<u8>> {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if hex.len() != 12 {
        return Err(Error::Other(anyhow::anyhow!("bad mac for wol: {mac}")));
    }
    let mut mac_bytes = [0u8; 6];
    for i in 0..6 {
        mac_bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    }
    let mut packet = vec![0xFFu8; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes);
    }
    debug_assert_eq!(packet.len(), 102);
    Ok(packet)
}

/// Compute the directed broadcast address for a lab subnet (`ip` + `netmask`).
pub fn directed_broadcast(ip: &str, netmask: &str) -> Option<Ipv4Addr> {
    let ip: Ipv4Addr = ip.trim().parse().ok()?;
    let mask: Ipv4Addr = netmask.trim().parse().ok()?;
    let mask_u32 = u32::from(mask);
    if mask_u32 == 0 {
        return None;
    }
    Some(Ipv4Addr::from(u32::from(ip) | !mask_u32))
}

/// Send a magic packet to both limited and directed broadcast addresses,
/// on UDP/9 and UDP/7, a few times (UDP is lossy).
pub fn send_magic_packet_to(mac: &str, directed: Option<Ipv4Addr>) -> Result<String> {
    let packet = build_magic_packet(mac)?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    sock.set_broadcast(true)
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;

    let mut targets: Vec<(Ipv4Addr, u16)> =
        vec![(Ipv4Addr::BROADCAST, 9), (Ipv4Addr::BROADCAST, 7)];
    if let Some(bcast) = directed {
        targets.push((bcast, 9));
        targets.push((bcast, 7));
    }
    targets.sort_by_key(|(ip, port)| (u32::from(*ip), *port));
    targets.dedup();

    let mut sent = 0usize;
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..3 {
        for (ip, port) in &targets {
            match sock.send_to(&packet, (*ip, *port)) {
                Ok(_) => sent += 1,
                Err(err) => last_err = Some(err),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if sent == 0 {
        return Err(Error::Other(anyhow::anyhow!(
            "wol send failed for {mac}: {}",
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown error".into())
        )));
    }
    let target_text = targets
        .iter()
        .map(|(ip, port)| format!("{ip}:{port}"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "magic_packet_sent mac={mac} packets={sent} targets={target_text}"
    ))
}

/// Backward-compatible send (limited broadcast only).
pub fn send_magic_packet(mac: &str) -> Result<String> {
    send_magic_packet_to(mac, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_packet_shape() {
        let p = build_magic_packet("02:aa:bb:cc:dd:ee").unwrap();
        assert_eq!(p.len(), 102);
        assert!(p[..6].iter().all(|b| *b == 0xFF));
        assert_eq!(&p[6..12], &[0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        assert_eq!(&p[96..102], &[0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
    }

    #[test]
    fn directed_broadcast_of_lab() {
        assert_eq!(
            directed_broadcast("10.110.0.2", "255.255.255.0"),
            Some(Ipv4Addr::new(10, 110, 0, 255))
        );
        assert_eq!(
            directed_broadcast("10.110.0.2", "255.255.252.0"),
            Some(Ipv4Addr::new(10, 110, 3, 255))
        );
        assert_eq!(directed_broadcast("", "255.255.255.0"), None);
        assert_eq!(directed_broadcast("10.110.0.2", "0.0.0.0"), None);
    }
}
