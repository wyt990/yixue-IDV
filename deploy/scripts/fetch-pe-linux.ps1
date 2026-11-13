# Download Alpine LTS netboot + build musl pe-agent overlay for cold capture PE.
# Usage: powershell -ExecutionPolicy Bypass -File deploy/scripts/fetch-pe-linux.ps1
$ErrorActionPreference = "Stop"
$Root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not (Test-Path "$Root\Cargo.toml")) { $Root = (Get-Location).Path }
$Out = Join-Path $Root "pe\linux"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

$AlpineVer = "v3.20"
$Base = "https://dl-cdn.alpinelinux.org/alpine/$AlpineVer/releases/x86_64/netboot"
Write-Host "Downloading Alpine netboot ($AlpineVer)..."
Invoke-WebRequest -Uri "$Base/vmlinuz-lts" -OutFile (Join-Path $Out "vmlinuz-lts") -UseBasicParsing
Invoke-WebRequest -Uri "$Base/initramfs-lts" -OutFile (Join-Path $Out "initramfs-lts") -UseBasicParsing
Invoke-WebRequest -Uri "$Base/modloop-lts" -OutFile (Join-Path $Out "modloop-lts") -UseBasicParsing

Write-Host "Building idv-pe-agent (linux-musl, rust-lld static)..."
Push-Location $Root
$env:RUSTFLAGS = "-C target-feature=+crt-static -C linker=rust-lld"
cargo build -p idv-pe-agent --release --target x86_64-unknown-linux-musl
Pop-Location
$Agent = Join-Path $Root "target\x86_64-unknown-linux-musl\release\idv-pe-agent"
if (-not (Test-Path $Agent)) { throw "musl pe-agent not found: $Agent" }
Copy-Item $Agent (Join-Path $Out "idv-pe-agent") -Force

# Minimal apkovl: local.d script runs pe-agent after Alpine netboot brings up network.
$Ovl = Join-Path $Out "ovl-work"
if (Test-Path $Ovl) { Remove-Item -Recurse -Force $Ovl }
New-Item -ItemType Directory -Force -Path "$Ovl\etc\local.d" | Out-Null
New-Item -ItemType Directory -Force -Path "$Ovl\usr\local\bin" | Out-Null
New-Item -ItemType Directory -Force -Path "$Ovl\etc\runlevels\default" | Out-Null
Copy-Item $Agent "$Ovl\usr\local\bin\idv-pe-agent" -Force

@'
#!/bin/sh
# Yixue IDV cold-capture entry (Alpine local.d)
export PATH=/usr/local/bin:/usr/bin:/bin
EDGE="${IDV_EDGE_URL:-}"
if [ -z "$EDGE" ]; then
  for arg in $(cat /proc/cmdline); do
    case "$arg" in
      idv.edge=*) EDGE="${arg#idv.edge=}" ;;
    esac
  done
fi
[ -z "$EDGE" ] && EDGE="http://10.101.0.36:8081"
export IDV_EDGE_URL="$EDGE"

MAC=""
for n in /sys/class/net/*; do
  IF=$(basename "$n")
  case "$IF" in lo) continue ;; esac
  MAC=$(cat "$n/address" 2>/dev/null | tr 'a-f' 'A-F')
  [ -n "$MAC" ] && break
done
DISK=""
for d in /dev/nvme0n1 /dev/sda /dev/vda; do
  if [ -b "$d" ]; then DISK="$d"; break; fi
done
echo "idv-pe: edge=$EDGE mac=$MAC disk=$DISK"
if [ -z "$MAC" ] || [ -z "$DISK" ]; then
  echo "idv-pe: missing mac/disk; dropping to shell"
  exec /bin/sh
fi
# Tiny config for pe-agent
mkdir -p /tmp/idv
cat > /tmp/idv/pe-agent.toml <<EOF
edge_url = "$EDGE"
log = "info,idv_pe_agent=info"
EOF
/usr/local/bin/idv-pe-agent run --config /tmp/idv/pe-agent.toml --mac "$MAC" --disk-file "$DISK" --os-family windows --boot-mode uefi
rc=$?
echo "idv-pe: pe-agent finished rc=$rc"
if [ "$rc" -eq 0 ]; then
  echo "idv-pe: OK — rebooting in 10s"
  sleep 10
  reboot -f
fi
echo "idv-pe: FAILED — not rebooting into retry loop; shell in 60s"
sleep 60
exec /bin/sh
'@ | Set-Content -Encoding ascii -NoNewline "$Ovl\etc\local.d\idv.start"
# Alpine local.d requires +x; mark via mode in tar later.

# Enable local service in default runlevel (symlink name only in apkovl)
"" | Set-Content -Encoding ascii "$Ovl\etc\runlevels\default\local"

# Create apkovl tar.gz (GNU tar if available, else .NET zip fallback note)
$Apkovl = Join-Path $Out "idv.apkovl.tar.gz"
if (Get-Command tar -ErrorAction SilentlyContinue) {
  Push-Location $Ovl
  # busybox/alpine expect executable bit on local.d scripts — tar from Windows may not set mode;
  # pe README documents chmod after extract if needed. We set mode via tar --mode if GNU tar.
  tar -czf $Apkovl --mode=755 etc/local.d/idv.start usr/local/bin/idv-pe-agent etc/runlevels/default/local 2>$null
  if (-not (Test-Path $Apkovl)) {
    tar -czf $Apkovl etc usr
  }
  Pop-Location
} else {
  throw "tar not found; install Windows tar or Git for Windows"
}

Write-Host "PE linux assets ready under $Out"
Write-Host "  vmlinuz-lts, initramfs-lts, modloop-lts, idv-pe-agent, idv.apkovl.tar.gz"
Write-Host "Edge must serve pe/ via /pxe (already configured)."
