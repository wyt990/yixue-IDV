$ErrorActionPreference = "Stop"
$tsRoot = "F:\Program Files\Microsoft Visual Studio\2022\Community\MSBuild\Microsoft\VC\v170\Platforms\x64\PlatformToolsets\WindowsKernelModeDriver10.0"
New-Item -ItemType Directory -Force -Path $tsRoot | Out-Null

# Minimal toolset: reuse MSVC v143 compiler; kernel includes come from Directory.Build.props
$props = @'
<?xml version="1.0" encoding="utf-8"?>
<Project xmlns="http://schemas.microsoft.com/developer/msbuild/2003">
  <Import Project="$(VCTargetsPath)\Platforms\$(Platform)\PlatformToolsets\v143\Toolset.props" Condition="Exists('$(VCTargetsPath)\Platforms\$(Platform)\PlatformToolsets\v143\Toolset.props')" />
  <PropertyGroup>
    <WDKContentRoot Condition="'$(WDKContentRoot)' == ''">H:\Windows Kits\10\</WDKContentRoot>
    <WindowsKitsRoot Condition="'$(WindowsKitsRoot)' == ''">H:\Windows Kits\10\</WindowsKitsRoot>
    <WindowsSdkDir Condition="'$(WindowsSdkDir)' == ''">H:\Windows Kits\10\</WindowsSdkDir>
    <WindowsTargetPlatformVersion Condition="'$(WindowsTargetPlatformVersion)' == '' or '$(WindowsTargetPlatformVersion)' == '10.0'">10.0.26100.0</WindowsTargetPlatformVersion>
    <TargetPlatformVersion>$(WindowsTargetPlatformVersion)</TargetPlatformVersion>
  </PropertyGroup>
</Project>
'@

$targets = @'
<?xml version="1.0" encoding="utf-8"?>
<Project xmlns="http://schemas.microsoft.com/developer/msbuild/2003">
  <Import Project="$(VCTargetsPath)\Platforms\$(Platform)\PlatformToolsets\v143\Toolset.targets" Condition="Exists('$(VCTargetsPath)\Platforms\$(Platform)\PlatformToolsets\v143\Toolset.targets')" />
</Project>
'@

Set-Content -Path (Join-Path $tsRoot "Toolset.props") -Value $props -Encoding UTF8
Set-Content -Path (Join-Path $tsRoot "Toolset.targets") -Value $targets -Encoding UTF8
Write-Host "OK minimal toolset"
