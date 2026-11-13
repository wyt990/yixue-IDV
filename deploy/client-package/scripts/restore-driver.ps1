param([Parameter(Mandatory=$true)][ValidateSet("status","protect-on","protect-off")][string]$Action,[string]$Volumes="system")
$ErrorActionPreference="Stop"
. (Join-Path $PSScriptRoot "_common.ps1")
$p=Get-YixuePaths
if(-not(Test-Path $p.Agent)){throw "missing idv-agent.exe"}
switch($Action){"status"{& $p.Agent restore-driver status}"protect-on"{& $p.Agent restore-driver protect-on --volumes $Volumes}"protect-off"{& $p.Agent restore-driver protect-off}}
exit $LASTEXITCODE
