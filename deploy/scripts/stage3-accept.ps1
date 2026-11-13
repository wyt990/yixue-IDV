# 阶段 3 验收：DHCP Offer、TFTP boot.ipxe、PE checkin、Center/Edge 可见会话、无任务提示
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $root

$centerPort = 18080
$edgePort = 18081
$dhcpPort = 16767
$tftpPort = 16969
$base = "http://127.0.0.1:$centerPort"
$edgeBase = "http://127.0.0.1:$edgePort"
$dataDir = Join-Path $root "data\stage3-accept"
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
$db = ((Join-Path $dataDir "center.db") -replace "\\", "/")
$centerCfg = Join-Path $dataDir "center.toml"
$edgeCfg = Join-Path $dataDir "edge.toml"
$peCfg = Join-Path $dataDir "pe-agent.toml"

Get-Process idv-center,idv-edge,idv-pe-agent -ErrorAction SilentlyContinue | Stop-Process -Force
foreach ($p in @($centerPort, $edgePort, $dhcpPort, $tftpPort)) {
  Get-NetTCPConnection -LocalPort $p -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
  }
}
Start-Sleep -Seconds 1
Remove-Item "$db*" -Force -ErrorAction SilentlyContinue

@"
listen = "127.0.0.1:$centerPort"
log = "warn"
database_url = "sqlite://${db}?mode=rwc"
web_dir = "web/dist"
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
cargo build -p idv-center -p idv-edge -p idv-pe-agent | Out-Host
$centerBin = Join-Path $root "target\debug\idv-center.exe"
$edgeBin = Join-Path $root "target\debug\idv-edge.exe"
$peBin = Join-Path $root "target\debug\idv-pe-agent.exe"

Write-Host "== start center =="
$center = Start-Process -PassThru -WindowStyle Hidden -FilePath $centerBin -ArgumentList @("serve","--config",$centerCfg) -WorkingDirectory $root
Wait-Http "$base/health" 60 | Out-Null
$login = Invoke-Json POST "$base/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token
$edgeName = "stage3-edge-$(Get-Random)"
$edge = Invoke-Json POST "$base/api/v1/edges" @{ name = $edgeName } $token

@"
listen = "127.0.0.1:$edgePort"
center_url = "$base"
name = "$edgeName"
node_token = "$($edge.node_token)"
cache_dir = "$((Join-Path $dataDir 'edge-cache') -replace '\\','/')"
log = "warn"
heartbeat_secs = 3
poll_secs = 1

[pxe]
enabled = true
mode = "full"
dhcp_bind = "127.0.0.1:$dhcpPort"
tftp_bind = "127.0.0.1:$tftpPort"
tftp_root = "pe"
boot_file = "boot.ipxe"
next_server = "127.0.0.1"
subnet_mask = "255.255.255.0"
router = "127.0.0.1"
offer_start = "10.10.0.100"
offer_end = "10.10.0.200"
lease_secs = 3600
"@ | Set-Content -Encoding utf8 $edgeCfg

@"
edge_url = "$edgeBase"
log = "warn"
"@ | Set-Content -Encoding utf8 $peCfg

Write-Host "== start edge =="
$edgeProc = Start-Process -PassThru -WindowStyle Hidden -FilePath $edgeBin -ArgumentList @("serve","--config",$edgeCfg) -WorkingDirectory $root
$health = Wait-Http "$edgeBase/health" 60
if (-not $health.pxe_enabled) { throw "pxe_enabled should be true" }

Write-Host "== DHCP probe =="
& $edgeBin dhcp-probe --server "127.0.0.1:$dhcpPort" --mac "02:AA:BB:CC:DD:01"
if ($LASTEXITCODE -ne 0) { throw "dhcp probe failed" }

Write-Host "== TFTP boot.ipxe =="
# lightweight TFTP RRQ via .NET sockets
$udp = New-Object System.Net.Sockets.UdpClient
$udp.Client.ReceiveTimeout = 3000
$rrq = New-Object System.Collections.Generic.List[byte]
$rrq.AddRange([byte[]](0,1)) # RRQ
$rrq.AddRange([System.Text.Encoding]::ASCII.GetBytes("boot.ipxe"))
$rrq.Add(0)
$rrq.AddRange([System.Text.Encoding]::ASCII.GetBytes("octet"))
$rrq.Add(0)
$remote = New-Object System.Net.IPEndPoint ([System.Net.IPAddress]::Parse("127.0.0.1"), $tftpPort)
[void]$udp.Send($rrq.ToArray(), $rrq.Count, $remote)
$ep = New-Object System.Net.IPEndPoint ([System.Net.IPAddress]::Any, 0)
$data = $udp.Receive([ref]$ep)
$udp.Close()
if ($data.Length -lt 4 -or $data[1] -ne 3) { throw "TFTP DATA expected" }
$text = [System.Text.Encoding]::ASCII.GetString($data, 4, $data.Length - 4)
if ($text -notmatch "易学 IDV" -and $text -notmatch "Yixue IDV" -and $text -notmatch "ipxe") {
  throw "unexpected boot.ipxe content"
}
Write-Host "TFTP boot.ipxe OK"

Write-Host "== PE checkin =="
$mac = "02:AA:BB:CC:DD:31"
$output = & $peBin checkin --config $peCfg --mac $mac | Out-String
if ($LASTEXITCODE -ne 0) { throw "pe checkin failed" }
if ($output -notmatch "exit_to_local_disk" -and $output -notmatch "has_task=false") {
  throw "expected no-task guidance, got: $output"
}

$edgeSessions = Invoke-Json GET "$edgeBase/api/v1/pe/sessions"
$hit = @($edgeSessions | Where-Object { $_.mac -eq "02:AA:BB:CC:DD:31" })
if ($hit.Count -lt 1) { throw "edge pe session missing" }

Start-Sleep -Seconds 2
$centerSessions = Invoke-Json GET "$base/api/v1/pe-sessions" $null $token
$hit2 = @($centerSessions | Where-Object { $_.mac -eq "02:AA:BB:CC:DD:31" })
if ($hit2.Count -lt 1) { throw "center pe session missing" }

Write-Host "== HTTP pxe scripts =="
$boot = Invoke-WebRequest -Uri "$edgeBase/pxe/boot.ipxe" -UseBasicParsing
if ($boot.StatusCode -ne 200) { throw "boot.ipxe http failed" }

Stop-Process -Id $center.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $edgeProc.Id -Force -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort $centerPort,$edgePort,$dhcpPort,$tftpPort -ErrorAction SilentlyContinue | ForEach-Object {
  Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue
}

Write-Host "STAGE3_ACCEPT_OK"
