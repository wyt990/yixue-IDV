# 模块 22 — 维护任务：批量自动更新客户端 Agent

> 需求：在“维护任务”中批量把客户端 `idv-agent` 升级到新版本。
> 客户端如处于还原状态，要先解除还原，下载新程序、替换自身、重启服务；
> 新版本上线后自动恢复还原策略并重启（回到冻结状态）。

---

## 1. 总体流程

```text
管理端（维护任务 → 更新 Agent）
   │ 上传新版 idv-agent.exe（版本号必须与二进制 CARGO_PKG_VERSION 一致）
   │ 创建 agent_update 任务 → 给目标 MAC 入队 update_agent 命令
   ▼
客户端 Agent（常驻服务）
   ├─ 当前处于还原/armed？
   │     ├─ 是：protect_off() 解除还原 → 重启（不下载，冻结卷写盘会丢）
   │     └─ 否：下载 exe → 校验 sha256 → 写临时目录 → 启动 PowerShell 更新脚本
   │
   ▼ 重启后 / 更新脚本
PowerShell updater（SYSTEM / 管理员）
   ├─ 停止 idv-agent 服务（或 YixueIdvAgentDaemon 计划任务）
   ├─ 等待进程退出
   ├─ 用新 exe 覆盖旧 exe（失败重试）
   ├─ 重新启动服务/计划任务
   ▼
新 Agent 心跳（版本号变化）
   ├─ Center 把任务目标标 done
   ├─ Agent 按机房模式重新 arm 还原（teaching=所有固定盘 / exam=仅 C）
   └─ 自动重启一次，回到冻结状态
```

两阶段重启是必须的：还原驱动生效时，对系统盘的写入会被重定向，直接替换 exe
会在重启后回滚；所以必须先解除还原并重启，在“解冻态”完成替换。

---

## 2. 服务端

### 2.1 包管理

上传（管理员，raw body；**无需填写版本号**）：

```powershell
Invoke-RestMethod -Method Put `
  -Uri http://10.110.0.1:8080/api/v1/agent/packages `
  -Headers @{Authorization="Bearer $token"} `
  -ContentType application/octet-stream `
  -InFile .\idv-agent.exe
```

- Center 从 exe 内嵌标记 `IDV_AGENT_BUILD_VERSION=<版本>` 自动识别版本；
- 保存为 `data/agent-packages/agent-<版本>.exe` + `.sha256`；
- 最小 64KB，路由上限 64MB；
- 列表：`GET /api/v1/agent/packages`；
- 下载（Agent 使用，当前无设备令牌）：`GET /api/v1/agent/packages/{version}/download`。

### 2.2 创建更新任务

```powershell
Invoke-RestMethod -Method Post `
  -Uri http://10.110.0.1:8080/api/v1/tasks/agent-update `
  -Headers @{Authorization="Bearer $token"} -ContentType application/json `
  -Body '{"version":"0.2.0","macs":["40:FE:95:B9:FF:A8"]}'
```

- 生成 `maintenance_tasks.kind = "agent_update"`（`image_id` 存版本号）；
- 每个目标写 `maintenance_task_targets` + 入队 `update_agent` 命令；
- 目标状态：`pending/running/done/failed`，进度与 detail 可见；
- 任务列表/取消/导出 CSV 复用现有维护任务逻辑。

### 2.3 心跳驱动重试

Agent 每次心跳（5 秒）会触发：

- 目标版本 == `agent_version` → 目标 `done`，任务聚合为完成；
- 否则把目标置 `running`，并在没有待执行命令、且距上次尝试 >15 秒时
  重新入队 `update_agent`（覆盖“先解除还原并重启”后的第二次执行）。

---

## 3. 客户端

### 3.1 命令处理（`update_agent`）

payload：

```json
{
  "version": "0.2.0",
  "download_url": "/api/v1/agent/packages/0.2.0/download",
  "sha256": "..."
}
```

- 版本相同 → 直接回执 `already_version`；
- `frozen || armed` → `protect_off()` → 回执 `reboot_for_update` → 5 秒后重启；
- 否则：下载 → 大小/sha256 校验 → 写 `C:\ProgramData\YixueIDV\update\` →
  启动隐藏的 PowerShell updater → 回执 `update_scheduled`。

### 3.2 更新脚本

- 位于 `C:\ProgramData\YixueIDV\update\apply-update.ps1`；
- 停止 `idv-agent` 服务或 `YixueIdvAgentDaemon` 计划任务；
- 等待进程退出，`Copy-Item` 覆盖当前 exe（最多重试 20 次）；
- 重启服务（失败重试 10 次）或计划任务；
- 日志：`C:\ProgramData\YixueIDV\update\update.log`。

### 3.3 更新后恢复还原

新 Agent 第一次心跳会走身份收敛：

- 若机房模式要求冻结（teaching/exam），当前未 frozen/armed；
- 身份状态机返回 `ArmReboot` → 重新 arm 对应范围 → 重启；
- 重启后回到“新版本 + 冻结”状态。

---

## 4. 版本号自动识别（重要）

客户端 `agent_version` 来自 `env!("CARGO_PKG_VERSION")`，同时构建时会在二进制中
嵌入标记：

```text
IDV_AGENT_BUILD_VERSION=0.2.0
```

Center 上传 exe 时自动扫描该标记得到版本号，并以此命名包、下发给客户端。
因此：

1. 发布新版时只需修改 `crates/idv-agent/Cargo.toml` 的 `version` 并重新构建；
2. 上传时**不需要手动填写版本号**（填了也不会使用）；
3. 客户端心跳的 `agent_version` 与包内标记一致，Center 才能正确判断更新完成；
4. 如果上传的 exe 找不到标记（例如 0.1.0 旧包），Center 会拒绝并提示。

当前版本：`0.3.3`（旧客户端 `0.1.0` / `0.2.0` / `0.3.0` / `0.3.1`）。

### 4.1 首次启用（必须手动一次）

`0.1.0` 的 Agent 还不认识 `update_agent` 命令，无法自举。第一次升级请手动做：

```powershell
# 客户端管理员 PowerShell
Stop-Service idv-agent
Copy-Item .\idv-agent.exe "C:\Program Files\YixueIDV\idv-agent.exe" -Force
Start-Service idv-agent
Get-Service idv-agent
```

（计划任务模式：`schtasks /End /TN YixueIdvAgentDaemon` → 覆盖 → `schtasks /Run /TN YixueIdvAgentDaemon`）

只要客户端到 `0.2.0+`，后续版本都可以用维护任务自动升级。

### 4.2 回滚

更新脚本在覆盖前会把旧 exe 备份为 `idv-agent.exe.bak`；如果新版本启动失败会
自动回滚。仍可手动回滚：

```powershell
Stop-Service idv-agent
Copy-Item "C:\Program Files\YixueIDV\idv-agent.exe.bak" `
  "C:\Program Files\YixueIDV\idv-agent.exe" -Force
Start-Service idv-agent
```

### 4.3 0.2.0 更新卡住时的手动恢复

0.2.0 的 updater 是服务的子进程，服务停止时可能被一起结束，表现为命令一直
`update_scheduled` 但版本不变。最后手动升一次 0.3.0：

```powershell
# 客户端管理员 PowerShell
$dst = "C:\Program Files\YixueIDV\idv-agent.exe"
Invoke-WebRequest "http://10.110.0.1:8080/api/v1/agent/packages/0.3.3/download" `
  -OutFile "$env:ProgramData\YixueIDV\idv-agent-0.3.0.exe"
Stop-Service idv-agent -Force -ErrorAction SilentlyContinue
Copy-Item "$env:ProgramData\YixueIDV\idv-agent-0.3.0.exe" $dst -Force
Start-Service idv-agent
Get-Service idv-agent
Get-Content "C:\ProgramData\YixueIDV\update\update.log" -Tail 30
```

计划任务模式：

```powershell
schtasks /End /TN YixueIdvAgentDaemon
Copy-Item ... $dst -Force
schtasks /Run /TN YixueIdvAgentDaemon
```

0.3.0 起 updater 通过 **Task Scheduler(SYSTEM)** 运行，不再随 Agent 服务停止被杀。

---

## 5. 操作步骤

1. Center 重启（应用迁移/新路由），确认能访问 `data/agent-packages`；
2. 构建新版 Agent：
   ```powershell
   cd H:\AI\idv_rust
   cargo build -p idv-agent --release
   ```
3. 维护任务页 → 「更新 Agent」→ 选择目标客户端（可按校区/机房筛选全选）→
   选择新版 exe（版本自动识别）→ 下发；
4. 观察任务列表：目标依次 `running`（或重启中）→ `done`；
5. 客户端验证：
   ```powershell
   Get-Service idv-agent
   (Get-Item "C:\Program Files\YixueIDV\idv-agent.exe").LastWriteTime
   Get-Content "C:\ProgramData\YixueIDV\update\update.log" -Tail 20
   ```
   终端页「在线」列 + 还原状态应正常；**「Agent 版本」列**会显示心跳上报的版本，
   可据此快速找出仍是旧版本的机器。

---

## 6. 失败处理

| 现象 | 排查 |
|---|---|
| 任务一直 running，客户端不重启 | Agent 未以服务/计划任务运行；看 `update.log`、agent 日志 |
| 命令反复 `update_scheduled` 但版本仍不变 | 0.2.0 updater 作为服务子进程可能随服务停止被杀；0.3.0 改为 Task Scheduler(SYSTEM) 启动，需手动升一次 |
| 回执 `download ...` 失败 | Center 下载地址/防火墙；`center_url` 必须客户端可达 |
| 回执 `sha256 mismatch` | 上传包与生成的 sha 不一致，重新上传 |
| 更新后仍显示旧版本 | 版本号未改二进制 `CARGO_PKG_VERSION`；或服务未重启成功 |
| 更新后未恢复还原 | 检查机房模式/`protect_scope`；客户端身份收敛日志 |
| 更新脚本复制失败 | 文件被占用/权限；`update.log` 会记录 retry |

安全说明：Agent 下载接口目前按 MAC 体系、无设备令牌，仅适合受控机房网络；
后续应加设备令牌/签名包校验。
