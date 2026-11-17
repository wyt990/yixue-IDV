# 模块 19 — 常驻 Agent 与即时命令通道

> 目标：课堂场景下，重启/关机/改还原策略/远程执行命令必须在 **1~2 秒** 内到达客户端，
> 客户端状态（在线、还原冻结状态）在 **5 秒** 内反映到终端页。
> 替代原「一次性 CLI + 计划任务轮询（1~2 分钟）」模式。

---

## 1. 结论摘要

| 能力 | 实现 |
|------|------|
| 常驻 | `idv-agent service` 注册为 Windows 服务（`idv-agent`），失败自动回退 SYSTEM 计划任务守护 |
| 心跳 | 常驻循环每 5 秒 `POST /api/v1/agent/heartbeat`（可在 `agent.toml` 配 `heartbeat_secs`） |
| 即时命令 | `GET /api/v1/agent/commands/next?mac=..&wait=20` 长轮询；命令入队即唤醒，实测 <1 秒 |
| 命令类型 | `apply_policy` / `sync_policy` / `reboot` / `shutdown` / `open_url` / `run_program` / `restore_now` / `assist_ack` |
| 在线状态 | 终端页「在线/离线」列：`agent_sessions.last_seen ≤ 15s` |
| 还原状态 | 终端页「还原状态」列：已冻结 / 未冻结 / 待重启生效 + 当前工作模式 |
| 状态刷新 | 终端页每 5 秒自动刷新 |

---

## 2. 服务端

### 2.1 表与接口

- 迁移 `019_agent_commands.sql`：
  - `agent_commands(id, mac, kind, payload, state, created_by, created_at, delivered_at, finished_at, error_message)`
  - `agent_sessions` 增加 `frozen / armed / mode_name / current_hostname / identity_state / last_error`
- 接口：
  | 方法 | 路径 | 说明 |
  |---|---|---|
  | GET | `/api/v1/agent/commands/next?mac=&wait=20` | Agent 长轮询；最多挂 30s |
  | POST | `/api/v1/agent/commands/{id}/report` | Agent 回执 `{ok, detail}` |
  | POST | `/api/v1/clients/{mac}/commands` | 管理端/脚本下发命令 `{kind, payload}` |
- 唤醒：`AppState.agent_notify`（`tokio::sync::Notify`），入队即 `notify_waiters()`，不需要轮询间隔。
- 在线/还原：`GET /api/v1/clients` 改为 LEFT JOIN `agent_sessions`，返回
  `online / agent_last_seen / frozen / armed / mode_name / current_hostname / identity_state`。

### 2.2 既有操作已接入即时通道

- 重启/关机：`POST /api/v1/power/commands`（单个/批量）→ 同时入队 `reboot`/`shutdown` 命令；
- 切换工作模式：`POST /api/v1/labs/{id}/mode` → 入队 `apply_policy`；模式变更还会排队重启；
- 软件分发/协助仍走心跳（5 秒内到达），后续可继续迁入命令通道。

---

## 3. 客户端

### 3.1 服务安装

```powershell
cd "C:\Program Files\YixueIDV"
.\scripts\install-agent-service.ps1
# 或指定 Center：
.\scripts\install-agent-service.ps1 -CenterUrl http://10.110.0.1:8080
```

脚本会：
1. 生成 `agent.toml`（如缺失）、清理旧的 `YixueIdvAgentBoot` / `YixueIdvAgentPoll` 任务；
2. `New-Service idv-agent`（自动启动 + 失败自动重启：5s/10s/30s）；
3. 服务启动失败时回退：注册 `YixueIdvAgentDaemon` 计划任务（SYSTEM、开机启动、失败每分钟重启）。

验证：

```powershell
Get-Service idv-agent
Get-Content "C:\ProgramData\YixueIDV\agent.log" -Tail 20   # 若配置了文件日志
```

### 3.2 运行逻辑

```text
service/daemon
 ├─ heartbeat loop：每 5s 心跳 + 策略/身份收敛（apply_policy 也走这里）
 └─ command loop：长轮询 next(wait=20) → 立即执行 → 回执
```

- `reboot`/`shutdown`：先回执，再 `shutdown.exe /r|/s /t 5`；
- `open_url`：`cmd /c start "" "<url>"`；
- `run_program`：`{ "path": "...", "args": [...] }` → 启动进程；
- 两者优先通过 `CreateProcessAsUser` 在登录用户桌面启动，程序/网页会显示在当前用户会话；无登录会话时回退服务会话；
- 执行结果不回传 stdout/退出码，仅回执已下发。

### 3.1 常用命令预置

管理端终端页勾选一台终端 →「命令」下拉，分别进入 **执行命令 / 打开网页 / 运行程序** 模态框；
每个模态框可列出、新增、编辑、删除常用项，点「执行」即可下发，无需重复输入。

预置项存 Center（迁移 `022_command_presets.sql`，`/api/v1/command-presets`），多管理员共享。
- 执行失败会回执 `ok=false + detail`，网页/日志可查。

---

## 4. 操作与验收

1. 客户端安装服务后，终端页该 MAC 应在 5 秒内显示 **在线**；
2. 管理端改机房工作模式 → 客户端 ≤2 秒收到 `apply_policy` 并按策略处理（需要时重启）；
3. 管理端点重启/关机 → ≤2 秒执行；
4. 用命令通道开网页/执行程序（API 方式）：
   ```powershell
   $t = (Invoke-RestMethod -Method Post -Uri http://10.110.0.1:8080/api/v1/auth/login `
     -ContentType application/json -Body '{"username":"admin","password":"admin123"}').token
   Invoke-RestMethod -Method Post -Uri "http://10.110.0.1:8080/api/v1/clients/40:FE:95:B9:FF:A8/commands" `
     -Headers @{Authorization="Bearer $t"} -ContentType application/json `
     -Body '{"kind":"open_url","payload":"https://www.example.com"}'
   ```
5. 还原状态列应显示：已冻结 / 未冻结 / 待重启生效（含当前模式名）。

---

## 5. 安全与容量

- 当前 Agent 接口仍按 MAC 识别（无设备令牌），**只能部署在受控机房网络**；下一步应加设备令牌与命令签名；
- `run_program`/`open_url` 具备任意命令执行能力，建议后续加白名单与审计；
- 500 台 × 5s 心跳 ≈ 100 写/秒：Center 的 SQLite 需开 WAL；如压力大可将 `heartbeat_secs` 调到 10 秒（命令仍即时）；
- 长轮询每客户端 1 条挂起连接，500 条约 500 个 HTTP 连接，axum/tokio 可承载；建议反代限制单 IP 连接数。

---

## 6. 兼容与回退

- 旧客户端（计划任务轮询）仍可用：服务端命令表只对长轮询客户端即时生效，旧客户端继续走心跳 `pending_*`；
- 不想用服务时：`install-agent-service.ps1 -FallbackTaskOnly`；
- 卸载服务：`sc.exe stop idv-agent; sc.exe delete idv-agent`（或删除对应计划任务）。

## 3.2 网络超时与容错

Agent 的普通 HTTP 请求（心跳、回执、普通上报）统一：

- 连接超时：5 秒；
- 总超时：30 秒；
- 长轮询/文件传输/更新下载在请求级单独设置更长超时，不受 30 秒限制。

Center 宕机或网络黑洞时，心跳任务会快速失败并重试，不会无限挂起，也不会触发重启或蓝屏。