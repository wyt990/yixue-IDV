# Stage 5: 2 edges online + sync, cache watermark, P2P ratio, scale limits
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$edge1Port = 18081
$edge2Port = 18082
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage5-accept"
$imagesDir = Join-Path $dataDir "images"
New-Item -ItemType Directory -Force -Path $dataDir, $imagesDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"

Get-Process idv-center,idv-edge,idv-pe-agent -ErrorAction SilentlyContinue | Stop-Process -Force
foreach ($p in @($centerPort, $edge1Port, $edge2Port)) {
  Get-NetTCPConnection -LocalPort $p -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
  }
}
Start-Sleep -Seconds 1
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $imagesDir "*") -Force -ErrorAction SilentlyContinue

$imagesDirUri = ($imagesDir -replace "\\", "/")
@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
images_dir = "$imagesDirUri"
max_concurrent_deploy_targets = 80
max_pending_sync_per_edge = 16
recommended_deploy_batch = 40
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

function Wait-Replica($Token, $EdgeId, $ImageId, $Seconds = 60) {
  for ($i = 0; $i -lt $Seconds; $i++) {
    $replicas = Invoke-Json GET "$base/api/v1/edges/$EdgeId/replicas" $null $Token
    $hit = @($replicas | Where-Object { $_.image_id -eq $ImageId -and $_.state -eq "ready" })
    if ($hit.Count -ge 1) { return $hit[0] }
    Start-Sleep -Seconds 1
  }
  throw "replica not ready edge=$EdgeId image=$ImageId"
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

$limits = Invoke-Json GET "$base/api/v1/scale/limits" $null $token
if ($limits.recommended_deploy_batch -lt 1) { throw "scale limits missing" }
Write-Host "scale limits OK batch=$($limits.recommended_deploy_batch)"

# Seed a packed-like raw image file large enough for multiple chunks
$rawPath = Join-Path $dataDir "seed.raw"
$raw = New-Object byte[] 50000
for ($i = 0; $i -lt $raw.Length; $i++) { $raw[$i] = [byte]($i % 251) }
[System.IO.File]::WriteAllBytes($rawPath, $raw)

# Use pe-agent capture path: create golden + capture via one edge after edges up
# Simpler: register image metadata after packing with a tiny helper via capture flow

$edgeName1 = "stage5-e1-$(Get-Random)"
$edgeName2 = "stage5-e2-$(Get-Random)"
$edge1 = Invoke-Json POST "$base/api/v1/edges" @{ name = $edgeName1 } $token
$edge2 = Invoke-Json POST "$base/api/v1/edges" @{ name = $edgeName2 } $token

$cache1 = Join-Path $dataDir "cache1"
$cache2 = Join-Path $dataDir "cache2"
New-Item -ItemType Directory -Force -Path $cache1, $cache2 | Out-Null

function Write-EdgeCfg($Path, $Port, $Name, $NodeToken, $CacheDir) {
  $quota = 200000
  @"
listen = "127.0.0.1:$Port"
center_url = "$base"
name = "$Name"
node_token = "$NodeToken"
cache_dir = "$(($CacheDir -replace '\\','/'))"
cache_quota_bytes = $quota
cache_warn_pct = 50
sync_bandwidth_bps = 0
max_concurrent_syncs = 2
p2p_chunk_size = 16384
log = "warn"
heartbeat_secs = 2
poll_secs = 1
"@ | Set-Content -Encoding utf8 $Path
}

$edge1Cfg = Join-Path $dataDir "edge1.toml"
$edge2Cfg = Join-Path $dataDir "edge2.toml"
Write-EdgeCfg $edge1Cfg $edge1Port $edgeName1 $edge1.node_token $cache1
Write-EdgeCfg $edge2Cfg $edge2Port $edgeName2 $edge2.node_token $cache2

$peCfg = Join-Path $dataDir "pe.toml"
@"
edge_url = "http://127.0.0.1:$edge1Port"
log = "warn"
"@ | Set-Content -Encoding utf8 $peCfg

Write-Host "== start two edges =="
$e1 = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edge1Cfg) -WorkingDirectory $root
$e2 = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edge2Cfg) -WorkingDirectory $root
Wait-Http "http://127.0.0.1:$edge1Port/health" 60 | Out-Null
Wait-Http "http://127.0.0.1:$edge2Port/health" 60 | Out-Null
Start-Sleep -Seconds 3

$edges = Invoke-Json GET "$base/api/v1/edges" $null $token
$online = @($edges | Where-Object { $_.online -eq $true })
if ($online.Count -lt 2) { throw "need 2 edges online, got $($online.Count)" }
Write-Host "2 edges online OK"

# Capture golden into center via edge1
$golden = Join-Path $dataDir "golden.bin"
[System.IO.File]::WriteAllBytes($golden, $raw)
$capMac = "02:55:55:55:55:01"
Invoke-Json POST "$base/api/v1/tasks/capture" @{
  mac = $capMac
  name = "stage5-img"
  edge_id = $edge1.id
  os_family = "linux"
} $token | Out-Null
& $peBin capture --config $peCfg --mac $capMac --disk-file $golden --os-family linux --boot-mode uefi
if ($LASTEXITCODE -ne 0) { throw "capture failed" }

$images = Invoke-Json GET "$base/api/v1/images" $null $token
$image = @($images | Where-Object { $_.name -eq "stage5-img" })[0]
if (-not $image) { throw "image missing" }

Write-Host "== sync image to both edges =="
Invoke-Json POST "$base/api/v1/edges/$($edge1.id)/sync" @{ image_id = $image.id; purge_first = $false } $token | Out-Null
Invoke-Json POST "$base/api/v1/edges/$($edge2.id)/sync" @{ image_id = $image.id; purge_first = $false } $token | Out-Null
Wait-Replica $token $edge1.id $image.id | Out-Null
Wait-Replica $token $edge2.id $image.id | Out-Null
Write-Host "dual sync OK"

Write-Host "== cache watermark via heartbeat =="
Start-Sleep -Seconds 4
$edges = Invoke-Json GET "$base/api/v1/edges" $null $token
$e1view = @($edges | Where-Object { $_.id -eq $edge1.id })[0]
if ($null -eq $e1view.cache_used_bytes) { throw "cache_used_bytes missing" }
if ($e1view.cache_used_bytes -le 0) { throw "cache_used_bytes should be > 0" }
Write-Host "watermark fields OK level=$($e1view.cache_level) used=$($e1view.cache_used_bytes)"

Write-Host "== P2P deploy ratio =="
$macs = @("02:55:55:55:AA:01", "02:55:55:55:AA:02", "02:55:55:55:AA:03")
Invoke-Json POST "$base/api/v1/tasks/deploy" @{
  image_id = $image.id
  macs = $macs
  disk_plan = "wipe_whole_disk"
  name = "stage5-p2p"
  edge_id = $edge1.id
  confirm_destructive = $true
} $token | Out-Null

$disksDir = Join-Path $dataDir "disks"
New-Item -ItemType Directory -Force -Path $disksDir | Out-Null
$p2pSeen = $false
foreach ($mac in $macs) {
  $disk = Join-Path $disksDir ("d-" + ($mac -replace ":", "") + ".bin")
  $junk = New-Object byte[] 16
  [System.IO.File]::WriteAllBytes($disk, $junk)
  & $peBin deploy --config $peCfg --mac $mac --disk-file $disk --p2p
  if ($LASTEXITCODE -ne 0) { throw "p2p deploy failed $mac" }
}

$tasks = Invoke-Json GET "$base/api/v1/tasks" $null $token
$p2pTask = @($tasks | Where-Object { $_.name -eq "stage5-p2p" })[0]
if (-not $p2pTask) { throw "p2p task missing" }
foreach ($tg in $p2pTask.targets) {
  $total = [int64]$tg.p2p_bytes + [int64]$tg.edge_bytes
  if ($total -le 0) { throw "transfer counters empty for $($tg.mac)" }
  if ([int64]$tg.p2p_bytes -gt 0) { $p2pSeen = $true }
  Write-Host ("  {0} p2p={1} edge={2}" -f $tg.mac, $tg.p2p_bytes, $tg.edge_bytes)
}
if (-not $p2pSeen) { throw "expected later peers to pull from swarm (p2p_bytes>0)" }
Write-Host "P2P ratio visible OK"

Write-Host "== rotate token =="
$rotated = Invoke-Json POST "$base/api/v1/edges/$($edge2.id)/rotate_token" @{} $token
if (-not $rotated.node_token -or $rotated.node_token -eq $edge2.node_token) {
  throw "token rotate failed"
}
Write-Host "rotate token OK"

Write-Host "== docs present =="
$opsHit = Get-ChildItem (Join-Path $root "docs\modules") -Filter "11-*.md" -ErrorAction SilentlyContinue
if (-not $opsHit) { throw "ops manual missing (docs/modules/11-*.md)" }

Stop-Process -Id $e1.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $e2.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE5_ACCEPT_OK"
exit 0
