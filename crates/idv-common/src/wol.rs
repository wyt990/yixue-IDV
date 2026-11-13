//! Wake-on-LAN magic packet builder / sender.

use std::net::UdpSocket;

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

/// Best-effort broadcast to 255.255.255.255:9. Campus VLANs may need directed broadcast.
pub fn send_magic_packet(mac: &str) -> Result<String> {
    let packet = build_magic_packet(mac)?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    sock.set_broadcast(true)
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    sock.send_to(&packet, "255.255.255.255:9")
        .map_err(|e| Error::Other(anyhow::anyhow!(e)))?;
    Ok(format!(
        "magic_packet_sent mac={} bytes={}",
        mac,
        packet.len()
    ))
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
}
