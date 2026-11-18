# SysRestoreDriver 影子空间方案（2026-11-18）

## 目的

旧实现从 NTFS 空闲扇区位图动态挑选重定向目标。NTFS 在同一次开机中也会继续分配这些空闲簇，导致：

- 驱动把重定向数据写到某个“空闲”物理扇区；
- NTFS 之后把同一物理扇区分配给元数据；
- 驱动重定向 NTFS 的写，但 NTFS 读回时拿到旧数据；
- 最终卷元数据不一致，下次启动出现 0xc0000225。

## 新方案

在保护卷根目录预先分配固定大小的影子文件：

```text
C:\$YixueShadow.bin
D:\$YixueShadow.bin
```

该文件必须是**真实已分配的 NTFS 文件**，不能是稀疏文件。

驱动初始化时：

1. 读取 `$YixueShadow.bin` 的所有已分配簇；
2. 保存物理扇区区间到 `ShadowRanges`；
3. 所有非 ProtectRanges 的写都从影子区间顺序分配目标扇区；
4. 不再读取 `$Bitmap` 空闲位图来动态分配目标；
5. 不再把空闲扇区直接写到原盘；
6. 影子空间用尽时返回 `STATUS_DISK_FULL`，不会写坏原盘。

`ProtectRanges` 仍只保留：

- `_qh_protect_state.data`
- `$Volume` MFT #3 主/镜像

这些是启动/状态必需的直接写白名单。

## 准备影子文件

在客户端管理员 PowerShell 中，**先关闭保护**，然后执行：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\prepare-sysrestore-shadow.ps1 -Drives C:,D: -SizeMB 1024
```

脚本会：

- 创建隐藏/系统文件 `\$YixueShadow.bin`；
- 用 1MB 缓冲写入零数据，强制实际分配簇；
- 默认每个卷 1GB，可按测试需要调整 `-SizeMB`。

然后重启加载新驱动，再开启保护。

## 删除（测试回滚）

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\prepare-sysrestore-shadow.ps1 -Drives C:,D: -Remove
```

删除前必须先关闭保护。

## 注意

- 影子文件占用磁盘空间，C/D 各 1GB 是测试默认值；
- 需要保护的卷都必须有影子文件，否则该卷保护不会激活；
- 影子空间不是 NTFS 快照，不能保护分区表/ESP/磁盘级写入；
- 本版本仍是测试签名驱动。
