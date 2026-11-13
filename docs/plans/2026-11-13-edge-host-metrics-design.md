# Edge 节点实时主机指标（CPU / 内存 / 网卡吞吐）

> 2026-11-13 · 已确认：带宽 = 网卡总 RX/TX（方案 A）

## 行为

- Edge 心跳携带 `cpu_pct`、`mem_used_bytes`、`mem_total_bytes`、`net_rx_bps`、`net_tx_bps`
- Center 写入 `edges`；列表列「节点负载」展示；管理端 ~2–3s 轮询局部更新
- 缓存水位仍入库，列表不再展示
