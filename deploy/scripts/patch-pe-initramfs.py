"""Patch Alpine initramfs so idv.edge= triggers pe-agent without alpine_repo root."""
from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LINUX = ROOT / "pe" / "linux"
WORK = LINUX / "_initrd_work"
INIT = WORK / "init"
OUT = LINUX / "initramfs-lts"

# Keep in sync with patch-initramfs-inplace.py (storage load + disk wait).
FN = r"""
# --- Yixue IDV cold-capture ---
idv_mount_modloop() {
	if [ -f /lib/modules/$(uname -r)/modules.dep ] && grep -q 'nvme\.ko' /lib/modules/$(uname -r)/modules.dep 2>/dev/null; then
		return 0
	fi
	ML=""
	for arg in $(cat /proc/cmdline); do
		case "$arg" in
			modloop=*) ML="${arg#modloop=}" ;;
		esac
	done
	[ -n "$ML" ] || return 1
	mkdir -p /tmp/idv /.modloop /lib/modules
	case "$ML" in
		http://*|https://*|ftp://*)
			if [ ! -f /tmp/idv/modloop ]; then
				echo "Downloading modloop (disk drivers)..."
				wget -O /tmp/idv/modloop "$ML" || return 1
			fi
			MLFILE=/tmp/idv/modloop
			;;
		*)
			MLFILE="$ML"
			;;
	esac
	if ! grep -q ' /.modloop ' /proc/mounts 2>/dev/null; then
		mount -t squashfs -o ro,loop "$MLFILE" /.modloop || return 1
	fi
	umount /lib/modules 2>/dev/null
	mount --bind /.modloop/modules /lib/modules || return 1
	echo "modloop mounted; modules=$(uname -r)"
	return 0
}

idv_load_storage() {
	echo "Loading storage modules..."
	idv_mount_modloop || echo "WARN: modloop mount failed, trying injected modules"
	for m in crc64 crc64_rocksoft t10_pi \
		scsi_common scsi_mod sd_mod \
		libata libahci ahci ata_piix vmd \
		nvme_common nvme_core nvme; do
		modprobe "$m" 2>/dev/null
	done
	_moddir=/lib/modules/$(uname -r)/kernel
	for f in \
		lib/crc64.ko lib/crc64-rocksoft.ko block/t10-pi.ko \
		drivers/scsi/scsi_common.ko drivers/scsi/scsi_mod.ko \
		drivers/ata/libata.ko drivers/ata/libahci.ko drivers/ata/ahci.ko drivers/ata/ata_piix.ko \
		drivers/pci/controller/vmd.ko \
		drivers/nvme/common/nvme-common.ko drivers/nvme/host/nvme-core.ko drivers/nvme/host/nvme.ko \
		drivers/scsi/sd_mod.ko
	do
		[ -f "$_moddir/$f" ] || continue
		modname=$(basename "$f" .ko | tr '-' '_')
		[ -d "/sys/module/$modname" ] && continue
		echo "insmod $f"
		insmod "$_moddir/$f" || echo "WARN: insmod failed: $f"
	done
	[ -x /sbin/mdev ] && mdev -s
	sleep 2
}

idv_find_disk() {
	DISK=""
	for b in /sys/block/*; do
		[ -e "$b" ] || continue
		name=$(basename "$b")
		case "$name" in
			loop*|ram*|sr*|fd*|dm-*|md*|zram*) continue ;;
		esac
		[ -f "$b/size" ] || continue
		sz=$(cat "$b/size" 2>/dev/null || echo 0)
		[ "$sz" -gt 8 ] || continue
		[ -b "/dev/$name" ] || continue
		DISK="/dev/$name"
		case "$name" in nvme*) return 0 ;; esac
	done
	[ -n "$DISK" ] && return 0
	for d in /dev/nvme0n1 /dev/nvme1n1 /dev/sda /dev/sdb /dev/vda /dev/mmcblk0; do
		if [ -b "$d" ]; then
			DISK="$d"
			return 0
		fi
	done
	return 1
}

idv_run_capture() {
	idv_edge=""
	for arg in $(cat /proc/cmdline); do
		case "$arg" in
			idv.edge=*) idv_edge="${arg#idv.edge=}" ;;
		esac
	done
	[ -z "$idv_edge" ] && return 1

	echo ""
	echo "=== Yixue IDV cold capture ==="
	echo "edge=$idv_edge kernel=$(uname -r)"
	mkdir -p /tmp/idv

	idv_load_storage
	i=0
	while [ "$i" -lt 15 ]; do
		idv_find_disk && break
		i=$((i + 1))
		echo "waiting for disk... ($i/15)"
		[ -x /sbin/mdev ] && mdev -s
		sleep 1
	done

	# Always prefer Edge-hosted binary so updates apply without rebuilding apkovl.
	AGENT=""
	echo "Downloading pe-agent from edge..."
	if wget -O /tmp/idv/idv-pe-agent "${idv_edge}/pxe/linux/idv-pe-agent"; then
		chmod +x /tmp/idv/idv-pe-agent
		AGENT=/tmp/idv/idv-pe-agent
	elif [ -f "$sysroot/usr/local/bin/idv-pe-agent" ]; then
		echo "WARN: edge download failed; using apkovl pe-agent"
		cp -f "$sysroot/usr/local/bin/idv-pe-agent" /tmp/idv/idv-pe-agent
		chmod +x /tmp/idv/idv-pe-agent
		AGENT=/tmp/idv/idv-pe-agent
	elif [ -f /usr/local/bin/idv-pe-agent ]; then
		echo "WARN: edge download failed; using local pe-agent"
		cp -f /usr/local/bin/idv-pe-agent /tmp/idv/idv-pe-agent
		chmod +x /tmp/idv/idv-pe-agent
		AGENT=/tmp/idv/idv-pe-agent
	fi

	MAC=""
	for n in /sys/class/net/*; do
		IF=$(basename "$n")
		case "$IF" in lo) continue ;; esac
		MAC=$(cat "$n/address" 2>/dev/null | tr 'a-f' 'A-F')
		[ -n "$MAC" ] && break
	done

	echo "mac=$MAC disk=$DISK agent=$AGENT"
	echo -n "storage modules: "
	for m in ahci nvme sd_mod t10_pi libata vmd; do
		[ -d "/sys/module/$m" ] && echo -n "$m "
	done
	echo
	ls -l /dev/nvme* /dev/sd* /dev/vd* /dev/mmcblk* 2>/dev/null || true
	ls /sys/block 2>/dev/null || true

	if [ -z "$AGENT" ] || [ -z "$MAC" ] || [ -z "$DISK" ]; then
		echo "IDV: missing agent/mac/disk - dropping to shell (not continuing broken root)"
		exec /bin/sh
	fi

	printf 'edge_url = "%s"\nlog = "info"\n' "$idv_edge" > /tmp/idv/pe-agent.toml
	echo "Starting pe-agent on $DISK (single-pass pack+upload; please wait)..."
	"$AGENT" run --config /tmp/idv/pe-agent.toml --mac "$MAC" --disk-file "$DISK" --os-family windows --boot-mode uefi
	rc=$?
	echo "IDV capture finished rc=$rc"
	if [ "$rc" -eq 0 ]; then
		echo "IDV capture OK - rebooting in 8s"
		sleep 8
		reboot -f
	fi
	echo "IDV capture FAILED - not rebooting into retry loop; shell in 60s"
	sleep 60
	exec /bin/sh
}
# --- end Yixue IDV ---

"""


def main() -> None:
    if not INIT.exists():
        raise SystemExit(f"missing extracted init: {INIT} (run extract first)")

    text = INIT.read_text(encoding="utf-8", errors="replace")
    import re

    # Always refresh function body if present.
    if "idv_run_capture() {" in text:
        text = re.sub(
            r"\nidv_run_capture\(\) \{.*?# --- end Yixue IDV ---\n",
            "\n" + FN.lstrip("\n"),
            text,
            count=1,
            flags=re.S,
        )
        print("refreshed idv_run_capture")
    else:
        anchor = "unpack_apkovl() {"
        if anchor not in text:
            raise SystemExit("unpack_apkovl anchor missing")
        text = text.replace(anchor, FN + anchor, 1)

        early = (
            '\t[ -d "$sysroot/root" ] && chmod 700 "$sysroot/root"\n'
            '\tpkgs="$pkgs $(cat $sysroot/etc/apk/world 2>/dev/null)"\n'
            "fi\n"
        )
        early_repl = (
            '\t[ -d "$sysroot/root" ] && chmod 700 "$sysroot/root"\n'
            '\tpkgs="$pkgs $(cat $sysroot/etc/apk/world 2>/dev/null)"\n'
            "\t# IDV: run capture before alpine_repo/apk root install\n"
            "\tidv_run_capture\n"
            "fi\n"
        )
        if early not in text:
            raise SystemExit("early apkovl anchor missing")
        text = text.replace(early, early_repl, 1)

        late = 'if [ ! -x "${sysroot}${KOPT_init}" ]; then'
        late_repl = (
            "# IDV fallback if alpine root missing\n"
            "idv_run_capture\n\n"
            'if [ ! -x "${sysroot}${KOPT_init}" ]; then'
        )
        if late not in text:
            raise SystemExit("late init-check anchor missing")
        text = text.replace(late, late_repl, 1)
        print("patched", INIT)

    INIT.write_text(text, encoding="utf-8", newline="\n")

    # Also call capture when there is no apkovl but idv.edge is set — after network.
    # The late fallback covers that path.

    # Kernel initramfs must be gzipped cpio (newc), NOT tar.gz.
    from pack_initramfs_cpio import pack  # type: ignore

    try:
        pack(WORK, OUT)
    except ImportError:
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "pack_initramfs_cpio", Path(__file__).with_name("pack-initramfs-cpio.py")
        )
        mod = importlib.util.module_from_spec(spec)
        assert spec.loader is not None
        spec.loader.exec_module(mod)
        mod.pack(WORK, OUT)


if __name__ == "__main__":
    main()
