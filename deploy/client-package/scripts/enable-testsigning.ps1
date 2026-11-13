$ErrorActionPreference = "Stop"
bcdedit /enum "{current}" | Out-Host
bcdedit /set testsigning on | Out-Host
bcdedit /set nointegritychecks on | Out-Host
if ($LASTEXITCODE -ne 0) { throw "bcdedit failed" }
Write-Host "testsigning enabled. Reboot, then install-testcert.ps1 and install-driver.ps1"
