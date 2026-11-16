# 在本目录完成学生机初始化（整包应放在 C:\Program Files\YixueIDV\）
# 默认使用同目录 agent.toml；-CenterUrl 等仅在传入时写回配置。
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\install-client.ps1 -RegisterBootTask
param(
  [string]$CenterUrl = "",
  [string]$Hostname = "",
  [string]$Mac = "",
  [switch]$RegisterBootTask,
  [switch]$RegisterPollTask,
  [int]$PollMinutes = 2
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_common.ps1")
$p = Get-YixuePaths

if (-not (Test-Path $p.Agent)) { throw "找不到 $($p.Agent)。请把整个包放到 C:\Program Files\YixueIDV\" }
if (-not (Test-Path $p.Config)) {
  $example = Join-Path $p.Root "agent.toml.example"
  if (-not (Test-Path $example)) {
    throw "找不到 $($p.Config)，且包内没有 agent.toml.example"
  }
  Copy-Item $example $p.Config
  Write-Host "已从 agent.toml.example 生成通用配置: $($p.Config)"
}

New-Item -ItemType Directory -Force -Path $p.StateDir | Out-Null

function Set-TomlKey([string]$path, [string]$key, [string]$value) {
  $text = Get-Content $path -Raw -Encoding UTF8
  $pattern = "(?m)^\s*#?\s*$key\s*=\s*""[^""]*"""
  $line = "$key = `"$value`""
  if ($text -match $pattern) {
    $text = [regex]::Replace($text, $pattern, $line, 1)
  } else {
    $text = $text.TrimEnd() + "`r`n$line`r`n"
  }
  Set-Content -Path $path -Value $text -Encoding UTF8
}

if ($CenterUrl) { Set-TomlKey $p.Config "center_url" $CenterUrl }
if ($Hostname) { Set-TomlKey $p.Config "hostname" $Hostname }
if ($Mac) { Set-TomlKey $p.Config "mac" $Mac }

Write-Host "安装目录: $($p.Root)"
Write-Host "程序:     $($p.Agent)"
Write-Host "配置:     $($p.Config)"
Write-Host "状态目录: $($p.StateDir)"

Write-Host ""
Write-Host "== 试连中心（heartbeat）=="
& $p.Agent heartbeat --config $p.Config --trigger manual
if ($LASTEXITCODE -ne 0) {
  Write-Host "WARNING: heartbeat 失败。常见原因：Center 只监听了 127.0.0.1（应改为 0.0.0.0:8080 并重启），或防火墙未放行 8080。"
} else {
  Write-Host "heartbeat 成功"
}

Write-Host ""
Write-Host "== 拉取策略（不立即改冻结态）=="
& $p.Agent sync-policy --config $p.Config --state-dir $p.StateDir --trigger manual
if ($LASTEXITCODE -ne 0) { Write-Host "WARNING: sync-policy 失败" }

if ($RegisterBootTask) {
  $hookScript = Join-Path $p.StateDir "idv-boot-hook.cmd"
  $cmd = @(
    "@echo off"
    "`"$($p.Agent)`" sync-policy --config `"$($p.Config)`" --state-dir `"$($p.StateDir)`" --apply --trigger boot >> `"$($p.StateDir)\boot-hook.log`" 2>&1"
  ) -join "`r`n"
  Set-Content -Path $hookScript -Value $cmd -Encoding ASCII

  $taskName = "YixueIdvAgentBoot"
  # 任务不存在时 Delete 会报错；勿因 ErrorActionPreference=Stop 中断
  cmd /c "schtasks /Delete /TN `"$taskName`" /F >nul 2>&1" | Out-Null
  $create = cmd /c "schtasks /Create /TN `"$taskName`" /SC ONSTART /RU SYSTEM /RL HIGHEST /TR `"\`"$hookScript\`"`" /F"
  Write-Host $create
  if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: 注册开机任务失败（exit=$LASTEXITCODE）。可稍后手动创建任务指向: $hookScript"
  } else {
    Write-Host "已注册开机任务: $taskName → $hookScript"
  }
} else {
  Write-Host "未注册开机任务。需要时加: -RegisterBootTask"
}

# 在线轮询：改模式/远程重启依赖周期 sync-policy 取 pending_power
if ($RegisterPollTask -or $RegisterBootTask) {
  if ($PollMinutes -lt 1) { $PollMinutes = 1 }
  $pollScript = Join-Path $p.StateDir "idv-poll.cmd"
  $pollCmd = @(
    "@echo off"
    "`"$($p.Agent)`" sync-policy --config `"$($p.Config)`" --state-dir `"$($p.StateDir)`" --trigger poll >> `"$($p.StateDir)\poll.log`" 2>&1"
  ) -join "`r`n"
  Set-Content -Path $pollScript -Value $pollCmd -Encoding ASCII

  $pollName = "YixueIdvAgentPoll"
  cmd /c "schtasks /Delete /TN `"$pollName`" /F >nul 2>&1" | Out-Null
  $pollCreate = cmd /c "schtasks /Create /TN `"$pollName`" /SC MINUTE /MO $PollMinutes /RU SYSTEM /RL HIGHEST /TR `"\`"$pollScript\`"`" /F"
  Write-Host $pollCreate
  if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: 注册轮询任务失败（exit=$LASTEXITCODE）。可稍后手动创建任务指向: $pollScript"
  } else {
    Write-Host "已注册轮询任务: $pollName 每 ${PollMinutes} 分钟 → $pollScript"
  }
} else {
  Write-Host "未注册轮询任务。需要时加: -RegisterPollTask 或 -RegisterBootTask"
}

Write-Host ""
Write-Host "INSTALL_CLIENT_OK"
Write-Host "驱动请另跑: .\scripts\enable-testsigning.ps1 → 重启 → .\scripts\install-driver.ps1 → 再重启"
