# 终端卷过滤还原 + 心跳策略 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 试点机上用卷过滤驱动实现开机还原，并由 Center ModePolicy 经 Agent 心跳下发冻结/解冻（重启生效）。

**Architecture:** 全量 PXE 仅封板；日常还原走 SysRestoreDriver 风格卷过滤。`idv-agent` 管驱动生命周期与策略应用；心跳扩展 revision / pending_reboot；其它管控能力只留协议字段。

**Tech Stack:** Rust (`idv-agent`, `idv-center`, `idv-common`)、Windows 驱动（C/WDK，参考 `temp/SysRestoreDriver`）、现有 ModePolicy / heartbeat API、SQLite。

**Design:** [2026-11-13-endpoint-restore-control-design.md](./2026-11-13-endpoint-restore-control-design.md)

## Global Constraints

- 不做开机镜像写回、不依赖 PE 做日常还原  
- 策略通道先心跳；长连仅预留字段  
- 本刀不实现协助真流、硬件/网址/分发收集等（协议可占位）  
- 试点允许 `bcdedit` 测试签名；不宣称量产签名完成  
- 用户可见文案用中文；代码标识英文  

---

## File map

| Path | Role |
|------|------|
| `crates/idv-common/src/endpoint_policy.rs` (new) | DesiredState / Capability / Job kind 共享 JSON 形状 |
| `crates/idv-center/src/api/agent.rs` | heartbeat 响应扩展 revision、restore 指令、capabilities 占位 |
| `crates/idv-center/migrations/*` | 如需 `policy_revision` 计数则迁移；尽量先用 mode 绑定更新时间 |
| `crates/idv-agent/src/*` | 心跳循环、pending 策略、调用驱动控制面 |
| `crates/idv-agent/src/restore_driver.rs` (new) | 安装/开保护/关保护/查状态（IOCTL 或包装 UI 工具） |
| `third_party/sys-restore-driver/` 或 `drivers/sys-restore/` | 从 temp 引入可构建的驱动树（保留 NOTICE/LICENSE） |
| `docs/modules/06-还原策略与多模式.md` | 补充「驱动还原 vs 镜像写回」与本刀验收 |
| `deploy/scripts/stage-restore-driver-accept.ps1` (new) | 虚拟机/测试签名冒烟 |

---

### Task 1: 共享策略文档模型

**Files:**
- Create: `crates/idv-common/src/endpoint_policy.rs`
- Modify: `crates/idv-common/src/lib.rs`

- [ ] Step 1: 定义并单测序列化：

```rust
pub struct DesiredEndpointState {
    pub revision: u64,
    pub restore: RestoreCapability,
    pub capabilities: Vec<String>, // 宣告支持的 id，占位
}

pub struct RestoreCapability {
    pub enabled: bool,
    pub apply: ApplyWhen, // Immediate | NextReboot
    pub protect_volumes: Vec<String>, // e.g. ["system"]
    pub frozen: bool, // true=应处于保护开
}
```

- [ ] Step 2: `cargo test -p idv-common endpoint_policy`
- [ ] Step 3: Commit `feat(common): endpoint DesiredState for restore control`

---

### Task 2: Center 心跳下发 DesiredState

**Files:**
- Modify: `crates/idv-center/src/api/agent.rs`
- Modify: models / heartbeat JSON as needed

- [ ] Step 1: 从 lab 的 ModePolicy 映射：`restore=reboot|exam` → `frozen=true`；`off|manual` → `frozen=false`（manual 细节可先等同 office 或仅手动 IOCTL）
- [ ] Step 2: `revision` = 绑定更新时间戳或单调计数
- [ ] Step 3: 响应带上 `desired_state`；保留原 `should_restore` 字段一段时间以免断旧客户端
- [ ] Step 4: 扩展 agent 上报：`applied_revision`、`pending_reboot`、`driver_status`
- [ ] Step 5: 手工或脚本打 heartbeat 看 JSON
- [ ] Step 6: Commit `feat(center): heartbeat DesiredState for freeze policy`

---

### Task 3: 引入驱动工程（可构建副本）

**Files:**
- Create: `drivers/sys-restore/`（从 `temp/SysRestoreDriver` 复制并保留 Apache NOTICE）
- Modify: `.gitignore`（忽略 `.sys` 构建产物若过大）
- Docs: 构建依赖 VS+WDK、测试签名步骤

- [ ] Step 1: 复制驱动+精简说明 `drivers/sys-restore/README.md`
- [ ] Step 2: 文档化 `bcdedit /set testsigning on` 与加载步骤
- [ ] Step 3: Commit `chore(drivers): vendor sys-restore volume filter (Apache-2.0)`

---

### Task 4: Agent 驱动控制面

**Files:**
- Create: `crates/idv-agent/src/restore_driver.rs`
- Modify: `crates/idv-agent/src/main.rs`

- [ ] Step 1: 封装：`status` / `install` / `protect_on(volumes)` / `protect_off`（先调已有 QHEngine 同类 IOCTL 或打包的 helper；接口稳定）
- [ ] Step 2: 单元测试用 mock trait，不加载真驱动
- [ ] Step 3: CLI：`idv-agent restore-driver status|protect-on|protect-off`
- [ ] Step 4: Commit `feat(agent): restore-driver control surface`

---

### Task 5: Agent 心跳应用策略（重启生效）

**Files:**
- Modify: `crates/idv-agent/src/main.rs`（或 `policy.rs`）

- [ ] Step 1: 本地存 `applied_revision` + `pending_desired.json`
- [ ] Step 2: 心跳若 `revision` 更新：写入 pending，设 `pending_reboot=true`，**不立刻改冻结态**（除非策略标明 Immediate 且安全）
- [ ] Step 3: 开机启动时：若有 pending，调用驱动 protect on/off，再 clear pending，上报 applied
- [ ] Step 4: 与 teaching/office 映射联调（可用模拟驱动后端）
- [ ] Step 5: Commit `feat(agent): apply freeze policy after reboot via heartbeat`

---

### Task 6: 文档与验收脚本

**Files:**
- Modify: `docs/modules/06-还原策略与多模式.md`
- Create: `deploy/scripts/stage-restore-driver-accept.ps1`
- Optionally touch: `docs/modules/08-补充能力与需求池.md`（链到能力总线规划）

- [ ] Step 1: 模块 06 增加「驱动还原本刀 / 非镜像写回 / 能力总线指针」
- [ ] Step 2: accept 脚本：心跳 JSON 含 desired_state；若无驱动环境则 skip 驱动项并打印 SKIP
- [ ] Step 3: Commit `docs: restore-driver slice acceptance and module 06`

---

### Task 7: 真机/VM 手工验收清单（非自动化关门）

- [ ] 测试签名 VM：开保护 → 改桌面/系统文件 → 重启 → 恢复  
- [ ] persistent 未保护数据仍在  
- [ ] Center 切 office → 重启后可解冻写入并保持  
- [ ] 切回 teaching → 重启后恢复保护点  

---

## 后续计划（不在本 plan 实现）

- ~~机房设置勾选还原（bind teaching/office）~~（已做：LabsPage 勾选）  
- WebSocket 推送  
- `open_url` / `run_program` / `deploy_files` / `collect_files`  
- `remote_assist` 真流、device/process/webfilter/network  
- EV 签名与量产部署包  

---

## 执行方式

实现时用 **subagent-driven-development** 或 **executing-plans**，按 Task 1→7 顺序；每 Task 提交一次。驱动签名环境缺失时 Task 3–4 以接口 + mock 验收，Task 7 人工勾选。
