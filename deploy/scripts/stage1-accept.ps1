# 阶段 1 API 验收：2 机房、≥10 MAC、镜像 checksum、重启持久化
$ErrorActionPreference = "Stop"
$base = "http://127.0.0.1:8080"

function Invoke-Json($Method, $Path, $Body = $null, $Token = $null) {
  $headers = @{}
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  if ($null -eq $Body) {
    return Invoke-RestMethod -Method $Method -Uri "$base$Path" -Headers $headers
  }
  $json = $Body | ConvertTo-Json -Compress -Depth 6
  return Invoke-RestMethod -Method $Method -Uri "$base$Path" -Headers $headers -ContentType "application/json" -Body $json
}

Write-Host "== login =="
$login = Invoke-Json POST "/api/v1/auth/login" @{ username = "admin"; password = "admin123" }
$token = $login.token

Write-Host "== campus + 2 labs =="
$campus = Invoke-Json POST "/api/v1/campuses" @{ name = "主校区-$(Get-Random)" } $token
$lab1 = Invoke-Json POST "/api/v1/labs" @{ campus_id = $campus.id; name = "机房A" } $token
$lab2 = Invoke-Json POST "/api/v1/labs" @{ campus_id = $campus.id; name = "机房B" } $token

Write-Host "== 10 clients =="
$macs = 1..10 | ForEach-Object { "02:11:22:33:44:{0:X2}" -f $_ }
$bulk = Invoke-Json POST "/api/v1/clients/bulk" @{ lab_id = $lab1.id; macs = $macs } $token
if ($bulk.created.Count -lt 10) { throw "expected 10 clients, got $($bulk.created.Count)" }

Write-Host "== image =="
$checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
$image = Invoke-Json POST "/api/v1/images" @{
  name = "Win11-教学金样"
  os_family = "windows"
  version_label = "2026.11"
  checksum_sha256 = $checksum
  size_bytes = 21474836480
  storage_uri = "file:///data/images/win11-lab.img"
} $token

Write-Host "OK before restart: labs=2 clients=$($bulk.created.Count) image=$($image.id)"
Write-Output @{
  token = $token
  campus_id = $campus.id
  lab1_id = $lab1.id
  lab2_id = $lab2.id
  image_id = $image.id
  checksum = $checksum
} | ConvertTo-Json
