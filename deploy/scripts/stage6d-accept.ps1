# Stage 6d: windows-lab volumes + boot hook + xinchuang matrix + wifi drivers
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$base = "http://127.0.0.1:$centerPort"
$dataDir = Join-Path $root "data\stage6d-accept"
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
if (Test-Path $db) { throw "unable to clear stage6d db: $db" }

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
cargo test -p idv-common volume -- --nocapture | Out-Host
cargo build -p idv-center -p idv-agent -p idv-pe-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$agentBin = Join-Path $root "target\debug\idv-agent.exe"
$peBin = Join-Path $root "target\debug\idv-pe-agent.exe"

Write-Host "== docs / themes / wifi pack =="
$doc13 = Get-ChildItem (Join-Path $root "docs\modules") -Filter "13-*.md" | Select-Object -First 1
if (-not $doc13) { throw "module 13 xinchuang matrix missing" }
$doc13Text = Get-Content $doc13.FullName -Raw -Encoding UTF8
if ($doc13Text -notmatch "Chromium") { throw "module 13 incomplete" }
$css = Get-Content (Join-Path $root "web\src\styles\themes.css") -Raw
if ($css -match ":has\(") { throw "themes.css must not rely on :has() for xinchuang" }
$manifest = Join-Path $root "pe\wifi-drivers\manifest.json"
if (-not (Test-Path $manifest)) { throw "wifi drivers manifest missing" }
& $peBin wifi-drivers --manifest $manifest | Out-Host
if ($LASTEXITCODE -ne 0) { throw "wifi-drivers list failed" }
Write-Host "docs/wifi OK"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

$modes = Invoke-Json GET "$base/api/v1/modes" $null $token
$teach = @($modes | Where-Object { $_.name -eq "teaching" })[0]
if (-not $teach) {
  $teach = @($modes | Where-Object { $_.restore -eq "reboot" -and $_.system_action -eq "restore" -and $_.name -ne "exam" })[0]
}
if (-not $teach) { throw "teaching mode missing" }
$campus = Invoke-Json POST "$base/api/v1/campuses" @{ name = "s6d-campus" } $token
$lab = Invoke-Json POST "$base/api/v1/labs" @{ campus_id = $campus.id; name = "s6d-lab" } $token
$mac = "02:6d:6d:6d:6d:01"
Invoke-Json POST "$base/api/v1/clients" @{ lab_id = $lab.id; mac = $mac; name = "pc-vol" } $token | Out-Null
Invoke-Json POST "$base/api/v1/labs/$($lab.id)/mode" @{ mode_policy_id = $teach.id; effective = "next_boot" } $token | Out-Null

@"
center_url = "$base"
mac = "$mac"
hostname = "s6d-pc"
log = "warn"
"@ | Set-Content -Encoding utf8 $agentCfg

Write-Host "== windows-lab volume restore =="
$baselineRoot = Join-Path $dataDir "baseline-vol"
$liveRoot = Join-Path $dataDir "live-vol"
& $agentBin pack-volume-baseline --out $baselineRoot --system-marker SYS-A --persistent-marker PER-A --volatile-marker VOL-A | Out-Host
if ($LASTEXITCODE -ne 0) { throw "pack-volume-baseline failed" }

New-Item -ItemType Directory -Force -Path (Join-Path $liveRoot "system"), (Join-Path $liveRoot "persistent"), (Join-Path $liveRoot "volatile") | Out-Null
Set-Content -Path (Join-Path $liveRoot "system\drift.txt") -Value "bad" -Encoding ascii
Set-Content -Path (Join-Path $liveRoot "persistent\student.doc") -Value "keep-me" -Encoding ascii
Set-Content -Path (Join-Path $liveRoot "volatile\tmp.bin") -Value "tmp" -Encoding ascii

& $agentBin boot --config $agentCfg --backend windows-lab --volume-root $liveRoot --baseline-root $baselineRoot | Out-Host
if ($LASTEXITCODE -ne 0) { throw "windows-lab boot restore failed" }
if (Test-Path (Join-Path $liveRoot "system\drift.txt")) { throw "system drift should be cleared" }
$sys = Get-Content (Join-Path $liveRoot "system\BASELINE.marker") -Raw
if ($sys.Trim() -ne "SYS-A") { throw "system marker not restored" }
if (-not (Test-Path (Join-Path $liveRoot "persistent\student.doc"))) { throw "persistent should keep" }
if (Test-Path (Join-Path $liveRoot "volatile\tmp.bin")) { throw "volatile should wipe" }
Write-Host "windows-lab restore OK"

Write-Host "== boot hook =="
$stateDir = Join-Path $dataDir "hook-state"
& $agentBin install-boot-hook --config $agentCfg --state-dir $stateDir --backend windows-lab --volume-root $liveRoot --baseline-root $baselineRoot --agent-bin $agentBin | Out-Host
if ($LASTEXITCODE -ne 0) { throw "install-boot-hook failed" }
$hook = Join-Path $stateDir "idv-boot-hook.cmd"
$marker = Join-Path $stateDir "boot_hook_installed.json"
if (-not (Test-Path $hook)) { throw "hook cmd missing" }
if (-not (Test-Path $marker)) { throw "hook marker missing" }
$hookText = Get-Content $hook -Raw
if ($hookText -notmatch "windows-lab") { throw "hook script missing backend" }
& (Join-Path $root "deploy\scripts\install-agent-boot-hook.ps1") -AgentBin $agentBin -Config $agentCfg -VolumeRoot $liveRoot -BaselineRoot $baselineRoot -StateDir (Join-Path $dataDir "hook-state2") | Out-Host
Write-Host "boot hook OK"

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Write-Host "STAGE6D_ACCEPT_OK"
exit 0
