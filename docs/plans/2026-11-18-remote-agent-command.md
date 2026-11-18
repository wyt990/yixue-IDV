# Remote Agent Command / Script (2026-11-18)

## Feature

Center can enqueue an `exec` command for a client. The Agent runs it as the
service account (SYSTEM) using either the Windows PowerShell or cmd shell and
returns a JSON result:

```json
{
  "shell": "powershell",
  "exit_code": 0,
  "timed_out": false,
  "stdout": "...",
  "stderr": "..."
}
```

Payload:

```json
{
  "shell": "powershell",
  "script": "Get-FileHash C:\\Windows\\System32\\drivers\\SysRestoreDriver.sys -Algorithm SHA256",
  "timeout_secs": 60
}
```

`timeout_secs` is clamped to 1..300 seconds. Output is capped at 64 KiB per
stream. PowerShell is started with:

```text
-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <temp.ps1>
```

so no machine-wide ExecutionPolicy change is required.

## Center API

```text
POST /api/v1/clients/{mac}/commands
{ "kind": "exec", "payload": "<json string>" }

GET /api/v1/agent/commands/{id}
```

The normal admin auth and write permission apply.

## Web UI

Terminal details now has a "远程脚本" box with PowerShell/CMD selector,
timeout, execute button, and returned stdout/stderr. The result is also stored
in `agent_commands.error_message`.

## Security

This is arbitrary code execution as SYSTEM. It must only be exposed to trusted
administrators on the management network. Every command is audited in
`agent_commands` with the creating user, MAC and timestamp.
