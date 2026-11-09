#!/usr/bin/env python3
"""r259 (T26): đếm flake `FS V2 OK` trên x86_64 với -smp 1 để tách SMP.

Không kiểm marker của harness: chỉ tìm `FS V2 OK` / `FS V2 BAD`.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.environ.get("VF_ROOT", "/Users/mps/dvchd/ViFastOs")
QEMU = shutil.which("qemu-system-x86_64")


def find_ovmf_code():
    for candidate in [
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-x86_64-code.fd",
        "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
    ]:
        if os.path.exists(candidate):
            return candidate
    import glob

    for candidate in sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd")):
        return candidate
    return None


def find_ovmf_vars():
    for candidate in [
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-i386-vars.fd",
        "/opt/homebrew/share/qemu/edk2-i386-vars.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ]:
        if os.path.exists(candidate):
            return candidate
    import glob

    for candidate in sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-i386-vars.fd")):
        return candidate
    return None


def one(index, smp, timeout=80):
    tmp = tempfile.mkdtemp(prefix=f"vf-smp{smp}-")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(
        os.path.join(ROOT, "target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi"),
        os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"),
    )
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(find_ovmf_vars(), vars_path)
    serial = os.path.join(tmp, "serial.txt")
    disk = os.path.join(tmp, "disk.img")
    with open(disk, "wb") as handle:
        handle.truncate(4 * 1024 * 1024)
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "vifast_test_qemu", os.path.join(ROOT, "scripts", "test-qemu.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod.populate_disk(disk, "x86_64")
    cmd = [
        QEMU,
        "-machine", "q35",
        "-cpu", "max",
        "-smp", str(smp),
        "-m", "256M",
        "-display", "none",
        "-serial", f"file:{serial}",
        "-drive", f"if=pflash,format=raw,readonly=on,file={find_ovmf_code()}",
        "-drive", f"if=pflash,format=raw,file={vars_path}",
        "-drive", f"format=raw,file=fat:rw:{esp}",
        "-device", "isa-ide",
        "-drive", f"id=vd1,if=none,format=raw,file={disk}",
        "-device", "ide-hd,drive=vd1,bus=ide.6,unit=0",
        "-device", "isa-debug-exit,iobase=0x501,iosize=0x03",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            break
        time.sleep(0.2)
    if proc.poll() is None:
        proc.kill()
        proc.wait()
    text = open(serial, "rb").read().decode("utf-8", "replace") if os.path.exists(serial) else ""
    if os.environ.get("VIFAST_KEEP_DISK") == "1" and os.path.exists(disk):
        shutil.copyfile(disk, f"{tempfile.gettempdir()}/vifast-smp{smp}-{index}.img")
    if os.environ.get("VIFAST_KEEP_SERIAL") == "1":
        shutil.copyfile(serial, f"{tempfile.gettempdir()}/vifast-smp{smp}-{index}.serial")
    ok = "FS V2 OK" in text
    bad = "FS V2 BAD" in text
    lines = len(text.splitlines())
    return ok and not bad, bad, lines


def main():
    smp = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    passed = 0
    for index in range(1, count + 1):
        good, bad, lines = one(index, smp)
        passed += 1 if good else 0
        print(f"smp={smp} run {index}: {'OK' if good else ('BAD' if bad else 'MISS')} lines={lines}", flush=True)
    print(f"smp={smp} {passed}/{count} có FS V2 OK")
    return 0 if passed == count else 1


if __name__ == "__main__":
    sys.exit(main())
