# 批量身份 / DHCP 保留 — 验收脚本（只读查询 Center，不改配置）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File deploy\scripts\identity-accept.ps1 `
#     -Mac "40:FE:95:B9:FF:A8" -ExpectName "stu001" -ExpectIp "10.110.0.31"
param(
  [string]$Center = "http://127.0.0.1:8080",
  [string]$Mac = "",
  [string]$ExpectName = "",
  [string]$ExpectIp = "",
  [string]$Username = "admin",
  [string]$Password = "admin123"
)
$ErrorActionPreference = "Stop"

function Fail($msg) { Write-Host "IDENTITY_ACCEPT_FAIL: $msg"; exit 1 }
function Ok($msg)   { Write-Host "OK: $msg" }

if (-not $Mac) { Fail "-Mac 必填（例如 40:FE:95:B9:FF:A8）" }
$normalized = ($Mac -replace '[^0-9A-Fa-f]', '').ToUpperInvariant()
if ($normalized.Length -ne 12) { Fail "MAC 格式非法: $Mac" }
$normalized = ($normalized -split '(.{2})' | Where-Object { $_ }) -join ':'

try {
  $login = Invoke-RestMethod -Method Post -Uri "$Center/api/v1/auth/login" `
    -ContentType "application/json" `
    -Body (@{ username = $Username; password = $Password } | ConvertTo-Json)
} catch {
  Fail "登录 Center 失败：$($_.Exception.Message)"
}
$headers = @{ Authorization = "Bearer $($login.token)" }

# 1) 终端台账：同一个 MAC 只能有一条，名字/IP 与 Center 分配一致
$clients = Invoke-RestMethod -Uri "$Center/api/v1/clients" -Headers $headers
$matches = @($clients | Where-Object { $_.mac -eq $normalized })
if ($matches.Count -eq 0) { Fail "clients 中没有 $normalized（Agent 心跳未到或未配置默认机房）" }
if ($matches.Count -gt 1) { Fail "clients 中 $normalized 存在多条记录" }
$c = $matches[0]
Ok "client id=$($c.id) name=$($c.name) assigned_ip=$($c.assigned_ip) last_seen=$($c.last_seen)"
if ($ExpectName -and $c.name -ne $ExpectName) { Fail "name=$($c.name)，期望 $ExpectName" }
if ($ExpectIp -and $c.assigned_ip -ne $ExpectIp) { Fail "assigned_ip=$($c.assigned_ip)，期望 $ExpectIp" }
if (-not $c.last_seen) { Fail "last_seen 为空（从未心跳/报到）" }

# 2) Agent 心跳会话：存在且 last_seen 在 10 分钟内（批量收敛后应持续刷新）
$sessions = Invoke-RestMethod -Uri "$Center/api/v1/agent/sessions" -Headers $headers
$s = @($sessions | Where-Object { $_.mac -eq $normalized })
if ($s.Count -eq 0) { Fail "agent_sessions 中没有 $normalized —— Agent 心跳未到 Center" }
$seen = [datetimeoffset]::Parse($s[0].last_seen)
if (([datetimeoffset]::UtcNow - $seen).TotalMinutes -gt 10) {
  Fail "Agent 最近心跳为 $($s[0].last_seen)（超过 10 分钟），检查计划任务/网络"
}
Ok "agent session hostname=$($s[0].hostname) last_seen=$($s[0].last_seen)"
if ($s[0].hostname -and $ExpectName -and $s[0].hostname -ne $ExpectName) {
  Write-Host "NOTE: agent 上报 hostname=$($s[0].hostname)（收敛中或未改名）"
}

# 3) 机房 IP 模式：DHCP 保留时提示检查 Edge 绑定
$labs = Invoke-RestMethod -Uri "$Center/api/v1/labs" -Headers $headers
$lab = @($labs | Where-Object { $_.id -eq $c.lab_id })[0]
if ($lab) { Ok "lab=$($lab.name) ip_mode=$($lab.ip_mode)" }

Write-Host "IDENTITY_ACCEPT_OK"
