#!/usr/bin/env python3
"""Hermetic two-boot persistence test for the user-space VFS server.

Phase 1 uses a blank disk and must not report persisted data.
Phase 2 seeds a unique nonce in sector 0 and must report that exact nonce.
A negative phase uses a wrong magic and must not report persistence.

The disks live in a temporary directory and are deleted at the end.
RISC-V is intentionally skipped until its block backend exists.
"""

from __future__ import annotations

import glob
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QEMU_RV = "qemu-system-riscv64"
QEMU_ARM = "qemu-system-aarch64"
QEMU_X86 = "qemu-system-x86_64"
SECTOR = 512
WRITE_TEXT = b"ViFastOS write\n"


def find_ovmf_code() -> str | None:
    candidates = [
        os.environ.get("OVMF_CODE"),
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-x86_64-code.fd",
        "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/qemu/OVMF_CODE.fd",
    ]
    candidates += sorted(
        glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd")
    )
    for path in candidates:
        if path and os.path.exists(path):
            return path
    return None


def find_ovmf_vars() -> str | None:
    candidates = [
        os.environ.get("OVMF_VARS"),
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-i386-vars.fd",
        "/opt/homebrew/share/qemu/edk2-i386-vars.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ]
    candidates += sorted(
        glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-i386-vars.fd")
    )
    for path in candidates:
        if path and os.path.exists(path):
            return path
    return None


def run_qemu(
    name: str, cmd: list[str], serial: str, timeout: int, ok_codes: set[int]
) -> tuple[int, str, bool]:
    if os.path.exists(serial):
        os.unlink(serial)
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    started = time.time()
    timed_out = False
    try:
        rc = proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        rc = proc.wait()
    elapsed = time.time() - started
    output = ""
    if os.path.exists(serial):
        output = open(serial, "rb").read().decode("utf-8", errors="replace")
    print(f"--- {name} (rc={rc}, {elapsed:.1f}s) ---")
    if timed_out:
        print("FAIL: timeout")
    elif rc not in ok_codes:
        print(f"FAIL: unexpected QEMU exit code {rc}")
    return rc, output, timed_out


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def check_phase1_disk(disk: str) -> None:
    with open(disk, "rb") as image:
        sector = image.read(SECTOR)
    require(len(sector) == SECTOR, "short disk image")
    require(sector[:4] == b"VF01", "phase 1 did not write VF01")
    length = int.from_bytes(sector[4:8], "little")
    require(length == len(WRITE_TEXT), "unexpected phase 1 scratch length")
    require(sector[8 : 8 + length] == WRITE_TEXT, "phase 1 scratch payload differs")


def seed_sector(disk: str, magic: bytes, payload: bytes) -> None:
    require(len(magic) == 4, "magic must be four bytes")
    require(len(payload) <= 64, "payload too large")
    sector = bytearray(SECTOR)
    sector[:4] = magic
    sector[4:8] = len(payload).to_bytes(4, "little")
    sector[8 : 8 + len(payload)] = payload
    with open(disk, "r+b") as image:
        image.seek(0)
        image.write(sector)


def riscv64_boot(disk: str, serial: str, timeout: int = 20) -> tuple[int, str, bool]:
    kernel = os.path.join(
        ROOT,
        "target",
        "riscv64gc-unknown-none-elf",
        "release",
        "vifast-kernel-riscv64",
    )
    require(os.path.exists(kernel), f"missing kernel: {kernel}")
    cmd = [
        QEMU_RV,
        "-machine",
        "virt",
        "-smp",
        "2",
        "-m",
        "128M",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        f"file:{serial}",
        "-global",
        "virtio-mmio.force-legacy=false",
        "-drive",
        f"if=none,id=vd0,file={disk},format=raw",
        "-device",
        "virtio-blk-device,drive=vd0",
        "-bios",
        "default",
        "-kernel",
        kernel,
    ]
    return run_qemu("riscv64", cmd, serial, timeout, {0})


def aarch64_boot(disk: str, serial: str, timeout: int = 20) -> tuple[int, str, bool]:
    kernel = os.path.join(
        ROOT,
        "target",
        "aarch64-unknown-none",
        "release",
        "vifast-kernel-aarch64",
    )
    require(os.path.exists(kernel), f"missing kernel: {kernel}")
    cmd = [
        QEMU_ARM,
        "-machine",
        "virt,gic-version=2",
        "-cpu",
        "max",
        "-smp",
        "2",
        "-m",
        "128M",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        f"file:{serial}",
        "-global",
        "virtio-mmio.force-legacy=false",
        "-drive",
        f"if=none,id=vd0,file={disk},format=raw",
        "-device",
        "virtio-blk-device,drive=vd0",
        "-kernel",
        kernel,
    ]
    return run_qemu("aarch64", cmd, serial, timeout, {0})


def prepare_x86(tmp: str) -> tuple[str, str, str]:
    efi = os.path.join(
        ROOT,
        "target",
        "x86_64-unknown-uefi",
        "release",
        "vifast-kernel-x86_64-uefi.efi",
    )
    require(os.path.exists(efi), f"missing EFI binary: {efi}")
    code = find_ovmf_code()
    require(code is not None, "missing OVMF code (set OVMF_CODE)")
    vars_src = find_ovmf_vars()
    require(vars_src is not None, "missing OVMF vars (set OVMF_VARS)")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(efi, os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"))
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(vars_src, vars_path)
    return code, esp, vars_path


def x86_boot(
    disk: str, serial: str, code: str, esp: str, vars_path: str, timeout: int = 40
) -> tuple[int, str, bool]:
    cmd = [
        QEMU_X86,
        "-machine",
        "q35",
        "-cpu",
        "max",
        "-smp",
        "2",
        "-m",
        "256M",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        f"file:{serial}",
        "-drive",
        f"if=pflash,format=raw,readonly=on,file={code}",
        "-drive",
        f"if=pflash,format=raw,file={vars_path}",
        "-drive",
        f"format=raw,file=fat:rw:{esp}",
        "-device",
        "isa-ide",
        "-drive",
        f"id=vd1,if=none,format=raw,file={disk}",
        "-device",
        "ide-hd,drive=vd1,bus=ide.6,unit=0",
        "-device",
        "isa-debug-exit,iobase=0x501,iosize=0x03",
    ]
    return run_qemu("x86_64", cmd, serial, timeout, {3})


def run_phases(name: str, tmp: str, boot, expected_code: int) -> None:
    disk = os.path.join(tmp, "disk.img")
    with open(disk, "wb") as image:
        image.truncate(4 * 1024 * 1024)
    serial = os.path.join(tmp, "serial.txt")

    rc, output, timed_out = boot(disk, serial)
    require(not timed_out, f"{name}: phase 1 timed out")
    require(rc == expected_code, f"{name}: phase 1 exit code {rc}")
    require("SELFTEST PASS" in output, f"{name}: phase 1 did not boot")
    require("FS DISK OK" in output, f"{name}: phase 1 missing FS DISK OK")
    require("FS PERSIST OK" not in output, f"{name}: blank disk claimed persistence")
    require("FS PERSIST DATA" not in output, f"{name}: blank disk printed data")
    check_phase1_disk(disk)
    print(f"{name}: phase 1 blank disk PASS")

    nonce = secrets.token_hex(8).encode("ascii")
    seed_sector(disk, b"VF01", nonce)
    rc, output, timed_out = boot(disk, serial)
    require(not timed_out, f"{name}: phase 2 timed out")
    require(rc == expected_code, f"{name}: phase 2 exit code {rc}")
    require("FS DISK OK" in output, f"{name}: phase 2 missing FS DISK OK")
    require("FS PERSIST OK" in output, f"{name}: phase 2 missing FS PERSIST OK")
    expected = b"FS PERSIST DATA " + nonce + b"\n"
    require(expected.decode("ascii") in output, f"{name}: nonce was not read back")
    print(f"{name}: phase 2 nonce {nonce.decode()} PASS")

    seed_sector(disk, b"BAD!", nonce)
    rc, output, timed_out = boot(disk, serial)
    require(not timed_out, f"{name}: negative phase timed out")
    require(rc == expected_code, f"{name}: negative phase exit code {rc}")
    require("FS DISK OK" in output, f"{name}: negative phase missing FS DISK OK")
    require("FS PERSIST OK" not in output, f"{name}: wrong magic claimed persistence")
    require("FS PERSIST DATA" not in output, f"{name}: wrong magic printed data")
    print(f"{name}: negative wrong-magic PASS")


def test_riscv64() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-riscv64-")
    try:
        run_phases("riscv64", tmp, riscv64_boot, 0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_aarch64() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-aarch64-")
    try:
        run_phases("aarch64", tmp, aarch64_boot, 0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_x86_64() -> None:
    tmp = tempfile.mkdtemp(prefix="vifast-persist-x86-")
    try:
        code, esp, vars_path = prepare_x86(tmp)
        run_phases(
            "x86_64",
            tmp,
            lambda disk, serial: x86_boot(disk, serial, code, esp, vars_path),
            3,
        )
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    if not all(shutil.which(q) for q in (QEMU_RV, QEMU_ARM, QEMU_X86)):
        print("missing QEMU binaries")
        return 2
    ok = True
    for name, test in (
        ("riscv64", test_riscv64),
        ("aarch64", test_aarch64),
        ("x86_64", test_x86_64),
    ):
        try:
            test()
            print(f"{name}: PASS")
        except Exception as error:
            print(f"{name}: FAIL: {error}")
            ok = False
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
