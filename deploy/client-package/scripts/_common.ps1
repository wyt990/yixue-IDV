function Get-YixueRoot {
  Split-Path $PSScriptRoot -Parent
}

function Get-YixuePaths {
  $root = Get-YixueRoot
  [pscustomobject]@{
    Root     = $root
    Agent    = Join-Path $root "idv-agent.exe"
    Config   = Join-Path $root "agent.toml"
    Driver   = Join-Path $root "driver"
    StateDir = Join-Path $env:ProgramData "YixueIDV"
  }
}
