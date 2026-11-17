# 跨 VLAN WOL：Edge 转发方案

> 日期：2026-11-17  
> 状态：Phase 1 已实现（Edge 转发、批量、本地兜底、电源记录）  
> 相关：模块 20（WOL）、模块 19（Edge 命令通道）、`power_commands`、`edge_commands`

## 1. 问题

当前 WOL 由 Center 直接发送：

```text
Center → 机房网段定向广播 + 255.255.255.255
```

- `255.255.255.255` 是本地广播，不会跨三层；
- 定向广播（如 `10.110.0.255`）默认被路由器丢弃或限制；
- 因此 Center 不在目标机房的二层广播域时，WOL 会“显示成功但机器不醒”。

## 2. 结论

**由机房内的 Edge 代为发送 magic packet** 最合适：

- Edge 本来就和学生机在同一机房/VLAN；
- Edge 已有成熟的 Center 拉取命令、回报状态通道；
- 不需要开放 Edge 入站端口，不改变现有网络策略；
- Center 只负责选择 Edge、下发任务、汇总结果。

## 3. 目标流程

```text
UI / 定时任务
   │
   ▼
Center 生成 WOL 任务
   │  选择目标机房绑定的在线 Edge
   ▼
edge_commands (kind=wol, payload={items:[...]})
   │  Edge 长轮询拉取
   ▼
Edge 校验 MAC / 广播地址
   │
   ├─ UDP/9 + UDP/7 发 magic packet
   │  定向广播（机房网段）
   │  可再发 255.255.255.255 兜底
   ▼
Edge report → Center 更新 power_commands
```

## 4. 数据与接口

### 4.1 `edge_commands`

现有表只有 `kind/image_id`，增加 payload：

```sql
ALTER TABLE edge_commands ADD COLUMN payload TEXT NOT NULL DEFAULT '{}';
```

`kind = "wol"` 时 payload：

```json
{
  "items": [
    {
      "power_id": "power-command-id",
      "mac": "40:FE:95:B9:FF:A8",
      "broadcast": "10.110.0.255"
    }
  ],
  "ports": [9, 7],
  "repeat": 3,
  "interval_ms": 100
}
```

- 一个 Edge 命令可携带多台机器，适合 500 台批量开机；
- `broadcast` 由 Center 按客户端所属机房 `ip_start + netmask` 计算；
- Edge 必须校验 broadcast 属于自己绑定的机房网段，避免被当成广播反射器。

### 4.2 Edge 选择

优先级：

1. `lab_edges` 中该机房的在线 Edge（`last_seen` 在 30~45 秒内），按 `priority` 升序；
2. `campus_edges` 中校区级 Edge；
3. 没有在线 Edge → 回退 Center 本地发送，并在 detail 标记 `no_edge_fallback_local`。

一个机房只选一台 Edge，避免重复包；500 台批量也按 Edge 聚合为一条命令。

### 4.3 Edge 发送

复用 `idv-common::wol` 的 magic packet 构造/发送能力：

- 目标：定向广播 + `255.255.255.255`（兜底）；
- 端口：UDP 9 和 7；
- 重复：默认 3 轮，每轮间隔 100ms；
- 发送成功即算完成（WOL 无回执，不能以“机器是否醒”判断）；
- 报告 `sent / failed_macs`，便于排查。

### 4.4 结果回写

`edge_commands` 报告成功后，Center 按 payload 里的 `power_id` 更新每条 `power_commands`：

```text
state  = done / failed
detail = wol via edge:<name>, sent=N, targets=...
```

UI 的电源/开机记录直接显示 detail，不需要新页面。

## 5. 失败与兜底

| 场景 | 处理 |
|------|------|
| 无在线 Edge | Center 本地发送，detail 标记 |
| Edge 离线/命令超时 | 保持 `pending`，前端提示“等待 Edge 转发”；可人工重试 |
| Edge 校验广播地址失败 | 命令失败并回报，不发送 |
| 客户端 MAC 非法 | Center 入队前拒绝 |
| 500 台批量 | 按 Edge 聚合，一条命令、多个 items |
| Edge 重启丢失命令 | 现有命令表 pending 状态可重新拉取；增加超时回收 |

不建议依赖“路由器转发定向广播”：需要网络设备特殊配置，且会扩大广播域，安全风险高。

## 6. 网络前提

- Edge 必须有一个接口在目标机房的二层广播域（物理口接机房 VLAN，或 trunk 上的该 VLAN 子接口）；
- Edge 到目标网段有正确路由/接口；Windows 下定向广播会按路由表选择出口；
- 学生机 BIOS + 网卡 WOL 已开启（现有 `enable-wol.ps1`），关闭快速启动；
- 交换机不能隔离 Edge 端口到学生机的二层广播（端口隔离/私有 VLAN 需例外）。

如果一台 Edge 接多个 VLAN，建议按机房显式指定发送接口（后续可加 `[wol] bind_ip_by_lab`），避免依赖默认路由。

## 7. 安全

- Edge 使用现有 `node_token` 身份；只接受 Center 命令；
- payload 中的 broadcast 必须是 Edge 已绑定机房的网段广播地址；
- 只允许 IPv4 私有地址，拒绝 0.0.0.0、组播、公网地址；
- MAC 严格校验；
- 记录操作者、Edge、目标 MAC、发送结果到 `power_commands.detail` / 审计日志；
- 不开放 Edge 的 WOL HTTP 入站接口，保持“Edge 主动拉取”模型。

## 8. 实施拆分

### Phase 1（推荐先做）

1. 迁移 `023_edge_command_payload.sql`；
2. Edge 增加 `wol` 命令处理 + `[wol]` 配置（ports/repeat/interval）；
3. Center `power.rs`：WOL 改为选 Edge、写 edge_commands、失败回退本地；
4. Center Edge 报告处理：回写 power_commands；
5. UI 电源记录显示转发 Edge 与发送结果；
6. 测试：跨 VLAN 真机、Edge 离线、批量 100 台。

### Phase 2（可选）

- 按 Edge 聚合 500 台批量，发送进度；
- 命令超时自动重试 / 切换备用 Edge；
- 多 VLAN Edge 的发送接口绑定；
- 定时电源任务复用同一转发路径。

## 9. 验收标准

- [ ] Center 在服务器 VLAN、客户端在机房 VLAN 时，客户端可被 WOL 唤醒；
- [ ] 在 Edge 机房网卡抓包能看到 magic packet；
- [ ] Edge 离线时 Center 回退本地发送并明确提示；
- [ ] 非法 MAC/广播地址被拒绝；
- [ ] 100 台批量开机在可接受时间内完成（目标 < 10 秒）；
- [ ] 电源记录能看到 `via edge:<name>` 和发送数量。

## 10. 涉及文件（实现时）

- `crates/idv-center/migrations/023_edge_command_payload.sql`
- `crates/idv-center/src/api/power.rs`
- `crates/idv-center/src/api/edge_agent.rs`
- `crates/idv-center/src/models.rs`
- `crates/idv-edge/src/worker.rs`
- `crates/idv-edge/src/main.rs`（`[wol]` 配置）
- `crates/idv-common/src/wol.rs`（如需批量发送辅助）
- `web/src/pages/ClientsPage.tsx` / 电源记录展示
