# Install idv-agent as a Windows service (preferred) or, if SCM registration is
# unavailable, as a SYSTEM startup task running the resident daemon.
#
# Windows PowerShell 5.1 note: with $ErrorActionPreference = "Stop", a native
# command that writes to stderr (e.g. `schtasks /Delete` on a missing task)
# becomes a terminating error even with `2>$null`. All best-effort cleanup below
# therefore runs inside try/catch or through `cmd /c ... >nul 2>&1`.
#
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\install-agent-service.ps1
#   .\scripts\install-agent-service.ps1 -CenterUrl http://10.110.0.1:8080
param(
  [string]$CenterUrl = "",
  [switch]$FallbackTaskOnly
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_common.ps1")

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principalCheck = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principalCheck.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "必须以管理员身份运行。"
}

$p = Get-YixuePaths
if (-not (Test-Path $p.Agent)) { throw "找不到 $($p.Agent)" }

if (-not (Test-Path $p.Config)) {
  $example = Join-Path $p.Root "agent.toml.example"
  if (Test-Path $example) {
    Copy-Item $example $p.Config
    Write-Host "已生成 $($p.Config)"
  } else {
    throw "找不到 $($p.Config)"
  }
}
if ($CenterUrl) {
  $text = Get-Content $p.Config -Raw -Encoding UTF8
  $text = [regex]::Replace($text, '(?m)^\s*center_url\s*=\s*"[^"]*"', "center_url = `"$CenterUrl`"")
  Set-Content -Path $p.Config -Value $text -Encoding UTF8
}

New-Item -ItemType Directory -Force -Path $p.StateDir | Out-Null

$serviceName = "idv-agent"
$daemonTask = "YixueIdvAgentDaemon"
$legacyTasks = @("YixueIdvAgentBoot", "YixueIdvAgentPoll", $daemonTask)

function Remove-AgentTask([string]$name) {
  # Best-effort: missing task must never abort the install.
  try {
    $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    if ($task) {
      Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction Stop | Out-Null
      Write-Host "已删除计划任务 $name"
    }
  } catch {
    cmd /c "schtasks /Delete /TN `"$name`" /F >nul 2>&1" | Out-Null
  }
}

function Remove-AgentService([string]$name) {
  # Best-effort: absent service must never abort the install.
  try {
    $svc = Get-Service -Name $name -ErrorAction SilentlyContinue
    if (-not $svc) { return }
    Write-Host "移除旧服务 $name ..."
    Stop-Service -Name $name -Force -ErrorAction SilentlyContinue
    & sc.exe delete $name | Out-Null
    Start-Sleep -Seconds 1
  } catch {
    cmd /c "sc delete `"$name`" >nul 2>&1" | Out-Null
  }
}

function Install-FallbackTask {
  Write-Host "使用计划任务守护模式（SYSTEM 常驻 daemon）"
  Remove-AgentTask $daemonTask
  $action = New-ScheduledTaskAction -Execute $p.Agent `
    -Argument "daemon --config `"$($p.Config)`" --heartbeat-secs 5"
  $trigger = New-ScheduledTaskTrigger -AtStartup
  $settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit (New-TimeSpan -Days 0)
  $principal = New-ScheduledTaskPrincipal -UserId "SYSTEM" -LogonType ServiceAccount -RunLevel Highest
  Register-ScheduledTask -TaskName $daemonTask -Action $action -Trigger $trigger `
    -Settings $settings -Principal $principal -Force | Out-Null
  Start-ScheduledTask -TaskName $daemonTask
  Start-Sleep -Seconds 2
  $t = Get-ScheduledTask -TaskName $daemonTask -ErrorAction SilentlyContinue
  if ($t) { Write-Host "已注册计划任务 $daemonTask（$($t.State)）" }
  Get-Process idv-agent -ErrorAction SilentlyContinue |
    Select-Object Id, ProcessName, StartTime | Format-Table -AutoSize
}

# Cleanup from previous installs (tasks may not exist at all).
foreach ($task in $legacyTasks) { Remove-AgentTask $task }
Remove-AgentService $serviceName

if ($FallbackTaskOnly) {
  Install-FallbackTask
  Write-Host "INSTALL_AGENT_SERVICE_OK mode=task"
  exit 0
}

$binPath = "`"$($p.Agent)`" service --config `"$($p.Config)`""
try {
  New-Service -Name $serviceName -BinaryPathName $binPath `
    -DisplayName "易学 IDV 客户端代理" -StartupType Automatic -ErrorAction Stop | Out-Null
  # Optional metadata / failure actions must not block a working service.
  try {
    & sc.exe description $serviceName "易学 IDV 客户端代理：心跳、即时命令、还原策略、远程协助与文件传输。" | Out-Null
  } catch { Write-Host "WARNING: 设置服务描述失败（不影响使用）" }
  try {
    & sc.exe failure $serviceName reset= 86400 actions= restart/5000/restart/10000/restart/30000 | Out-Null
  } catch { Write-Host "WARNING: 设置服务失败重启策略失败（不影响使用）" }

  Start-Service -Name $serviceName -ErrorAction Stop
  Start-Sleep -Seconds 3
  $svc = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
  $status = if ($svc) { "$($svc.Status)" } else { "not-found" }
  Write-Host "服务状态: $status"
  if ($status -ne "Running") { throw "服务未进入 Running 状态（$status）" }
  Write-Host "INSTALL_AGENT_SERVICE_OK mode=service"
} catch {
  Write-Host "服务安装/启动失败：$($_.Exception.Message)"
  Remove-AgentService $serviceName
  Write-Host "回退到计划任务守护模式..."
  Install-FallbackTask
  Write-Host "INSTALL_AGENT_SERVICE_OK mode=task"
}
