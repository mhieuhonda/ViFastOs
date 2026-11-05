#!/usr/bin/env python3
"""FS v2 persistence, migration and journal-recovery boot tests."""

from __future__ import annotations

import glob
import os
import secrets
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QEMU_RV = "qemu-system-riscv64"
QEMU_ARM = "qemu-system-aarch64"
QEMU_X86 = "qemu-system-x86_64"
SECTOR = 512
FSUTIL = os.path.join(ROOT, "target", "debug", "vifast-fsutil")


def find_ovmf_code() -> str | None:
    candidates = [
        os.environ.get("OVMF_CODE"),
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-x86_64-code.fd",
        "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/qemu/OVMF_CODE.fd",
    ]
    candidates += sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd"))
    return next((path for path in candidates if path and os.path.exists(path)), None)


def find_ovmf_vars() -> str | None:
    candidates = [
        os.environ.get("OVMF_VARS"),
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-i386-vars.fd",
        "/opt/homebrew/share/qemu/edk2-i386-vars.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ]
    candidates += sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-i386-vars.fd"))
    return next((path for path in candidates if path and os.path.exists(path)), None)


def fsutil(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(
        [FSUTIL, *args], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"fsutil {' '.join(args)} failed: {result.stderr.decode(errors='replace')}"
        )
    return result


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def make_disk(path: str) -> None:
    with open(path, "wb") as image:
        image.truncate(4 * 1024 * 1024)


def seed_legacy(path: str, nonce: bytes) -> None:
    sector = bytearray(SECTOR)
    sector[:4] = b"VF01"
    sector[4:8] = len(nonce).to_bytes(4, "little")
    sector[8 : 8 + len(nonce)] = nonce
    with open(path, "r+b") as image:
        image.write(sector)


def sector0(path: str) -> bytes:
    with open(path, "rb") as image:
        return image.read(4)


def run_qemu(name: str, cmd: list[str], serial: str, timeout: int, ok: set[int]) -> tuple[int, str, bool]:
    if os.path.exists(serial):
        os.unlink(serial)
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    timed_out = False
    try:
        rc = proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        rc = proc.wait()
    output = ""
    if os.path.exists(serial):
        output = open(serial, "rb").read().decode("utf-8", errors="replace")
    print(f"--- {name} (rc={rc}) ---")
    if timed_out:
        print("FAIL: timeout")
        return rc, output, True
    if rc not in ok:
        print(f"FAIL: unexpected exit code {rc}")
    return rc, output, False


def boot_riscv(disk: str, serial: str) -> tuple[int, str, bool]:
    kernel = os.path.join(ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64")
    cmd = [QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
           "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
           "-global", "virtio-mmio.force-legacy=false", "-drive", f"if=none,id=vd0,file={disk},format=raw",
           "-device", "virtio-blk-device,drive=vd0", "-bios", "default", "-kernel", kernel]
    return run_qemu("riscv64", cmd, serial, 30, {0})


def boot_aarch(disk: str, serial: str) -> tuple[int, str, bool]:
    kernel = os.path.join(ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64")
    cmd = [QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting", "-cpu", "max",
           "-smp", "2", "-m", "128M", "-display", "none", "-monitor", "none",
           "-serial", f"file:{serial}", "-global", "virtio-mmio.force-legacy=false",
           "-drive", f"if=none,id=vd0,file={disk},format=raw",
           "-device", "virtio-blk-device,drive=vd0", "-kernel", kernel]
    return run_qemu("aarch64", cmd, serial, 30, {0})


def prepare_x86(tmp: str) -> tuple[str, str, str]:
    efi = os.path.join(ROOT, "target", "x86_64-unknown-uefi", "release", "vifast-kernel-x86_64-uefi.efi")
    code = find_ovmf_code()
    vars_src = find_ovmf_vars()
    require(code is not None, "missing OVMF code")
    require(vars_src is not None, "missing OVMF vars")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(efi, os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"))
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(vars_src, vars_path)
    return code, esp, vars_path


def boot_x86(disk: str, serial: str, code: str, esp: str, vars_path: str) -> tuple[int, str, bool]:
    cmd = [QEMU_X86, "-machine", "q35", "-cpu", "max", "-smp", "2", "-m", "256M",
           "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
           "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
           "-drive", f"if=pflash,format=raw,file={vars_path}",
           "-drive", f"format=raw,file=fat:rw:{esp}", "-device", "isa-ide",
           "-drive", f"id=vd1,if=none,format=raw,file={disk}",
           "-device", "ide-hd,drive=vd1,bus=ide.6,unit=0",
           "-device", "isa-debug-exit,iobase=0x501,iosize=0x03"]
    return run_qemu("x86_64", cmd, serial, 45, {3})


def run_phases(name: str, tmp: str, boot) -> None:
    disk = os.path.join(tmp, "disk.img")
    serial = os.path.join(tmp, "serial.txt")
    make_disk(disk)

    # 1. Blank disk: guest format, no persistence, host fsck.
    rc, output, timeout = boot(disk, serial)
    require(not timeout, f"{name}: blank timeout")
    require("SELFTEST PASS" in output and "FS DISK OK" in output, f"{name}: blank boot")
    require("FS FORMAT OK" in output, f"{name}: blank missing FS FORMAT OK")
    require("FS PERSIST OK" not in output, f"{name}: blank claimed persistence")
    require("FS PERSIST DATA" not in output, f"{name}: blank printed data")
    fsutil("fsck", disk)
    print(f"{name}: phase 1 blank format PASS")

    # 2. Host put, guest reads/reverses, host reads new value.
    nonce = secrets.token_hex(8)[:15].encode("ascii")
    fsutil("put", disk, "/p/nonce", nonce.decode())
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS PERSIST OK" in output, f"{name}: persist boot")
    require(b"FS PERSIST DATA " + nonce + b"\n" in output.encode(), f"{name}: nonce read")
    expected = nonce[::-1]
    got = fsutil("get", disk, "/p/nonce").stdout
    require(got == expected, f"{name}: reverse persistence {got!r} != {expected!r}")
    fsutil("fsck", disk)
    print(f"{name}: phase 2 two-way persistence PASS")

    # 3. Legacy VF01 at LBA0 migrates to /scratch and LBA0 is cleared.
    make_disk(disk)
    legacy = secrets.token_hex(6).encode("ascii")
    seed_legacy(disk, legacy)
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS MIGRATE OK" in output, f"{name}: migrate boot")
    require(fsutil("get", disk, "/scratch").stdout == legacy, f"{name}: migrated data")
    require(sector0(disk) == b"\0\0\0\0", f"{name}: legacy sector not cleared")
    fsutil("fsck", disk)
    print(f"{name}: phase 3 migration PASS")

    # 4. Valid journal replay.
    make_disk(disk)
    fsutil("mkfs", disk)
    fsutil("inject-journal", disk, "--valid")
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS RECOVER OK (replayed=1)" in output, f"{name}: valid replay boot")
    require(fsutil("get", disk, "/j").stdout == b"journal-new\n", f"{name}: replayed data")
    fsutil("fsck", disk)
    print(f"{name}: phase 4 valid journal replay PASS")

    # 5. Torn commit is ignored; old data survives.
    make_disk(disk)
    fsutil("mkfs", disk)
    fsutil("put", disk, "/j", "stable")
    fsutil("inject-journal", disk, "--torn")
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS RECOVER OK (replayed=0)" in output, f"{name}: torn replay boot")
    require(fsutil("get", disk, "/j").stdout == b"stable", f"{name}: torn changed data")
    fsutil("fsck", disk)
    print(f"{name}: phase 5 torn journal PASS")

    # 7. Large file through fsutil: indirect block mount/fsck + host readback.
    make_disk(disk)
    fsutil("mkfs", disk)
    large_path = os.path.join(tmp, "large.bin")
    large = bytes(((index * 31 + 7) & 0xFF) for index in range(20 * 1024))
    with open(large_path, "wb") as handle:
        handle.write(large)
    fsutil("put-file", disk, "/p/large", large_path)
    fsutil("link", disk, "/p/large", "/p/large2")
    fsutil("symlink", disk, "/p/large", "/p/large-sym")
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS LARGE OK" in output, f"{name}: large boot")
    require("FS FSCK OK" in output, f"{name}: large fsck")
    got = fsutil("get", disk, "/p/large").stdout
    require(got == large, f"{name}: large host readback")
    got2 = fsutil("get", disk, "/p/large2").stdout
    require(got2 == large, f"{name}: hard-link host readback")
    got3 = fsutil("get", disk, "/p/large-sym").stdout
    require(got3 == large, f"{name}: symlink host readback")
    link_target = fsutil("readlink", disk, "/p/large-sym").stdout
    require(link_target == b"/p/large", f"{name}: readlink host result")
    fsutil("fsck", disk)
    print(f"{name}: phase 7 large indirect file PASS")

    # 6. Corrupt superblock forces a new format, never persistence.
    make_disk(disk)
    fsutil("mkfs", disk)
    with open(disk, "r+b") as image:
        image.seek(SECTOR)
        image.write(b"BAD!")
    rc, output, timeout = boot(disk, serial)
    require(not timeout and "FS FORMAT OK" in output, f"{name}: corrupt boot")
    require("FS PERSIST OK" not in output, f"{name}: corrupt claimed persistence")
    fsutil("fsck", disk)
    print(f"{name}: phase 6 corrupt superblock PASS")


def test_riscv() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-riscv-")
    try:
        run_phases("riscv64", tmp, boot_riscv)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_aarch() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-aarch-")
    try:
        run_phases("aarch64", tmp, boot_aarch)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_x86() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-x86-")
    try:
        code, esp, vars_path = prepare_x86(tmp)
        run_phases("x86_64", tmp, lambda disk, serial: boot_x86(disk, serial, code, esp, vars_path))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    if not all(shutil.which(q) for q in (QEMU_RV, QEMU_ARM, QEMU_X86)):
        print("missing QEMU binaries")
        return 2
    if not os.path.exists(FSUTIL):
        print(f"missing {FSUTIL}; run ./scripts/build.sh")
        return 2
    ok = True
    for name, test in (("riscv64", test_riscv), ("aarch64", test_aarch), ("x86_64", test_x86)):
        try:
            test()
            print(f"{name}: PASS")
        except Exception as error:
            print(f"{name}: FAIL: {error}")
            ok = False
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
