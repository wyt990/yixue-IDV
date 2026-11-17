# 安装/卸载易学 IDV 中心服务（Windows SCM）
#
# 用法（管理员 PowerShell）：
#   .\install-center-service.ps1
#   .\install-center-service.ps1 -InstallDir "D:\YixueIDV\Center"
#   .\install-center-service.ps1 -Uninstall
#
# 前提：
#   - idv-center.exe 和 center.toml 已放在安装目录；
#   - web\dist 已放在安装目录（否则管理端页面打开是空白）。
param(
  [string]$InstallDir = (Join-Path $env:ProgramFiles "YixueIDV\Center"),
  [string]$Config = "",
  [switch]$Uninstall,
  [switch]$NoStart
)
$ErrorActionPreference = "Stop"
$serviceName = "idv-center"

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "请以管理员身份运行此脚本。"
}

if (-not $Config) { $Config = Join-Path $InstallDir "center.toml" }
$exe = Join-Path $InstallDir "idv-center.exe"

function Remove-CenterService {
  try {
    $svc = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if ($svc) {
      Write-Host "停止并删除已有服务 $serviceName ..."
      Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
      Start-Sleep -Seconds 1
      cmd /c "sc.exe delete $serviceName >nul 2>&1" | Out-Null
      Start-Sleep -Seconds 1
    }
  } catch {
    cmd /c "sc.exe delete $serviceName >nul 2>&1" | Out-Null
  }
}

if ($Uninstall) {
  Remove-CenterService
  Write-Host "已卸载服务 $serviceName"
  exit 0
}

if (-not (Test-Path $exe)) { throw "找不到 $exe" }
if (-not (Test-Path $Config)) {
  throw "找不到 $Config。请从 deploy\config\center.toml.example 复制并修改后放到安装目录。"
}
$web = Join-Path $InstallDir "web\dist"
if (-not (Test-Path (Join-Path $web "index.html"))) {
  Write-Warning "未找到 $web\index.html；服务能启动，但管理端页面可能不可用。"
}

Remove-CenterService

# Service 命令需要传入配置和工作目录；相对路径（web/dist、data、images）
# 都相对于 InstallDir 解析。
$binPath = "`"$exe`" service --config `"$Config`" --workdir `"$InstallDir`""
New-Service -Name $serviceName -BinaryPathName $binPath `
  -DisplayName "易学 IDV 中心服务" -StartupType Automatic | Out-Null

try {
  cmd /c "sc.exe description $serviceName `"易学 IDV 中心管理服务：台账、镜像、任务、远程控制与 WOL 中继。`" >nul 2>&1" | Out-Null
} catch {
  Write-Warning "设置服务描述失败（不影响启动）"
}
try {
  cmd /c "sc.exe failure $serviceName reset= 86400 actions= restart/5000/restart/10000/restart/30000 >nul 2>&1" | Out-Null
} catch {
  Write-Warning "设置失败自动重启策略失败（不影响启动）"
}

if (-not $NoStart) {
  Start-Service -Name $serviceName
  Start-Sleep -Seconds 2
  $svc = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
  $status = if ($svc) { $svc.Status } else { "not-found" }
  Write-Host "服务状态: $status"
  if ($status -ne "Running") {
    throw "服务未进入 Running 状态（$status）。请检查 Event Viewer 或配置路径：$Config"
  }
}

Write-Host "服务已安装：$serviceName"
Write-Host "显示名：易学 IDV 中心服务"
Write-Host "启动类型：自动（开机启动）"
Write-Host "安装目录：$InstallDir"
Write-Host "配置文件：$Config"
Write-Host "INSTALL_CENTER_SERVICE_OK"
