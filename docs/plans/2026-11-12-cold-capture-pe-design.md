# 冷采 Linux PE 整盘采集

> 2026-11-12 · 已确认（方案 A / Linux PE）

## 目标

金样机 PXE → 报到登记 → 有 capture 任务时进入精简 Linux PE → `pe-agent` 流式读物理盘 → Edge → Center 镜像入库。

## 非目标（本轮）

- 不在块级采集中自动跳过 `pagefile.sys` 等文件（见过滤规划）
- 不实现 WinPE
- 不做热采作为主路径

## 架构

1. 管理端对 MAC 创建 capture 任务  
2. iPXE `checkin.ipxe`：无任务 sanboot；有 capture → 引导 Alpine netboot + overlay（含 musl `idv-pe-agent`）  
3. PE 内 DHCP 后 `pe-agent run`：流式读 `/dev/sdX`（或 cmdline 指定），写出 `yixue-image-v1` 到临时文件并流式 PUT  
4. Edge/Center：请求体落盘，校验 header/payload，登记 Image  

## 过滤规划（记录，本轮不实现自动过滤）

| 阶段 | 做法 |
|------|------|
| v1.1 预处理 | PE 内短时挂载 NTFS，删除/置零 pagefile/swapfile/hiberfil、Windows\\Temp、用户 Temp 后再采 |
| v2 格式 | 稀疏/排除列表或按文件角色采集；任务可选排除模板 |
| 管理端 | 采集向导勾选排除模板 |

默认排除候选：`pagefile.sys`、`swapfile.sys`、`hiberfil.sys`、`$Recycle.Bin`、`Windows\Temp`、`Temp`、浏览器缓存（可配置）。

## 镜像效率演进（2026-11-12 确认，待当前整盘上传完成后再开工）

业务背景：

- **首次金盘**：全盘采集可接受（当前 v1 路径）。
- **后续常态**：软件更新、补丁、小调整频繁 → **CDC 分块去重会经常用到**，但不要与前两步捆一次性上线。

推荐分阶段（下发效率 + 成功率优先）：

| 阶段 | 内容 | 目的 |
|------|------|------|
| **①+②（先做）** | 跳过空闲区（partclone 类）+ 边传边校验 / 分块校验 | 少读少传；去掉「先整盘 hashing 再上传」双遍读；块失败可重试，利于下发成功率 |
| **验证** | ①+② 采集/入库/下发跑通并对比体积与耗时 | 确认无回归再继续 |
| **③（后做）** | CDC 内容分块去重（casync 类） | 金盘小版本迭代、多 Edge 同步、断点续传；首次满镜像增益次之，**补丁/升级场景主收益** |

约束：

- 当前整盘上传/测试未完成前：**只记录，不改采集格式代码**。
- 上线顺序固定为 **先 ①+② → 测通 → 再 ③**，三样不并行堆进同一版本。
