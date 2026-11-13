//! Sample host CPU, memory, and aggregate NIC throughput (non-loopback).

use std::sync::Mutex;
use std::time::Instant;

use sysinfo::{Networks, System};

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct HostMetrics {
    pub cpu_pct: f64,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
}

pub struct HostMetricsSampler {
    inner: Mutex<SamplerState>,
}

struct SamplerState {
    sys: System,
    networks: Networks,
    last_rx: u64,
    last_tx: u64,
    last_at: Instant,
    primed: bool,
}

impl HostMetricsSampler {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        let networks = Networks::new_with_refreshed_list();
        let (rx, tx) = sum_nic_bytes(&networks);
        Self {
            inner: Mutex::new(SamplerState {
                sys,
                networks,
                last_rx: rx,
                last_tx: tx,
                last_at: Instant::now(),
                primed: false,
            }),
        }
    }

    pub fn sample(&self) -> HostMetrics {
        let Ok(mut st) = self.inner.lock() else {
            return HostMetrics::default();
        };
        st.sys.refresh_cpu_all();
        st.sys.refresh_memory();
        st.networks.refresh(true);

        let cpu_pct = st.sys.global_cpu_usage() as f64;
        let mem_used_bytes = st.sys.used_memory();
        let mem_total_bytes = st.sys.total_memory();

        let (rx, tx) = sum_nic_bytes(&st.networks);
        let now = Instant::now();
        let elapsed = now.duration_since(st.last_at).as_secs_f64().max(0.001);
        let (net_rx_bps, net_tx_bps) = if st.primed {
            let drx = rx.saturating_sub(st.last_rx) as f64;
            let dtx = tx.saturating_sub(st.last_tx) as f64;
            ((drx / elapsed) as u64, (dtx / elapsed) as u64)
        } else {
            (0, 0)
        };
        st.last_rx = rx;
        st.last_tx = tx;
        st.last_at = now;
        st.primed = true;

        HostMetrics {
            cpu_pct,
            mem_used_bytes,
            mem_total_bytes,
            net_rx_bps,
            net_tx_bps,
        }
    }
}

fn sum_nic_bytes(networks: &Networks) -> (u64, u64) {
    let mut rx = 0u64;
    let mut tx = 0u64;
    for (name, data) in networks.iter() {
        let lower = name.to_ascii_lowercase();
        if lower.contains("loopback")
            || lower == "lo"
            || lower.starts_with("lo:")
            || lower.starts_with("lo ")
        {
            continue;
        }
        rx = rx.saturating_add(data.total_received());
        tx = tx.saturating_add(data.total_transmitted());
    }
    (rx, tx)
}
