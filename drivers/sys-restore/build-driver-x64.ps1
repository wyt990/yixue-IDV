# Build SysRestoreDriver.sys (Release|x64) without full WDK VS integration.
$ErrorActionPreference = "Stop"

$Kit = "H:\Windows Kits\10"
$Ver = "10.0.26100.0"
$Src = Join-Path $PSScriptRoot "SysRestoreDriver"
$Out = Join-Path $Src "x64\Release"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

$vsDevCmd = "F:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmd)) { throw "VsDevCmd.bat not found: $vsDevCmd" }

$km = "$Kit\Include\$Ver\km"
$shared = "$Kit\Include\$Ver\shared"
$crt = "$Kit\Include\$Ver\km\crt"
$lib = "$Kit\Lib\$Ver\km\x64"
foreach ($p in @("$km\ntddk.h", "$km\ntifs.h", "$lib\ntoskrnl.lib", "$lib\bufferoverflowfastfailk.lib")) {
  if (-not (Test-Path $p)) { throw "Missing: $p" }
}

$clFlags = @(
  "/c", "/nologo", "/W3", "/O2", "/Zp8", "/Gy", "/Gm-", "/EHs-c-", "/GR-", "/GF", "/GS",
  "/Zi", "/kernel",
  "/D_WIN64", "/D_AMD64_", "/DAMD64",
  "/DNTDDI_VERSION=0xA000010", "/D_WIN32_WINNT=0x0A00", "/DWINVER=0x0A00", "/DWINNT=1",
  "/I`"$km`"", "/I`"$crt`"", "/I`"$shared`"",
  "/Fo`"$Out\\`""
) -join " "

$objs = @("Driver.obj", "QHBitmap.obj", "QHIrpDispatchs.obj", "QHOffsetHash.obj") |
  ForEach-Object { "`"$Out\$_`"" }
$objsLine = $objs -join " "

$cmd = @"
@echo off
call "$vsDevCmd" -arch=amd64 -host_arch=amd64 >nul
cd /d "$Src"
cl.exe $clFlags Driver.c QHBitmap.c QHIrpDispatchs.c QHOffsetHash.c
if errorlevel 1 exit /b 1
link.exe /nologo /DRIVER:WDM /SUBSYSTEM:NATIVE /ENTRY:GsDriverEntry /NODEFAULTLIB /LIBPATH:"$lib" /OUT:"$Out\SysRestoreDriver.sys" /PDB:"$Out\SysRestoreDriver.pdb" $objsLine ntoskrnl.lib hal.lib wmilib.lib bufferoverflowfastfailk.lib
if errorlevel 1 exit /b 1
echo BUILD_OK
"@

$bat = Join-Path $Out "_build.bat"
Set-Content -Path $bat -Value $cmd -Encoding ASCII
Write-Host "Running $bat"
& cmd.exe /c $bat
if ($LASTEXITCODE -ne 0) { throw "driver build failed exit=$LASTEXITCODE" }

$sys = Join-Path $Out "SysRestoreDriver.sys"
$inf = Join-Path $Src "SysRestoreDriver.inf"
$uiRes = Join-Path $PSScriptRoot "QHEngineUI\resources"
New-Item -ItemType Directory -Force -Path $uiRes | Out-Null
Copy-Item -Force $sys $uiRes
Copy-Item -Force $inf $uiRes
Write-Host "Built: $sys ($((Get-Item $sys).Length) bytes)"
