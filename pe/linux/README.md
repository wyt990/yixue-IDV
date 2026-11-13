# Linux PE（冷采）

金样机有 **capture** 任务时，iPXE 会引导本目录资源。

## 准备

在仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File deploy/scripts/fetch-pe-linux.ps1
```

将下载 Alpine LTS netboot，并编译 `x86_64-unknown-linux-musl` 的 `idv-pe-agent`，生成：

| 文件 | 作用 |
|------|------|
| `vmlinuz-lts` / `initramfs-lts` / `modloop-lts` | Alpine 网络启动 |
| `idv-pe-agent` | 采集代理（musl） |
| `idv.apkovl.tar.gz` | 开机自动跑采集的 overlay |

Edge 通过已有 `/pxe/` 静态目录提供这些文件（`tftp_root = pe`）。

## 操作步骤

1. 管理端对金样机 MAC 创建「采集」任务  
2. 金样机 PXE 启动 → 报到登记 → 进入 Linux PE  
3. PE 内 `idv-pe-agent run` 流式读盘并上传  
4. 管理端「镜像」列表出现新镜像  

## 过滤（规划）

块级采集不会自动跳过 `pagefile.sys` 等。后续见 `docs/plans/2026-11-12-cold-capture-pe-design.md`：预处理清零 / v2 排除列表。

## 注意

- Edge 机器需要足够磁盘存放镜像（约等于金样机磁盘大小）  
- 首次需能访问 `dl-cdn.alpinelinux.org` 以下载 Alpine 与 modloop  
- 网卡驱动依赖 Alpine `modloop`；特殊网卡可能需定制内核模块  
