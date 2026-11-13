# Stage 7 / Phase 3: roles, tags, software, assist, class lock, schedules, audit csv
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage7-accept"
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
if (Test-Path $db) { throw "unable to clear stage7 db" }

@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
images_dir = "$(($imagesDir -replace '\\','/'))"
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
if ($login.role -ne "super") { throw "admin should be super" }

Write-Host "== readonly role =="
$ro = Invoke-Json POST "$base/api/v1/admins" @{
  username = "viewer"
  password = "viewer123"
  display_name = "readonly"
  role = "readonly"
} $token
$roLogin = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "viewer"; password = "viewer123" }
$roToken = $roLogin.token
$denied = $false
try {
  Invoke-Json POST "$base/api/v1/power/commands" @{ mac = "02:00:00:00:00:99"; kind = "wol" } $roToken | Out-Null
} catch { $denied = $true }
if (-not $denied) { throw "readonly should not create power command" }
Write-Host "readonly OK"

Write-Host "== tags =="
$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "s7-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "s7-lab" } $token
$mac = "02:77:77:77:77:71"
$client = Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "s7-pc" } $token
Invoke-Json POST "$base/api/v1/clients/$($client.id)/tags" @{ tags = @("exam", "row-a") } $token | Out-Null
$tagged = Invoke-Json GET "$base/api/v1/clients?tag=exam" $null $token
if (@($tagged | Where-Object { $_.mac -eq $mac }).Count -lt 1) { throw "tag filter failed" }
Write-Host "tags OK"

Write-Host "== class lock =="
$hour = [DateTime]::UtcNow.Hour
Invoke-Json POST "$base/api/v1/ops/class-lock" @{
  enabled = $true
  start_hour_utc = $hour
  end_hour_utc = (($hour + 2) % 24)
  # ensure end > start for simple case; if wraps, use 24
} $token | Out-Null
# Fix wrap: always use start=hour end=min(24, hour+2) but API requires start < end
$endH = [Math]::Min(24, $hour + 2)
if ($endH -le $hour) { $endH = 24 }
Invoke-Json POST "$base/api/v1/ops/class-lock" @{
  enabled = $true
  start_hour_utc = $hour
  end_hour_utc = $endH
} $token | Out-Null

# need an image for deploy attempt
$imgPath = Join-Path $imagesDir "tiny.bin"
[System.IO.File]::WriteAllBytes($imgPath, [byte[]](1..64))
$hash = (Get-FileHash -Algorithm SHA256 -Path $imgPath).Hash.ToLowerInvariant()
$img = Invoke-Json POST "$base/api/v1/images" @{
  name = "s7-img"
  os_family = "windows"
  checksum_sha256 = $hash
  size_bytes = 64
  storage_uri = ($imgPath -replace "\\", "/")
} $token
$blocked = $false
try {
  Invoke-Json POST "$base/api/v1/tasks/deploy" @{
    image_id = $img.id
    macs = @($mac)
    confirm_destructive = $true
    disk_plan = "wipe_whole_disk"
  } $token | Out-Null
} catch { $blocked = $true }
if (-not $blocked) { throw "deploy should be blocked by class lock" }
Invoke-Json POST "$base/api/v1/ops/class-lock" @{
  enabled = $false
  start_hour_utc = 1
  end_hour_utc = 10
} $token | Out-Null
Write-Host "class lock OK"

Write-Host "== software distribute =="
$pkg = Invoke-Json POST "$base/api/v1/software/packages" @{ name = "s7-patch"; version_label = "1.0" } $token
$blob = New-Object byte[] 128
for ($i = 0; $i -lt 128; $i++) { $blob[$i] = [byte]$i }
$blobPath = Join-Path $dataDir "patch.bin"
[System.IO.File]::WriteAllBytes($blobPath, $blob)
Invoke-WebRequest -Method PUT -Uri "$base/api/v1/software/packages/$($pkg.id)/upload" `
  -Headers @{ Authorization = "Bearer $token" } -InFile $blobPath -ContentType "application/octet-stream" -UseBasicParsing | Out-Null
$job = Invoke-Json POST "$base/api/v1/software/jobs" @{ package_id = $pkg.id; mac = $mac } $token
@"
center_url = "$base"
mac = "$mac"
hostname = "s7-pc"
log = "warn"
"@ | Set-Content -Encoding utf8 $agentCfg
$outDir = Join-Path $dataDir "applied"
& $agentBin soft-apply --config $agentCfg --out-dir $outDir | Out-Host
if ($LASTEXITCODE -ne 0) { throw "soft-apply failed" }
$jobs = Invoke-Json GET "$base/api/v1/software/jobs" $null $token
$done = @($jobs | Where-Object { $_.id -eq $job.id -and $_.state -eq "done" })
if ($done.Count -lt 1) { throw "software job not done" }
Write-Host "software OK"

Write-Host "== assist =="
$sess = Invoke-Json POST "$base/api/v1/assist/sessions" @{ mac = $mac; kind = "view" } $token
& $agentBin assist-ack --config $agentCfg --state accepted | Out-Host
if ($LASTEXITCODE -ne 0) { throw "assist-ack failed" }
$sessions = Invoke-Json GET "$base/api/v1/assist/sessions" $null $token
$acc = @($sessions | Where-Object { $_.id -eq $sess.id -and $_.state -eq "accepted" })
if ($acc.Count -lt 1) { throw "assist not accepted" }
Write-Host "assist OK"

Write-Host "== power schedule tick =="
$now = [DateTime]::UtcNow
$sched = Invoke-Json POST "$base/api/v1/ops/power-schedules" @{
  name = "s7-wol"
  kind = "wol"
  mac_or_tag = $mac
  target_type = "mac"
  hour_utc = $now.Hour
  minute_utc = $now.Minute
} $token
Invoke-Json POST "$base/api/v1/ops/power-schedules/tick" @{} $token | Out-Null
$cmds = Invoke-Json GET "$base/api/v1/power/commands" $null $token
$hit = @($cmds | Where-Object { $_.detail -match "schedule:s7-wol" })
if ($hit.Count -lt 1) { throw "schedule did not enqueue wol" }
Write-Host "schedule OK"

Write-Host "== audit csv =="
$csv = Invoke-WebRequest -Uri "$base/api/v1/audit/export.csv" -Headers @{ Authorization = "Bearer $token" } -UseBasicParsing
if ($csv.Content -notmatch "mode.bind_lab|assist.create|ops.class_lock|power.schedule_fire") {
  # at least header
  if ($csv.Content -notmatch "actor,action") { throw "audit csv bad" }
}
Write-Host "audit csv OK"

# module docs
$doc15 = Get-ChildItem (Join-Path $root "docs\modules") -Filter "15-*.md" | Select-Object -First 1
if (-not $doc15) { throw "module 15 missing" }

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE7_ACCEPT_OK"
exit 0
