# 批量身份与 DHCP 保留 — 设计

> 日期：2026-11-16
> 状态：已确认（方案 A：首启身份收敛状态机 + Edge DHCP 保留 IP）
> 关联：`docs/modules/17-驱动测试签名与蓝屏恢复.md`

## 1. 目标

500 台学生机用同一镜像批量部署时：

- 客户端配置全通用（`agent.toml` 只写 `center_url`）；
- 每台机自动取「到 Center 路由网卡」的 MAC，不再手工配置；
- Center 自动按机房规则分配 `stu0NN` 与 IP（已有能力）；
- 首次开机自动改名/配网，**还原开启后身份保持正确**；
- 之后重启无需人工干预。

## 2. 范围

**一期**
- Agent：MAC/网卡自动识别、通用配置、首启身份收敛状态机、身份状态上报；
- Center：机房 `ip_mode`（dhcp/static）、Edge IP 绑定接口、身份状态落库；
- Edge：DHCP 按 MAC 保留地址（Center 拉取 + 本地缓存）；
- 客户端包：配置模板、金样机准备脚本、验收脚本、模块文档。

**不在本期**
- PE 部署时离线写身份（方案 3）；
- 驱动增加身份排除（方案 4）；
- Linux 客户端、终端页在线状态列。

## 3. 数据流

```text
金样机镜像（预装 agent + 开机/轮询任务 + 驱动，保护关闭）
  → PXE 批量部署
  → 首次开机（DHCP 临时地址）
  → Agent 自动 MAC → heartbeat
  → Center 自动登记 + 分配 name/assigned_ip（已有）
  → Agent 状态机：关保护（如需）→ 改名/配网 → 开保护
  → Edge DHCP 按 MAC 下发保留 IP
  → 之后重启身份不变
```

## 4. 组件设计

### 4.1 Agent：MAC 与网卡自动识别

- `agent.toml` 的 `mac` 可选；空/`auto`/缺省 → 自动探测；显式值仍可覆盖。
- 算法：解析 `center_url` → `GetBestInterface` 取到 Center 的接口 → `GetAdaptersAddresses` 按 IfIndex 取 `PhysicalAddress`；回退为有网关的 Up 物理网卡；再回退第一块非回环网卡。
- 同一接口用于：MAC、`local_ip`、静态 IP 设置（不再用 `8.8.8.8` / “第一块 Up 网卡”）。
- 结果进程内缓存；Linux 仅保证可编译（读 `/sys/class/net` 回退）。

### 4.2 Agent：首启身份收敛状态机（核心）

每次 boot/poll 收敛一次，一次最多一个动作、最多一次重启：

| 当前状态 | 动作 |
|---|---|
| 主机名不一致 且 卷在保护中 | 写 `_qh_protect_state.data=0` → 重启 |
| 主机名不一致 且 未保护 | 设 IP（`ip_mode=static`）→ 改名 → 重启（仅设 IP 不重启） |
| 主机名一致、未保护、策略要求冻结 | 写状态文件=1 → 重启进入冻结 |
| 主机名一致、已冻结 | 完成；`dhcp` 不动 IP；`static` 每次开机重设 IP（不重启） |

- 冻结状态下仅 IP 不一致（static 模式）只重设 IP，不触发 disarm。
- 动作失败不重启，上报错误，下一次 poll 重试。
- 首启重启次数：镜像保护关闭时 2 次；镜像已 armed 时 3 次。
- 决策逻辑做成纯函数，便于单测。

### 4.3 IP：Edge DHCP 保留

- 机房 `ip_mode`：`dhcp`（推荐）| `static`（兼容）。
- Center `GET /api/v1/edge/ip-bindings`（AuthEdge）：返回本 Edge 关联机房（`lab_edges`，回退 `campus_edges`）的 `mac → assigned_ip`。
- Edge：启动读缓存 `data/edge-cache/ip-bindings.json`；每 30s 刷新并落盘；DHCP 命中保留 MAC 直接下发保留 IP，地址池跳过保留 IP；非本子网保留忽略。
- Agent：`desired_identity.ip_mode`：
  - `dhcp`：不设静态 IP；当前 IP 不等于分配 IP 时执行一次 `ipconfig /release` + `/renew`（无重启）；连续失败回退静态并上报。
  - `static`：保留现有行为，网卡按 Center 路由选择。

### 4.4 批量安装

- 客户端包新增 `agent.toml.example`（仅 `center_url`，`mac`/`hostname` 留空注释）；`install-client.ps1` 缺配置时自动复制。
- 新增 `deploy/scripts/prepare-golden-client.ps1`：安装 Agent、注册开机/轮询任务、确保保护关闭、清理身份状态。
- 流程：金样机准备一次 → 采集/部署镜像 → 500 台首启收敛。

### 4.5 Center

- 迁移 018：`labs.ip_mode TEXT NOT NULL DEFAULT 'static'`。
- `DesiredIdentity` 增加 `ip_mode`；心跳把 `current_hostname/current_ip/identity_state/last_error` 写入 `agent_sessions.detail`。
- 新增 Edge IP 绑定接口；不预分配身份（首启心跳分配）。

## 5. 错误处理

- 名称由 Center 保证 ASCII 且 ≤15 字符。
- Agent 动作写 `%ProgramData%\YixueIDV\identity.log`，心跳上报状态。
- 心跳失败/无网：不改身份、不重启。
- Edge 绑定拉取失败用旧缓存；无缓存回退地址池。
- 重复保留 IP 告警并只保留第一个。
- 换网卡/重装：新 MAC → 新身份；旧记录保留可手工清理。

## 6. 测试

- 单测：MAC 解析/回退、`decide()` 状态迁移、DHCP 保留匹配与池跳过。
- 集成：本机 Center+Edge，`dhcp_probe` 验证保留 IP；模拟 driver/heartbeat 跑状态机。
- VM 验收：首启收敛、连续重启身份不变、还原开启后不变、换 MAC、`static` 兼容。

## 7. 交付物

Agent、Center、Edge、客户端包/脚本、验收脚本、`docs/modules/18-批量身份与DHCP保留.md`、设计/计划文档。
