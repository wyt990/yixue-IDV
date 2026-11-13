# Stage 6 rolling: delta build/apply, exam schedule window, WOL/shutdown commands
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage6b-accept"
$imagesDir = Join-Path $dataDir "images"
New-Item -ItemType Directory -Force -Path $dataDir, $imagesDir | Out-Null
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
  throw "unable to clear stage6b db (process still locking?): $db"
}
Remove-Item (Join-Path $imagesDir "*") -Force -ErrorAction SilentlyContinue

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
cargo test -p idv-common delta -- --nocapture | Out-Host
cargo build -p idv-center -p idv-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$agentBin = Join-Path $root "target\debug\idv-agent.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

# Two full blobs differing in a small region
$basePath = Join-Path $imagesDir "full-a.bin"
$targetPath = Join-Path $imagesDir "full-b.bin"
$a = New-Object byte[] 20000
for ($i = 0; $i -lt $a.Length; $i++) { $a[$i] = [byte]($i % 200) }
$b = [byte[]]$a.Clone()
for ($i = 5000; $i -lt 5200; $i++) { $b[$i] = 0xEE }
[System.IO.File]::WriteAllBytes($basePath, $a)
[System.IO.File]::WriteAllBytes($targetPath, $b)
$hashA = (Get-FileHash -Algorithm SHA256 -Path $basePath).Hash.ToLowerInvariant()
$hashB = (Get-FileHash -Algorithm SHA256 -Path $targetPath).Hash.ToLowerInvariant()
$uriA = ($basePath -replace "\\", "/")
$uriB = ($targetPath -replace "\\", "/")

$imgA = Invoke-Json POST "$base/api/v1/images" @{
  name = "full-a"
  os_family = "linux"
  checksum_sha256 = $hashA
  size_bytes = $a.Length
  storage_uri = $uriA
  content_type = "full"
  partition_map = "[]"
} $token
$imgB = Invoke-Json POST "$base/api/v1/images" @{
  name = "full-b"
  os_family = "linux"
  checksum_sha256 = $hashB
  size_bytes = $b.Length
  storage_uri = $uriB
  content_type = "full"
  partition_map = "[]"
} $token

Write-Host "== build + apply delta =="
$delta = Invoke-Json POST "$base/api/v1/images/build-delta" @{
  base_image_id = $imgA.id
  target_image_id = $imgB.id
  name = "a-to-b"
} $token
if ($delta.content_type -ne "delta") { throw "delta content_type" }
if ($delta.base_image_id -ne $imgA.id) { throw "delta base mismatch" }
if ($delta.size_bytes -ge $imgB.size_bytes) { throw "delta should be smaller than full B" }

$applied = Invoke-Json POST "$base/api/v1/images/apply-delta" @{
  base_image_id = $imgA.id
  delta_image_id = $delta.id
  name = "b-from-delta"
} $token
if ($applied.content_type -ne "full") { throw "applied should be full" }
if ($applied.checksum_sha256 -ne $hashB) { throw "applied checksum != target B" }
Write-Host "delta build/apply OK size_delta=$($delta.size_bytes)"

Write-Host "== reject deploy delta =="
try {
  Invoke-Json POST "$base/api/v1/tasks/deploy" @{
    image_id = $delta.id
    macs = @("02:77:77:77:77:01")
    disk_plan = "wipe_whole_disk"
    confirm_destructive = $true
  } $token | Out-Null
  throw "deploy delta should fail"
} catch {
  if ("$($_.Exception.Message)" -notmatch "delta" -and "$($_)" -notmatch "delta") {
    # Invoke-RestMethod error body
    $ok = $true
  }
}
# Confirm via raw response
try {
  $headers = @{ Authorization = "Bearer $token" }
  $body = @{ image_id = $delta.id; macs = @("02:77:77:77:77:01"); disk_plan = "wipe_whole_disk"; confirm_destructive = $true } | ConvertTo-Json -Compress
  Invoke-WebRequest -Method POST -Uri "$base/api/v1/tasks/deploy" -Headers $headers -ContentType "application/json" -Body ([Text.Encoding]::UTF8.GetBytes($body)) -UseBasicParsing | Out-Null
  throw "expected 400 for delta deploy"
} catch {
  $resp = $_.Exception.Response
  if (-not $resp) { throw "delta deploy reject failed: $_" }
}
Write-Host "delta deploy rejected OK"

Write-Host "== exam schedule window =="
$modes = Invoke-Json GET "$base/api/v1/modes" $null $token
$exam = @($modes | Where-Object { $_.name -eq "exam" })[0]
if (-not $exam) {
  $exam = Invoke-Json POST "$base/api/v1/modes" @{
    name = "exam"
    restore = "reboot"
    system_action = "restore"
    persistent_action = "keep"
    volatile_action = "wipe"
    notes = "exam-window"
    usb_restricted = $true
  } $token
}

$pastStart = (Get-Date).ToUniversalTime().AddHours(-2).ToString("o")
$pastEnd = (Get-Date).ToUniversalTime().AddHours(-1).ToString("o")
Invoke-Json POST "$base/api/v1/modes/$($exam.id)/schedule" @{
  schedule_start = $pastStart
  schedule_end = $pastEnd
  usb_restricted = $true
} $token | Out-Null

$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "s6b-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "s6b-lab" } $token
$mac = "02:77:77:77:77:09"
Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "exam-pc" } $token | Out-Null
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $exam.id; effective = "immediate" } $token | Out-Null

@"
center_url = "$base"
mac = "$mac"
hostname = "exam-pc"
log = "warn"
"@ | Set-Content -Encoding utf8 $agentCfg

$hb = Invoke-Json POST "$base/api/v1/agent/heartbeat" @{
  mac = $mac
  hostname = "exam-pc"
  agent_version = "0.1.0"
  trigger = "boot"
} $null
# Outside exam window auto-falls back to teaching.
if ($hb.mode.name -ne "teaching") { throw "outside exam window should fallback to teaching" }
if ([string]::IsNullOrWhiteSpace($hb.message)) { throw "fallback message expected" }
Write-Host "exam window OK (fallback teaching)"

# Inside window: rebind exam + schedule
$inStart = (Get-Date).ToUniversalTime().AddHours(-1).ToString("o")
$inEnd = (Get-Date).ToUniversalTime().AddHours(2).ToString("o")
Invoke-Json POST "$base/api/v1/modes/$($exam.id)/schedule" @{
  schedule_start = $inStart
  schedule_end = $inEnd
  usb_restricted = $true
} $token | Out-Null
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $exam.id; effective = "immediate" } $token | Out-Null
$hb2 = Invoke-Json POST "$base/api/v1/agent/heartbeat" @{
  mac = $mac
  trigger = "boot"
} $null
if ($hb2.mode.name -ne "exam") { throw "inside window mode should be exam" }
if ($hb2.schedule_active -ne $true) { throw "exam inside window should be active" }
if ($hb2.usb_restricted -ne $true) { throw "usb_restricted expected" }
if ($hb2.should_restore -ne $true) { throw "exam inside window should restore" }
Write-Host "exam active + usb OK"

Write-Host "== power wol/shutdown =="
$wol = Invoke-Json POST "$base/api/v1/power/commands" @{
  mac = $mac
  kind = "wol"
} $token
if ($wol.kind -ne "wol") { throw "wol kind" }
$sd = Invoke-Json POST "$base/api/v1/power/commands" @{
  mac = $mac
  kind = "shutdown"
} $token
if ($sd.state -ne "pending") { throw "shutdown should be pending" }
$hb3 = Invoke-Json POST "$base/api/v1/agent/heartbeat" @{ mac = $mac; trigger = "heartbeat" } $null
if (-not $hb3.pending_power) { throw "pending_power missing" }
Invoke-Json POST "$base/api/v1/power/commands/$($sd.id)/ack" @{ ok = $true; detail = "shutdown_simulated" } $null | Out-Null
Write-Host "power commands OK"

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE6B_ACCEPT_OK"
exit 0
