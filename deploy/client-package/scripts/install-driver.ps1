# Install SysRestore volume filter driver (TEST-SIGNED pilot only).
#
# Preconditions checked BEFORE touching the boot configuration:
#   - running as Administrator
#   - Secure Boot OFF   (testsigning cannot work with Secure Boot on)
#   - testsigning = Yes (already active, i.e. rebooted after enable-testsigning.ps1)
#   - SysRestoreDriver.sys / .cat are test-signed and the test cert is trusted
#
# A BOOT_START volume filter that fails signature checks = unbootable machine
# (0xC0000428 / 0x7B). Never register UpperFilters without passing the checks.
#
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\install-testcert.ps1
#   .\scripts\install-driver.ps1
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_common.ps1")

function Assert-Admin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($id)
  if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "必须以管理员身份运行此脚本。"
  }
}

Assert-Admin

$p = Get-YixuePaths
$DriverDir = $p.Driver
$sys = Join-Path $DriverDir "SysRestoreDriver.sys"
$inf = Join-Path $DriverDir "SysRestoreDriver.inf"
$cat = Join-Path $DriverDir "SysRestoreDriver.cat"
$cer = Join-Path $DriverDir "YixueIDV-SysRestore-Test.cer"

foreach ($f in @($sys, $inf, $cat, $cer)) {
  if (-not (Test-Path $f)) {
    throw "missing $f（包不完整；驱动必须先在构建机用 sign-sysrestore-testcert.ps1 签名并生成 cat）"
  }
}

# ---- Gate 1: Secure Boot must be off -------------------------------------
$secureBoot = $false
try { $secureBoot = [bool](Confirm-SecureBootUEFI) } catch { $secureBoot = $false }
if ($secureBoot) {
  throw "Secure Boot 已开启：testsigning 不会生效，测试签名驱动无法加载。请先关闭 Secure Boot，或改用正式签名的驱动。"
}

# ---- Gate 2: testsigning must already be active (reboot done) -------------
$bcd = (bcdedit /enum "{current}" | Out-String)
if ($bcd -notmatch "(?im)^\s*testsigning\s+Yes\s*$") {
  throw "testsigning 未生效。请先运行 .\scripts\enable-testsigning.ps1，重启后再运行本脚本。"
}
Write-Host "testsigning = Yes"

# ---- Gate 3: import cert, then verify signatures --------------------------
Write-Host "导入测试证书..."
& (Join-Path $PSScriptRoot "install-testcert.ps1")
if ($LASTEXITCODE -ne 0) { throw "install-testcert.ps1 failed" }

$sysSig = Get-AuthenticodeSignature $sys
if ($sysSig.Status -ne "Valid") {
  throw "SysRestoreDriver.sys 签名校验失败：Status=$($sysSig.Status)。该文件不是用随包 .cer 对应的测试证书签名的，禁止注册 BOOT_START 过滤驱动（会导致开机 0xC0000428/0x7B）。请在构建机重跑 sign-sysrestore-testcert.ps1 并重新打包。"
}
$catSig = Get-AuthenticodeSignature $cat
if ($catSig.Status -ne "Valid") {
  throw "SysRestoreDriver.cat 签名校验失败：Status=$($catSig.Status)"
}
Write-Host "签名校验通过：sys=$($sysSig.SignerCertificate.Subject) cat=$($catSig.SignerCertificate.Subject)"

# ---- Install INF ----------------------------------------------------------
$temp = Join-Path $env:TEMP "SysRestoreDriver"
New-Item -ItemType Directory -Force -Path $temp | Out-Null
Copy-Item -Force $sys, $inf, $cat $temp
$infTemp = Join-Path $temp "SysRestoreDriver.inf"

Write-Host "InstallHinfSection DefaultInstall 132 $infTemp"
$sig = '[DllImport("setupapi.dll", CharSet=CharSet.Unicode)] public static extern void InstallHinfSection(IntPtr hwnd, IntPtr hMod, string cmd, int show);'
Add-Type -MemberDefinition $sig -Name SetupApi -Namespace Native -ErrorAction SilentlyContinue
[Native.SetupApi]::InstallHinfSection([IntPtr]::Zero, [IntPtr]::Zero, "DefaultInstall 132 $infTemp", 0)

$svc = Get-Service -Name SysRestoreDriver -ErrorAction SilentlyContinue
if (-not $svc) {
  throw "Service SysRestoreDriver not found after INF install."
}
Write-Host "Service OK: StartType=$($svc.StartType) Status=$($svc.Status)"

# ---- Register as upper volume filter (last step; rollback ready) ----------
$volumeClassKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\{71a27cdd-812a-11d0-bec7-08002be2092f}"
$name = "SysRestoreDriver"
$existing = @()
try {
  $raw = (Get-ItemProperty -Path $volumeClassKey -Name UpperFilters -ErrorAction Stop).UpperFilters
  if ($raw) { $existing = @($raw) }
} catch { $existing = @() }

if ($existing -contains $name) {
  Write-Host "UpperFilters 已包含 $name"
} else {
  $newFilters = @($name) + $existing
  Set-ItemProperty -Path $volumeClassKey -Name UpperFilters -Value $newFilters -Type MultiString
  Write-Host "UpperFilters updated: $($newFilters -join ', ')"
}

Write-Host ""
Write-Host "驱动已注册为 BOOT_START 卷过滤驱动，重启后生效。"
Write-Host "重启前请确认：本机可进 WinRE 或已有快照；保留 .\scripts\uninstall-driver.ps1 用于回滚。"
Write-Host "若重启出现 0xC0000428 / 0x7B：进 WinRE 按 scripts\recover-boot.txt 移除 UpperFilters。"
Write-Host "INSTALL_DRIVER_OK"
