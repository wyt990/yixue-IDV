# 阶段 6c 设计：P2P 停种 · USB 落地 · PE 网络向导 · 主题补齐

## 范围

| 项 | 做法 | 验收 |
|----|------|------|
| P2P 做种生命周期 | Edge `POST /api/v1/p2p/swarm/{image_id}/stop` 清空 swarm 分片与 peers；部署完成后 pe-agent 可调用 | swarm_chunks→0 |
| USB 限制落地 | Agent 按策略写本地 `usb_policy.json` 并上报；Center 记 `usb_enforced` | stage6c-accept |
| PE 网络向导 | `idv-pe-agent network-wizard`：iface/SSID/Edge URL → 会话文件 → 探测 /health；WiFi 源 checkin 需令牌 | 无令牌 401；有令牌可 checkin |
| 管理端主题 | 补齐 `pine-rack` / `slate-ops`（共 4 套 Token） | Appearance 可选 |

## 非本刀

- 真机卷管理 / Windows 驱动级还原  
- 真机 USB 驱动级禁用、真 WiFi 驱动包  
- 信创浏览器专项兼容矩阵（主题字体已用 Noto Sans SC）
