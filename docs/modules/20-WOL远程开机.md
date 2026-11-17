# 模块 20 — WOL 远程开机

> 目标：学生在管理端选中终端点「开机」后，已关机（S5）的机器能被网络唤醒。
> 远程重启/关机走 Agent 命令通道；WOL 不依赖 Agent（机器已关机），由服务端/边缘节点发幻数据包。

---

## 1. 服务端实现（2026-11-17：Edge 转发）

WOL 现在优先由**客户端所属机房的 Edge** 转发，Center 不再直接向跨 VLAN 广播：

```text
Center 选 Edge（lab_edges → campus_edges，在线优先）
  → edge_commands(kind=wol, payload={items:[{power_id,mac,broadcast}]})
  → Edge 校验 broadcast 属于本机房网段
  → Edge 在本机发 UDP 9/7 magic packet（定向广播 + 255.255.255.255）
  → Edge report → Center 更新 power_commands.detail
```

- 同一 Edge 的批量开机合并为一条命令，payload 可含多台 MAC；
- 无在线 Edge 时回退 Center 本地发送，detail 标记 `wol local_fallback`；
- Center 仍按机房 `ip_start + netmask` 计算定向广播；
- Edge 没有机房网段缓存时，只接受私有地址广播，避免成为广播反射器；
- 迁移：`023_edge_command_payload.sql`。

> 前提：Edge 必须有接口在目标机房的二层广播域（物理口接机房 VLAN，或 trunk 子接口）。
> 如果 Edge 只在服务器 VLAN，仍然无法跨 VLAN 唤醒，需要把 Edge/轻量转发器放到机房侧。

旧实现（Center 直接发）仍作为兜底保留。

---

## 2. 客户端前提（缺一不可）

| 层 | 要求 |
|---|---|
| BIOS/UEFI | 启用 `Wake on LAN` / `Power On by PCIe/PCI`；关闭 `ErP Ready` / `Deep Sleep`（不同主板名称不同） |
| 网卡驱动 | 设备管理器 → 网卡 → 属性 → 电源管理：勾选「允许此设备唤醒计算机」+「只允许幻数据包唤醒计算机」；Realtek 等还需把「关机 Wake on LAN」设为 Enabled |
| Windows | 关闭**快速启动**（快速启动的“关机”是混合关机，部分网卡不监听 WOL）；`powercfg /h off` |
| 供电/连接 | 必须**有线以太网**；关机后网口指示灯应常亮（5VSB 供电正常） |
| 网络 | 交换机/VLAN 不隔离广播；WOL 包发往本机所在子网的广播地址 |

一键设置（客户端管理员）：

```powershell
cd "C:\Program Files\YixueIDV"
.\scripts\enable-wol.ps1
```

脚本会启用 `WakeOnMagicPacket`、关闭 `WakeOnPattern`、执行 `powercfg /h off` 并打印当前能力。
驱动不支持命令行时，按提示在设备管理器手动勾选。

---

## 3. 验收

1. 客户端正常关机（不是睡眠/休眠）；确认网口灯亮。
2. 管理端选中终端 → 「开机」；或 API：
   ```powershell
   $t = (Invoke-RestMethod -Method Post -Uri http://10.110.0.1:8080/api/v1/auth/login `
     -ContentType application/json -Body '{"username":"admin","password":"admin123"}').token
   Invoke-RestMethod -Method Post -Uri "http://10.110.0.1:8080/api/v1/power/commands" `
     -Headers @{Authorization="Bearer $t"} -ContentType application/json `
     -Body '{"mac":"40:FE:95:B9:FF:A8","kind":"wol","detail":"manual test"}'
   ```
3. 查询命令 detail 应包含 `wol via edge:<name>` + `magic_packet_sent ...`；无 Edge 时包含 `wol local_fallback`；客户端应在数秒内上电。
4. 跨 VLAN 测试：Center 在服务器 VLAN、客户端在机房 VLAN，在 Edge 机房网卡抓包应能看到 magic packet。
4. 若 detail 显示发送成功但仍不亮：按第 2 节逐项排查（BIOS → 网卡电源管理 → 快速启动 → 有线/供电）。

---

## 4. 常见现象对照

| 现象 | 原因 |
|---|---|
| 命令 detail 是 `wol_send_failed` | 服务端 socket/广播发送失败（权限/网络栈） |
| detail 成功、机器不亮、网口灯灭 | 关机后 NIC 未保持供电：BIOS ErP / 电源管理未勾选 / 非有线 |
| 网口灯亮但不亮机 | BIOS 未启用 PCIe 唤醒；或快速启动导致混合关机 |
| 重启/关机正常、WOL 不行 | 正常，重启/关机走 Agent；WOL 依赖上述硬件/固件设置 |
| 多网卡机器 | WOL 只对“有线且启用了魔术包”的网卡生效；客户端脚本会分别处理 |
