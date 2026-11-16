# 金样机准备（批量部署用）：安装 Agent、注册开机/轮询任务、关闭还原保护、
# 清理身份状态。之后采集/部署镜像，500 台首启自动收敛。
#
# 以管理员 PowerShell 执行：
#   powershell -ExecutionPolicy Bypass -File deploy\scripts\prepare-golden-client.ps1
#   powershell -ExecutionPolicy Bypass -File deploy\scripts\prepare-golden-client.ps1 -CenterUrl http://10.110.0.1:8080
param(
  [string]$PackageDir = "C:\Program Files\YixueIDV",
  [string]$CenterUrl = "",
  [int]$PollMinutes = 1,
  [switch]$EnableWakeOnLan
)
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "必须以管理员身份运行（注册计划任务 / 关闭驱动保护需要管理员）。"
}

if (-not (Test-Path $PackageDir)) { throw "找不到客户端包目录: $PackageDir" }
$scripts = Join-Path $PackageDir "scripts"
if (-not (Test-Path (Join-Path $scripts "install-client.ps1"))) {
  throw "客户端包不完整（缺少 scripts\install-client.ps1）: $PackageDir"
}

# 1) 通用配置：agent.toml 只保留 center_url；mac/hostname 留空自动
$config = Join-Path $PackageDir "agent.toml"
$example = Join-Path $PackageDir "agent.toml.example"
if (-not (Test-Path $config)) {
  if (-not (Test-Path $example)) { throw "缺少 $example（请使用新版客户端包）" }
  Copy-Item $example $config
  Write-Host "已生成 $config"
}

# 2) 安装常驻 Agent（Windows 服务；失败回退计划任务守护）
$serviceScript = Join-Path $scripts "install-agent-service.ps1"
if (Test-Path $serviceScript) {
  $installArgs = @(
    "-NoProfile", "-ExecutionPolicy", "Bypass",
    "-File", $serviceScript
  )
  if ($CenterUrl) { $installArgs += @("-CenterUrl", $CenterUrl) }
  & powershell.exe @installArgs
  if ($LASTEXITCODE -ne 0) { throw "install-agent-service.ps1 failed: $LASTEXITCODE" }
} else {
  # 兼容旧包：注册开机 + 轮询任务
  $installArgs = @(
    "-NoProfile", "-ExecutionPolicy", "Bypass",
    "-File", (Join-Path $scripts "install-client.ps1"),
    "-RegisterBootTask", "-PollMinutes", "$PollMinutes"
  )
  if ($CenterUrl) { $installArgs += @("-CenterUrl", $CenterUrl) }
  & powershell.exe @installArgs
  if ($LASTEXITCODE -ne 0) { throw "install-client.ps1 failed: $LASTEXITCODE" }
}

# 2.1) 可选：为远程开机启用有线网卡 WOL / 关闭快速启动
if ($EnableWakeOnLan) {
  $wolScript = Join-Path $scripts "enable-wol.ps1"
  if (Test-Path $wolScript) {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $wolScript
    if ($LASTEXITCODE -ne 0) { Write-Host "WARNING: enable-wol.ps1 返回 $LASTEXITCODE" }
  } else {
    Write-Host "WARNING: 客户端包缺少 scripts\\enable-wol.ps1，请手动配置 WOL"
  }
}

# 3) 确保还原保护处于关闭状态（首启先应用身份，再开保护）
$restore = Join-Path $scripts "restore-driver.ps1"
if (Test-Path $restore) {
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $restore protect-off
  if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: restore-driver.ps1 protect-off 失败（若驱动未安装可忽略）"
  }
}

# 4) 清理上一轮测试残留的身份状态
$stateDir = Join-Path $env:ProgramData "YixueIDV"
foreach ($name in @("identity-state.json", "identity.log")) {
  $p = Join-Path $stateDir $name
  if (Test-Path $p) { Remove-Item $p -Force; Write-Host "已清理 $p" }
}

Write-Host ""
Write-Host "金样机准备完成。检查清单："
Write-Host "  - $config 中 center_url 指向 Center，mac/hostname 保持注释/留空"
Write-Host "  - 常驻进程/服务：idv-agent 服务（或 YixueIdvAgentDaemon 计划任务）"
Write-Host "  - 还原保护关闭（restore-driver.ps1 status 查看）"
Write-Host "  - 不要 sysprep 后丢失服务/计划任务；如 sysprep，请在首次开机前重新执行本脚本"
Write-Host "  - 然后采集镜像 / 批量部署：每台机首启会自动改名、配网、开保护"
Write-Host "PREPARE_GOLDEN_OK"
