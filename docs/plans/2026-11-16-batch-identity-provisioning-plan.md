# 批量身份与 DHCP 保留 — 实现计划

> 日期：2026-11-16
> 设计：[2026-11-16-batch-identity-provisioning-design.md](./2026-11-16-batch-identity-provisioning-design.md)
> 状态：**已实施（2026-11-16）**；真机/VM 验收（阶段 7）待执行

## 阶段 0：准备

- [ ] 确认离线依赖可用（`windows-sys 0.61.2` 已在 Cargo.lock；如 Agent 改用 PowerShell 探测则无需新依赖）
- [ ] 基线：`cargo test --workspace` 通过（当前 23 个测试）

## 阶段 1：Center（数据与接口）

文件：
- `crates/idv-center/migrations/018_lab_ip_mode.sql`
  - `ALTER TABLE labs ADD COLUMN ip_mode TEXT NOT NULL DEFAULT 'static';`
- `crates/idv-center/src/models.rs`
  - `Lab` 增 `ip_mode`
  - `DesiredIdentity` 增 `ip_mode`
  - `AgentHeartbeat` 增可选 `current_hostname`、`identity_state`、`last_error`
- `crates/idv-center/src/lab_naming.rs`
  - `LAB_COLS` 增 `ip_mode`
- `crates/idv-center/src/api/labs.rs`
  - create/update 接收并保存 `ip_mode`（校验 `dhcp|static`）
- `crates/idv-center/src/api/agent.rs`
  - `desired_identity` 携带 `ip_mode`
  - 心跳把 current_hostname/identity_state/last_error 并入 `agent_sessions.detail`
- 新增 `crates/idv-center/src/api/edge_bindings.rs`
  - `GET /api/v1/edge/ip-bindings`（`AuthEdge`）
  - 过滤：`lab_edges` → 回退 `campus_edges`；返回 `{ bindings: [{ mac, ip, lab_id }] }`
- `crates/idv-center/src/api/mod.rs` 注册路由

验收：
- 迁移后旧库可启动；`labs` 返回 `ip_mode`
- 心跳响应含 `desired_identity.ip_mode`
- 用 Edge token 调 `/api/v1/edge/ip-bindings` 只返回关联机房的绑定

## 阶段 2：Edge（DHCP 保留）

文件：
- 新增 `crates/idv-edge/src/ip_bindings.rs`
  - `IpBindings`：`load(path)` / `save(path)` / `replace(json)` / `get(mac)` / `reserved_ips()` / 子网过滤
  - JSON 缓存格式：`{ "bindings": [ { "mac": "AA:BB:..", "ip": "10.110.0.31" } ] }`
- `crates/idv-edge/src/dhcp.rs`
  - `DhcpConfig` 增 `reservations: IpBindings`
  - Discover/Request 命中保留 MAC → 直接 Offer/ACK 保留 IP
  - 池分配跳过保留 IP 与已有租约
  - 保留 IP 不在本子网 → 忽略并告警
  - 单测：保留命中、池跳过、子网过滤
- `crates/idv-edge/src/worker.rs`
  - `fetch_ip_bindings()`（Edge token 调 Center）
- `crates/idv-edge/src/main.rs`
  - 启动加载缓存；后台每 30s 刷新并落盘；把 `IpBindings` 传入 `DhcpService`
- `crates/idv-edge/src/pe_api.rs` 不改（PE 不感知）

验收：
- 无 Center 时 Edge 可用磁盘缓存下发保留 IP
- `idv-edge dhcp-probe --mac <reserved>` 收到指定 IP 的 Offer

## 阶段 3：Agent（自动 MAC + 首启收敛）

文件：
- `crates/idv-agent/src/main.rs`
  - `AgentConfig.mac` 默认空；`hostname` 默认空
  - `effective_mac()`：空/`auto` → 自动；显式值保留（兼容旧配置）
  - 心跳使用 `effective_mac` / 当前主机名
  - `DesiredIdentity` 增 `ip_mode`
  - 新增纯函数 `decide_identity_action(IdentityState, ProtectionState, Policy) -> Action`
  - 执行动作：`disarm → reboot` / `rename+reboot` / `arm+reboot` / `set_ip` / `dhcp_renew`
  - 动作失败不重启，上报 `identity_state/last_error`
  - 写 `%ProgramData%\YixueIDV\identity.log`
- 新增 `crates/idv-agent/src/netinfo.rs`
  - `route_interface(center_url)`：PowerShell `Find-NetRoute` + `Get-NetAdapter` + `Get-NetIPAddress`；回退 `Get-NetAdapter | Status Up`
  - `mac_of_interface()` / `local_ip_of_interface()` / `renew_dhcp(alias)`
  - 单测：MAC 规范化、输出解析
- `crates/idv-agent/Cargo.toml`：无新依赖（用 PowerShell），或按实现需要加 `windows-sys`

验收（单测 + 手工）：
- 不写 `mac` 时心跳上报真实 MAC
- 模拟状态机覆盖：保护中不一致→disarm；未保护不一致→改名；一致未保护→arm；一致已冻结→无动作
- 首启 2~3 次重启内收敛，之后重启身份不变

## 阶段 4：客户端包与金样机

文件：
- 新增 `deploy/client-package/agent.toml.example`
- `deploy/client-package/scripts/install-client.ps1`
  - 缺 `agent.toml` 时复制模板；`-CenterUrl` 写入
- 新增 `deploy/scripts/prepare-golden-client.ps1`
  - 检查包目录、生成配置、注册开机+轮询任务、`restore-driver.ps1 protect-off`、清理身份状态
- 更新 `使用说明-测试签名驱动.txt`（或新增客户端批量部署说明）

验收：`prepare-golden-client.ps1` 在 VM 上可重复执行；重复运行不产生重复任务

## 阶段 5：前端

文件：
- `web/src/api.ts`：`Lab` 增可选 `ip_mode`
- `web/src/pages/LabsPage.tsx`：机房表单/详情增加 `ip_mode` 选择（DHCP 保留 / 静态）
- 如时间允许：`npm run build` 更新 `web/dist`

## 阶段 6：文档与验收

- 新增 `docs/modules/18-批量身份与DHCP保留.md`，并加索引
- 新增 `deploy/scripts/identity-accept.ps1`
  - 校验：首启收敛、连续重启身份不变、还原开启后不变、换 MAC、static 兼容
- 更新 `docs/易学IDV机房管理系统-开发文档.md` 阶段说明（新增一期能力）

## 阶段 7：验证

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Agent/Edge 单测覆盖新增逻辑
- [ ] PowerShell 脚本 PS 5.1 解析 0 错误（UTF-8 BOM）
- [ ] `npm run build`（前端）
- [ ] 手工：Center+Edge 起服务，`dhcp-probe` 验证保留 IP；Agent 手动 heartbeat 验证自动 MAC（不启新服务时可用现有运行实例的旧二进制，重启后验证新功能）

## 风险与回退

- 迁移 018 只加列，默认 `static`，旧行为不变；回退只需忽略字段
- Edge DHCP 保留默认关闭（`ip_mode=static` 或绑定为空时行为与现在一致）
- 状态机首启重启：动作失败不重启；必要时可 `uninstall-driver.ps1` 回滚
- 不修改现有运行实例的数据库；迁移在下一次 Center 启动时应用
