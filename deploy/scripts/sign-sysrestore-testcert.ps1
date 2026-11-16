# Sign SysRestoreDriver.sys with the local test code-signing certificate,
# generate/sign SysRestoreDriver.cat, then publish the signed package.
#
# IMPORTANT: testsigning only allows *test-signed* drivers. An unsigned .sys is
# rejected even with `bcdedit /set testsigning on`. This script must be run on the
# build machine after build-driver-x64.ps1; never copy the unsigned build output
# into deploy/client-package/driver.
#
#   powershell -ExecutionPolicy Bypass -File deploy\scripts\sign-sysrestore-testcert.ps1
#
# Client machines additionally need:
#   testsigning=on (Secure Boot off) + import YixueIDV-SysRestore-Test.cer
param(
  [string]$SysPath = "",
  [string]$OutDir = "",
  [string]$CertName = "YixueIDV-SysRestore-Test",
  [string]$PfxPath = "",
  [string]$PfxPassword = "yixue-test-only"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not (Test-Path (Join-Path $Root "Cargo.toml"))) { $Root = (Get-Location).Path }

function Find-Tool([string]$name, [string[]]$globs) {
  foreach ($g in $globs) {
    $hits = Get-ChildItem $g -Recurse -Filter $name -ErrorAction SilentlyContinue |
      Sort-Object FullName -Descending
    if ($hits) { return $hits[0].FullName }
  }
  throw "$name not found. Install Windows SDK/WDK (signtool) or pass the path explicitly."
}

$signTool = Find-Tool "signtool.exe" @(
  "H:\Windows Kits\10\bin",
  "C:\Program Files (x86)\Windows Kits\10\bin",
  "C:\Program Files\Windows Kits\10\bin"
)
# Inf2Cat ships as a 32-bit tool; prefer x86, but accept any hit.
try {
  $inf2cat = Find-Tool "Inf2Cat.exe" @(
    "H:\Windows Kits\10\bin",
    "C:\Program Files (x86)\Windows Kits\10\bin",
    "C:\Program Files\Windows Kits\10\bin"
  )
} catch {
  throw "Inf2Cat.exe not found (WDK required): $($_.Exception.Message)"
}

if (-not $SysPath) {
  $SysPath = Join-Path $Root "drivers\sys-restore\SysRestoreDriver\x64\Release\SysRestoreDriver.sys"
}
if (-not (Test-Path $SysPath)) {
  throw "unsigned driver not found: $SysPath (run drivers\sys-restore\build-driver-x64.ps1 first)"
}
if (-not $OutDir) {
  $OutDir = Join-Path $Root "deploy\client-package\driver"
}
$infSrc = Join-Path $Root "drivers\sys-restore\SysRestoreDriver\SysRestoreDriver.inf"
if (-not (Test-Path $infSrc)) { throw "INF not found: $infSrc" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$work = Join-Path $env:TEMP ("idv-driver-sign-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null

try {
  # 1) Stage an isolated copy first: never publish a half-signed package.
  Copy-Item $SysPath (Join-Path $work "SysRestoreDriver.sys") -Force
  Copy-Item $infSrc (Join-Path $work "SysRestoreDriver.inf") -Force

  # 2) Reuse the existing PFX (its public cert is already shipped to clients).
  #    Only create a new cert when the PFX is missing; doing so invalidates every
  #    client that imported the old .cer.
  if (-not $PfxPath) {
    $PfxPath = Join-Path $Root "drivers\sys-restore\testcert\YixueIDV-SysRestore-Test.pfx"
  }
  $createdNewCert = $false
  if (-not (Test-Path $PfxPath)) {
    Write-Host "WARNING: PFX not found, creating a NEW test cert CN=$CertName"
    Write-Host "WARNING: every client must re-import the new .cer after this run."
    $cert = New-SelfSignedCertificate `
      -Type CodeSigningCert `
      -Subject "CN=$CertName" `
      -KeyAlgorithm RSA `
      -KeyLength 2048 `
      -HashAlgorithm SHA256 `
      -CertStoreLocation Cert:\CurrentUser\My `
      -KeyExportPolicy Exportable `
      -NotAfter (Get-Date).AddYears(5)
    New-Item -ItemType Directory -Force -Path (Split-Path $PfxPath -Parent) | Out-Null
    $pfxPass = ConvertTo-SecureString -String $PfxPassword -Force -AsPlainText
    Export-PfxCertificate -Cert $cert -FilePath $PfxPath -Password $pfxPass -Force | Out-Null
    $createdNewCert = $true
  } else {
    $pfxPass = ConvertTo-SecureString -String $PfxPassword -Force -AsPlainText
    # EphemeralKeySet: reading the public part must not depend on key-container ACLs.
    $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(
      $PfxPath, $pfxPass,
      [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet
    )
  }
  $cerPath = Join-Path $work "YixueIDV-SysRestore-Test.cer"
  Export-Certificate -Cert $cert -FilePath $cerPath -Force | Out-Null
  Write-Host "Using cert: $($cert.Subject) thumbprint=$($cert.Thumbprint)"
  if ($cert.Thumbprint -ne "CE0A9BDFACE742AD65176E40EB3DA5D98039EDB4" -and -not $createdNewCert) {
    Write-Host "NOTE: PFX thumbprint differs from the previously shipped .cer; clients must re-import."
  }

  # 3) Sign the .sys (embedded signature is what testsigning validates at boot).
  Write-Host "Signing $($work)\SysRestoreDriver.sys"
  & $signTool sign /fd SHA256 /f $PfxPath /p $PfxPassword /v (Join-Path $work "SysRestoreDriver.sys")
  if ($LASTEXITCODE -ne 0) { throw "signtool sign (sys) failed: $LASTEXITCODE" }

  $sig = Get-AuthenticodeSignature (Join-Path $work "SysRestoreDriver.sys")
  if (-not $sig.SignerCertificate) { throw "signature missing after signtool sign" }
  Write-Host "sys signature status: $($sig.Status) (Valid/NotTrusted is expected on build machine)"

  # 4) Catalog: INF declares CatalogFile = SysRestoreDriver.cat, so it must exist
  #    and be signed, otherwise setupapi / signature enforcement reports errors.
  Write-Host "Generating catalog with Inf2Cat"
  & $inf2cat /driver:"$work" /os:10_X64
  if ($LASTEXITCODE -ne 0) { throw "Inf2Cat failed: $LASTEXITCODE" }
  $catRaw = Get-ChildItem $work -Filter "*.cat" | Select-Object -First 1
  if (-not $catRaw) { throw "Inf2Cat produced no .cat" }
  $catPath = Join-Path $work "SysRestoreDriver.cat"
  if ($catRaw.FullName -cne $catPath) { Move-Item $catRaw.FullName $catPath -Force }

  Write-Host "Signing catalog $catPath"
  & $signTool sign /fd SHA256 /f $PfxPath /p $PfxPassword /v $catPath
  if ($LASTEXITCODE -ne 0) { throw "signtool sign (cat) failed: $LASTEXITCODE" }
  $catSig = Get-AuthenticodeSignature $catPath
  if (-not $catSig.SignerCertificate) { throw "catalog signature missing" }
  Write-Host "cat signature status: $($catSig.Status)"

  # 5) Publish only after sys+cat are signed.
  foreach ($name in @("SysRestoreDriver.sys", "SysRestoreDriver.cat", "SysRestoreDriver.inf", "YixueIDV-SysRestore-Test.cer")) {
    Copy-Item (Join-Path $work $name) (Join-Path $OutDir $name) -Force
  }
  foreach ($extra in @("LICENSE", "NOTICE")) {
    $src = Join-Path $Root "drivers\sys-restore\$extra"
    if (Test-Path $src) { Copy-Item -Force $src (Join-Path $OutDir $extra) }
  }

  Write-Host ""
  Write-Host "Signed package ready: $OutDir"
  Get-ChildItem $OutDir | Select-Object Name, Length | Format-Table -AutoSize
  Write-Host ("OK thumbprint=" + $cert.Thumbprint)
  Write-Host ("public cer: " + (Join-Path $OutDir "YixueIDV-SysRestore-Test.cer"))
  Write-Host ("pfx build-only (never ship): " + $PfxPath)
  Write-Host "Next: copy the package to the client, run enable-testsigning.ps1 -> reboot -> install-testcert.ps1 -> install-driver.ps1 -> reboot."
} finally {
  Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
