"""Pack Alpine initramfs workdir as gzipped newc cpio (NOT tar.gz)."""
from __future__ import annotations

import gzip
import os
import stat
from pathlib import Path


def pad(n: int, align: int = 4) -> int:
    return (align - (n % align)) % align


def pack(work: Path, out: Path) -> None:
    items: list[tuple[str, Path]] = []
    items.append((".", work))
    for root, dirs, files in os.walk(work):
        dirs.sort()
        files.sort()
        rel_root = Path(root).relative_to(work).as_posix()
        if rel_root != ".":
            items.append(("./" + rel_root, Path(root)))
        for name in files:
            p = Path(root) / name
            rel = p.relative_to(work).as_posix()
            items.append(("./" + rel, p))

    buf = bytearray()
    ino = 1
    for name, p in items:
        st = p.stat()
        # Windows loses Unix +x; force sensible newc modes for Alpine init.
        if p.is_dir():
            mode = 0o040755
            data = b""
        elif p.is_symlink():
            mode = 0o120777
            data = os.readlink(p).encode()
        else:
            data = p.read_bytes()
            base = p.name
            rel = name
            execish = (
                rel == "./init"
                or "/bin/" in rel
                or "/sbin/" in rel
                or base.endswith(".sh")
                or data[:2] == b"#!"
                or data[:4] == b"\x7fELF"
            )
            mode = 0o100755 if execish else 0o100644
        filesize = len(data)
        names = name.encode("utf-8") + b"\0"
        hdr = (
            b"070701"
            + f"{ino & 0xffffffff:08x}".encode()
            + f"{mode & 0xffffffff:08x}".encode()
            + f"{0:08x}".encode()
            + f"{0:08x}".encode()
            + f"{1:08x}".encode()
            + f"{int(st.st_mtime) & 0xffffffff:08x}".encode()
            + f"{filesize & 0xffffffff:08x}".encode()
            + f"{0:08x}".encode()
            + f"{0:08x}".encode()
            + f"{0:08x}".encode()
            + f"{0:08x}".encode()
            + f"{len(names) & 0xffffffff:08x}".encode()
            + f"{0:08x}".encode()
        )
        buf += hdr
        buf += names
        buf += b"\0" * pad(len(hdr) + len(names))
        if filesize:
            buf += data
            buf += b"\0" * pad(filesize)
        ino += 1

    tname = b"TRAILER!!!\0"
    th = (
        b"070701"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + b"00000001"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + b"00000000"
        + f"{len(tname):08x}".encode()
        + b"00000000"
    )
    buf += th + tname
    buf += b"\0" * pad(len(th) + len(tname))

    with gzip.open(out, "wb", compresslevel=9) as g:
        g.write(buf)

    with gzip.open(out, "rb") as g:
        magic = g.read(6)
    print(f"wrote {out} size={out.stat().st_size} magic={magic!r} entries={len(items)}")
    if magic != b"070701":
        raise SystemExit("bad cpio magic after pack")


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    work = root / "pe" / "linux" / "_initrd_work"
    out = root / "pe" / "linux" / "initramfs-lts"
    if not (work / "init").exists():
        raise SystemExit(f"missing {work / 'init'}")
    pack(work, out)


if __name__ == "__main__":
    main()
