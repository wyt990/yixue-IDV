# 易学 IDV 集成说明（SysRestoreDriver）

本目录由 `temp/SysRestoreDriver` 引入，协议 **Apache License 2.0**（见 `LICENSE` / `NOTICE`）。

## 在 IDV 中的角色

- **封板**：仍用全量 PXE 部署镜像  
- **日常还原**：卷过滤写时重定向；重启丢弃写入，回到开启保护时的状态  
- **策略**：`idv-agent sync-policy` 经 Center 心跳下发 `DesiredEndpointState`；默认 **重启后生效**

## 构建（需本机安装）

- Visual Studio 2022 + Windows SDK + **WDK**
- 打开 `SysRestoreDriver.sln`，配置 `Release | x64`，生成

产物：`SysRestoreDriver.sys`、`QHEngineUI.exe`（UI 内嵌驱动资源）。

## 试点加载（测试签名）

管理员 PowerShell：

```powershell
bcdedit /set testsigning on
# 重启后
# 用 QHEngineUI 勾选系统卷并「开启保护」，或后续由 idv-agent IOCTL 路径接管
```

桌面出现「测试模式」水印属正常。量产需 EV 代码签名，本刀不覆盖。

## Agent 控制面

```text
idv-agent restore-driver status
idv-agent restore-driver protect-on --volumes system
idv-agent restore-driver protect-off
idv-agent sync-policy --apply
```

开/关保护与 QHEngineUI 相同：在系统盘根目录写 `_qh_protect_state.data`（1MB，首字节 1/0）。**需重启后驱动才按文件激活**。  
`status` 中 `armed`=状态文件意图，`frozen`=IOCTL 查询到的运行时保护（驱动未加载时回落为 armed）。

强制 Mock（不写真实卷）：

```powershell
$env:IDV_RESTORE_MOCK=1
idv-agent restore-driver protect-on --volumes system
```

## 无驱动 / Mock

未设置 Mock 时 Windows 默认走 **StateFile**。设置 `IDV_RESTORE_MOCK=1` 时用 `%ProgramData%\YixueIDV\restore-driver-state.json`，仅联调策略，**不提供真还原**。

## 参考

- 上游架构：`ARCHITECTURE.md`  
- IDV 设计：`docs/plans/2026-11-13-endpoint-restore-control-design.md`
