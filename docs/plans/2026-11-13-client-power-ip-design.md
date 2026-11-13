# 终端电源 / IP / 改模式自动重启（设计）

> 2026-11-13 · 已确认（范围 A：整机房下发重启）

## 做

1. 机房改工作模式 / 开机还原勾选导致绑定变化 → 该机房全部已登记终端排队 `reboot`
2. 终端页多选：重启 / 关机 / 开机(WOL)
3. 自动登记不再写入 `notes`
4. 终端列表显示 `last_ip`（Agent 心跳上报）

## 通道

仍走心跳 `pending_power`；Agent 对 `shutdown`/`reboot` 执行后 ack。开机用 Center 侧 WOL。

## Agent

心跳上报 `local_ip`；收到 reboot 时先 `sync-policy --apply`（若有 pending）再重启。
