# 易学 IDV 机房管理系统

学校计算机机房的 **分布式镜像管控与 PXE 装机平台**（工程名 `idv_rust`）。

中心负责台账、镜像库与任务调度；边缘节点负责就近缓存与 DHCP/PXE；学生机通过维护环境（Linux PE）完成 **冷采集** 与 **部署写盘**。桌面运算仍在本地完成，不是云桌面流式虚拟化。

## 能做什么

- **台账**：校区 / 机房 / 客户端（MAC）管理，PXE 报到自动登记  
- **镜像库**：`yixue-image-v1` 入库（支持 NTFS 跳空闲 `ntfs_used_v1`、流式校验）  
- **边缘同步**：镜像同步到 Edge 缓存，配额与水位告警  
- **冷采集**：金样机 PXE → Alpine PE → 流式打包上传 → Center 入库  
- **批量部署**：勾选客户端（按校区/机房筛选）→ 预览磁盘策略 → PE 流式写盘  
- **管理端**：React 多主题 Web UI（由 Center 托管 `web/dist`）

## 架构（简图）

```text
管理端 (Web)
    │
 idv-center  ──SQLite/镜像目录──  data/
    │
 idv-edge   ──缓存/PXE/TFTP──  pe/ + data/edge-cache/
    │
 学生机 PXE → iPXE → Linux PE (idv-pe-agent) → 采集或部署
```

## 仓库结构

```text
crates/
  idv-common/      镜像格式、NTFS 跳空闲、共享工具
  idv-center/      中心 API + 管理端静态资源托管
  idv-edge/        边缘缓存、DHCP/PXE/TFTP、PE API
  idv-pe-agent/    PE 内采集/部署代理（另编 musl 静态版）
  idv-agent/       学生机常驻 Agent（心跳/策略骨架）
web/               管理端（React + Vite）
deploy/
  config/*.toml.example   配置模板（复制后改本地，勿提交密钥）
  scripts/                分阶段验收与 PE 构建脚本
docs/              开发总纲与模块文档
pe/                iPXE 脚本；linux/ 下大文件需脚本生成（已 gitignore）
```

## 环境要求

- Rust **stable**（见 `rust-toolchain.toml`）
- Node.js 22+（构建管理端）
- Windows 上编 musl PE 代理需 `x86_64-unknown-linux-musl` 目标与 `rust-lld`
- 真机 PXE：机房网卡、DHCP/TFTP 端口权限（Windows 上 67/69 通常需管理员）

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

```powershell
copy deploy\config\center.toml.example deploy\config\center.toml
copy deploy\config\edge.toml.example deploy\config\edge.toml
# 管理端创建 Edge 后，把 node_token 写入 edge.toml
```

运行时数据目录 `data/`（SQLite、镜像、边缘缓存）已忽略。

## 快速开始

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

- 默认账号：`admin` / `admin123`（务必在生产环境修改）  
- 健康检查：`GET /health`

开发期前后端分离：

```powershell
cargo run -p idv-center -- serve --config deploy/config/center.toml
cd web; npm run dev
```

Vite：<http://127.0.0.1:5173/>（已代理 `/api`）。

### 启动 Edge（PXE / 同步）

1. 管理端创建边缘节点，复制 `node_token` 到 `edge.toml`  
2. 按机房修改 `[pxe]` 网段、`next_server` 等  
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

## 典型机房流程

1. 台账：校区 → 机房 → 客户端（或 PXE 报到自动登记）  
2. **采集**：对金样机下发 capture → PXE → 镜像入库  
3. **同步到边缘**：镜像就绪到 Edge 缓存  
4. **部署**：勾选目标机 → 选择磁盘策略并确认「确认覆盖」→ PXE 写盘 → 重启进系统  

磁盘策略含义见 [`docs/modules/02-镜像部署与磁盘处理.md`](docs/modules/02-镜像部署与磁盘处理.md)。

## 文档

- [开发与验收总纲](docs/易学IDV机房管理系统-开发文档.md)  
- [功能模块索引](docs/modules/README.md)  
- [采集规范与分区角色](docs/modules/10-采集规范与分区角色.md)  
- [外部依赖与版本策略](docs/modules/09-外部依赖与版本策略.md)  

分阶段验收脚本：`deploy/scripts/stage*-accept.ps1`。

## 开发阶段（摘要）

| 阶段 | 内容 | 状态 |
|------|------|------|
| 0～3 | 工程骨架、台账、Edge、PXE 报到 | 已完成 |
| 4 | 真机冷采集 + 流式部署闭环 | 已打通（试点） |
| 5 | 多 Edge、P2P 试点、规模化参数 | 软件已完成 |
| 6～7 | 还原/模式/运维增值等 | 软件骨架已完成；硬件试点见模块 14 |

后续增强（非阻塞主路径）：CDC 增量、更细分区角色向导等。

## 许可证与说明

内部机房管理系统工程；对外部署前请更换默认口令、轮换 Edge `node_token`，并按机房网络规范配置 DHCP（避免与现网 DHCP 冲突）。
