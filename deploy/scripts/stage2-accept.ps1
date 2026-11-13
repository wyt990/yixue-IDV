# 阶段 2 验收：Edge 在线、同步 checksum、清空缓存、Center 断开后 Edge 仍存活
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$edgePort = 18081
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage2-accept"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $dataDir "images") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $dataDir "edge-cache") | Out-Null

$db = (Join-Path $dataDir "center.db") -replace "\\", "/"
$imgPath = Join-Path $dataDir "images\sample.bin"
$cacheDir = Join-Path $dataDir "edge-cache"
$centerCfg = Join-Path $dataDir "center.toml"
$edgeCfg = Join-Path $dataDir "edge.toml"

Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $cacheDir "*") -Force -ErrorAction SilentlyContinue

$payload = [System.Text.Encoding]::UTF8.GetBytes("yixue-idv-stage2-sample-image-payload")
[System.IO.File]::WriteAllBytes($imgPath, $payload)
$hash = (Get-FileHash -Algorithm SHA256 -Path $imgPath).Hash.ToLowerInvariant()
$size = (Get-Item $imgPath).Length
$imgUri = ($imgPath -replace "\\", "/")

@"
listen = "127.0.0.1:$centerPort"
log = "info,idv_center=warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
"@ | Set-Content -Encoding utf8 $centerCfg

function Invoke-Json($Method, $Path, $Body = $null, $Token = $null, $EdgeToken = $null) {
  $headers = @{}
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  if ($EdgeToken) { $headers."X-Idv-Edge-Token" = $EdgeToken }
  if ($null -eq $Body) {
    return Invoke-RestMethod -Method $Method -Uri "$base$Path" -Headers $headers
  }
  $json = $Body | ConvertTo-Json -Compress -Depth 8
  return Invoke-RestMethod -Method $Method -Uri "$base$Path" -Headers $headers -ContentType "application/json" -Body ([System.Text.Encoding]::UTF8.GetBytes($json))
}

function Wait-Http($Url, $Seconds = 30) {
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
cargo build -p idv-center -p idv-edge | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$edgeBin = Join-Path $root "target\debug\idv-edge.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null

Write-Host "== login / create edge / image =="
$login = Invoke-Json POST "/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token
$edge = Invoke-Json POST "/api/v1/edges" @{ name = "accept-edge" } $token
$image = Invoke-Json POST "/api/v1/images" @{
  name = "stage2-sample"
  os_family = "linux"
  version_label = "accept"
  checksum_sha256 = $hash
  size_bytes = $size
  storage_uri = $imgUri
} $token

@"
listen = "127.0.0.1:$edgePort"
center_url = "$base"
name = "accept-edge"
node_token = "$($edge.node_token)"
cache_dir = "$($cacheDir -replace '\\','/')"
log = "info,idv_edge=warn"
heartbeat_secs = 3
poll_secs = 1
"@ | Set-Content -Encoding utf8 $edgeCfg

Write-Host "== start edge =="
$edgeProc = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edgeCfg) -WorkingDirectory $root
Wait-Http "http://127.0.0.1:$edgePort/health" 60 | Out-Null
Start-Sleep -Seconds 4

$edges = Invoke-Json GET "/api/v1/edges" $null $token
$online = @($edges | Where-Object { $_.online -eq $true })
if ($online.Count -lt 1) { throw "edge not online" }
Write-Host "edge online OK"

Write-Host "== sync image =="
Invoke-Json POST "/api/v1/edges/$($edge.id)/sync" @{ image_id = $image.id; purge_first = $false } $token | Out-Null

$ready = $false
for ($i = 0; $i -lt 60; $i++) {
  $replicas = Invoke-Json GET "/api/v1/edges/$($edge.id)/replicas" $null $token
  $hit = @($replicas | Where-Object { $_.image_id -eq $image.id -and $_.state -eq "ready" })
  if ($hit.Count -ge 1) {
    if ($hit[0].checksum_sha256 -ne $hash) { throw "checksum mismatch on replica" }
    $ready = $true
    break
  }
  Start-Sleep -Seconds 1
}
if (-not $ready) { throw "sync did not become ready" }

$cached = Join-Path $cacheDir "$($image.id).bin"
if (-not (Test-Path $cached)) { throw "cache file missing: $cached" }
$cachedHash = (Get-FileHash -Algorithm SHA256 -Path $cached).Hash.ToLowerInvariant()
if ($cachedHash -ne $hash) { throw "local cache checksum mismatch" }
Write-Host "sync + checksum OK"

Write-Host "== purge then resync =="
Invoke-Json POST "/api/v1/edges/$($edge.id)/replicas/$($image.id)/purge" @{} $token | Out-Null
Start-Sleep -Seconds 3
if (Test-Path $cached) { throw "cache file still exists after purge" }
Invoke-Json POST "/api/v1/edges/$($edge.id)/sync" @{ image_id = $image.id; purge_first = $false } $token | Out-Null
$ready2 = $false
for ($i = 0; $i -lt 60; $i++) {
  $replicas = Invoke-Json GET "/api/v1/edges/$($edge.id)/replicas" $null $token
  $hit = @($replicas | Where-Object { $_.image_id -eq $image.id -and $_.state -eq "ready" })
  if ($hit.Count -ge 1) { $ready2 = $true; break }
  Start-Sleep -Seconds 1
}
if (-not $ready2) { throw "resync failed" }
Write-Host "purge + resync OK"

Write-Host "== stop center, edge should survive =="
Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort $centerPort -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 2
$edgeHealth = Invoke-RestMethod -Uri "http://127.0.0.1:$edgePort/health"
if ($edgeHealth.status -ne "ok") { throw "edge died after center stop" }
Write-Host "edge survived center disconnect OK"

Stop-Process -Id $edgeProc.Id -Force -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort $edgePort -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}

Write-Host "STAGE2_ACCEPT_OK"
