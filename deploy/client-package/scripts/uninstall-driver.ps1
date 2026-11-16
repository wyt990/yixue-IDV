# Remove SysRestore volume filter driver registration and service.
# Run as Administrator BEFORE rebooting, or from WinRE if the machine is already
# unbootable (see recover-boot.txt).
#
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\uninstall-driver.ps1
#   .\scripts\uninstall-driver.ps1 -DisableTestSigning   # also turn testsigning off
param(
  [switch]$DisableTestSigning
)
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "必须以管理员身份运行此脚本。"
}

$name = "SysRestoreDriver"
$volumeClassKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\{71a27cdd-812a-11d0-bec7-08002be2092f}"

# 1) Remove from volume UpperFilters (must happen before the service/file go away,
#    otherwise the next boot still tries to load a missing/invalid filter).
$upper = @()
try {
  $raw = (Get-ItemProperty -Path $volumeClassKey -Name UpperFilters -ErrorAction Stop).UpperFilters
  if ($raw) { $upper = @($raw) }
} catch { $upper = @() }

if ($upper -contains $name) {
  $remaining = @($upper | Where-Object { $_ -ne $name })
  $backup = Join-Path $env:TEMP "idv-upperfilters-backup.txt"
  $upper | Set-Content -Path $backup -Encoding UTF8
  if ($remaining.Count -eq 0) {
    Remove-ItemProperty -Path $volumeClassKey -Name UpperFilters -ErrorAction SilentlyContinue
  } else {
    Set-ItemProperty -Path $volumeClassKey -Name UpperFilters -Value $remaining -Type MultiString
  }
  Write-Host "UpperFilters 已移除 $name（原值备份：$backup）"
} else {
  Write-Host "UpperFilters 中未找到 $name"
}

# 2) Stop / delete the kernel service.
& sc.exe stop $name | Out-Host
& sc.exe delete $name | Out-Host

# 3) Delete the driver file.
$driverFile = Join-Path $env:SystemRoot "System32\drivers\SysRestoreDriver.sys"
if (Test-Path $driverFile) {
  try { Remove-Item $driverFile -Force; Write-Host "已删除 $driverFile" }
  catch { Write-Host "WARNING: 删除 $driverFile 失败（可能仍被占用），重启后手动删除或忽略（服务已删则不会加载）" }
}

# 4) Optionally turn testsigning off.
if ($DisableTestSigning) {
  bcdedit /set testsigning off | Out-Host
  if ($LASTEXITCODE -ne 0) { Write-Host "WARNING: bcdedit /set testsigning off failed" }
  else { Write-Host "testsigning 已关闭" }
}

Write-Host ""
Write-Host "卸载完成，请重启。若当前已因 0xC0000428/0x7B 无法进系统，请在 WinRE 中按 recover-boot.txt 操作。"
Write-Host "UNINSTALL_DRIVER_OK"
