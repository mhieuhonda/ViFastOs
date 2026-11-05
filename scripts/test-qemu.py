#!/usr/bin/env python3
"""Boot each architecture port under QEMU and check the portable self-tests.

This is intentionally a small, dependency-free harness so it also works on
macOS and in CI.  Each QEMU process is killed after a timeout; success is
defined by the kernel printing its banner and SELFTEST PASS.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import glob

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
    return (
        ticks > 0
        and wakeups >= timer_wakes + sgi_wakes
        and entries >= ticks - 2
        and timer_wakes >= ticks - 2
    )


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
    ok = (
        "SELFTEST PASS" in output
        and "booting on" in output
        and "BOOT UNTYPED" in output
        and "PREEMPT OK" in output
        and root_switch_count(output) > 1
        and "Isolation" in output
        and "Server" in output
        and "Client" in output
        and "Mapped" in output
        and "Cap OK" in output
        and "ViFastOS ramfs" in output
        and "ViFastOS write" in output
        and "IPC BUF OK" in output
        and "Reply OK" in output
        and output.count("VFS SERVER OK") == 1
        and output.count("VFS OK") == 1
        and output.count("VFS STAT OK") == 1
        and output.count("IOMMU OK") == 1
        and output.count("PERCPU CLOBBER OK") == 1
        and "PERCPU CLOBBER BAD" not in output
        and "VFS BAD" not in output
        and len(faults) == 4
        and 0x8000_0000 in faults
        and 0x1000002000 in faults
        and 0x1000301000 in faults
        and output.count("ROOT RUST") == 1
        and output.count("BOOTINFO OK") == 1
        and output.count("RUST MODULES OK") == 1
        and output.count("CLIENT VSPACE OK") == 1
        and output.count("FAULT EP OK") == 1
        and output.count("ROOT READY") == 1
        and "ROOT PANIC" not in output
        and "STALL" not in output
        and root_rust_at >= 0
        and server_at > root_rust_at
        and client_at > root_rust_at
        and "Zero OK" in output
        and (name != "x86_64" or "KERNEL DATA NX OK" in output)
        and (not require_smp or ("SMP OK" in output and "ipi=1" in output))
        and (not require_block or "BLOCK OK" in output)
        and (not require_disk or "FS DISK OK" in output)
        and (
            name != "aarch64"
            or (
                output.count("SMP TIMER OK") == 1
                and output.count("SMP IDLE OK") == 1
                and aarch64_idle_ok(output)
            )
        )
        and (
            name != "riscv64"
            or (
                output.count("SMP IDLE OK") == 1
                and hart1_wakeups(output) > 0
                and output.count("SMP SCHED OK") == 1
                and smp_sched(output)[0] > 1
                and smp_sched(output)[1] >= 100
                and smp_sched(output)[3] > 0
                and smp_sched(output)[2] <= smp_sched(output)[4]
                and output.count("SMP WORKERS OK (a=50, b=50)") == 1
            )
        )
    )
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


def test_riscv(index: int = 0) -> bool:
    kernel = os.path.join(
        ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
    )
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
        {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
    )


def test_aarch64(index: int = 0) -> bool:
    kernel = os.path.join(
        ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64"
    )
    serial = os.path.join(tempfile.gettempdir(), "vifast-aarch64.serial")
    disk = make_disk("aarch64")
    monitor = make_monitor("aarch64", index)
    cmd = [
        QEMU_ARM,
        "-machine", "virt,gic-version=2",
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
        {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
    )


def test_x86_64(index: int = 0) -> bool:
    code = find_ovmf_code()
    if not code:
        print("SKIP x86_64: OVMF firmware not found (set OVMF_CODE)")
        return False
    efi = os.path.join(
        ROOT,
        "target",
        "x86_64-unknown-uefi",
        "release",
        "vifast-kernel-x86_64-uefi.efi",
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
        {3},
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
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
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be at least 1")
    if not all(shutil.which(q) for q in (QEMU_RV, QEMU_ARM, QEMU_X86)):
        print("missing QEMU binaries")
        return 2
    tests = {
        "riscv64": test_riscv,
        "aarch64": test_aarch64,
        "x86_64": test_x86_64,
    }
    names = [args.arch] if args.arch else ["riscv64", "aarch64", "x86_64"]
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
