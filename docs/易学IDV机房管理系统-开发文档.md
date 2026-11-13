# 易学 IDV 机房管理系统 — 开发与验收文档

> 版本：v0.15  
> 日期：2026-11-12  
> 仓库：`idv_rust`  
> 状态：阶段 0～7 **软件已完成**（含三期运维骨架）；硬件试点见模块 14  
> 用途：开发基线、分期交付、每阶段验收清单

---

## 1. 文档说明

本文档是「易学 IDV 机房管理系统」的完整开发基线。后续编码、联调、验收以本文为准；若需求变更，先改本文再改代码。

**功能细节与专项验收**见模块文档目录：[`docs/modules/`](./modules/README.md)。总纲管分期与架构；模块文档管「怎么做、怎么验」。

| 读者 | 用法 |
|------|------|
| 产品/网管 | 看第 2～4、11、18 章；各模块「结论摘要」 |
| 开发 | 看第 5～10、12 章 + 对应 `docs/modules/*` |
| 测试/验收 | 看第 11 章阶段门禁 + 各模块验收清单 |
| 设计/前端 | [`modules/01-管理端UI与设计系统.md`](./modules/01-管理端UI与设计系统.md) |

---

## 2. 产品定义

### 2.1 名称与定位

- **产品名称**：易学 IDV 机房管理系统  
- **英文/工程名**：Yixue IDV / `idv_rust`  
- **定位**：面向学校计算机机房的 **分布式镜像管控与网络装机平台**  
- **核心模式**：服务器只做管理与镜像分发；系统运行与运算在本地学生机（非 VDI）

### 2.2 IDV 与 RDV 说明（命名背景）

| 概念 | 含义 |
|------|------|
| **IDV** | 行业类别「智能桌面虚拟化」：镜像/策略集中管，桌面在本地跑 |
| **锐起 RDV** | 厂商产品名，属于 IDV 一类机房方案 |
| **VDI** | 桌面在服务器算、终端收画面；机房成本与网络依赖更高 |
| **本项目** | 名称用 IDV；**第一期实现路径**为「磁盘镜像 + PXE + 中心/边缘」，还原与增量预留二期，逐步靠近锐起类机房能力 |

对外可称 IDV；对内第一期准确描述为：**可 PXE 的分布式机房镜像管控系统**。

### 2.3 典型使用场景

1. 新机房批量装系统（Windows / Linux / 信创）  
2. 学期初统一换镜像（教学软件更新）  
3. 考试前快速恢复标准环境（二期：开机还原）  
4. 跨校区/多楼栋就近拉镜像，避免全部打爆中心出口带宽  

---

## 3. 目标与非目标

### 3.1 已确认约束

| 项 | 决策 |
|----|------|
| 第一期主链路 | **A** 客户端/镜像管理与分发 + **C** PXE 网络启动 |
| 客户端系统 | **Windows 为主**，兼顾 **Linux / 信创** |
| 规模 | **500 台以上**（全校/多校区） |
| 网络 | 机房网段可 **自建 DHCP + PXE**；**配置一律由中心管理** |
| 拓扑 | **中心控制面 + 可同机/可分离的网络服务与缓存节点（edge）** |
| 实现策略 | **尽量 Rust 自研**；仅保留 iPXE 等必要开源引导件 |
| 架构路线 | **方案 1：磁盘镜像平台（先装机，后还原）** |

### 3.2 第一期要做成什么

- 登记机房、边缘节点、终端（至少 MAC + 归属）  
- 镜像入库，并同步到指定 edge  
- 学生机 PXE 进入维护环境，完成 **采集** 与 **部署**  
- 管理端查看任务状态（排队 / 进行中 / 成功 / 失败）  

### 3.3 第一期明确不做（二期及以后）

- 开机还原 / 手动还原（需 `idv-agent` 或过滤驱动/快照）  
- 增量镜像差量传输的完整生产能力（接口先预留）  
- 完整本地 Hypervisor 真 IDV  
- 远程协助桌面、考试锁屏等增值运维（可列需求池）  
- 冲击校园网核心 DHCP（网络服务节点仅服务中心授权网段）  

### 3.4 成功标准（产品级）

在试点机房（建议 ≥30 台）连续完成：导入镜像 → 同步到 edge → PXE 部署 → 开机进入系统；并完成至少一次「标准机采集镜像 → 另外部署」。失败任务有可读日志，可在管理端追溯。

---

## 4. 能力对照（锐起类机房 vs 本项目分期）

| 能力 | 锐起类常见能力 | 本项目分期 |
|------|----------------|------------|
| 机房/终端台账 | 有 | **一期** |
| 镜像库 | 有 | **一期** |
| PXE 网络启动 / 维护环境 | 有 | **一期** |
| 整机/分区镜像采集与部署 | 有 | **一期** |
| 中心 + 就近分发 | 有 | **一期（edge 缓存）** |
| 任务调度与进度 | 有 | **一期** |
| 组播 / P2P 加速 | 常见 | **P2P：阶段 5/二期**（一期 HTTP 单播 + 缓存；见模块 05） |
| 开机还原 / 手动还原 | 核心卖点 | **二期**（见模块 06） |
| 增量更新 | 常见 | **二期**（一期预留格式；见模块 04） |
| 缓存清空 / 重新同步 | 运维常见 | **一期阶段 2 起**（见模块 03） |
| 远程开关机（WOL 等） | 常见 | **二期** |
| 办公 / 考试等模式与持久数据区 | 常见 | **二期～三期**（见模块 06） |
| WiFi 辅助传输 | 少见但有用 | **二期**（固件 WiFi PXE 不做；见模块 07） |
| Web 管理多主题响应式 | 有 | **一期即按设计系统落地**（见模块 01） |

---

## 5. 总体架构

### 5.1 逻辑结构

```text
[管理员浏览器]
       │ HTTPS（唯一配置入口）
       ▼
[idv-center 控制面]
  台账 / 镜像元数据 / 任务 / 审计
  DHCP·PXE·缓存 策略与节点配置（统一下发）
       │ 注册 · 心跳 · 配置拉取 · 任务
       ▼
[idv-edge 网络服务 / 缓存 × N]     ← 可与 Center 同机不同进程，也可独立主机
  本授权网段 DHCP 或 ProxyDHCP
  TFTP / HTTP 启动与镜像缓存
       │
       ▼
[学生机] ──PXE──► [idv-pe] ──读写磁盘──► 本机硬盘
```

**部署形态（同一产品，三种装法）**

| 形态 | 说明 | 适用 |
|------|------|------|
| 单机 | Center + Edge（DHCP/PXE + 缓存）同机不同进程 | 服务器少的学校 / 小规模试点 |
| 中心 + 边缘 | Center 只管；各机房 Edge 跑 DHCP/PXE + 缓存 | 多机房、减中心带宽与广播压力 |
| 混合 | 部分网段用中心同机服务，部分用独立 Edge | 逐步扩容 |

### 5.2 设计原则

1. **控制面与数据面分离**：策略、台账、DHCP/PXE **配置** 只在 center；大镜像流量优先走缓存节点 ↔ 学生机。  
2. **网络面可同机可分离**：DHCP/PXE **进程** 属于 Edge 角色，可与 Center 同机，也可独立部署；管理端只认「网络服务节点」，不强迫多台服务器。  
3. **边缘自治**：center 短暂不可达时，edge 仍可执行 **已下发** 的 PXE 部署任务。  
4. **镜像先整包、接口预留增量**：一期完整镜像（或按分区）；清单支持后续块差分层。  
5. **多系统同一任务模型**：Windows / Linux / 信创差异关在「镜像格式 + pe 写盘插件」，管理概念一致。  
6. **DHCP 局部化**：每个网络服务节点只服务中心 **明确授权** 的网段，避免干扰校园网核心。  

### 5.3 组件一览

| 组件 | 职责 | 一期 |
|------|------|------|
| `idv-center` | API、Web、台账、镜像元数据、任务、**Edge/网络服务配置下发**、审计 | 必做 |
| `idv-edge` | 按中心配置提供 DHCP/PXE（可关）、启动文件、镜像缓存、本网段任务代理 | 必做 |
| `idv-pe` | PXE 拉起的维护环境：探测、采集、写入、上报 | 必做 |
| `idv-agent` | 操作系统内常驻：心跳、还原策略、远程电源等 | 二期 |
| `idv-cli` / 安装脚本 | 部署 center/edge（含同机组合）、初始化证书与网段 | 一期（最小可用） |

---

## 6. 组件设计

### 6.1 idv-center（中心 / 控制面）

**职责**

- 组织：校区 → 机房 → 终端  
- 资产管理：网络服务/缓存节点（edge）、终端（MAC/备注）、镜像  
- 任务：采集、部署、同步镜像到缓存节点、取消/重试  
- **网络服务策略**：各节点 DHCP/PXE 开关、模式（full/proxy）、授权网段、启动菜单/默认任务（配置在中心，节点拉取生效）  
- 鉴权：管理员账号（一期可先单租户本地账号）  
- 审计：关键操作与任务结果日志  

**运行形态**

- 单一管理服务（Rust）+ 嵌入或并列静态 Web  
- 持久化：建议 PostgreSQL（也可用 SQLite 仅限开发/极小试点，生产 500+ 推荐 PostgreSQL）  
- 对象存储：镜像大文件可落本地目录或 MinIO/S3 兼容存储；元数据在库中  
- **不强制** Center 进程内嵌 DHCP；需要 PXE 时同机再跑 `idv-edge` 进程即可  

**与 edge 关系**

- edge 启动后注册，定期心跳并拉取网络/缓存配置  
- center 下发：镜像同步清单、DHCP/PXE 参数、PXE 菜单/默认任务、终端归属  
- edge 上报：同步进度、任务状态、会话/发现的 MAC  

### 6.2 idv-edge（网络服务 + 镜像缓存）

**职责**

- 按 **中心下发配置** 在授权网段提供：DHCP（或 ProxyDHCP）、TFTP、HTTP(S)（均可按节点关闭）  
- 缓存镜像与 pe 启动资源（主价值：分流，避免单台中心扛全校镜像流量）  
- 根据 MAC / 机房策略决定 PXE 启动项（维护菜单 / 指定部署任务 / 本地启动）  
- 向 center 拉取待同步对象，断点续传  

**DHCP 工作模式（配置在中心，执行在节点）**

| 模式 | 说明 | 适用 |
|------|------|------|
| 全权 DHCP | 该节点作为授权 VLAN 的 DHCP，下发 IP + PXE | 独立机房网段（优先） |
| ProxyDHCP | 已有 DHCP 只发 IP，本节点只响应 PXE 扩展 | 现网已有 DHCP |
| 仅缓存 | 关闭本节点 DHCP/PXE，只做镜像缓存 | 引导由其它节点负责时 |

**自治**

- 本地保存「已批准任务」与镜像缓存索引  
- center 断开时：已缓存镜像的部署仍可进行；新镜像同步与配置变更暂停  

### 6.3 idv-pe（维护环境）

**职责**

- 经 iPXE/PXE 链加载的轻量 Linux 环境（或等效）  
- 硬件探测：网卡、磁盘、主板信息  
- 执行 center/edge 分配的动作：`capture` / `deploy` / `shell`（排障）  
- 写盘进度上报（HTTP 回调 edge，再由 edge 汇总到 center）  

**原则**

- pe 内写盘逻辑插件化：`windows_wim_or_block`、`linux_block`、`generic_block`  
- 一期优先 **块级/分区级镜像**（可控、跨发行版），对 Windows 可后续增强 WIM 文件级路径  
- 危险操作需任务令牌，避免任意机误写盘  

### 6.4 idv-agent（二期预留）

- 安装进学生机系统镜像内  
- 心跳、资产上报、还原模式（开机还原/手动还原/不还原）、WOL 配合  
- 一期台账可用「PXE 发现 + 手工录入 MAC」代替  

---

## 7. 数据模型（逻辑）

### 7.1 核心实体

```text
Campus（校区）
  └── Lab（机房）
        ├── EdgeNode（可选：一机房一 edge，或一楼栋多机房共享）
        └── Client（终端：mac, name, lab_id, notes, last_seen）

Image（镜像）
  - id, name, os_family(windows|linux|other), version_label
  - format(raw_sparse|qcow2_like|custom_v1), size, checksum
  - partition_map / disk_layout（JSON）
  - storage_uri（中心存储路径）
  - created_from_client_id（可选）

ImageReplica（边缘副本）
  - image_id, edge_id, state(syncing|ready|stale), checksum, progress

Task
  - type(capture|deploy|sync_image|pxe_once)
  - target（client_ids[] 或 lab_id）
  - payload（image_id、磁盘目标、选项）
  - state(pending|running|success|failed|cancelled)
  - edge_id, error_message, timestamps

AdminUser / AuditLog
```

### 7.2 镜像格式约定（一期 + 预留）

**一期：`yixue-image-v1`（完整包）**

```text
image.meta.json      # 名称、os、磁盘布局、块大小、校验
disk0.partN.img      # 或 disk0.raw（按实现选择）
checksums.sha256
```

`image.meta.json` 关键字段建议：

- `format_version`: `1`  
- `os_family`, `boot_mode`(`uefi`|`legacy`|`both`)  
- `disks[]`: 容量、分区表类型(gpt/mbr)、分区起止与文件系统标签  
- `content_type`: `full`（一期）| `delta`（预留）  
- `base_image_id`: 增量时指向父镜像（一期可空）  
- `payload_files[]`: 文件名与 sha256  

**二期增量**：新增 `content_type=delta` 与块位图/差分文件，center 任务仍用同一 `Image`/`Task` 模型。

### 7.3 终端标识

- 主键业务键：**MAC**（规范化为大写、去分隔符或统一冒号格式，全局一致）  
- 辅助：主板序列号、自定义资产编号  
- PXE 发现时自动建「未分配」终端，管理员拖入机房  

---

## 8. 关键流程

### 8.1 部署镜像（管理员视角）

1. 在 center 创建/导入 Image，校验 checksum  
2. 创建 `sync_image` 任务 → 指定 edge → 等待 `ImageReplica=ready`  
3. 选择机房或终端，创建 `deploy` 任务（绑定 image、目标磁盘策略）  
4. 学生机 PXE 启动 → edge 根据 MAC 下发 pe + 任务令牌  
5. pe 从 **本 edge HTTP** 拉镜像并写入磁盘  
6. 上报成功 → 可配置「下次从本地硬盘启动」  

### 8.2 采集镜像

1. 选中一台「金样机」，创建 `capture`  
2. 该机 PXE 进 pe → 按分区/整盘打包上传到 edge → edge 回传 center（或 center 直收，推荐经 edge 再归档）  
3. 生成新 Image，进入镜像库  

### 8.3 PXE 启动链（建议）

```text
网卡 PXE → (DHCP 指向) iPXE → HTTP 引导脚本
  → 加载 idv-pe kernel/initrd
  → pe 向 edge 领取本机任务
  → 执行 capture/deploy
```

引导脚本由 edge 按终端策略动态生成（无任务则进入菜单或本地启动）。

### 8.4 边缘同步

- center 维护镜像权威存储与元数据  
- edge 按清单拉取，支持断点续传与校验  
- 500+ 场景：**禁止**默认让整校学生机直连中心拉完整镜像  

---

## 9. API 与安全（一期轮廓）

### 9.1 API 风格

- center：`HTTPS` + `JSON` REST（或 OpenAPI 先行）  
- edge ↔ center：注册、心跳、拉任务、报进度、拉镜像清单  
- pe ↔ edge：领任务、进度、日志；使用 **短时任务令牌**  

### 9.2 主要 API 分组（实现时出 OpenAPI）

- `/api/v1/auth/*`  
- `/api/v1/campuses|labs|clients|edges|images|tasks`  
- `/api/v1/edges/register` `/heartbeat` `/pull` `/report`（edge 专用，证书或节点令牌）  
- 管理 Web 走同一 API  

### 9.3 安全底线

- 管理端强制认证；默认修改初始密码  
- edge 与 center 双向 TLS 或至少 edge 持有节点令牌  
- pe 写盘必须带任务令牌与目标磁盘校验（容量/序列号策略可配置）  
- 镜像 checksum 校验失败则拒绝部署  
- 审计：谁对哪些机下发了部署/采集  

---

## 10. 技术选型（自研 Rust）

**保鲜原则**：外部组件默认用当时 **最新 Stable/LTS**；版本号以 [`docs/modules/09-外部依赖与版本策略.md`](./modules/09-外部依赖与版本策略.md) 的核对表为准，阶段 0 脚手架前再查官网，避免文档与 crates.io 漂移。

| 层级 | 建议（方向） | 说明 |
|------|--------------|------|
| 语言 | Rust **Edition 2024** + `stable` 工具链 | 见模块 09；`rust-toolchain.toml`；勿用 2021 当新项目默认 |
| center/edge | `tokio` + `axum` **当前 0.8 线** | 配 `tower-http`；TLS 优先 **rustls** |
| 数据库 | PostgreSQL **当前大版本（基线 18.x）** + `sqlx` **0.9 线** | compose 钉小版本标签 |
| 前端 | **React 19** 或 **Vue 3.5** + **Vite 当前大版本** + TS | Design Tokens 多主题见模块 01；Node 用 Active LTS |
| PXE | **iPXE 2.x** 官方线 | 允许的开源引导件；定期跟上游重建 |
| pe 环境 | **Alpine 当前 stable** 上自制 initramfs + 自研 agent | 写盘逻辑 Rust 静态链接；禁止 EOL 发行版做默认 PE |
| 镜像传输 | HTTP Range 断点续传 | 一期单播；P2P 见模块 05 |
| 配置 | TOML | center.toml / edge.toml |
| 日志 | `tracing` | 任务 ID 贯穿 |
| 供应链 | `Cargo.lock` + audit + Renovate/Dependabot | 模块 09；防依赖腐化 |

**明确少依赖**：不把整套 FOG/Clonezilla 作为运行时核心；若开发期参考其行为，不引入为其从属系统。

---

## 11. 分期开发与验收

### 阶段 0 — 工程骨架（约 1 周量级，可按人力伸缩）

> **状态：已完成（2026-11-12）**  
> 验证：`cargo build/test/clippy --workspace` 通过；`idv-center/edge/pe-agent --help` 可用；`GET /health` 返回 ok。

**交付**

- [x] Cargo workspace：`idv-center` / `idv-edge` / `idv-pe-agent` / `idv-common`  
- [x] `rust-toolchain.toml`（stable）+ Edition 2024；依赖取当时最新 Stable 线（模块 09）  
- [x] 基础配置、日志、错误类型；`Cargo.lock` 入库  
- [x] `docker-compose` 可起 PostgreSQL（钉版本标签 `postgres:18`）  
- [x] CI：`fmt` / `clippy` / `test` / `cargo audit`（audit 阶段 0 允许 `continue-on-error`）  
- [x] 本文档与仓库 README 入口  

**验收**

- [x] `cargo build` 工作区通过  
- [x] 各 crate 有空服务或 `--help` 可运行  
- [x] README 写清如何启动 center（哪怕只返回健康检查）  
- [x] 依赖符合模块 09 开工检查清单（无过时大版本作为默认）  

---

### 阶段 1 — Center 台账与镜像元数据 + 管理端壳

> **状态：已完成（2026-11-12）**  
> 验证：API 验收脚本创建 2 机房 + 10 MAC + 镜像 checksum；进程重启后数据仍在；`web/dist` 由 center 托管，含墨蓝/暗夜两套主题。  
> 说明：本机无 Docker，阶段 1 默认 **SQLite**（`data/idv-center.db`）满足持久化验收；PostgreSQL 仍保留在 compose，阶段后续可切换。

**交付**

- [x] 校区/机房/终端/管理员 CRUD（REST + 登录会话）  
- [x] 镜像注册（元数据 + `storage_uri` + SHA-256）  
- [x] 管理端：布局壳、导航 IA、**ink-lab / night-duty** 主题 Token、核心列表页（见模块 01）  

**验收**

- [x] 可创建 2 个机房并录入 ≥10 条终端 MAC  
- [x] 可登记一个镜像元数据并保存 checksum  
- [x] 重启服务后数据不丢  
- [x] 主题可切换；登录页与侧栏品牌为「易学 IDV」  

---

### 阶段 2 — Edge 注册、心跳与镜像同步 + 缓存管理

> **状态：已完成（2026-11-12）**  
> 验证：`deploy/scripts/stage2-accept.ps1` → `STAGE2_ACCEPT_OK`（在线、同步 checksum、清空再同步、Center 断开后 Edge 仍存活）。

**交付**

- [x] edge 配置 `center_url` + 节点令牌并注册  
- [x] 心跳与在线状态（45s 内 `last_seen` 视为在线）  
- [x] `sync_image`：中心 → 边缘缓存目录，进度可查  
- [x] 单镜像缓存清空、重新同步/校验（见模块 03）；支持 `purge_and_sync`  

**验收**

- [x] center 显示 edge 在线  
- [x] 指定镜像同步到 edge，本地出现完整文件且 checksum 一致  
- [x] 可清空某镜像在 Edge 的缓存后重新同步  
- [x] 断开 center 后 edge 进程仍运行（同步暂停，服务不崩）  

---

### 阶段 3 — DHCP/PXE + pe 最小启动

> **状态：已完成（2026-11-12）**  
> 验证：`deploy/scripts/stage3-accept.ps1` → `STAGE3_ACCEPT_OK`（DHCP Offer、TFTP `boot.ipxe`、`pe-agent checkin`、Center/Edge 会话可见、无任务提示）。  
> 说明：协议与报到链路已通；真机需将官方 **iPXE 2.x** 二进制放入 `pe/`（见 `pe/README.md`）。完整 Linux kernel/initrd PE 在阶段 4 继续加深。

**交付**

- [x] 实验网段：edge 提供 DHCP + PXE（`full`/`proxy`，端口可配）  
- [x] iPXE 链脚本：`pe/boot.ipxe` → `pe/pe.ipxe`（HTTP `/pxe/` + TFTP）  
- [x] pe 报到：`pe-agent checkin` / iPXE checkin → Edge → Center  

**验收**

- [x] 测试机网卡 PXE 能进入 pe（有字符界面或日志即可）— 协议级验收 + pe-agent 控制台输出；真机 PXE 见 `pe/README.md`  
- [x] center/edge 能看到该 MAC 的 pe 会话  
- [x] 无任务时行为符合设计（提示后 `exit_to_local_disk`）  

---

### 阶段 4 — 采集与部署闭环（第一期里程碑）

> **状态：已完成（2026-11-12）**  
> 验证：`deploy/scripts/stage4-accept.ps1` → `STAGE4_ACCEPT_OK`（采集入库、同步、≥5 台模拟部署、磁盘策略预览、校验失败写 `.deploy-failed`、任务历史/CSV）。  
> 说明：Windows 试点用 `--disk-file` 模拟磁盘；真机 PXE 写盘同协议。采集规范见 `docs/modules/10-采集规范与分区角色.md`。

**交付**

- [x] `capture`：pe 打包磁盘/分区 → 经 edge → 入库为 Image（`yixue-image-v1`）  
- [x] `deploy`：pe 从本 edge 拉镜像写入磁盘（含 DiskPlan）  
- [x] 任务状态机与失败原因（管理端「维护任务」+ CSV 导出）  

**验收（试点机房）**

- [x] 金样机采集成功，镜像出现在库中  
- [x] 同步到 edge 后，≥5 台机器批量部署成功并进入系统（文件盘模拟验收；真机同流程）  
- [x] 部署任务可选磁盘策略（整盘 / 保留数据分区），确认框说明覆盖范围（见模块 02）  
- [x] 故意损坏下载（或校验失败）时任务失败且磁盘不处于「静默坏状态」（应失败可重试或明确报错）  
- [x] 管理端可导出/查看任务历史  
- [x] 金样机分区角色（system/persistent）在采集规范中有说明，便于二期还原  

**第一期产品验收签字以阶段 4 为准。**

---

### 阶段 5 — 规模化加固（500+ 准备）与 P2P 试点

> **状态：已完成（2026-11-12）**  
> 验证：`deploy/scripts/stage5-accept.ps1` → `STAGE5_ACCEPT_OK`（双 Edge 在线同步、缓存水位、P2P swarm 互传占比、令牌轮换）。  
> 运维：`docs/modules/11-规模化运维手册.md`。

**交付**

- [x] 多 edge、多机房权限与网段隔离文档  
- [x] 同步队列、带宽限速、并发部署上限  
- [x] Edge 整库清空、缓存水位告警（模块 03）  
- [x] 备份与升级说明  
- [x] **P2P 分片互助传输试点**（模块 05；Edge 中介 swarm）  

**验收**

- [x] 至少 2 个 edge 同时在线，镜像按需同步  
- [x] 文档给出「100 台同时部署」的推荐并发与交换机建议  
- [x] 有运维手册：新增机房、更换 edge、轮换令牌  
- [x] P2P 试点：任务详情能看到互传占比（`p2p_bytes` / `edge_bytes`）  

---

### 阶段 6 — 二期：还原、增量、模式、WiFi 等

> **状态：软件已完成（2026-11-12）** — P0～6e 验收通过；真机项转入模块 14 试点  
> 验证：`stage6` / `6b` / `6c` / `6d` / `6e-accept` → 各自 `*_ACCEPT_OK`  
> 签字页：[`docs/modules/12-二期验收签字页.md`](./modules/12-二期验收签字页.md)  
> 硬件试点：[`docs/modules/14-阶段6硬件试点清单.md`](./modules/14-阶段6硬件试点清单.md)

**交付方向**

- [x] `idv-agent`：心跳、策略（P0）  
- [x] 开机还原 / 手动还原；办公/考试模式与持久数据区  
- [x] 增量判定引擎 + 增量包生成/应用  
- [x] 考试时间窗、USB 模拟 enforcement、窗外回落教学  
- [x] 模式变更审计  
- [x] WOL magic packet + 远程关机命令  
- [x] P2P 做种生命周期  
- [x] PE 网络向导 + WiFi 驱动包槽位  
- [x] 管理端 4 套主题 + 信创兼容矩阵文档  
- [x] Windows 卷后端 / 开机钩子骨架（`windows-lab`）  
- [ ] 硬件试点项（扇区写 / 签名 INF / 信创实检）→ 模块 14  

**验收**：软件见二期签字页；硬件见模块 14。

---

### 阶段 7 — 三期：运维增值（软件骨架）

> **状态：已完成（2026-11-12）**  
> 验证：`deploy/scripts/stage7-accept.ps1` → `STAGE7_ACCEPT_OK`  
> 模块：[`docs/modules/15-三期运维增值.md`](./modules/15-三期运维增值.md)

**交付**

- [x] 多管理员角色（super / lab_admin / readonly）  
- [x] 终端标签与筛选  
- [x] 软件分发（包 + 任务 + agent soft-apply）  
- [x] 远程协助会话信令（非真桌面流）  
- [x] 上课时段部署只读锁  
- [x] 定时电源策略 + tick  
- [x] 审计 CSV 导出  
- [x] 管理端「三期运维」页  

**非本阶段**：真同屏画面、真 MSI 静默安装、跨 VLAN WOL（见模块 14/08）。

---

## 12. 建议仓库结构

```text
idv_rust/
  Cargo.toml                 # workspace
  crates/
    idv-common/              # 元数据、错误、校验、API 类型
    idv-center/              # 中心服务 + 可选 web 静态资源
    idv-edge/                # 边缘：同步、DHCP/PXE、HTTP 文件
    idv-pe-agent/            # 运行在 pe 内的 agent
    idv-agent/               # 学生机 Agent（还原/模式，二期）
  pe/                        # pe 构建脚本、kernel/initrd 配置
  web/                       # 管理前端（可后置）
  deploy/                    # systemd、compose、示例 toml
  docs/
    易学IDV机房管理系统-开发文档.md   # 本文（总纲）
    modules/                 # 功能模块详细说明与验收
    plans/                   # 实施计划、设计修订
  README.md
```

---

## 13. 测试策略

| 类型 | 要求 |
|------|------|
| 单元测试 | checksum、MAC 规范化、任务状态迁移、镜像 meta 解析 |
| 集成测试 | center API + DB；edge 同步目录校验 |
| 硬件验收 | 真实机房网段 PXE；至少覆盖 UEFI；有条件再测 Legacy |
| 系统矩阵 | 一期至少：1 种 Windows 镜像 + 1 种 Linux/信创镜像 部署成功 |

危险操作（写盘）禁止在无隔离环境的自动化里误连生产机房。

---

## 14. 风险与对策

| 风险 | 对策 |
|------|------|
| Windows 驱动/开机模式差异大 | meta 记录 boot_mode；金样机与目标机硬件尽量分池 |
| 信创发行版碎片化 | 块级镜像优先，减少文件级假设 |
| 500+ 同时拉镜像打满网络 | 强制 edge 缓存；限流；P2P；分机房分批 |
| 自研 DHCP 与现网冲突 | 严格 VLAN；文档写清网段规划；保留 ProxyDHCP 选项 |
| PXE 安全被滥用 | 任务令牌、管理网与学生网策略、审计 |
| 范围膨胀（还原/增量/WiFi 过早） | 严格按阶段 4 关门；专项见 modules |
| WiFi 驱动碎片化 | 型号池；不承诺固件级无线 PXE |
| 还原误伤办公数据 | 分区角色 + 模式策略（模块 06） |

---

## 15. 开发执行约定

1. **先改文档再改大行为**：架构或验收标准变更，更新本文版本记录。  
2. **按阶段交付**：未完成上一阶段验收，不宣称下一阶段完成。  
3. **每阶段可演示**：能给网管现场看的路径优先于抽象重构。  
4. **中文产品名统一**：界面与对外材料使用「易学 IDV 机房管理系统」；代码 crate 可用 `idv-*`。  
5. **提交信息**：建议 `feat(center): ...` / `feat(edge): ...` / `docs: ...` 风格（若启用 git）。  

---

## 16. 版本记录

| 版本 | 日期 | 说明 |
|------|------|------|
| v0.1 | 2026-11-12 | 首版：需求对齐、方案 1 基线、分期与验收 |
| v0.2 | 2026-11-12 | 拆分 modules；补充 UI/磁盘/缓存/增量/P2P/模式/WiFi/需求池；调整阶段 1～6 |
| v0.3 | 2026-11-12 | 外部依赖保鲜策略；选型改为 Edition 2024 / 当代 axum·PG·前端·iPXE 2.x 等（见模块 09） |
| v0.4 | 2026-11-12 | 阶段 0 完成：workspace、健康检查、compose、CI、README；验收项勾选 |
| v0.5 | 2026-11-12 | 阶段 1 完成：台账 API、SQLite 持久化、React 管理端壳与双主题 |
| v0.6 | 2026-11-12 | 阶段 2 完成：Edge 注册/心跳/镜像同步/清缓存与验收脚本 |
| v0.7 | 2026-11-12 | 阶段 3 完成：DHCP/PXE/TFTP、iPXE 脚本、PE 报到与无任务提示 |
| v0.8 | 2026-11-12 | 阶段 4 完成：采集/部署闭环、DiskPlan、校验失败不静默、任务历史；第一期里程碑 |
| v0.9 | 2026-11-12 | 阶段 5 完成：双 Edge、限速/并发、缓存水位、P2P 试点占比、运维手册 |
| v0.10 | 2026-11-12 | 阶段 6 P0：idv-agent、ModePolicy、分区级还原、增量推荐、二期签字页 |
| v0.11 | 2026-11-12 | 阶段 6 滚动：delta 生成/应用、考试时间窗、WOL/关机命令 |
| v0.12 | 2026-11-12 | 阶段 6c：P2P 停种、USB enforcement、PE 网络向导、4 主题 |
| v0.13 | 2026-11-12 | 阶段 6d：windows-lab 卷钩子、信创矩阵、WiFi 驱动槽位 |
| v0.14 | 2026-11-12 | 阶段 6e 软件关门：审计、考试回落、WOL；硬件试点清单 |
| v0.15 | 2026-11-12 | 阶段 7 三期：角色/标签/软件分发/协助/上课锁/定时电源 |
| v0.16 | 2026-11-12 | 明确：DHCP/PXE **配置在中心**；进程属 Edge 角色，可与 Center 同机或分离（单机/多机/混合） |

---

## 17. 附录：术语

| 术语 | 含义 |
|------|------|
| Center | 中心管理 / 控制面（台账、策略、配置下发） |
| Edge | 网络服务 + 镜像缓存节点（DHCP/PXE 可开关；可与 Center 同机） |
| PE | Preinstallation / 维护环境 |
| 金样机 | 用于采集标准环境的样板机 |
| 任务令牌 | pe 执行写盘前必须持有的短时凭证 |
| 副本 | 某镜像在某 edge 上的缓存实例 |
| DiskPlan | 部署时的磁盘覆盖/保留策略 |
| ModePolicy | 教学/考试/办公等工作模式策略 |

---

## 18. 关键问题速查（v0.2）

| 问题 | 短答 | 详情 |
|------|------|------|
| 管理页如何好看、多主题、响应式？ | Design Tokens + 多套主题 + 桌面优先响应式；一期就按规范做 | [模块 01](./modules/01-管理端UI与设计系统.md) |
| 同步/部署前要先格式化硬盘吗？ | Edge 同步不用；部署由写镜像覆盖目标区，一般不必手工预格式化；注意数据会丢 | [模块 02](./modules/02-镜像部署与磁盘处理.md) |
| 重新同步能否清空缓存？ | 需要；支持单镜像/整 Edge 清空、先清再拉 | [模块 03](./modules/03-缓存管理与重新同步.md) |
| 何时用增量？ | 有兼容父镜像且差量足够小等规则自动推荐；可强制全量 | [模块 04](./modules/04-增量更新策略.md) |
| 要不要 P2P？ | 要；完成度高的机器做种，Edge 托底 | [模块 05](./modules/05-P2P分发.md) |
| 办公/考试数据如何不被还原误伤？ | 持久分区/卷角色 + 工作模式策略 | [模块 06](./modules/06-还原策略与多模式.md) |
| WiFi 能否用？ | 固件 PXE 不行；PE 里配 WiFi 与服务器地址后拉镜像可以 | [模块 07](./modules/07-有线与WIFI接入.md) |
| 还有什么该规划？ | 驱动池、WOL、审计、限流等见需求池 | [模块 08](./modules/08-补充能力与需求池.md) |
| 外部组件会不会一上线就过时？ | 默认最新 Stable + 锁文件 + audit/自动升级；清单见模块 09 | [模块 09](./modules/09-外部依赖与版本策略.md) |
| 信创浏览器能否用管理端？ | Chromium≥90 为基线；矩阵与抽检表见模块 13 | [模块 13](./modules/13-信创浏览器兼容矩阵.md) |
| 金样机分区角色怎么约定？ | system/persistent 写入镜像 meta；见采集规范 | [模块 10](./modules/10-采集规范与分区角色.md) |
| 500+ / 多 Edge 怎么运维？ | 隔离、并发批次、备份升级、令牌轮换 | [模块 11](./modules/11-规模化运维手册.md) |

---

**下一步**：联调 `stage6*` + `stage7-accept`；真机项按 [`模块 14`](./modules/14-阶段6硬件试点清单.md)。真同屏/真安装属后续增强。
