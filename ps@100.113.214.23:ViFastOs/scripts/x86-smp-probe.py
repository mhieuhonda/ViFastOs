#!/usr/bin/env python3
"""Đo flake boot x86_64 (r259/r262): chạy N lần, đếm `FS V2 OK`, giữ serial/đĩa.

Dùng để tái hiện hồi quy `FS V2 OK` (Part 4 của báo cáo r262) và để kiểm lại
sau khi sửa. Cố ý KHÔNG kiểm marker của harness — chỉ trả lời một câu: lần boot
này có in `FS V2 OK` hay không, và nếu không thì in gì.

Ví dụ:
    python3 scripts/x86-smp-probe.py 2 12      # 12 lần, -smp 2 (mặc định gate)
    python3 scripts/x86-smp-probe.py 1 8       # 8 lần, -smp 1 (đối chứng)
    VIFAST_KEEP_SERIAL=1 VIFAST_KEEP_DISK=1 \
        python3 scripts/x86-smp-probe.py 2 12  # giữ serial + ảnh đĩa để soi

Biến môi trường:
    VF_ROOT            gốc cây nguồn (mặc định: thư mục cha của scripts/)
    VIFAST_KEEP_SERIAL giữ serial từng lần chạy trong thư mục tạm
    VIFAST_KEEP_DISK   giữ ảnh đĩa từng lần chạy trong thư mục tạm
"""
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.environ.get(
    "VF_ROOT", os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
)
QEMU = shutil.which("qemu-system-x86_64")


def load_harness():
    """Nạp `scripts/test-qemu.py` (tên có dấu gạch) để dùng lại hàm dựng đĩa."""
    path = os.path.join(ROOT, "scripts", "test-qemu.py")
    spec = importlib.util.spec_from_file_location("vifast_test_qemu", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def find_ovmf_vars():
    """Bản vars mẫu — cùng danh sách với `test-qemu.py` (edk2-i386-vars)."""
    import glob

    candidates = [
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-i386-vars.fd",
        "/opt/homebrew/share/qemu/edk2-i386-vars.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
        "/usr/share/qemu/OVMF_VARS.fd",
    ]
    candidates += sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-i386-vars.fd"))
    for path in candidates:
        if os.path.exists(path):
            return path
    return None


def one(harness, index, smp, timeout=80):
    tmp = tempfile.mkdtemp(prefix=f"vf-smp{smp}-")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(
        os.path.join(
            ROOT, "target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi"
        ),
        os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"),
    )
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(find_ovmf_vars(), vars_path)
    serial = os.path.join(tmp, "serial.txt")
    disk = os.path.join(tmp, "disk.img")
    with open(disk, "wb") as handle:
        handle.truncate(4 * 1024 * 1024)
    harness.populate_disk(disk, "x86_64")
    cmd = [
        QEMU,
        "-machine", "q35",
        "-cpu", "max",
        "-smp", str(smp),
        "-m", "256M",
        "-display", "none",
        "-serial", f"file:{serial}",
        "-drive", f"if=pflash,format=raw,readonly=on,file={harness.find_ovmf_code()}",
        "-drive", f"if=pflash,format=raw,file={vars_path}",
        "-drive", f"format=raw,file=fat:rw:{esp}",
        "-device", "isa-ide",
        "-drive", f"id=vd1,if=none,format=raw,file={disk}",
        "-device", "ide-hd,drive=vd1,bus=ide.6,unit=0",
        "-device", "isa-debug-exit,iobase=0x501,iosize=0x03",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.time() + timeout
    while time.time() < deadline and proc.poll() is None:
        time.sleep(0.2)
    if proc.poll() is None:
        proc.kill()
        proc.wait()
    text = ""
    if os.path.exists(serial):
        text = open(serial, "rb").read().decode("utf-8", "replace")
    name = tempfile.gettempdir()
    if os.environ.get("VIFAST_KEEP_DISK") == "1" and os.path.exists(disk):
        shutil.copyfile(disk, os.path.join(name, f"vifast-smp{smp}-{index}.img"))
    if os.environ.get("VIFAST_KEEP_SERIAL") == "1" and os.path.exists(serial):
        shutil.copyfile(serial, os.path.join(name, f"vifast-smp{smp}-{index}.serial"))
    ok = "FS V2 OK" in text
    bad = "FS V2 BAD" in text
    return ok and not bad, bad, len(text.splitlines())


def main():
    if not QEMU:
        print("missing qemu-system-x86_64")
        return 2
    if not find_ovmf_vars():
        print("missing OVMF vars")
        return 2
    harness = load_harness()
    if not harness.find_ovmf_code():
        print("missing OVMF code")
        return 2
    smp = int(sys.argv[1]) if len(sys.argv) > 1 else 2
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    passed = 0
    for index in range(1, count + 1):
        good, bad, lines = one(harness, index, smp)
        passed += 1 if good else 0
        verdict = "OK" if good else ("BAD" if bad else "MISS")
        print(f"smp={smp} run {index}: {verdict} lines={lines}", flush=True)
    print(f"smp={smp} {passed}/{count} có FS V2 OK")
    return 0 if passed == count else 1


if __name__ == "__main__":
    sys.exit(main())
