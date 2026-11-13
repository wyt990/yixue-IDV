//! Minimal read-only TFTP server for PXE boot files (RFC 1350 + 2347 options).

use std::path::{Path, PathBuf};

use tokio::net::UdpSocket;
use tracing::{info, warn};

pub async fn run(bind: &str, root: PathBuf) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&root).await?;
    let sock = UdpSocket::bind(bind).await?;
    info!(%bind, root = %root.display(), "TFTP service listening");
    // UEFI RRQ with options is often > 516; Windows returns WSAEMSGSIZE (10040) if buffer is short.
    let mut buf = vec![0u8; 4096];
    loop {
        let (n, peer) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(err) => {
                warn!(error = %err, "tftp recv error");
                continue;
            }
        };
        if n < 4 {
            continue;
        }
        let opcode = u16::from_be_bytes([buf[0], buf[1]]);
        info!(%peer, opcode, bytes = n, "tftp packet received");
        if opcode != 1 {
            let _ = send_error(&sock, peer, 4, "only RRQ supported").await;
            continue;
        }
        let rrq = match parse_rrq(&buf[2..n]) {
            Ok(v) => v,
            Err(err) => {
                warn!(%peer, error = %err, "tftp bad RRQ");
                let _ = send_error(&sock, peer, 4, &err.to_string()).await;
                continue;
            }
        };
        let root = root.clone();
        tokio::spawn(async move {
            if let Err(err) = serve_file(&root, &rrq, peer).await {
                warn!(file = %rrq.filename, error = %err, "tftp RRQ failed");
                if let Ok(s) = UdpSocket::bind("0.0.0.0:0").await {
                    let _ = send_error(&s, peer, 0, "transfer failed").await;
                }
            }
        });
    }
}

#[derive(Debug)]
struct Rrq {
    filename: String,
    blksize: usize,
    tsize: bool,
}

fn parse_rrq(bytes: &[u8]) -> anyhow::Result<Rrq> {
    let parts: Vec<&[u8]> = bytes.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        anyhow::bail!("invalid RRQ");
    }
    let filename = String::from_utf8(parts[0].to_vec())?;
    if filename.contains("..") || Path::new(&filename).is_absolute() {
        anyhow::bail!("invalid filename");
    }
    let mut blksize = 512usize;
    let mut tsize = false;
    let mut i = 2;
    while i + 1 < parts.len() {
        let key = std::str::from_utf8(parts[i]).unwrap_or("");
        let val = std::str::from_utf8(parts[i + 1]).unwrap_or("");
        match key.to_ascii_lowercase().as_str() {
            "blksize" => {
                if let Ok(n) = val.parse::<usize>() {
                    // Keep under typical Ethernet MTU payload.
                    blksize = n.clamp(8, 1428);
                }
            }
            "tsize" => tsize = true,
            _ => {}
        }
        i += 2;
    }
    Ok(Rrq {
        filename,
        blksize,
        tsize,
    })
}

async fn serve_file(root: &Path, rrq: &Rrq, peer: std::net::SocketAddr) -> anyhow::Result<()> {
    let path = root.join(&rrq.filename);
    let data = tokio::fs::read(&path).await.map_err(|err| {
        warn!(file = %rrq.filename, error = %err, "tftp file missing");
        err
    })?;
    info!(
        file = %rrq.filename,
        bytes = data.len(),
        blksize = rrq.blksize,
        tsize = rrq.tsize,
        %peer,
        "TFTP RRQ"
    );

    let xfer = UdpSocket::bind("0.0.0.0:0").await?;
    let mut rx = vec![0u8; 4096];

    // RFC 2347: if client requested options, send OACK and wait for ACK#0.
    let has_options = rrq.blksize != 512 || rrq.tsize;
    if has_options {
        let mut oack = Vec::new();
        oack.extend_from_slice(&6u16.to_be_bytes()); // OACK
        if rrq.blksize != 512 {
            oack.extend_from_slice(b"blksize\0");
            oack.extend_from_slice(rrq.blksize.to_string().as_bytes());
            oack.push(0);
        }
        if rrq.tsize {
            oack.extend_from_slice(b"tsize\0");
            oack.extend_from_slice(data.len().to_string().as_bytes());
            oack.push(0);
        }
        xfer.send_to(&oack, peer).await?;
        let (n, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), xfer.recv_from(&mut rx))
                .await??;
        if n >= 4 && u16::from_be_bytes([rx[0], rx[1]]) == 4 {
            let ack_block = u16::from_be_bytes([rx[2], rx[3]]);
            if ack_block != 0 {
                anyhow::bail!("expected ACK#0 after OACK, got {ack_block}");
            }
        } else {
            // Client rejected options; continue on this TID with classic 512-byte blocks.
            warn!(%peer, bytes = n, "OACK not ACKed; falling back to blksize 512");
            return serve_data(&xfer, &rrq.filename, &data, peer, 512, &mut rx).await;
        }
    }

    serve_data(&xfer, &rrq.filename, &data, peer, rrq.blksize, &mut rx).await
}

async fn serve_data(
    xfer: &UdpSocket,
    filename: &str,
    data: &[u8],
    peer: std::net::SocketAddr,
    blksize: usize,
    rx: &mut [u8],
) -> anyhow::Result<()> {
    let mut block: u16 = 1;
    let mut offset = 0usize;
    loop {
        let end = (offset + blksize).min(data.len());
        let mut pkt = Vec::with_capacity(4 + end.saturating_sub(offset));
        pkt.extend_from_slice(&3u16.to_be_bytes()); // DATA
        pkt.extend_from_slice(&block.to_be_bytes());
        pkt.extend_from_slice(&data[offset..end]);

        let mut acked = false;
        for _ in 0..5 {
            xfer.send_to(&pkt, peer).await?;
            match tokio::time::timeout(std::time::Duration::from_secs(2), xfer.recv_from(&mut *rx))
                .await
            {
                Ok(Ok((n, _))) if n >= 4 => {
                    let op = u16::from_be_bytes([rx[0], rx[1]]);
                    if op == 5 {
                        let msg = String::from_utf8_lossy(&rx[4..n]);
                        anyhow::bail!("client ERROR: {msg}");
                    }
                    if op == 4 {
                        let ack_block = u16::from_be_bytes([rx[2], rx[3]]);
                        if ack_block == block {
                            acked = true;
                            break;
                        }
                    }
                }
                Ok(Ok(_)) => continue,
                Ok(Err(err)) => return Err(err.into()),
                Err(_) => continue, // timeout → retransmit
            }
        }
        if !acked {
            anyhow::bail!("no ACK for block {block}");
        }

        let chunk_len = end - offset;
        offset = end;
        if chunk_len < blksize {
            info!(file = %filename, blocks = block, "TFTP transfer complete");
            break;
        }
        block = block.wrapping_add(1);
    }
    Ok(())
}

async fn send_error(
    sock: &UdpSocket,
    peer: std::net::SocketAddr,
    code: u16,
    msg: &str,
) -> anyhow::Result<()> {
    let mut pkt = Vec::new();
    pkt.extend_from_slice(&5u16.to_be_bytes());
    pkt.extend_from_slice(&code.to_be_bytes());
    pkt.extend_from_slice(msg.as_bytes());
    pkt.push(0);
    sock.send_to(&pkt, peer).await?;
    Ok(())
}
