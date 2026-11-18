param(
  [string[]]$Drives = @("C:", "D:"),
  [int]$SizeMB = 1024,
  [switch]$Remove
)

# Pre-allocate the SysRestoreDriver shadow file on each protected volume.
# Must be run while protection is OFF (or from PE/another OS for the target disk).
# The driver uses only the allocated clusters of this file as redirect targets.

$ErrorActionPreference = "Stop"
if ($SizeMB -lt 64) { throw "SizeMB must be >= 64" }
$size = [int64]$SizeMB * 1MB

foreach ($drive in $Drives) {
  $root = "$drive\"
  if (-not (Test-Path $root)) {
    Write-Host "SKIP $drive (not present)"
    continue
  }
  $path = Join-Path $root '$YixueShadow.bin'
  if ($Remove) {
    if (Test-Path $path) {
      Set-ItemProperty -Path $path -Name Attributes -Value Normal -ErrorAction SilentlyContinue
      Remove-Item $path -Force
      Write-Host "REMOVED $path"
    }
    continue
  }

  $fs = $null
  try {
    $fs = [System.IO.File]::Open($path, [System.IO.FileMode]::OpenOrCreate,
      [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
    if ($fs.Length -ne $size) {
      $fs.SetLength($size)
    }
    # Force real cluster allocation (SetLength alone can leave sparse holes).
    $buf = New-Object byte[] (1MB)
    $fs.Position = 0
    $remaining = $size
    while ($remaining -gt 0) {
      $n = [int][Math]::Min($buf.Length, $remaining)
      $fs.Write($buf, 0, $n)
      $remaining -= $n
    }
    $fs.Flush($true)
    $fs.Close()
    $fs = $null
    $item = Get-Item $path -Force
    $item.Attributes = [System.IO.FileAttributes]::Hidden -bor [System.IO.FileAttributes]::System
    Write-Host ("READY {0} ({1} MB)" -f $path, $SizeMB)
  } finally {
    if ($fs) { $fs.Dispose() }
  }
}
