# Stage 6 P0: modes, agent restore, delta recommend
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage6-accept"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"
$agentCfg = Join-Path $dataDir "agent.toml"

Get-Process idv-center,idv-agent -ErrorAction SilentlyContinue | Stop-Process -Force
Get-NetTCPConnection -LocalPort $centerPort -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 2
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
if (Test-Path $db) {
  throw "unable to clear stage6 db (process still locking?): $db"
}

@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
images_dir = "$((Join-Path $dataDir 'images') -replace '\\','/')"
"@ | Set-Content -Encoding utf8 $centerCfg

function Invoke-Json($Method, $Url, $Body = $null, $Token = $null) {
  $headers = @{}
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  if ($null -eq $Body) {
    return Invoke-RestMethod -Method $Method -Uri $Url -Headers $headers
  }
  $json = $Body | ConvertTo-Json -Compress -Depth 8
  return Invoke-RestMethod -Method $Method -Uri $Url -Headers $headers -ContentType "application/json" -Body ([System.Text.Encoding]::UTF8.GetBytes($json))
}

function Wait-Http($Url, $Seconds = 40) {
  for ($i = 0; $i -lt $Seconds; $i++) {
    try {
      $r = Invoke-RestMethod -Uri $Url -TimeoutSec 2
      if ($r.status -eq "ok") { return $r }
    } catch {}
    Start-Sleep -Seconds 1
  }
  throw "timeout waiting for $Url"
}

Write-Host "== build =="
cargo build -p idv-center -p idv-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$agentBin = Join-Path $root "target\debug\idv-agent.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

$modes = Invoke-Json GET "$base/api/v1/modes" $null $token
# Prefer stable English names; fall back to restore semantics for older seeds.
$teach = @($modes | Where-Object { $_.name -eq "teaching" -or ($_.restore -eq "reboot" -and $_.system_action -eq "restore") })[0]
$office = @($modes | Where-Object { $_.name -eq "office" -or $_.restore -eq "off" })[0]
if (-not $teach -or -not $office) {
  throw ("default modes missing count=" + @($modes).Count)
}

$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "stage6-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "stage6-lab" } $token
$mac = "02:66:66:66:66:01"
Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "pc1" } $token | Out-Null
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $teach.id; effective = "next_boot" } $token | Out-Null

@"
center_url = "$base"
mac = "$mac"
hostname = "stage6-pc"
log = "warn"
"@ | Set-Content -Encoding utf8 $agentCfg

# Baseline disk: system 60 = 0x11, persistent 30 = 0x22
$baseDisk = Join-Path $dataDir "baseline-disk.bin"
$bytes = New-Object byte[] 90
for ($i = 0; $i -lt 60; $i++) { $bytes[$i] = 0x11 }
for ($i = 60; $i -lt 90; $i++) { $bytes[$i] = 0x22 }
[System.IO.File]::WriteAllBytes($baseDisk, $bytes)
$baseline = Join-Path $dataDir "baseline.yixue"
& $agentBin pack-baseline --disk-file $baseDisk --out $baseline --os-family windows
if ($LASTEXITCODE -ne 0) { throw "pack baseline failed" }

# Dirty local disk
$local = Join-Path $dataDir "local-disk.bin"
$dirty = New-Object byte[] 90
for ($i = 0; $i -lt 60; $i++) { $dirty[$i] = 0xAA }
for ($i = 60; $i -lt 90; $i++) { $dirty[$i] = 0x77 }
[System.IO.File]::WriteAllBytes($local, $dirty)

Write-Host "== teaching restore on boot =="
& $agentBin boot --config $agentCfg --disk-file $local --baseline-file $baseline
if ($LASTEXITCODE -ne 0) { throw "boot restore failed" }
$got = [System.IO.File]::ReadAllBytes($local)
for ($i = 0; $i -lt 60; $i++) {
  if ($got[$i] -ne 0x11) { throw "system not restored" }
}
for ($i = 60; $i -lt 90; $i++) {
  if ($got[$i] -ne 0x77) { throw "persistent not kept" }
}
Write-Host "teaching restore OK"

Write-Host "== office mode skips restore =="
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $office.id; effective = "immediate" } $token | Out-Null
for ($i = 0; $i -lt 60; $i++) { $dirty[$i] = 0xBB }
[System.IO.File]::WriteAllBytes($local, $dirty)
& $agentBin boot --config $agentCfg --disk-file $local --baseline-file $baseline
if ($LASTEXITCODE -ne 0) { throw "office boot failed" }
$got = [System.IO.File]::ReadAllBytes($local)
for ($i = 0; $i -lt 60; $i++) {
  if ($got[$i] -ne 0xBB) { throw "office should not restore system" }
}
Write-Host "office skip OK"

Write-Host "== delta recommend =="
# register two full images with same partition_map
$pm = '[{"name":"system","role":"system","offset":0,"length":60},{"name":"data","role":"persistent","offset":60,"length":30}]'
$imgA = Invoke-Json POST "$base/api/v1/images" @{
  name = "base-a"
  os_family = "windows"
  checksum_sha256 = ("a" * 64)
  size_bytes = 1000
  partition_map = $pm
  content_type = "full"
} $token
$imgB = Invoke-Json POST "$base/api/v1/images" @{
  name = "target-b"
  os_family = "windows"
  checksum_sha256 = ("b" * 64)
  size_bytes = 1100
  partition_map = $pm
  content_type = "full"
} $token
$rec = Invoke-Json POST "$base/api/v1/images/recommend-delta" @{
  base_image_id = $imgA.id
  target_image_id = $imgB.id
  estimated_delta_ratio = 0.2
} $token
if ($rec.recommendation -ne "delta") { throw "expected delta recommendation" }
$recFull = Invoke-Json POST "$base/api/v1/images/recommend-delta" @{
  base_image_id = $imgA.id
  target_image_id = $imgB.id
  force_full = $true
} $token
if ($recFull.recommendation -ne "full") { throw "expected forced full" }
Write-Host "delta recommend OK"

$sign = Get-ChildItem (Join-Path $root "docs\modules") -Filter "12-*.md" -ErrorAction SilentlyContinue
if (-not $sign) { throw "phase2 signoff page missing" }

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE6_ACCEPT_OK"
exit 0
