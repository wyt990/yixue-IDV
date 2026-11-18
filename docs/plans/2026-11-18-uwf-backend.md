# UWF Restore Backend

## Requirements

UWF is a Microsoft feature and is only supported on:

- Windows 10/11 Enterprise
- Windows 10/11 Education
- Windows IoT Enterprise

It is **not supported on Windows Pro or Pro Education**. On those editions the
feature can be enabled, but `UWF_Volume.Protect` returns `0x80070005`.

## Enable feature (once per image)

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName Client-UnifiedWriteFilter -All -NoRestart
# reboot
```

## Agent backend

`agent.toml`:

```toml
restore_backend = "uwf"
```

`auto` selects UWF when `C:\Windows\System32\uwfmgr.exe` exists.

The Agent then:

- status: queries `UWF_Filter` / `UWF_Volume` through WMI
- protect on: `uwfmgr filter enable` + `uwfmgr volume protect C:` / `D:`
- protect off: `uwfmgr filter disable`
- reboot after the mode transition (same one-reboot path as before)

## Recommended extra settings

- Disable Fast Startup: UWF does not support it.
- Use a disk overlay for larger write volume, e.g.:

```powershell
uwfmgr overlay set-type disk
uwfmgr overlay set-size 8192
```

- UWF protects volumes, not the partition table, ESP or the whole disk.
- UWF is Microsoft-signed: test signing can stay off and Secure Boot can stay on.
