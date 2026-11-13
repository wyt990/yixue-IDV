# Install SysRestore volume filter driver (admin).
# Prerequisite: testsigning on + reboot; import test cert first.
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\install-testcert.ps1
#   .\scripts\install-driver.ps1
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_common.ps1")
$p = Get-YixuePaths
$DriverDir = $p.Driver
$sys = Join-Path $DriverDir "SysRestoreDriver.sys"
$inf = Join-Path $DriverDir "SysRestoreDriver.inf"
$cer = Join-Path $DriverDir "YixueIDV-SysRestore-Test.cer"
if (-not (Test-Path $sys)) { throw "missing $sys" }
if (-not (Test-Path $inf)) { throw "missing $inf" }

if (Test-Path $cer) {
  Write-Host "Ensuring test certificate is imported..."
  & (Join-Path $PSScriptRoot "install-testcert.ps1")
}

$bcd = (bcdedit /enum "{current}" | Out-String)
if ($bcd -notmatch "testsigning\s+Yes") {
  Write-Host "WARNING: testsigning not Yes. Run enable-testsigning.ps1 and reboot first."
}

$temp = Join-Path $env:TEMP "SysRestoreDriver"
New-Item -ItemType Directory -Force -Path $temp | Out-Null
Copy-Item -Force $sys, $inf $temp
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

$volumeClassKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\{71a27cdd-812a-11d0-bec7-08002be2092f}"
$name = "SysRestoreDriver"
$existing = @()
try {
  $raw = (Get-ItemProperty -Path $volumeClassKey -Name UpperFilters -ErrorAction Stop).UpperFilters
  if ($raw) { $existing = @($raw) }
} catch { $existing = @() }

if ($existing -notcontains $name) {
  $newFilters = @($name) + $existing
  Set-ItemProperty -Path $volumeClassKey -Name UpperFilters -Value $newFilters -Type MultiString
  Write-Host "UpperFilters updated: $($newFilters -join ', ')"
} else {
  Write-Host "UpperFilters already contains SysRestoreDriver"
}

Write-Host "Reboot required for BOOT_START filter. Then: .\scripts\restore-driver.ps1 protect-on"
Write-Host "INSTALL_DRIVER_OK"
