# 模块 18 — 批量身份与 DHCP 保留

> 目标：500 台学生机用同一镜像批量部署，客户端不逐台配置；首次开机自动取 MAC、
> 由 Center 分配计算机名与 IP，并**在还原开启的情况下保持身份正确**。
> 设计/计划：[`docs/plans/2026-11-16-batch-identity-provisioning-design.md`](../plans/2026-11-16-batch-identity-provisioning-design.md)

---

## 1. 结论摘要

| 能力 | 实现方式 |
|------|----------|
| 不写 MAC | Agent 自动取“到 Center 路由网卡”的 MAC（`netinfo.rs`） |
| 不写计算机名 | Center 按机房 `name_prefix/suffix_width/next_suffix` 生成 `stu0NN`，Agent `Rename-Computer` |
| 不写 IP | 机房 `ip_mode=dhcp`：Edge DHCP 按 MAC 下发 Center 分配的 `assigned_ip`；`static` 模式保留旧行为 |
| 还原开启仍保持 | 首启身份状态机：先关保护 → 改名/配网 → 再开保护；冻结基线里就是本机身份 |
| 批量安装 | 金样机预装 Agent + 开机/轮询任务；客户端配置全通用（只写 `center_url`） |
| 可观测 | 心跳上报 `current_hostname / identity_state / last_error`，落到 `agent_sessions.detail` |

---

## 2. 客户端配置（一台机都不用改）

客户端包内 `agent.toml.example` 即通用模板：

```toml
center_url = "http://10.110.0.1:8080"
# mac = ""      # 留空/auto = 自动探测
# hostname = "" # 留空 = 当前计算机名
log = "info,idv_agent=info"
```

`install-client.ps1` 发现没有 `agent.toml` 会自动复制模板；`prepare-golden-client.ps1`
会一次完成：生成配置、安装 Agent、注册开机任务（`sync-policy --apply`）与轮询任务、关闭还原保护、清理旧身份状态。

---

## 3. 首启身份收敛状态机

每次 boot/poll 执行一次，**一次最多一个动作、最多一次重启**：

| 当前状态 | 动作 |
|----------|------|
| 主机名不一致 + 卷在保护中 | 写 `_qh_protect_state.data=0` → 重启 |
| 主机名不一致 + 未保护 | 设 IP（仅 static）→ 改名 → 重启 |
| 仅 IP 不一致（static） | 设静态 IP，不重启 |
| 身份一致 + 未保护 + 策略要求冻结 | 写状态文件=1 → 重启（基线即正确身份） |
| 身份一致 + 已冻结 | 完成 |

- 镜像保护关闭时：首启 **2 次重启** 内收敛；镜像已 armed 时 3 次。
- 动作失败不重启，写 `%ProgramData%\YixueIDV\identity-state.json` 并上报；下一次 poll 重试。
- 冻结后静态 IP 会随还原回退，这是 `ip_mode=static` 的固有限制；批量场景推荐 `dhcp`。

---

## 4. Edge DHCP 保留

- Center：`GET /api/v1/edge/ip-bindings`（Edge 令牌鉴权）返回本 Edge 关联机房
  （`lab_edges`，回退 `campus_edges`）中 `ip_mode=dhcp` 的 `mac → assigned_ip`。
- Edge：启动读缓存 `data/edge-cache/ip-bindings.json`；每 30s 刷新并落盘；
  DHCP 命中保留 MAC 直接 Offer/ACK 保留 IP，地址池跳过保留 IP；非本子网保留忽略。
- Agent（`ip_mode=dhcp`）：不设静态 IP；当前 IP ≠ `assigned_ip` 时按 5 分钟节流执行
  `ipconfig /release` + `/renew`，拿到 Edge 保留地址。

---

## 5. 机房配置

管理端「机房设置」新增 **IP 分配方式**：

- **DHCP 保留（批量部署推荐）**：`ip_mode=dhcp`，Edge 必须开启 DHCP，机房绑定到该 Edge；
- **客户端静态 IP**：`ip_mode=static`，兼容旧部署；Agent 每次开机重设静态 IP。

---

## 6. 验收

1. 金样机执行 `deploy/scripts/prepare-golden-client.ps1 -CenterUrl http://<center>:8080`
2. 采集/部署镜像 → 目标机首启（2~3 次自动重启）后：
   - `hostname` = `stu0NN`，IP = Center 的 `assigned_ip`；
   - 终端页出现该 MAC 与名称，「最近上报」持续更新；
   - 重启 2 次、以及还原开启后再重启，身份保持不变。
3. 可用 `deploy/scripts/identity-accept.ps1 -Mac <MAC> -ExpectName stu001 -ExpectIp <IP>`
   做只读校验（台账、Agent 会话、最近心跳、ip_mode）。

---

## 7. 排障

| 现象 | 检查 |
|------|------|
| 终端页没有该 MAC | 客户端能否访问 `http://<center>:8080/health`；Center 是否配置默认机房；Agent 是否以 SYSTEM 计划任务运行 |
| 名字/ IP 不收敛 | `%ProgramData%\YixueIDV\identity-state.json` 的 `state/last_error`；Center `agent_sessions.detail`；驱动是否安装、保护状态 |
| 一直重启 | 看是否改名失败（PowerShell 权限/名称非法 ≤15 字符）；动作失败不会重启，正常收敛是有限步 |
| DHCP 没下发保留 IP | 机房是否 `ip_mode=dhcp`；机房是否绑定该 Edge；Edge 日志是否刷新到 bindings；客户端执行 `ipconfig /renew` |
| 改了 MAC/换网卡 | 会登记成新终端（新名字/IP），旧记录可手工删除 |

---

## 8. 分期

- 一期：本模块（自动 MAC + 首启收敛 + DHCP 保留）。
- 后续：PE 部署时离线写身份（首启零重启）、驱动身份排除、终端页在线状态列。
