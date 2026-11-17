# 安装/卸载易学 IDV 边缘节点服务（Windows SCM）
#
# 用法（管理员 PowerShell）：
#   .\install-edge-service.ps1
#   .\install-edge-service.ps1 -InstallDir "D:\YixueIDV\Edge"
#   .\install-edge-service.ps1 -Uninstall
#
# 前提：
#   - idv-edge.exe、edge.toml 已放在安装目录；
#   - PE 资源在安装目录的 pe\ 下（DHCP/TFTP 需要）；
#   - DHCP 67 / TFTP 69 需要管理员权限，服务以 LocalSystem 运行。
param(
  [string]$InstallDir = (Join-Path $env:ProgramFiles "YixueIDV\Edge"),
  [string]$Config = "",
  [switch]$Uninstall,
  [switch]$NoStart
)
$ErrorActionPreference = "Stop"
$serviceName = "idv-edge"

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "请以管理员身份运行此脚本。"
}

if (-not $Config) { $Config = Join-Path $InstallDir "edge.toml" }
$exe = Join-Path $InstallDir "idv-edge.exe"

function Remove-EdgeService {
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
  Remove-EdgeService
  Write-Host "已卸载服务 $serviceName"
  exit 0
}

if (-not (Test-Path $exe)) { throw "找不到 $exe" }
if (-not (Test-Path $Config)) {
  throw "找不到 $Config。请从 deploy\config\edge.toml.example 复制并修改后放到安装目录。"
}
$peDir = Join-Path $InstallDir "pe"
if (-not (Test-Path $peDir)) {
  Write-Warning "未找到 $peDir；Edge 能启动，但 PXE/TFTP 资源可能不完整。"
}

Remove-EdgeService

# 工作目录设为安装目录，edge.toml 中的相对路径（data、cache、pe）才能正确解析。
$binPath = "`"$exe`" service --config `"$Config`" --workdir `"$InstallDir`""
New-Service -Name $serviceName -BinaryPathName $binPath `
  -DisplayName "易学 IDV 边缘节点服务" -StartupType Automatic | Out-Null

try {
  cmd /c "sc.exe description $serviceName `"易学 IDV 边缘节点：镜像缓存、DHCP/PXE/TFTP 与 WOL 转发。`" >nul 2>&1" | Out-Null
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
Write-Host "显示名：易学 IDV 边缘节点服务"
Write-Host "启动类型：自动（开机启动）"
Write-Host "安装目录：$InstallDir"
Write-Host "配置文件：$Config"
Write-Host "INSTALL_EDGE_SERVICE_OK"
