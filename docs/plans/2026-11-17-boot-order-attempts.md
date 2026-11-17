# 启动顺序自动切换尝试、失败与总结（2026-11-17）

## 背景

目标：在管理端按机房选择“硬盘优先 / PXE 优先”，由 Agent 自动修改客户端启动顺序，避免 500 台机器逐台进 BIOS。

现场机器：

- AMI Aptio Setup 5.19
- Project JRKLD 0.05 x64
- BIOS Boot 页面为 `FIXED BOOT ORDER`
- `Boot Option #1` 可选 `Network:UEFI: PXE IPv4` / `Hard Disk:Windows Boot Manager`
- Secure Boot 关闭，单网卡

## 尝试过程

### 1. `bcdedit /set {fwbootmgr} displayorder`

做法：把 Windows Boot Manager 排到固件启动顺序第一。

结果：

- `bcdedit` 内显示成功；
- 重启仍进入 PXE；
- BIOS 自定义的 `FIXED BOOT ORDER` 会在下次 POST 重新决定顺序。

结论：该 BIOS 不把 Windows 写入的 BootOrder 作为最终依据。

### 2. `bcdedit /set {fwbootmgr} bootsequence`

做法：使用一次性启动项覆盖。

结果：

- 重启仍然 PXE；
- 该固件没有按预期使用这个一次性覆盖。

### 3. 直接写 UEFI `BootOrder` / `BootNext`

做法：通过 `GetFirmwareEnvironmentVariableExW` / `SetFirmwareEnvironmentVariableExW` 直接读写固件变量，相当于 `efibootmgr -o/-n`。

结果：

- 写入后立即可读到 Windows 第一；
- 重启后又被 BIOS 恢复成 PXE 第一；
- 期间出现通过网络重启后卡在 BIOS POST，必须强制断电；
- 该方案风险过高，已撤销。

### 4. Windows Boot Manager 链式加载 `ipxe.efi`

做法：

- 复制 `ipxe.efi` 到 ESP 的 `\EFI\Yixue\ipxe.efi`；
- 在 BCD 中复制 `{bootmgr}` 启动项，path 指向该文件；
- 部署时设置 `{bootmgr} bootsequence` 到该启动项。

结果：

```text
Windows 未能启动
文件: \EFI\Yixue\ipxe.efi
状态: 0xc000007b
信息: 无法加载应用程序或操作系统，原因是所需文件丢失或包含错误
```

根因：Windows Boot Manager 的 BCD 启动项不是通用 EFI 链加载器，不能直接把非 Windows EFI 应用（iPXE）当作 BCD osloader 启动。0xc000007b 是镜像格式/启动类型不匹配，不是文件没复制。

结论：方案不可行，必须删除该 BCD 启动项，避免重启即坏。

### 5. 仅写 `BootNext` 一次性 PXE

做法：不改 BootOrder，只把 BootNext 指向现有 PXE 启动项。

结果：

- 自检能看到 `BootNext=0003`；
- 但现场最终决定不再做任何启动项自动修改，避免再次引入不可控的固件差异。

## 最终决定

1. 继续使用原方案：**BIOS 中 PXE 永久第一**；
2. 无维护任务时，iPXE check-in 后 `sanboot` 本地硬盘；
3. 删除所有启动项自动修改相关代码、数据库字段、API、Web 设置和自检入口；
4. 管理端不再提供“启动方式”选择；
5. 部署/采集仍按原流程：客户端 PXE → iPXE → PE。

## 代码清理

- 删除 Agent 的 boot_mode / BootNext / BootOrder / BCD 启动项 / boot_diag 代码；
- 删除 Center 的 lab boot_mode 字段、心跳字段、API、命令类型；
- 删除 Edge DHCP 的 iPXE user-class 特殊分支；
- 删除 Web 的机房启动方式下拉框和客户端启动自检按钮；
- 迁移 025 增加过 labs.boot_mode，迁移 026 删除该列，保持已有数据库兼容。

## 经验

- BIOS 的 `FIXED BOOT ORDER` 不等于可写的 UEFI BootOrder；
- `bcdedit` 显示成功不代表固件重启后仍采用；
- BCD 不能作为任意 EFI 应用的通用链加载器；
- 任何启动相关写入都必须先验证、失败可回滚，否则可能让整批机器无法启动；
- 对 500 台现场，优先选择“不修改固件启动顺序”的方案。
