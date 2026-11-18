param(
  [string]$ExpectedHash = "",
  [string]$DevicePath = "\\.\QHEngineControlDevice"
)

# Diagnostic: verify which SysRestoreDriver is loaded and whether shadow ranges
# were loaded. New shadow-build drivers answer IOCTL 0x80002008.

$ErrorActionPreference = "Stop"

$driverPath = Join-Path $env:SystemRoot "System32\drivers\SysRestoreDriver.sys"
if (Test-Path $driverPath) {
  $f = Get-Item $driverPath
  $h = (Get-FileHash $driverPath -Algorithm SHA256).Hash
  $sig = Get-AuthenticodeSignature $driverPath
  Write-Host "driver_file : $driverPath"
  Write-Host "driver_size : $($f.Length)"
  Write-Host "driver_time : $($f.LastWriteTime)"
  Write-Host "driver_hash : $h"
  Write-Host "driver_sig  : $($sig.Status)"
  if ($ExpectedHash -and $h -ne $ExpectedHash.ToUpperInvariant()) {
    Write-Host "HASH_MISMATCH: installed driver is NOT the expected package build"
  }
} else {
  Write-Host "driver_file : NOT FOUND"
}

try {
  $svc = Get-CimInstance Win32_SystemDriver -Filter "Name='SysRestoreDriver'"
  Write-Host "service     : State=$($svc.State) Started=$($svc.Started) Path=$($svc.PathName)"
} catch {
  Write-Host "service     : query failed: $($_.Exception.Message)"
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class QHStatusNative {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern IntPtr CreateFileW(string name, uint access, uint share,
    IntPtr sa, uint create, uint flags, IntPtr template);
  [DllImport("kernel32.dll", SetLastError=true)]
  public static extern bool DeviceIoControl(IntPtr h, uint code,
    IntPtr inBuf, uint inSize, byte[] outBuf, uint outSize,
    out uint returned, IntPtr overlapped);
  [DllImport("kernel32.dll")]
  public static extern bool CloseHandle(IntPtr h);
}
"@ -ErrorAction SilentlyContinue

$handle = [QHStatusNative]::CreateFileW($DevicePath, [uint32]::Parse("3221225472"), [uint32]0x3,
  [IntPtr]::Zero, [uint32]3, [uint32]0, [IntPtr]::Zero)
if ($handle -eq [IntPtr](-1)) {
  Write-Host "control     : OPEN FAILED (driver not loaded or old driver without status IOCTL)"
  exit 2
}

try {
  $buf = New-Object byte[] 64
  $returned = 0
  $ok = [QHStatusNative]::DeviceIoControl($handle, [uint32]::Parse("2147491848"),
    [IntPtr]::Zero, 0, $buf, [uint32]$buf.Length, [ref]$returned, [IntPtr]::Zero)
  if (-not $ok) {
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Host "control     : STATUS IOCTL FAILED win32=$err (old driver build)"
    exit 3
  }
  $magic = [BitConverter]::ToUInt32($buf, 0)
  $ver = [BitConverter]::ToUInt32($buf, 4)
  $init = [BitConverter]::ToUInt32($buf, 8)
  $shadowReady = [BitConverter]::ToUInt32($buf, 12)
  $rangeMax = [BitConverter]::ToUInt32($buf, 16)
  $sectors = [BitConverter]::ToUInt64($buf, 24)
  $used = [BitConverter]::ToUInt64($buf, 32)
  $last = [BitConverter]::ToUInt32($buf, 40)
  Write-Host ("control     : magic=0x{0:X8} version={1}" -f $magic, $ver)
  Write-Host "volumes     : initialized=$init shadow_ready=$shadowReady protect_range_max=$rangeMax"
  Write-Host "shadow      : total_sectors=$sectors used_sectors=$used"
  Write-Host ("last_init   : 0x{0:X8}" -f $last)
  if ($ver -lt 2 -or $shadowReady -eq 0) {
    Write-Host "SHADOW_NOT_ACTIVE: check shadow file and driver build"
    exit 4
  }
  Write-Host "SHADOW_STATUS_OK"
} finally {
  [QHStatusNative]::CloseHandle($handle) | Out-Null
}
