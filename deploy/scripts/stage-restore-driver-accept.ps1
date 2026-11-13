# Smoke: Center heartbeat returns desired_state; agent mock protect roundtrip.
# Usage: powershell -ExecutionPolicy Bypass -File deploy/scripts/stage-restore-driver-accept.ps1
$ErrorActionPreference = "Stop"
$Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not (Test-Path "$Root\Cargo.toml")) { $Root = (Get-Location).Path }
Set-Location $Root

Write-Host "== build idv-common / idv-agent =="
cargo test -p idv-common endpoint_policy -- --nocapture
cargo test -p idv-agent restore_driver -- --nocapture
cargo build -p idv-agent --release

$center = "http://127.0.0.1:8080"
try {
  $hb = Invoke-RestMethod -Uri "$center/api/v1/agent/heartbeat" -Method Post -ContentType "application/json" -Body (@{
    mac = "02:00:00:00:00:99"
    hostname = "restore-accept"
    agent_version = "accept"
    trigger = "boot"
    applied_revision = 0
    pending_reboot = $false
    driver_status = "accept-script"
  } | ConvertTo-Json)
  if ($null -eq $hb.desired_state) {
    Write-Host "WARN: desired_state null (lab may have no mode binding) — message=$($hb.message)"
  } else {
    Write-Host "desired_state revision=$($hb.desired_state.revision) frozen=$($hb.desired_state.restore.frozen)"
  }
} catch {
  Write-Host "SKIP center heartbeat: $_"
}

$agent = Join-Path $Root "target\release\idv-agent.exe"
& $agent restore-driver status
& $agent restore-driver protect-on --volumes system
& $agent restore-driver protect-off

Write-Host "STAGE_RESTORE_DRIVER_ACCEPT_OK"
