# 导入 SysRestore 测试签名公钥（管理员）。配合 bcdedit testsigning on。
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\install-testcert.ps1
$ErrorActionPreference = "Stop"
$Root = Split-Path $PSScriptRoot -Parent
$cer = Join-Path $Root "driver\YixueIDV-SysRestore-Test.cer"
if (-not (Test-Path $cer)) { throw "找不到 $cer" }

$cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($cer)
Write-Host "导入测试证书: $($cert.Subject) thumbprint=$($cert.Thumbprint)"

foreach ($storeName in @("Root", "TrustedPublisher")) {
  $store = New-Object System.Security.Cryptography.X509Certificates.X509Store($storeName, "LocalMachine")
  $store.Open("ReadWrite")
  $dup = $store.Certificates | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
  if ($dup) {
    Write-Host "已存在于 LocalMachine\$storeName"
  } else {
    $store.Add($cert)
    Write-Host "已加入 LocalMachine\$storeName"
  }
  $store.Close()
}

$bcd = (bcdedit /enum "{current}" | Out-String)
if ($bcd -notmatch "testsigning\s+Yes") {
  Write-Host "WARNING: testsigning 未开。请先 .\scripts\enable-testsigning.ps1 并重启。"
} else {
  Write-Host "testsigning 已开启。"
}
Write-Host "INSTALL_TESTCERT_OK"
