# Install idv-agent boot hook on a Windows lab PC (optional schtasks).
# Usage:
#   .\install-agent-boot-hook.ps1 -AgentBin ... -Config ... -VolumeRoot ... -BaselineRoot ... -StateDir ...
param(
  [Parameter(Mandatory = $true)][string]$AgentBin,
  [Parameter(Mandatory = $true)][string]$Config,
  [Parameter(Mandatory = $true)][string]$VolumeRoot,
  [Parameter(Mandatory = $true)][string]$BaselineRoot,
  [Parameter(Mandatory = $true)][string]$StateDir,
  [string]$Backend = "windows-lab",
  [switch]$RegisterTask
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $StateDir | Out-Null

& $AgentBin install-boot-hook `
  --config $Config `
  --state-dir $StateDir `
  --backend $Backend `
  --volume-root $VolumeRoot `
  --baseline-root $BaselineRoot `
  --agent-bin $AgentBin
if ($LASTEXITCODE -ne 0) { throw "install-boot-hook failed" }

$script = Join-Path $StateDir "idv-boot-hook.cmd"
if (-not (Test-Path $script)) { throw "hook script missing: $script" }

if ($RegisterTask) {
  $taskName = "YixueIdvAgentBoot"
  # Windows PowerShell 5.1 + EAP=Stop turns schtasks stderr into a terminating
  # error; go through cmd with redirection so a missing task is harmless.
  cmd /c "schtasks /Delete /TN `"$taskName`" /F >nul 2>&1" | Out-Null
  cmd /c "schtasks /Create /TN `"$taskName`" /SC ONSTART /RU SYSTEM /RL HIGHEST /TR `"\`"$script\`"`" /F" | Out-Host
  Write-Host "schtasks registered: $taskName"
} else {
  Write-Host "hook files ready (pass -RegisterTask to create ONSTART task)"
}

Write-Host "INSTALL_BOOT_HOOK_OK"
