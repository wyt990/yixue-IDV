param(
  [Parameter(Mandatory=$true)][string]$Mac,
  [Parameter(Mandatory=$true)][string]$Script,
  [string]$Center = "http://127.0.0.1:8080",
  [ValidateSet("powershell","cmd")][string]$Shell = "powershell",
  [int]$TimeoutSecs = 60,
  [string]$Username = "admin",
  [Parameter(Mandatory=$true)][string]$Password
)

$ErrorActionPreference = "Stop"
$Center = $Center.TrimEnd('/')

$login = Invoke-RestMethod -Method Post -Uri "$Center/api/v1/auth/login" `
  -ContentType "application/json" `
  -Body (@{ username = $Username; password = $Password } | ConvertTo-Json -Compress)
$headers = @{ Authorization = "Bearer $($login.token)" }

$execPayload = @{
  shell = $Shell
  script = $Script
  timeout_secs = $TimeoutSecs
} | ConvertTo-Json -Compress

$created = Invoke-RestMethod -Method Post `
  -Uri "$Center/api/v1/clients/$([uri]::EscapeDataString($Mac))/commands" `
  -Headers $headers -ContentType "application/json" `
  -Body (@{ kind = "exec"; payload = $execPayload } | ConvertTo-Json -Compress)

Write-Host "command_id=$($created.id)"
$deadline = (Get-Date).AddSeconds($TimeoutSecs + 25)
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 1
  $cmd = Invoke-RestMethod -Uri "$Center/api/v1/agent/commands/$($created.id)" -Headers $headers
  if ($cmd.state -notin @("pending","delivered","running")) {
    Write-Host "state=$($cmd.state)"
    try {
      $r = $cmd.error_message | ConvertFrom-Json
      Write-Host "exit_code=$($r.exit_code) timed_out=$($r.timed_out)"
      if ($r.stdout) { Write-Host "--- stdout ---"; Write-Host $r.stdout }
      if ($r.stderr) { Write-Host "--- stderr ---"; Write-Host $r.stderr }
    } catch {
      Write-Host $cmd.error_message
    }
    exit $(if ($cmd.state -eq "done") { 0 } else { 1 })
  }
}
throw "timeout waiting for client result"
