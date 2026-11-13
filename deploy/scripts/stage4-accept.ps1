# Stage 4 accept: capture -> ingest -> sync -> >=5 deploy -> disk plan preview -> corrupt fail -> task history
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$edgePort = 18081
$base = "http://127.0.0.1:$centerPort"
$edgeBase = "http://127.0.0.1:$edgePort"
$dataDir = Join-Path $root "data\stage4-accept"
$imagesDir = Join-Path $dataDir "images"
$cacheDir = Join-Path $dataDir "edge-cache"
$disksDir = Join-Path $dataDir "disks"
New-Item -ItemType Directory -Force -Path $dataDir, $imagesDir, $cacheDir, $disksDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"
$edgeCfg = Join-Path $dataDir "edge.toml"
$peCfg = Join-Path $dataDir "pe-agent.toml"

Get-Process idv-center,idv-edge,idv-pe-agent -ErrorAction SilentlyContinue | Stop-Process -Force
foreach ($p in @($centerPort, $edgePort)) {
  Get-NetTCPConnection -LocalPort $p -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
  }
}
Start-Sleep -Seconds 1
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $imagesDir "*") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $cacheDir "*") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $disksDir "*") -Force -ErrorAction SilentlyContinue

$imagesDirUri = ($imagesDir -replace "\\", "/")
@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
images_dir = "$imagesDirUri"
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
cargo build -p idv-center -p idv-edge -p idv-pe-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$edgeBin = Join-Path $root "target\debug\idv-edge.exe"
$peBin = Join-Path $root "target\debug\idv-pe-agent.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token
$edgeName = "stage4-edge-$(Get-Random)"
$edge = Invoke-Json POST "$base/api/v1/edges" @{ name = $edgeName } $token

@"
listen = "127.0.0.1:$edgePort"
center_url = "$base"
name = "$edgeName"
node_token = "$($edge.node_token)"
cache_dir = "$((Join-Path $dataDir 'edge-cache') -replace '\\','/')"
log = "warn"
heartbeat_secs = 3
poll_secs = 1
"@ | Set-Content -Encoding utf8 $edgeCfg

@"
edge_url = "$edgeBase"
log = "warn"
"@ | Set-Content -Encoding utf8 $peCfg

Write-Host "== start edge =="
$edgeProc = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edgeCfg) -WorkingDirectory $root
Wait-Http "$edgeBase/health" 60 | Out-Null
Start-Sleep -Seconds 2

# Golden disk: 90 bytes — system 60 + persistent 30 (default_partitions)
$golden = Join-Path $disksDir "golden.bin"
$bytes = New-Object byte[] 90
for ($i = 0; $i -lt 60; $i++) { $bytes[$i] = 0x11 }
for ($i = 60; $i -lt 90; $i++) { $bytes[$i] = 0x22 }
[System.IO.File]::WriteAllBytes($golden, $bytes)

$captureMac = "02:AA:BB:CC:DD:01"
Write-Host "== capture =="
$cap = Invoke-Json POST "$base/api/v1/tasks/capture" @{
  mac = $captureMac
  name = "stage4-golden"
  edge_id = $edge.id
  os_family = "windows"
} $token

& $peBin capture --config $peCfg --mac $captureMac --disk-file $golden --os-family windows --boot-mode uefi
if ($LASTEXITCODE -ne 0) { throw "capture failed" }

$images = Invoke-Json GET "$base/api/v1/images" $null $token
$image = @($images | Where-Object { $_.name -eq "stage4-golden" })[0]
if (-not $image) { throw "captured image not in library" }
if (-not $image.partition_map -or $image.partition_map -eq "[]") { throw "partition_map missing" }
Write-Host "capture ingest OK image=$($image.id)"

Write-Host "== sync to edge =="
Invoke-Json POST "$base/api/v1/edges/$($edge.id)/sync" @{ image_id = $image.id; purge_first = $false } $token | Out-Null
$ready = $false
for ($i = 0; $i -lt 60; $i++) {
  $replicas = Invoke-Json GET "$base/api/v1/edges/$($edge.id)/replicas" $null $token
  $hit = @($replicas | Where-Object { $_.image_id -eq $image.id -and $_.state -eq "ready" })
  if ($hit.Count -ge 1) { $ready = $true; break }
  Start-Sleep -Seconds 1
}
if (-not $ready) { throw "sync not ready" }

Write-Host "== disk plan preview =="
$preview = Invoke-Json POST "$base/api/v1/tasks/deploy/preview" @{
  image_id = $image.id
  macs = @("02:AA:BB:CC:DD:10")
  disk_plan = "wipe_target_partitions"
  confirm_destructive = $false
} $token
if (-not $preview.warning) { throw "preview missing warning" }
if (@($preview.overwrite_partitions).Count -lt 1) { throw "preview overwrite empty" }
Write-Host "disk plan preview OK"

$deployMacs = @(
  "02:AA:BB:CC:DD:10",
  "02:AA:BB:CC:DD:11",
  "02:AA:BB:CC:DD:12",
  "02:AA:BB:CC:DD:13",
  "02:AA:BB:CC:DD:14"
)
Write-Host "== deploy x5 wipe_whole_disk =="
$dep = Invoke-Json POST "$base/api/v1/tasks/deploy" @{
  image_id = $image.id
  macs = $deployMacs
  disk_plan = "wipe_whole_disk"
  name = "stage4-batch"
  edge_id = $edge.id
  confirm_destructive = $true
} $token

foreach ($mac in $deployMacs) {
  $disk = Join-Path $disksDir ("client-" + ($mac -replace ":", "") + ".bin")
  # seed with junk so overwrite is observable
  $junk = New-Object byte[] 90
  for ($i = 0; $i -lt 90; $i++) { $junk[$i] = 0x99 }
  [System.IO.File]::WriteAllBytes($disk, $junk)
  & $peBin deploy --config $peCfg --mac $mac --disk-file $disk
  if ($LASTEXITCODE -ne 0) { throw "deploy failed for $mac" }
  $got = [System.IO.File]::ReadAllBytes($disk)
  if ($got.Length -ne 90) { throw "deploy size mismatch $mac" }
  for ($i = 0; $i -lt 60; $i++) {
    if ($got[$i] -ne 0x11) { throw "deploy content mismatch system $mac" }
  }
  for ($i = 60; $i -lt 90; $i++) {
    if ($got[$i] -ne 0x22) { throw "deploy content mismatch data $mac" }
  }
}
Write-Host "5 deploys OK"

Write-Host "== write_system_only preserves data =="
$sysMac = "02:AA:BB:CC:DD:20"
$sysDisk = Join-Path $disksDir "client-sysonly.bin"
$old = New-Object byte[] 90
for ($i = 0; $i -lt 60; $i++) { $old[$i] = 0x55 }
for ($i = 60; $i -lt 90; $i++) { $old[$i] = 0x77 }
[System.IO.File]::WriteAllBytes($sysDisk, $old)
Invoke-Json POST "$base/api/v1/tasks/deploy" @{
  image_id = $image.id
  macs = @($sysMac)
  disk_plan = "write_system_only"
  name = "stage4-sysonly"
  edge_id = $edge.id
  confirm_destructive = $true
} $token | Out-Null
& $peBin deploy --config $peCfg --mac $sysMac --disk-file $sysDisk
if ($LASTEXITCODE -ne 0) { throw "sysonly deploy failed" }
$got = [System.IO.File]::ReadAllBytes($sysDisk)
for ($i = 0; $i -lt 60; $i++) {
  if ($got[$i] -ne 0x11) { throw "sysonly system not overwritten" }
}
for ($i = 60; $i -lt 90; $i++) {
  if ($got[$i] -ne 0x77) { throw "sysonly data not preserved" }
}
Write-Host "write_system_only OK"

Write-Host "== corrupt cache must fail loudly =="
$cached = Join-Path $cacheDir "$($image.id).bin"
if (-not (Test-Path $cached)) { throw "missing cache $cached" }
$raw = [System.IO.File]::ReadAllBytes($cached)
if ($raw.Length -lt 10) { throw "cache too small" }
$raw[8] = ($raw[8] + 1) % 256
[System.IO.File]::WriteAllBytes($cached, $raw)

$badMac = "02:AA:BB:CC:DD:99"
$badDisk = Join-Path $disksDir "client-corrupt.bin"
[System.IO.File]::WriteAllBytes($badDisk, $old)
Invoke-Json POST "$base/api/v1/tasks/deploy" @{
  image_id = $image.id
  macs = @($badMac)
  disk_plan = "wipe_whole_disk"
  name = "stage4-corrupt"
  edge_id = $edge.id
  confirm_destructive = $true
} $token | Out-Null
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"
& $peBin deploy --config $peCfg --mac $badMac --disk-file $badDisk 1>$null 2>$null
$deployCode = $LASTEXITCODE
$ErrorActionPreference = $prevEap
if ($deployCode -eq 0) { throw "corrupt deploy should fail" }
$marker = "$badDisk.deploy-failed"
if (-not (Test-Path $marker)) { throw "missing .deploy-failed marker" }
$tasks = Invoke-Json GET "$base/api/v1/tasks" $null $token
$corruptTask = @($tasks | Where-Object { $_.name -eq "stage4-corrupt" })[0]
if (-not $corruptTask) { throw "corrupt task missing" }
$failedTg = @($corruptTask.targets | Where-Object { $_.state -eq "failed" })
if ($failedTg.Count -lt 1) { throw "corrupt target not failed" }
Write-Host "corrupt fail OK"

Write-Host "== task history / csv =="
$headers = @{ Authorization = "Bearer $token" }
$csv = Invoke-WebRequest -Uri "$base/api/v1/tasks/export.csv" -Headers $headers -UseBasicParsing
if ($csv.StatusCode -ne 200) { throw "csv export failed" }
$csvText = $csv.Content
if ($csvText -is [byte[]]) {
  $csvText = [System.Text.Encoding]::UTF8.GetString($csvText)
}
if ($csvText -notmatch "stage4-batch") { throw "csv missing deploy history" }
$capTask = @($tasks | Where-Object { $_.kind -eq "capture" -and $_.state -eq "done" })
if ($capTask.Count -lt 1) { throw "capture task not done" }
Write-Host "task history OK"

Stop-Process -Id $edgeProc.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE4_ACCEPT_OK"
exit 0
