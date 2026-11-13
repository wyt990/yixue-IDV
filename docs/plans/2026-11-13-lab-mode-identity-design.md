# 机房模式下拉 + 终端名/IP 下发（设计）

> 2026-11-13 · 方案 1 已确认

## 做

1. **取消「工作模式」独立页**；机房列表增加「工作模式」下拉，改绑仍走 `bindLab` + 整机房 `reboot`。
2. **机房设置**增加掩码 / 网关 / DNS（与 `ip_start`/`ip_end` 同级）。
3. **登记时**按区间自动分配 `clients.assigned_ip`（可手改）；台账 `name` 即目标计算机名。
4. **心跳**返回 `desired_identity`；Agent 比对后设静态 IP + 重命名计算机（改名后重启）。

## 不做

- Edge DHCP 预留 / Proxy 绑定（以后可接同一台账）
- 把身份塞进 `DesiredState` revision

## 数据

- `labs`: `netmask`, `gateway`, `dns`（TEXT，可空）
- `clients`: `assigned_ip`（TEXT，可空；登记时从机房区间分配）

## Agent

`sync-policy` / 轮询：若 `desired_identity` 与本机不一致 → 应用网卡静态地址 → 必要时 `Rename-Computer` 并重启。
