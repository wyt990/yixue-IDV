# 模块 20 — WOL 远程开机

> 目标：学生在管理端选中终端点「开机」后，已关机（S5）的机器能被网络唤醒。
> 远程重启/关机走 Agent 命令通道；WOL 不依赖 Agent（机器已关机），由服务端/边缘节点发幻数据包。

---

## 1. 服务端实现（2026-11-16 修复）

原实现只向 `255.255.255.255:9` 发 1 个包，依赖默认路由，容易：
- 从错误网卡（VPN/虚拟网卡）发出；
- 广播包丢失（UDP 无重传）；
- 部分网卡/交换机只认定向广播。

现改为（`idv-common/src/wol.rs` + `api/power.rs`）：

- 按客户端所属机房的 `ip_start + netmask` 计算 **定向广播地址**（如 `10.110.0.255`）；
- 同时发 `255.255.255.255` 与定向广播，端口 **UDP 9 和 7**，重复 **3 次**；
- 电源命令 `detail` 记录实际发送情况（目标、包数），可在 `power_commands` 表/接口查看。

> 如果 Center 不在机房 VLAN（Center 与 Edge 分离部署），定向广播无法跨路由；
> 此时应由该机房的 Edge 发 WOL（后续增强项，当前先保证 Center/Edge 同网段场景）。

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
3. 查询命令 detail 应包含 `magic_packet_sent ... targets=...`；客户端应在数秒内上电。
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
