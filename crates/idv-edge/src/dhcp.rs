//! Minimal DHCP / ProxyDHCP responder for PXE boot (stage 3).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Clone)]
pub struct DhcpConfig {
    pub mode: String, // full | proxy | off
    pub bind: String,
    pub next_server: Ipv4Addr,
    pub boot_file: String,
    pub subnet_mask: Ipv4Addr,
    pub router: Ipv4Addr,
    pub offer_start: Ipv4Addr,
    pub offer_end: Ipv4Addr,
    pub lease_secs: u32,
    pub dns: Option<Ipv4Addr>,
}

#[derive(Clone)]
pub struct DhcpService {
    cfg: DhcpConfig,
    leases: Arc<Mutex<HashMap<[u8; 6], Ipv4Addr>>>,
    next_ip: Arc<AtomicU32>,
}

impl DhcpService {
    pub fn new(cfg: DhcpConfig) -> Self {
        let start = u32::from(cfg.offer_start);
        Self {
            cfg,
            leases: Arc::new(Mutex::new(HashMap::new())),
            next_ip: Arc::new(AtomicU32::new(start)),
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        if self.cfg.mode == "off" {
            return Ok(());
        }
        // Bind specific reply addr first (Windows: before wildcard) so Offer/ACK
        // egress as next_server:67 — PXE ignores replies from ephemeral ports.
        let reply_addr = SocketAddr::from((self.cfg.next_server, 67));
        let reply_sock = match bind_udp(&reply_addr, true) {
            Ok(s) => {
                info!(%reply_addr, "DHCP reply socket bound (src ip+port 67)");
                Some(s)
            }
            Err(err) => {
                warn!(
                    %reply_addr,
                    error = %err,
                    "DHCP reply socket bind failed; falling back to recv socket"
                );
                None
            }
        };
        // Recv on 0.0.0.0:67 to hear limited-broadcast Discover (Windows).
        let sock = bind_udp(&self.cfg.bind.parse()?, true)?;
        info!(
            bind = %self.cfg.bind,
            reply_from = %self.cfg.next_server,
            mode = %self.cfg.mode,
            "DHCP/PXE service listening"
        );
        // Also answer ProxyDHCP on 4011 for firmware that still does extended discovery.
        let proxy = self.clone();
        let next = self.cfg.next_server;
        tokio::spawn(async move {
            if let Err(err) = proxy.run_proxy4011(next).await {
                warn!(error = %err, "ProxyDHCP :4011 stopped");
            }
        });

        let mut buf = vec![0u8; 1500];
        loop {
            let (n, peer) = match sock.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(err) => {
                    warn!(error = %err, "dhcp recv error");
                    continue;
                }
            };
            let reply = reply_sock.as_ref().unwrap_or(&sock);
            if let Err(err) = self.handle_packet(reply, &buf[..n], peer, false).await {
                warn!(error = %err, bytes = n, %peer, "dhcp packet ignored");
            }
        }
    }

    async fn run_proxy4011(self, next_server: Ipv4Addr) -> anyhow::Result<()> {
        let reply_addr = SocketAddr::from((next_server, 4011));
        let sock = match bind_udp(&reply_addr, true) {
            Ok(s) => s,
            Err(_) => bind_udp(&"0.0.0.0:4011".parse()?, true)?,
        };
        info!(bind = %sock.local_addr().unwrap_or(reply_addr), "ProxyDHCP listening");
        let mut buf = vec![0u8; 1500];
        loop {
            let (n, peer) = match sock.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(err) => {
                    warn!(error = %err, "proxy4011 recv error");
                    continue;
                }
            };
            if let Err(err) = self.handle_packet(&sock, &buf[..n], peer, true).await {
                warn!(error = %err, bytes = n, %peer, "proxy4011 packet ignored");
            }
        }
    }

    async fn handle_packet(
        &self,
        reply_sock: &UdpSocket,
        packet: &[u8],
        peer: SocketAddr,
        proxy4011: bool,
    ) -> anyhow::Result<()> {
        if packet.len() < 240 {
            anyhow::bail!("packet too short");
        }
        if packet[0] != 1 {
            anyhow::bail!("not a bootrequest");
        }
        if packet[236..240] != [99, 130, 83, 99] {
            anyhow::bail!("missing DHCP magic");
        }

        let xid = &packet[4..8];
        let flags = u16::from_be_bytes([packet[10], packet[11]]);
        let mut chaddr = [0u8; 6];
        chaddr.copy_from_slice(&packet[28..34]);
        let msg_type = dhcp_option(packet, 53).and_then(|v| v.first().copied());
        let Some(msg_type) = msg_type else {
            anyhow::bail!("no DHCP message type");
        };
        let req_ip = dhcp_option(packet, 50).and_then(opt_ipv4);
        let server_id = dhcp_option(packet, 54).and_then(opt_ipv4);
        let vendor = dhcp_option(packet, 60)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("-");
        let arch = dhcp_option(packet, 93).and_then(|b| {
            if b.len() >= 2 {
                Some(u16::from_be_bytes([b[0], b[1]]))
            } else {
                None
            }
        });
        info!(
            bytes = packet.len(),
            %peer,
            mac = %format_mac(&chaddr),
            msg_type,
            flags = format!("0x{flags:04X}"),
            req_ip = %req_ip
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            server_id = %server_id
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            vendor,
            arch = arch.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            "dhcp packet received"
        );

        if let Some(sid) = server_id {
            if sid != self.cfg.next_server {
                warn!(
                    mac = %format_mac(&chaddr),
                    server_id = %sid,
                    expected = %self.cfg.next_server,
                    "client Request targets another DHCP server; still ACKing ours"
                );
            }
        }

        let as_proxy = proxy4011 || self.cfg.mode == "proxy";
        // 1=Discover 3=Request; some PXE stacks also retry Discover (1) after Offer.
        let (yiaddr, resp_type) = match (as_proxy, msg_type) {
            (true, 1 | 3) => (Ipv4Addr::UNSPECIFIED, if msg_type == 1 { 2 } else { 5 }),
            (false, 1) => (self.allocate(&chaddr).await?, 2),
            (false, 3) => {
                let mut leases = self.leases.lock().await;
                let ip = if let Some(ip) = leases.get(&chaddr).copied() {
                    ip
                } else if let Some(ip) = req_ip.filter(|ip| {
                    let n = u32::from(*ip);
                    n >= u32::from(self.cfg.offer_start) && n <= u32::from(self.cfg.offer_end)
                }) {
                    leases.insert(chaddr, ip);
                    ip
                } else {
                    drop(leases);
                    self.allocate(&chaddr).await?
                };
                (ip, 5)
            }
            // Decline / release / inform: ignore quietly at caller via error
            _ => anyhow::bail!("unsupported mode/type proxy={as_proxy}/{}", msg_type),
        };

        // Discover: force broadcast so firmware without IP can hear Offer.
        // Request/ACK: honor client flag; many UEFI stacks clear broadcast and expect unicast.
        let force_broadcast = msg_type == 1 || (flags & 0x8000) != 0;
        let boot_file = select_boot_file(&self.cfg.boot_file, arch);
        let reply = build_reply(
            packet,
            xid,
            &chaddr,
            yiaddr,
            self.cfg.next_server,
            boot_file,
            resp_type,
            self.cfg.subnet_mask,
            self.cfg.router,
            self.cfg.lease_secs,
            self.cfg.dns,
            as_proxy,
            force_broadcast,
        )?;

        let directed = SocketAddr::from((
            subnet_broadcast(self.cfg.next_server, self.cfg.subnet_mask),
            68,
        ));
        let limited = SocketAddr::from(([255, 255, 255, 255], 68));
        let unicast = SocketAddr::from((yiaddr, 68));

        let mut sent = 0usize;
        for dest in [directed, limited] {
            match reply_sock.send_to(&reply, dest).await {
                Ok(n) => {
                    sent += n;
                    info!(%dest, bytes = n, "dhcp reply egress");
                }
                Err(err) => warn!(%dest, error = %err, "dhcp reply send failed"),
            }
        }
        if !yiaddr.is_unspecified() {
            match reply_sock.send_to(&reply, unicast).await {
                Ok(n) => {
                    sent += n;
                    info!(%unicast, bytes = n, "dhcp reply unicast");
                }
                Err(err) => warn!(%unicast, error = %err, "dhcp unicast send failed"),
            }
        }
        if !peer.ip().is_unspecified()
            && !matches!(peer.ip(), std::net::IpAddr::V4(v4) if v4.is_unspecified() || v4.is_broadcast())
        {
            let dest = SocketAddr::from((peer.ip(), peer.port()));
            let _ = reply_sock.send_to(&reply, dest).await;
        }

        info!(
            mac = %format_mac(&chaddr),
            yiaddr = %yiaddr,
            dhcp_reply = resp_type,
            boot_file,
            force_broadcast,
            peer = %peer,
            dest = %directed,
            sent,
            "DHCP PXE reply sent"
        );
        Ok(())
    }

    async fn allocate(&self, mac: &[u8; 6]) -> anyhow::Result<Ipv4Addr> {
        let mut leases = self.leases.lock().await;
        if let Some(ip) = leases.get(mac) {
            return Ok(*ip);
        }
        let start = u32::from(self.cfg.offer_start);
        let end = u32::from(self.cfg.offer_end);
        for _ in start..=end {
            let candidate = self.next_ip.fetch_add(1, Ordering::Relaxed);
            let ip_u32 = if candidate > end { start } else { candidate };
            if candidate > end {
                self.next_ip.store(start + 1, Ordering::Relaxed);
            }
            let ip = Ipv4Addr::from(ip_u32);
            if !leases.values().any(|v| *v == ip) {
                leases.insert(*mac, ip);
                return Ok(ip);
            }
        }
        anyhow::bail!("DHCP pool exhausted")
    }
}

fn bind_udp(addr: &SocketAddr, broadcast: bool) -> anyhow::Result<UdpSocket> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    if broadcast {
        socket.set_broadcast(true)?;
    }
    socket.bind(&(*addr).into())?;
    socket.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(socket.into())?)
}

fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn opt_ipv4(b: &[u8]) -> Option<Ipv4Addr> {
    if b.len() == 4 {
        Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
    } else {
        None
    }
}

fn select_boot_file(configured: &str, arch: Option<u16>) -> &str {
    // 0=BIOS, 7=x64 UEFI, 9=x64 UEFI (HTTP), 6=x86 UEFI
    match arch {
        Some(0) if configured.ends_with(".efi") => "undionly.kpxe",
        Some(7 | 9 | 6) if configured.ends_with(".kpxe") || configured.ends_with(".pxe") => {
            "ipxe.efi"
        }
        _ => configured,
    }
}

fn subnet_broadcast(ip: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(ip) | !u32::from(mask))
}

fn dhcp_option(packet: &[u8], code: u8) -> Option<&[u8]> {
    let mut i = 240;
    while i < packet.len() {
        let opt = packet[i];
        if opt == 255 {
            break;
        }
        if opt == 0 {
            i += 1;
            continue;
        }
        if i + 1 >= packet.len() {
            break;
        }
        let len = packet[i + 1] as usize;
        if i + 2 + len > packet.len() {
            break;
        }
        if opt == code {
            return Some(&packet[i + 2..i + 2 + len]);
        }
        i += 2 + len;
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn build_reply(
    req: &[u8],
    xid: &[u8],
    chaddr: &[u8; 6],
    yiaddr: Ipv4Addr,
    next_server: Ipv4Addr,
    boot_file: &str,
    msg_type: u8,
    subnet: Ipv4Addr,
    router: Ipv4Addr,
    lease_secs: u32,
    dns: Option<Ipv4Addr>,
    proxy: bool,
    force_broadcast: bool,
) -> anyhow::Result<Vec<u8>> {
    let mut out = vec![0u8; 300];
    out[0] = 2; // bootreply
    out[1] = req[1];
    out[2] = req[2];
    out[3] = req[3];
    out[4..8].copy_from_slice(xid);
    // Preserve client flags; force broadcast bit so firmware without IP can hear Offer.
    let mut flags = u16::from_be_bytes([req[10], req[11]]);
    if force_broadcast {
        flags |= 0x8000;
    }
    out[10..12].copy_from_slice(&flags.to_be_bytes());
    out[16..20].copy_from_slice(&yiaddr.octets());
    out[20..24].copy_from_slice(&next_server.octets());
    out[28..34].copy_from_slice(chaddr);
    // boot file name at offset 108 (128 bytes)
    let file_bytes = boot_file.as_bytes();
    let n = file_bytes.len().min(127);
    out[108..108 + n].copy_from_slice(&file_bytes[..n]);
    // magic cookie
    out[236..240].copy_from_slice(&[99, 130, 83, 99]);

    let mut opts = Vec::new();
    opts.extend_from_slice(&[53, 1, msg_type]); // DHCP message type
    opts.extend_from_slice(&[54, 4]); // server id
    opts.extend_from_slice(&next_server.octets());
    if !proxy {
        opts.extend_from_slice(&[1, 4]);
        opts.extend_from_slice(&subnet.octets());
        opts.extend_from_slice(&[3, 4]);
        opts.extend_from_slice(&router.octets());
        opts.extend_from_slice(&[51, 4]);
        opts.extend_from_slice(&lease_secs.to_be_bytes());
        if let Some(dns) = dns {
            opts.extend_from_slice(&[6, 4]);
            opts.extend_from_slice(&dns.octets());
        }
    }
    // option 66 TFTP server name
    let ns = next_server.to_string();
    opts.push(66);
    opts.push(ns.len() as u8);
    opts.extend_from_slice(ns.as_bytes());
    // option 67 bootfile
    opts.push(67);
    opts.push(boot_file.len() as u8);
    opts.extend_from_slice(boot_file.as_bytes());
    // Option 43: PXE discovery control 0x08 = use DHCP bootfile/siaddr, skip PXE discovery.
    // Without this (and with option 60), many UEFI stacks wait forever on UDP/4011.
    opts.extend_from_slice(&[43, 4, 6, 1, 0x08, 0xff]);
    // Option 60 only for ProxyDHCP. On a combined DHCP+PXE server it makes firmware
    // ignore filename/siaddr and wait for a separate ProxyDHCP offer.
    if proxy {
        let pxe = b"PXEClient";
        opts.push(60);
        opts.push(pxe.len() as u8);
        opts.extend_from_slice(pxe);
    }
    opts.push(255);

    // BOOTP legacy minimum size — some UEFI PXE stacks drop short replies.
    let need = (240 + opts.len()).max(300);
    if need > out.len() {
        out.resize(need, 0);
    }
    out[240..240 + opts.len()].copy_from_slice(&opts);
    out.truncate(need);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_offer_with_bootfile() {
        let mut req = vec![0u8; 240];
        req[0] = 1;
        req[1] = 1;
        req[2] = 6;
        req[236..240].copy_from_slice(&[99, 130, 83, 99]);
        req.extend_from_slice(&[53, 1, 1, 255]);
        let reply = build_reply(
            &req,
            &[1, 2, 3, 4],
            &[0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
            "10.10.0.50".parse().unwrap(),
            "10.10.0.1".parse().unwrap(),
            "undionly.kpxe",
            2,
            "255.255.255.0".parse().unwrap(),
            "10.10.0.1".parse().unwrap(),
            3600,
            None,
            false,
            true,
        )
        .unwrap();
        assert_eq!(reply[10] & 0x80, 0x80);
        assert_eq!(reply[0], 2);
        assert!(dhcp_option(&reply, 67).unwrap().starts_with(b"undionly"));
        assert_eq!(dhcp_option(&reply, 53), Some(&[2][..]));
        assert!(dhcp_option(&reply, 60).is_none());
        assert_eq!(dhcp_option(&reply, 43), Some(&[6, 1, 0x08, 0xff][..]));
    }
}
