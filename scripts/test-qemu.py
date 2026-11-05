#!/usr/bin/env python3
"""Boot each architecture port under QEMU and check the portable self-tests.

This is intentionally a small, dependency-free harness so it also works on
macOS and in CI.  Each QEMU process is killed after a timeout; success is
defined by the kernel printing its banner and SELFTEST PASS.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import glob

from harness_spec import accept

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QEMU_RV = "qemu-system-riscv64"
QEMU_ARM = "qemu-system-aarch64"
QEMU_X86 = "qemu-system-x86_64"


def find_ovmf_code() -> str | None:
    candidates = [
        os.environ.get("OVMF_CODE"),
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-x86_64-code.fd",
        "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/qemu/OVMF_CODE.fd",
    ]
    candidates += sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd"))
    for path in candidates:
        if path and os.path.exists(path):
            return path
    return None


DISK_DIRS: list[str] = []
ARTIFACT_ROOT = os.path.join(ROOT, "artifacts")


def make_disk(name: str) -> str:
    """Create a fresh disk in a unique per-run directory."""
    directory = tempfile.mkdtemp(prefix=f"vifast-{name}-disk-")
    DISK_DIRS.append(directory)
    path = os.path.join(directory, "disk.img")
    with open(path, "wb") as disk:
        disk.truncate(4 * 1024 * 1024)
    return path


def cleanup_disks() -> None:
    for directory in DISK_DIRS:
        shutil.rmtree(directory, ignore_errors=True)
    DISK_DIRS.clear()


def make_monitor(name: str, index: int) -> str:
    directory = tempfile.mkdtemp(prefix=f"vifast-{name}-mon-")
    DISK_DIRS.append(directory)
    return os.path.join(directory, "mon.sock")


def dump_timeout_artifacts(
    name: str, index: int, monitor: str, output: str
) -> None:
    """Collect frozen-machine diagnostics before QEMU is killed."""
    directory = os.path.join(ARTIFACT_ROOT, f"flake-{name}-{index}")
    os.makedirs(directory, exist_ok=True)
    registers = ""
    if os.path.exists(monitor):
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(2.0)
                sock.connect(monitor)
                # Drain the human monitor banner and its first prompt.
                banner = b""
                while b"(qemu)" not in banner:
                    try:
                        chunk = sock.recv(65536)
                    except socket.timeout:
                        break
                    if not chunk:
                        break
                    banner += chunk
                sock.sendall(b"info registers -a\n")
                chunks: list[bytes] = []
                while True:
                    try:
                        chunk = sock.recv(65536)
                    except socket.timeout:
                        break
                    if not chunk:
                        break
                    chunks.append(chunk)
                    if b"(qemu)" in chunk:
                        break
                registers = b"".join(chunks).decode("utf-8", errors="replace")
        except OSError as error:
            registers = f"monitor unavailable: {error}\n"
    with open(os.path.join(directory, "registers.txt"), "w") as handle:
        handle.write(registers)
    tail = "\n".join(output.splitlines()[-80:])
    with open(os.path.join(directory, "tail.txt"), "w") as handle:
        handle.write(tail + "\n")
    important = [
        line
        for line in output.splitlines()
        if re.search(r"STALL|PANIC|SMP |USER FAULT|KERNEL EXCEPTION|BKL", line)
    ]
    with open(os.path.join(directory, "status.txt"), "w") as handle:
        handle.write("\n".join(important[-40:]) + "\n")
    print(f"artifacts: {directory}")


def smp_idle(output: str) -> tuple[int, int, int, int]:
    match = re.search(
        r"SMP IDLE OK \(hart1_wakeups=(\d+), idle_entries=(\d+), "
        r"wake_timer=(\d+), wake_sgi=(\d+)\)",
        output,
    )
    if not match:
        return (-1, -1, -1, -1)
    return tuple(int(match.group(i)) for i in range(1, 5))


def hart1_wakeups(output: str) -> int:
    return smp_idle(output)[0]


def aarch64_idle_ok(output: str) -> bool:
    idle = smp_idle(output)
    timer = re.search(r"SMP TIMER OK \(hart1_ticks=(\d+)\)", output)
    if not timer or min(idle) < 0:
        return False
    ticks = int(timer.group(1))
    wakeups, entries, timer_wakes, sgi_wakes = idle
    # Before workers exist, every tick forces one masked WFI (I and Wt track
    # T).  Once CPU1 runs workers continuously, ticks arrive as user IRQs
    # instead, so only positive activity remains meaningful.
    return ticks > 0 and wakeups > 0 and entries >= 1


def smp_sched(output: str) -> tuple[int, int, int, int, int]:
    match = re.search(
        r"SMP SCHED OK \(hart0=(\d+), hart1=(\d+), resched_sent=(\d+), "
        r"resched_wake=(\d+), idle_entries=(\d+)\)",
        output,
    )
    if not match:
        return (-1, -1, -1, -1, -1)
    return tuple(int(match.group(i)) for i in range(1, 6))


def root_switch_count(output: str) -> int:
    match = re.search(r"ROOT SWITCH\s+(\d+)", output)
    return int(match.group(1)) if match else -1


def fault_addresses(output: str) -> list[int]:
    return [int(value, 16) for value in re.findall(r"USER FAULT.*?addr=(0x[0-9a-fA-F]+)", output)]


def run_qemu(
    name: str,
    cmd: list[str],
    serial: str,
    timeout: int,
    ok_codes: set[int],
    require_smp: bool = False,
    require_block: bool = False,
    require_disk: bool = False,
    monitor: str | None = None,
    index: int = 0,
    feature_mode: str | None = None,
) -> bool:
    if os.path.exists(serial):
        os.unlink(serial)
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    started = time.time()
    timed_out = False
    try:
        rc = proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        output_at_timeout = ""
        if os.path.exists(serial):
            output_at_timeout = open(serial, "rb").read().decode("utf-8", errors="replace")
        if monitor:
            dump_timeout_artifacts(name, index, monitor, output_at_timeout)
        proc.kill()
        rc = proc.wait()
    elapsed = time.time() - started
    output = ""
    if os.path.exists(serial):
        output = open(serial, "rb").read().decode("utf-8", errors="replace")
    print(f"\n--- {name} (rc={rc}, {elapsed:.1f}s) ---")
    print(output[-1800:])
    faults = fault_addresses(output)
    root_rust_at = output.find("ROOT RUST")
    server_at = output.find("Server")
    client_at = output.find("Client")
    if feature_mode == "test-skip-tlb-ipi":
        bad = "TLB REMAP BAD (stale)" in output
        good = "TLB REMAP OK" in output
        if not bad or good:
            if timed_out:
                print("FAIL: mutation timeout without TLB REMAP BAD")
            else:
                print("FAIL: mutation was not detected")
            return False
        print("PASS (expected TLB failure detected)")
        return True
    ok = accept(name, output, rc, require_block, require_disk, feature_mode)
    if timed_out:
        print("FAIL: timeout")
        return False
    if rc not in ok_codes:
        print(f"FAIL: unexpected QEMU exit code {rc}")
        return False
    if not ok:
        print("FAIL: banner or SELFTEST PASS missing")
        return False
    print("PASS")
    return True


def test_riscv(index: int = 0, feature: str | None = None) -> bool:
    if feature == "test-boot-handoff":
        kernel_name = "vifast-kernel-riscv64-handoff"
    elif feature == "test-panic-hart1":
        kernel_name = "vifast-kernel-riscv64-panic"
    elif feature == "test-skip-tlb-ipi":
        kernel_name = "vifast-kernel-riscv64-skip-tlb"
    else:
        kernel_name = "vifast-kernel-riscv64"
    kernel = os.path.join(
        ROOT, "target", "riscv64gc-unknown-none-elf", "release", kernel_name
    )
    if not os.path.exists(kernel):
        print(f"missing kernel: {kernel}; run ./scripts/build.sh first")
        return False
    serial = os.path.join(tempfile.gettempdir(), "vifast-riscv64.serial")
    disk = make_disk("riscv64")
    monitor = make_monitor("riscv64", index)
    cmd = [
        QEMU_RV,
        "-machine", "virt",
        "-smp", "2",
        "-m", "128M",
        "-display", "none",
        "-monitor", f"unix:{monitor},server,nowait",
        "-serial", f"file:{serial}",
        "-global", "virtio-mmio.force-legacy=false",
        "-drive", f"if=none,id=vd0,file={disk},format=raw",
        "-device", "virtio-blk-device,drive=vd0",
        "-bios", "default",
        "-kernel", kernel,
    ]
    return run_qemu(
        "riscv64",
        cmd,
        serial,
        20,
        {1} if feature == "test-panic-hart1" else {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
        feature_mode=feature,
    )


def test_aarch64(index: int = 0, feature: str | None = None) -> bool:
    name = "vifast-kernel-aarch64-panic" if feature == "test-panic-hart1" else "vifast-kernel-aarch64"
    kernel = os.path.join(ROOT, "target", "aarch64-unknown-none", "release", name)
    serial = os.path.join(tempfile.gettempdir(), "vifast-aarch64.serial")
    disk = make_disk("aarch64")
    monitor = make_monitor("aarch64", index)
    cmd = [
        QEMU_ARM,
        "-machine", "virt,gic-version=2",
        "-semihosting",
        "-cpu", "max",
        "-smp", "2",
        "-m", "128M",
        "-display", "none",
        "-monitor", f"unix:{monitor},server,nowait",
        "-serial", f"file:{serial}",
        "-global", "virtio-mmio.force-legacy=false",
        "-drive", f"if=none,id=vd0,file={disk},format=raw",
        "-device", "virtio-blk-device,drive=vd0",
        "-kernel", kernel,
    ]
    return run_qemu(
        "aarch64",
        cmd,
        serial,
        20,
        {1} if feature == "test-panic-hart1" else {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
        feature_mode=feature,
    )


def test_x86_64(index: int = 0, feature: str | None = None) -> bool:
    code = find_ovmf_code()
    if not code:
        print("SKIP x86_64: OVMF firmware not found (set OVMF_CODE)")
        return False
    efi = os.path.join(
        ROOT,
        "target",
        "x86_64-unknown-uefi",
        "release",
        "vifast-kernel-x86_64-uefi-panic.efi"
        if feature == "test-panic-hart1"
        else (
            "vifast-kernel-x86_64-uefi-skip-tlb.efi"
            if feature == "test-skip-tlb-ipi"
            else "vifast-kernel-x86_64-uefi.efi"
        ),
    )
    tmp = tempfile.mkdtemp(prefix="vifast-uefi-")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(efi, os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"))
    vars_src = None
    for candidate in [
        "/opt/homebrew/Cellar/qemu/11.1.3/share/qemu/edk2-i386-vars.fd",
        "/opt/homebrew/share/qemu/edk2-i386-vars.fd",
        "/usr/share/OVMF/OVMF_VARS.fd",
    ] + sorted(glob.glob("/opt/homebrew/Cellar/qemu/*/share/qemu/edk2-i386-vars.fd")):
        if os.path.exists(candidate):
            vars_src = candidate
            break
    if not vars_src:
        print("SKIP x86_64: OVMF vars not found")
        return False
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(vars_src, vars_path)
    serial = os.path.join(tmp, "serial.txt")
    disk = make_disk("x86_64")
    monitor = make_monitor("x86_64", index)
    cmd = [
        QEMU_X86,
        "-machine", "q35",
        "-cpu", "max",
        "-smp", "2",
        "-m", "256M",
        "-display", "none",
        "-monitor", f"unix:{monitor},server,nowait",
        "-serial", f"file:{serial}",
        "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
        "-drive", f"if=pflash,format=raw,file={vars_path}",
        "-drive", f"format=raw,file=fat:rw:{esp}",
        "-device", "isa-ide",
        "-drive", f"id=vd1,if=none,format=raw,file={disk}",
        "-device", "ide-hd,drive=vd1,bus=ide.6,unit=0",
        "-device", "isa-debug-exit,iobase=0x501,iosize=0x03",
    ]
    # The kernel writes 0x501 with value 1, which QEMU turns into exit code 3.
    return run_qemu(
        "x86_64",
        cmd,
        serial,
        40,
        {7} if feature == "test-panic-hart1" else {3},
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
        feature_mode=feature,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--arch",
        choices=("riscv64", "aarch64", "x86_64"),
        help="run only one architecture (default: all three)",
    )
    parser.add_argument(
        "--repeat",
        type=int,
        default=1,
        help="run the selected cases N times and print the PASS ratio",
    )
    parser.add_argument(
        "--feature",
        choices=("test-boot-handoff", "test-panic-hart1", "test-skip-tlb-ipi"),
        help="run a test-only feature build",
    )
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be at least 1")
    if args.feature == "test-boot-handoff" and args.arch not in (None, "riscv64"):
        parser.error("--feature test-boot-handoff only applies to riscv64")
    if args.feature == "test-skip-tlb-ipi" and args.arch == "aarch64":
        parser.error("--feature test-skip-tlb-ipi only applies to riscv64/x86_64")
    if not all(shutil.which(q) for q in (QEMU_RV, QEMU_ARM, QEMU_X86)):
        print("missing QEMU binaries")
        return 2
    tests = {
        "riscv64": lambda index: test_riscv(index, args.feature),
        "aarch64": lambda index: test_aarch64(index, args.feature),
        "x86_64": lambda index: test_x86_64(index, args.feature),
    }
    if args.arch:
        names = [args.arch]
    elif args.feature == "test-skip-tlb-ipi":
        names = ["riscv64", "x86_64"]
    else:
        names = ["riscv64", "aarch64", "x86_64"]
    results: dict[str, list[bool]] = {name: [] for name in names}
    try:
        for index in range(1, args.repeat + 1):
            for name in names:
                if args.repeat > 1:
                    print(f"\n===== {name} run {index}/{args.repeat} =====")
                results[name].append(tests[name](index))
    finally:
        cleanup_disks()
    print("\n=== summary ===")
    payload = {
        "repeat": args.repeat,
        "results": {
            name: [sum(results[name]), len(results[name])] for name in names
        },
    }
    result_path = os.path.join(ROOT, "target", "vifast-qemu-last.json")
    os.makedirs(os.path.dirname(result_path), exist_ok=True)
    with open(result_path, "w") as handle:
        json.dump(payload, handle, indent=2)
    failed = False
    for name in names:
        runs = results[name]
        passed = sum(runs)
        print(f"{name:8} {passed}/{len(runs)} PASS")
        if passed != len(runs):
            failed = True
    return 1 if failed else 0

if __name__ == "__main__":
    sys.exit(main())
