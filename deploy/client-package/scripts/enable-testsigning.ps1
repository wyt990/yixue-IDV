# Enable testsigning for the TEST-SIGNED pilot driver.
#
# Notes:
#   - Requires Administrator and Secure Boot OFF. With Secure Boot on, Windows
#     ignores/refuses testsigning; the test-signed driver will not load.
#   - Do NOT set `nointegritychecks on`: a properly signed driver does not need it,
#     and it weakens driver integrity checks machine-wide.
#   - An UNSIGNED driver is rejected even with testsigning on. The driver must be
#     signed on the build machine with sign-sysrestore-testcert.ps1 first.
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "必须以管理员身份运行此脚本。"
}

$secureBoot = $false
try { $secureBoot = [bool](Confirm-SecureBootUEFI) } catch { $secureBoot = $false }
if ($secureBoot) {
  throw "Secure Boot 已开启，testsigning 无法生效。请先在 UEFI/BIOS 中关闭 Secure Boot（试点机限定），或改用正式签名的驱动。"
}

bcdedit /enum "{current}" | Out-Host
bcdedit /set testsigning on | Out-Host
if ($LASTEXITCODE -ne 0) { throw "bcdedit /set testsigning on failed" }

Write-Host ""
Write-Host "testsigning 已开启。请重启，然后按顺序执行："
Write-Host "  .\scripts\install-testcert.ps1"
Write-Host "  .\scripts\install-driver.ps1"
Write-Host "驱动安装后再重启一次；若开机报 0xC0000428 / 0x7B，按 scripts\recover-boot.txt 回滚。"
