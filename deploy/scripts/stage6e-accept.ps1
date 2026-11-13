# Stage 6e software closeout: mode audit, exam fallback, WOL packet
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage6e-accept"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"

Get-Process idv-center -ErrorAction SilentlyContinue | Stop-Process -Force
Get-NetTCPConnection -LocalPort $centerPort -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 2
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
if (Test-Path $db) { throw "unable to clear stage6e db: $db" }

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
cargo test -p idv-common wol -- --nocapture | Out-Host
cargo build -p idv-center | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

$modes = Invoke-Json GET "$base/api/v1/modes" $null $token
$teach = @($modes | Where-Object { $_.name -eq "teaching" })[0]
$exam = @($modes | Where-Object { $_.name -eq "exam" })[0]
if (-not $teach -or -not $exam) { throw "modes missing" }

Write-Host "== mode bind audit =="
$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "s6e-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "s6e-lab" } $token
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $exam.id; effective = "immediate" } $token | Out-Null
$audits = Invoke-Json GET "$base/api/v1/audit?action=mode.bind_lab" $null $token
if (@($audits).Count -lt 1) { throw "bind audit missing" }
Write-Host "bind audit OK"

Write-Host "== exam fallback + audit =="
$pastStart = (Get-Date).ToUniversalTime().AddHours(-3).ToString("o")
$pastEnd = (Get-Date).ToUniversalTime().AddHours(-2).ToString("o")
Invoke-Json POST "$base/api/v1/modes/$($exam.id)/schedule" @{
  schedule_start = $pastStart
  schedule_end = $pastEnd
  usb_restricted = $true
} $token | Out-Null
$mac = "02:6e:6e:6e:6e:01"
Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "s6e-pc" } $token | Out-Null
$hb = Invoke-Json POST "$base/api/v1/agent/heartbeat" @{
  mac = $mac
  hostname = "s6e"
  agent_version = "0.1.0"
  trigger = "boot"
} $null
if ($hb.mode.name -ne "teaching") { throw "expected teaching after fallback" }
$bindings = Invoke-Json GET "$base/api/v1/modes/bindings" $null $token
$b = @($bindings | Where-Object { $_.lab_id -eq $lab.id })[0]
if ($b.mode_policy_id -ne $teach.id) { throw "lab binding not switched to teaching" }
$fb = Invoke-Json GET "$base/api/v1/audit?action=mode.exam_fallback" $null $token
if (@($fb).Count -lt 1) { throw "exam_fallback audit missing" }
Write-Host "exam fallback OK"

Write-Host "== WOL magic packet =="
$wol = Invoke-Json POST "$base/api/v1/power/commands" @{
  mac = $mac
  kind = "wol"
} $token
if ($wol.state -ne "done") { throw "wol state" }
if ($wol.detail -notmatch "magic_packet_sent") { throw "wol detail missing magic_packet_sent: $($wol.detail)" }
Write-Host "wol OK"

# Pilot checklist doc must exist
$pilot = Get-ChildItem (Join-Path $root "docs\modules") -Filter "14-*.md" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $pilot) { throw "module 14 pilot checklist missing" }

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE6E_ACCEPT_OK"
exit 0
