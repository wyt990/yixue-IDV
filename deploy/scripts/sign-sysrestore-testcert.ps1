# Sign SysRestoreDriver.sys with a local test code-signing certificate.
# Requires admin for LocalMachine cert stores. Target PCs need testsigning=on + import .cer
param(
  [string]$SysPath = "",
  [string]$OutDir = "",
  [string]$CertName = "YixueIDV-SysRestore-Test"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not (Test-Path (Join-Path $Root "Cargo.toml"))) { $Root = (Get-Location).Path }

if (-not $SysPath) {
  $SysPath = Join-Path $Root "drivers\sys-restore\SysRestoreDriver\x64\Release\SysRestoreDriver.sys"
}
if (-not (Test-Path $SysPath)) { throw "driver not found: $SysPath" }

if (-not $OutDir) {
  $OutDir = Join-Path $Root "deploy\client-package\driver"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$signTool = "H:\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe"
if (-not (Test-Path $signTool)) {
  $found = Get-ChildItem "H:\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
    Select-Object -First 1
  if (-not $found) { throw "signtool.exe not found" }
  $signTool = $found.FullName
}

$cert = Get-ChildItem Cert:\CurrentUser\My |
  Where-Object { $_.Subject -eq "CN=$CertName" -and $_.HasPrivateKey } |
  Select-Object -First 1

if (-not $cert) {
  Write-Host "Creating test code-signing cert CN=$CertName"
  $cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject "CN=$CertName" `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -HashAlgorithm SHA256 `
    -CertStoreLocation Cert:\CurrentUser\My `
    -KeyExportPolicy Exportable `
    -NotAfter (Get-Date).AddYears(5)
}

$cerPath = Join-Path $OutDir "YixueIDV-SysRestore-Test.cer"
Export-Certificate -Cert $cert -FilePath $cerPath -Force | Out-Null

$pfxDir = Join-Path $Root "drivers\sys-restore\testcert"
New-Item -ItemType Directory -Force -Path $pfxDir | Out-Null
$pfxPath = Join-Path $pfxDir "YixueIDV-SysRestore-Test.pfx"
$pfxPassPlain = "yixue-test-only"
$pfxPass = ConvertTo-SecureString -String $pfxPassPlain -Force -AsPlainText
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $pfxPass -Force | Out-Null
# Do not import into Root here (can hang on UI prompts). Client runs install-testcert.ps1.

$outSys = Join-Path $OutDir "SysRestoreDriver.sys"
$infSrc = Join-Path $Root "drivers\sys-restore\SysRestoreDriver\SysRestoreDriver.inf"
Copy-Item -Force $SysPath $outSys
Copy-Item -Force $infSrc (Join-Path $OutDir "SysRestoreDriver.inf")
# copy license if present
foreach ($extra in @("LICENSE", "NOTICE")) {
  $src = Join-Path $Root "drivers\sys-restore\$extra"
  if (Test-Path $src) { Copy-Item -Force $src (Join-Path $OutDir $extra) }
}

Write-Host "Signing $outSys"
& $signTool sign /fd SHA256 /f $pfxPath /p $pfxPassPlain /v $outSys
if ($LASTEXITCODE -ne 0) { throw "signtool sign failed" }

Write-Host "Verify:"
& $signTool verify /pa /v $outSys
Write-Host ("OK thumbprint=" + $cert.Thumbprint)
Write-Host ("signed sys: " + $outSys)
Write-Host ("public cer: " + $cerPath)
Write-Host ("pfx build-only: " + $pfxPath)
