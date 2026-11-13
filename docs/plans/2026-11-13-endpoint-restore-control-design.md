# 终端还原引擎 + 管控能力总线（设计）

> 2026-11-13 · 已确认  
> 参考：`temp/SysRestoreDriver`（卷过滤写时重定向）、`temp/PCFreeze`（冰点式产品语义）、模块 06/15

## 1. 定位

| 层 | 职责 |
|----|------|
| Center | 策略与任务源、审计；日后可加推送通道 |
| Edge | 镜像/大文件；策略仍以 Center 为准 |
| Agent | 拉策略、执行指令、上报；挂载各 Capability |
| 还原引擎 | 卷过滤驱动（SysRestoreDriver 路线）；可换 UWF |
| 封板 | 现有全量 PXE 部署；**不做开机镜像写回** |

策略通道：**先心跳**，协议预留长连/推送，不绑死传输。

## 2. 本刀范围（还原）

### 做

1. 封装卷过滤重启还原（装驱动、选卷、开/关保护；技术参考 `temp/SysRestoreDriver`）
2. Agent 配置 Center 地址；开机后心跳拉取 ModePolicy  
   - `teaching` ≈ 冻结（保护开）  
   - `office` ≈ 解冻  
   - `exam` 可叠加 USB 等既有标志  
3. 策略变更：本地 `pending` → **重启后生效**
4. 保护点：封板部署后开启保护；换镜像重部署后再开保护
5. 默认保护 `system` 卷；`persistent` 默认不保护（可配）
6. 管控 UI：工作模式绑定 + 机房设置「开机还原」勾选（映射 teaching/office）
7. 心跳扩展：`capabilities[]`、`desired_config_revision` / `applied_revision`、`pending_reboot`

### 不做

- 开机镜像写回 / 日常依赖 PE 还原  
- WebSocket 长连落地（只留形状）  
- 驱动 EV 量产签名流程（试点测试签名）  
- 协助真画面、硬件/网址等能力实现（仅规划）

### 风险

内核蓝屏/WinRE、签名、BitLocker/杀软兼容 → 试点限虚拟机 + 可重装真机。

## 3. 能力总线（规划，本刀不实现）

Agent = 插件式能力总线；同一套「策略文档 + revision + 心跳/日后长连」。

### 3.1 常驻策略类 Capability

| id | 含义 |
|----|------|
| `restore` | 冻结/解冻/待重启（**本刀**） |
| `remote_assist` | 观看 / 控制（现有信令 → 真流媒体） |
| `device` | USB/摄像头等硬件 |
| `network` | 防火墙/代理/限速 |
| `process` | 进程黑白名单 |
| `webfilter` | 网址/域名管控 |

统一字段：`enabled`、`revision`、`apply: immediate | next_reboot`。  
Agent 上报：`applied_revision`、`pending_reboot`、`capability_status`。

### 3.2 即时指令类 AgentJob

| kind | 含义 |
|------|------|
| `open_url` | 打开网址（默认可限白名单） |
| `run_program` | 运行程序（路径/参数；建议哈希校验） |
| `deploy_files` | 分发文件到**可指定目录**（经 Edge；覆盖策略可配） |
| `collect_files` | 从**可指定目录**收集上传（大小/扩展名限制；收卷） |

与常驻策略分通道，避免塞进 ModePolicy。可与上课锁/考试模式联动（如考试禁 `run_program`）。

### 3.3 分期建议

1. **本刀**：restore 驱动 + 心跳策略生效  
2. **下一刀**：~~机房勾选还原~~（已做）；远程协助观看优先  
3. **再后**：`deploy_files` / `collect_files` / `open_url` → `device` / `process` / `webfilter` → `network`；长连推送

## 4. 与现有代码的衔接

- ModePolicy / `lab_mode_bindings` / agent heartbeat：已有，扩展字段即可  
- 模块 15 协助会话：保留信令，媒体通道后接 `remote_assist`  
- USB 策略：并入 `device`  
- 软件分发骨架：可演化为 `deploy_files` 的一种包形态  

## 5. 验收（本刀）

- [ ] 测试签名下驱动可装、可对 system 卷开保护  
- [ ] 保护开启后改系统区文件，重启后恢复  
- [ ] persistent（若未保护）数据仍在  
- [ ] Center 改绑定 teaching↔office 后，Agent 心跳得 pending，重启后冻结状态符合策略  
- [ ] 心跳可见 `applied_revision` / `pending_reboot`  
- [ ] 文档标明：非镜像写回；长连与其它 Capability 为规划  

## 6. 非目标回顾

用户已否决「C：开机镜像写回」；日常还原不依赖 PE；全量部署仅用于封板/换金样。
