# 易学 IDV 机房管理系统

学校计算机机房的 **分布式镜像管控、还原与运维平台**（工程名 `idv_rust`）。

中心负责台账、镜像库、任务与远程控制中继；边缘节点负责就近缓存与 DHCP/PXE；学生机通过维护环境（Linux PE）完成 **冷采集** 与 **部署写盘**，系统内由常驻 Agent 负责还原、身份收敛、即时命令与远程协助。桌面运算仍在本地完成，不是云桌面流式虚拟化。

## 能做什么

### 镜像与装机

- **台账**：校区 / 机房 / 客户端管理；PXE 报到自动登记
- **镜像库**：`yixue-image-v1` 入库（NTFS 跳空闲 `ntfs_used_v1`、流式校验）
- **边缘同步**：镜像缓存到 Edge，配额、水位、P2P
- **冷采集**：金样机 PXE → Alpine PE → 流式打包上传 → Center 入库
- **批量部署**：按校区/机房勾选客户端 → 预览磁盘策略 → PE 流式写盘，支持 500 台规模

### 还原与课堂模式

- **卷过滤还原驱动**：Windows 卷级冻结/还原（测试签名，见模块 17）
- **多盘还原**：`teaching` 保护所有固定 NTFS 盘（C/D/E…），`exam` 仅系统盘
- **模式策略**：teaching / office / exam / manual，按机房绑定；变更后自动重启生效
- **手动还原**：manual 模式终端页下发 `restore_now`，开启保护并重启，重启后恢复基线再解冻
- **USB 管控 / 上课只读 / 定时电源** 等策略能力

### 批量身份与网络

- 500 台镜像无需逐台配置主机名/IP/MAC；Agent 自动探测网卡 MAC
- 首启身份收敛：自动改名、配 IP、再开启还原，支持自动重启
- `ip_mode=dhcp` 时由 Edge DHCP 按 MAC 保留 IP，还原状态下身份不丢
- **WOL 远程开机**：机房子网定向广播 + UDP 9/7 + 重发；跨 VLAN 由机房 Edge 转发，无 Edge 时 Center 本地兜底

### 常驻 Agent 与运维

- **Windows 服务 / SYSTEM 计划任务守护**：5 秒心跳 + 长轮询即时命令
- 即时命令：重启、关机、切换模式、打开网页、执行程序、WOL
- **远程控制**：观察（只看屏幕）/ 协助（键鼠控制），文本剪贴板共享
- **文件传输**：一对一目录浏览/上传下载；按指定目录批量收集/下发 zip
- **Agent 自动更新**：维护任务上传 exe → 自动识别版本 → 解冻/下载/替换/重启/恢复还原；Task Scheduler updater + 回滚
- 终端页显示在线、还原状态与 **Agent 版本**

### 管理端

React 多主题 Web UI（Center 托管 `web/dist`），完全离线可用；字体使用本机系统字体，不依赖 Google Fonts/CDN。

## 架构（简图）

```text
管理端 Web (React)
      │ HTTP
  idv-center ──SQLite/镜像目录/远程帧中继── data/
      │
  idv-edge ──DHCP/PXE/TFTP/镜像缓存── pe/ + data/edge-cache/

学生机：
  PXE → iPXE → Linux PE (idv-pe-agent) → 采集 / 部署
  Windows → idv-agent 服务（心跳/还原/命令）
              ├─ SysRestoreDriver 卷过滤驱动
              └─ 登录会话 assist-helper（截屏/键鼠/剪贴板/文件）
```

## 仓库结构

```text
crates/
  idv-common/      镜像格式、NTFS、DesiredState、WOL、测试工具
  idv-center/      Center API + Web 静态托管 + 远程控制中继
  idv-edge/        Edge 缓存、DHCP/PXE/TFTP、PE API、IP 保留
  idv-pe-agent/    PE 内采集/部署代理（另编 musl 静态版）
  idv-agent/       学生机常驻 Agent（0.3.6：还原/身份/命令/远程协助）
web/               管理端（React + Vite）
deploy/
  config/*.toml.example   配置模板（复制后改本地，勿提交密钥）
  client-package/         学生机客户端整包（脚本 + exe 本地打包）
  scripts/                验收、PE 构建、金样机、签名脚本
data/              运行时 SQLite/镜像/缓存/Agent 包/远程文件（gitignore）
docs/              开发总纲、模块文档、设计/计划
pe/                iPXE 脚本；Linux 大文件由脚本生成（gitignore）
```

## 环境要求

- Rust **stable**（见 `rust-toolchain.toml`）
- Node.js 22+（构建管理端）
- Windows 10/11 客户端：PowerShell 5.1、管理员权限
- 还原驱动：测试签名（`enable-testsigning.ps1`），生产需正式签名
- 真机 PXE：Windows 上 DHCP 67 / TFTP 69 需管理员权限
- 编 musl PE 代理：`x86_64-unknown-linux-musl` 目标与 `rust-lld`

```powershell
rustup update stable
rustup target add x86_64-unknown-linux-musl
rustc -V
node -v
```

## 配置（重要）

现场配置 **不要提交**。仓库只保留示例：

| 示例 | 本地文件（已 gitignore） |
|------|-------------------------|
| `deploy/config/center.toml.example` | `center.toml` |
| `deploy/config/edge.toml.example` | `edge.toml`（含 `node_token`） |
| `deploy/config/pe-agent.toml.example` | `pe-agent.toml` |
| `deploy/config/agent.toml.example` | `agent.toml` |
| `deploy/client-package/agent.toml.example` | 客户端包内 `agent.toml` |

```powershell
copy deploy\config\center.toml.example deploy\config\center.toml
copy deploy\config\edge.toml.example deploy\config\edge.toml
# 管理端创建 Edge 后，把 node_token 写入 edge.toml
```

运行时数据：SQLite、镜像、Edge 缓存、Agent 包、远程文件、任务归档都在 `data/`，已忽略。

## 快速开始（Center）

```powershell
# 1. 配置
copy deploy\config\center.toml.example deploy\config\center.toml

# 2. 构建管理端
cd web
npm install
npm run build
cd ..

# 3. 启动 Center
cargo run -p idv-center --release -- serve --config deploy/config/center.toml
```

浏览器打开 <http://127.0.0.1:8080/>

- 默认账号：`admin` / `admin123`（生产环境务必修改）
- 健康检查：`GET /health`
- 数据库迁移启动时自动执行

开发期前后端分离：

```powershell
cargo run -p idv-center -- serve --config deploy/config/center.toml
cd web; npm run dev
```

Vite：<http://127.0.0.1:5173/>（已代理 `/api`）。

## 启动 Edge（PXE / 同步）

1. 管理端创建边缘节点，复制 `node_token` 到 `edge.toml`
2. 按机房修改 `[pxe]` 网段、`next_server` 等；推荐把
   `subnet_mask / router / offer_start / offer_end` **留空**，
   Edge 会自动读取本 Edge 绑定的机房 IP 段作为 DHCP 池（填值则覆盖）；
   机房分配段过小时会自动改用同子网全地址池
3. 准备 PE 资源（见下）
4. **管理员权限**启动：

```powershell
cargo run -p idv-edge --release -- serve --config deploy/config/edge.toml
```

### PE Linux 资源

大文件（内核、initramfs、modloop、musl pe-agent）不进 Git，请在仓库根目录生成：

```powershell
powershell -ExecutionPolicy Bypass -File deploy/scripts/fetch-pe-linux.ps1
# 或按 pe/linux/README.md、deploy/scripts/build-pe-initramfs.py 构建
```

并将官方 iPXE 二进制放入 `pe/`（`ipxe.efi` 等），说明见 [`pe/README.md`](pe/README.md)。

## 客户端安装（Windows）

把 `deploy/client-package/` 整份复制到学生机 `C:\Program Files\YixueIDV\`，管理员 PowerShell：

```powershell
cd "C:\Program Files\YixueIDV"

# 1) 编辑 Center 地址（复制 agent.toml.example → agent.toml）
notepad .\agent.toml

# 2) 测试签名驱动（需重启）
powershell -ExecutionPolicy Bypass -File .\scripts\enable-testsigning.ps1

# 3) 重启后安装还原驱动（再重启）
powershell -ExecutionPolicy Bypass -File .\scripts\install-driver.ps1

# 4) 安装常驻 Agent 服务
powershell -ExecutionPolicy Bypass -File .\scripts\install-agent-service.ps1

# 5) 可选：启用有线 WOL、关闭快速启动
powershell -ExecutionPolicy Bypass -File .\scripts\enable-wol.ps1
```

详细说明见 [`deploy/client-package/使用说明-测试签名驱动.txt`](deploy/client-package/使用说明-测试签名驱动.txt) 与模块 17/18。

## 典型流程

1. **台账**：校区 → 机房 → 客户端（或 PXE 报到自动登记）
2. **采集**：金样机下发 capture → PXE → 镜像入库
3. **同步**：镜像就绪到 Edge 缓存
4. **部署**：勾选目标机 → 磁盘策略 → 确认覆盖 → PXE 写盘
5. **课堂**：绑定机房模式（teaching/exam/office）→ 自动冻结/重启
6. **运维**：终端页远程控制、批量文件、Agent 升级、WOL 开机

磁盘策略见 [`docs/modules/02-镜像部署与磁盘处理.md`](docs/modules/02-镜像部署与磁盘处理.md)。  
500 台批量身份见 [`docs/modules/18-批量身份与DHCP保留.md`](docs/modules/18-批量身份与DHCP保留.md)。

## 文档

- [开发与验收总纲](docs/易学IDV机房管理系统-开发文档.md)
- [功能模块索引](docs/modules/README.md)
- [驱动测试签名与蓝屏恢复](docs/modules/17-驱动测试签名与蓝屏恢复.md)
- [批量身份与 DHCP 保留](docs/modules/18-批量身份与DHCP保留.md)
- [常驻 Agent 与即时命令](docs/modules/19-常驻Agent与即时命令.md)
- [WOL 远程开机](docs/modules/20-WOL远程开机.md)
- [多盘还原作用域](docs/modules/21-多盘还原作用域.md)
- [客户端 Agent 自动更新](docs/modules/22-客户端Agent自动更新.md)
- [远程控制与文件传输](docs/modules/23-远程控制与文件传输.md)

分阶段验收脚本：`deploy/scripts/stage*-accept.ps1`。

## 开发阶段（摘要）

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0～3 | 工程骨架、台账、Edge、PXE 报到 | 已完成 |
| 4 | 真机冷采集 + 流式部署闭环 | 已打通（试点） |
| 5 | 多 Edge、P2P、规模化参数 | 软件已完成 |
| 6 | 还原驱动、模式策略、多盘还原 | 软件已完成（测试签名试点） |
| 7 | 批量身份、常驻 Agent、即时命令、远程控制、Agent 更新 | 软件已完成 |

当前客户端 Agent：**0.3.6**（GDI 截屏、Task Scheduler updater、远程协助/文件传输）。

后续增强：更细分区角色向导、远程控制 WebRTC/音频、Agent 设备令牌与命令签名、跨 VLAN WOL 等。

## 安全与部署注意

- 生产前修改默认管理员密码、轮换 Edge `node_token`
- 测试签名驱动仅适合试点机；正式部署需正式签名
- Agent 命令、远程控制、Agent 包下载目前按 MAC / 随机会话 ID 识别，**只应部署在受控机房网络**
- `run_program`、远程键鼠、文件传输具备高权限能力，建议配合网络隔离与审计
- DHCP 地址池/保留需与现网规划，避免与校园 DHCP 冲突
- 内网环境不依赖任何外部 CDN；管理端字体使用本机系统字体

## 许可证与说明

内部机房管理系统工程。对外部署前请完成上述安全项，并先在虚拟机/可重装机试点验证还原驱动与 PXE 部署流程。
