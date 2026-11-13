# Stage 6c: P2P stop seeding, USB enforce, PE network wizard, themes present
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$edgePort = 18081
$base = "http://127.0.0.1:$centerPort"
$edgeBase = "http://127.0.0.1:$edgePort"
$dataDir = Join-Path $root "data\stage6c-accept"
$imagesDir = Join-Path $dataDir "images"
$cacheDir = Join-Path $dataDir "edge-cache"
New-Item -ItemType Directory -Force -Path $dataDir, $imagesDir, $cacheDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"
$edgeCfg = Join-Path $dataDir "edge.toml"
$agentCfg = Join-Path $dataDir "agent.toml"
$peCfg = Join-Path $dataDir "pe-agent.toml"

Get-Process idv-center,idv-edge,idv-agent,idv-pe-agent -ErrorAction SilentlyContinue | Stop-Process -Force
foreach ($p in @($centerPort, $edgePort)) {
  Get-NetTCPConnection -LocalPort $p -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
  }
}
Start-Sleep -Seconds 2
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
if (Test-Path $db) { throw "unable to clear stage6c db: $db" }
Remove-Item (Join-Path $imagesDir "*") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $cacheDir "*") -Force -Recurse -ErrorAction SilentlyContinue

$imagesDirUri = ($imagesDir -replace "\\", "/")
$cacheDirUri = ($cacheDir -replace "\\", "/")
@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
images_dir = "$imagesDirUri"
"@ | Set-Content -Encoding utf8 $centerCfg

function Invoke-Json($Method, $Url, $Body = $null, $Token = $null, $ExtraHeaders = $null) {
  $headers = @{}
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  if ($ExtraHeaders) { foreach ($k in $ExtraHeaders.Keys) { $headers[$k] = $ExtraHeaders[$k] } }
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
cargo test -p idv-common usb_policy -- --nocapture | Out-Host
cargo build -p idv-center -p idv-edge -p idv-agent -p idv-pe-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$edgeBin = Join-Path $root "target\debug\idv-edge.exe"
$agentBin = Join-Path $root "target\debug\idv-agent.exe"
$peBin = Join-Path $root "target\debug\idv-pe-agent.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

Write-Host "== themes tokens present =="
$css = Get-Content (Join-Path $root "web\src\styles\themes.css") -Raw
foreach ($t in @("ink-lab", "pine-rack", "slate-ops", "night-duty")) {
  if ($css -notmatch [regex]::Escape("data-theme=`"$t`"")) {
    if ($t -eq "ink-lab" -and $css -notmatch 'ink-lab') { throw "theme missing: $t" }
    if ($t -ne "ink-lab" -and $css -notmatch [regex]::Escape("[data-theme=`"$t`"]")) { throw "theme missing: $t" }
  }
}
Write-Host "themes OK"

Write-Host "== USB enforce =="
$modes = Invoke-Json GET "$base/api/v1/modes" $null $token
$exam = @($modes | Where-Object { $_.name -eq "exam" })[0]
if (-not $exam) {
  $exam = Invoke-Json POST "$base/api/v1/modes" @{
    name = "exam"
    restore = "reboot"
    system_action = "restore"
    persistent_action = "keep"
    volatile_action = "wipe"
    notes = "usb-test"
    usb_restricted = $true
  } $token
}
$inStart = (Get-Date).ToUniversalTime().AddHours(-1).ToString("o")
$inEnd = (Get-Date).ToUniversalTime().AddHours(2).ToString("o")
Invoke-Json POST "$base/api/v1/modes/$($exam.id)/schedule" @{
  schedule_start = $inStart
  schedule_end = $inEnd
  usb_restricted = $true
} $token | Out-Null
$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "s6c-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "s6c-lab" } $token
$mac = "02:6c:6c:6c:6c:01"
Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "pc-usb" } $token | Out-Null
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $exam.id; effective = "next_boot" } $token | Out-Null

@"
center_url = "$base"
mac = "$mac"
hostname = "s6c-pc"
log = "warn"
"@ | Set-Content -Encoding utf8 $agentCfg
$usbState = Join-Path $dataDir "usb_policy.json"
& $agentBin enforce-usb --config $agentCfg --state-file $usbState | Out-Host
if (-not (Test-Path $usbState)) { throw "usb state file missing" }
$sessions = Invoke-Json GET "$base/api/v1/agent/sessions" $null $token
$sess = @($sessions | Where-Object { $_.mac -eq $mac })[0]
if (-not $sess) { throw "agent session missing" }
if (-not $sess.usb_enforced) { throw "usb_enforced not set on session" }
Write-Host "usb enforce OK"

Write-Host "== register edge + P2P stop =="
$edgeRow = Invoke-Json POST "$base/api/v1/edges" @{
  name = "s6c-edge-$(Get-Random)"
} $token
$nodeToken = $edgeRow.node_token
if (-not $nodeToken) { throw "edge token missing" }

@"
listen = "127.0.0.1:$edgePort"
center_url = "$base"
node_token = "$nodeToken"
name = "s6c-edge"
cache_dir = "$cacheDirUri"
log = "warn"
pe_deploy_token = "lab-pe-token"
p2p_chunk_size = 4096
"@ | Set-Content -Encoding utf8 $edgeCfg

$edge = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edgeCfg) -WorkingDirectory $root
Wait-Http "$edgeBase/health" 60 | Out-Null

# Donate a swarm chunk without authoritative cache (skip checksum gate)
$imgId = "img-s6c-p2p"
$chunkPath = Join-Path $dataDir "chunk0.bin"
$chunk0 = New-Object byte[] 4096
for ($i = 0; $i -lt $chunk0.Length; $i++) { $chunk0[$i] = [byte]($i % 251) }
[System.IO.File]::WriteAllBytes($chunkPath, $chunk0)
$put = Invoke-WebRequest -Method PUT -Uri "$edgeBase/api/v1/p2p/chunk/$imgId/0" -InFile $chunkPath -ContentType "application/octet-stream" -UseBasicParsing
if ($put.StatusCode -ge 400) { throw "put chunk failed" }
$stats = Invoke-RestMethod "$edgeBase/api/v1/p2p/swarm/$imgId"
if ($stats.swarm_chunks -lt 1) { throw "swarm should have chunks before stop" }
Invoke-RestMethod -Method POST -Uri "$edgeBase/api/v1/p2p/swarm/$imgId/stop" | Out-Null
$stats2 = Invoke-RestMethod "$edgeBase/api/v1/p2p/swarm/$imgId"
if ($stats2.swarm_chunks -ne 0) { throw "swarm not cleared after stop" }
if ($stats2.seeding -ne $false) { throw "seeding should be false" }
Write-Host "p2p stop OK"

Write-Host "== PE network wizard =="
@"
edge_url = "$edgeBase"
log = "warn"
"@ | Set-Content -Encoding utf8 $peCfg
$sessionOut = Join-Path $dataDir "network-session.json"
& $peBin network-wizard --config $peCfg --out $sessionOut --iface ethernet --edge-url $edgeBase | Out-Host
if (-not (Test-Path $sessionOut)) { throw "network session missing" }

# wifi without token must fail checkin
$failed = $false
try {
  & $peBin network-wizard --config $peCfg --out (Join-Path $dataDir "wifi-bad.json") --iface wifi --ssid "LabWiFi" --psk "x" --edge-url $edgeBase --mac $mac --checkin 2>&1 | Out-Host
} catch {
  $failed = $true
}
if (-not $failed) {
  # pe-agent returns non-zero via exit; also catch empty success
  $LASTEXITCODE = 0
}
# Re-run capturing exit code
$p = Start-Process -PassThru -Wait -NoNewWindow -FilePath $peBin -ArgumentList @(
  "network-wizard","--config",$peCfg,"--out",(Join-Path $dataDir "wifi-bad.json"),
  "--iface","wifi","--ssid","LabWiFi","--psk","x","--edge-url",$edgeBase,"--mac",$mac,"--checkin"
) -WorkingDirectory $root
if ($p.ExitCode -eq 0) { throw "wifi checkin without token should fail" }

$p2 = Start-Process -PassThru -Wait -NoNewWindow -FilePath $peBin -ArgumentList @(
  "network-wizard","--config",$peCfg,"--out",(Join-Path $dataDir "wifi-ok.json"),
  "--iface","wifi","--ssid","LabWiFi","--psk","x","--edge-url",$edgeBase,
  "--pe-token","lab-pe-token","--mac",$mac,"--checkin"
) -WorkingDirectory $root
if ($p2.ExitCode -ne 0) { throw "wifi checkin with token failed exit=$($p2.ExitCode)" }
Write-Host "network wizard OK"

# Doc claim: do not advertise firmware WiFi PXE
$doc07Path = Get-ChildItem (Join-Path $root "docs\modules") -Filter "07-*.md" | Select-Object -First 1
if (-not $doc07Path) { throw "module 07 doc missing" }
$doc07 = Get-Content $doc07Path.FullName -Raw -Encoding UTF8
if ($doc07 -notmatch "WiFi PXE") { throw "module 07 must mention WiFi PXE limitation" }
Write-Host "wifi pxe disclaimer OK"

Stop-Process -Id $edge.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE6C_ACCEPT_OK"
exit 0
