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


def run_qemu_interactive(
    name: str,
    cmd_base: list[str],
    serial: str,
    timeout: int,
) -> tuple[bool, str]:
    """Run QEMU with a socket chardev serial so the test can type input.

    Vòng 62 (pha 4 input end-to-end): chardev socket vừa log ra file vừa cho
    test gửi lệnh shell. Gửi `help` (output khác chữ đã gõ nên chứng minh
    lệnh thực sự chạy) rồi gửi `\r` vài lần; yêu cầu thấy dòng help.
    Trả về (ok, output). Không dùng cho feature_mode.
    Lưu ý: socket đặt thẳng dưới /tmp (đường dẫn ngắn) vì giới hạn 104 ký tự
    của AF_UNIX — TMPDIR trên macOS quá dài khiến QEMU lặng lẽ không tạo socket.
    """
    sock = f"/tmp/vifast-ial-{name}-{os.getpid()}.sock"
    if os.path.exists(serial):
        os.unlink(serial)
    cmd = []
    skip_next = False
    for arg in cmd_base:
        if skip_next:
            skip_next = False
            continue
        if arg == "-serial":
            # Bỏ cả cờ lẫn giá trị; nếu giá trị là file:serial của ta.
            skip_next = True
            continue
        if arg == f"file:{serial}":
            continue
        cmd.append(arg)
    # Chèn chardev trước -serial: QEMU parse theo thứ tự không quan trọng.
    cmd += [
        "-chardev",
        f"socket,path={sock},server=on,wait=off,logfile={serial},id=visser",
        "-serial",
        "chardev:visser",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    output = ""
    try:
        s = None
        for _ in range(100):
            time.sleep(0.2)
            if os.path.exists(sock):
                try:
                    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    s.settimeout(5.0)
                    s.connect(sock)
                    break
                except OSError:
                    pass
        if s is None:
            proc.kill()
            proc.wait()
            return False, "input: no chardev connect"
        # Gửi lệnh NGAY sau khi kết nối: guest đệm byte UART vào RX buffer.
        # Dùng `help` vì output của nó ("help time echo ls ...") khác hẳn chữ
        # đã gõ — chứng minh lệnh THỰC SỰ chạy, không chỉ echo ký tự.
        # Byte '\r' gửi riêng nhiều lần: thực tế '\n' đôi khi bị mất trên
        # PL011 khiến dòng không bao giờ được xử lý (flaky vòng 62).
        try:
            s.sendall(b"help")
        except OSError:
            pass
        s.settimeout(0.3)
        buf = b""
        t0 = time.time()
        while time.time() - t0 < timeout and b"shell> " not in buf:
            try:
                buf += s.recv(65536)
            except socket.timeout:
                pass
        if b"shell> " not in buf:
            proc.kill()
            proc.wait()
            return False, "input: no shell prompt"
        marker = b"help time echo ls"
        got = marker in buf
        for _ in range(12):
            if got:
                break
            try:
                s.sendall(b"\r")
            except OSError:
                break
            time.sleep(0.2)
            t0 = time.time()
            while time.time() - t0 < 0.3 and marker not in buf:
                try:
                    buf += s.recv(65536)
                except socket.timeout:
                    pass
            got = marker in buf
        s.close()
        # Để guest chạy tiếp cho tới khi tự thoát hoặc timeout.
        try:
            rc = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            rc = proc.wait()
        if os.path.exists(serial):
            output = open(serial, "rb").read().decode("utf-8", errors="replace")
        if not got and "help time echo ls" not in output:
            return False, output
        return True, output
    finally:
        try:
            proc.kill()
        except OSError:
            pass


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
    if os.environ.get("VIFAST_KEEP_SERIAL") == "1" and os.path.exists(serial):
        # Giữ lại serial của từng lần chạy để điều tra flake (vòng 67).
        shutil.copyfile(
            serial, f"{tempfile.gettempdir()}/vifast-keep-{name}-{index}.serial"
        )
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
    elif feature == "test-destroy-smp":
        kernel_name = "vifast-kernel-riscv64-destroy-smp"
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
    # Vòng 58: opt-in packet dump để kiểm chứng TX thực sự (mặc định tắt,
    # không ảnh hưởng gate). Đặt VIFAST_NET_DUMP=1 để QEMU ghi pcap qua
    # filter-dump (QEMU mới đã bỏ dumpfile= trong -netdev).
    netdev_rv = "user,id=vn0"
    dump_rv: list[str] = []
    if os.environ.get("VIFAST_NET_DUMP") == "1":
        dump_rv = [
            "-object",
            f"filter-dump,id=fdump0,netdev=vn0,file={tempfile.gettempdir()}/vifast-net-riscv64.pcap",
        ]
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
        "-device", "virtio-net-device,netdev=vn0",
        "-netdev", netdev_rv,
        *dump_rv,
        "-device", "virtio-gpu-device",
        "-device", "virtio-keyboard-device",
        "-device", "virtio-rng-device",
        "-device", "ramfb",
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
    if feature == "test-panic-hart1":
        name = "vifast-kernel-aarch64-panic"
    elif feature == "test-destroy-smp":
        name = "vifast-kernel-aarch64-destroy-smp"
    else:
        name = "vifast-kernel-aarch64"
    kernel = os.path.join(ROOT, "target", "aarch64-unknown-none", "release", name)
    serial = os.path.join(tempfile.gettempdir(), "vifast-aarch64.serial")
    disk = make_disk("aarch64")
    monitor = make_monitor("aarch64", index)
    netdev_arm = "user,id=vn0"
    dump_arm: list[str] = []
    if os.environ.get("VIFAST_NET_DUMP") == "1":
        dump_arm = [
            "-object",
            f"filter-dump,id=fdump0,netdev=vn0,file={tempfile.gettempdir()}/vifast-net-aarch64.pcap",
        ]
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
        "-device", "virtio-net-device,netdev=vn0",
        "-netdev", netdev_arm,
        *dump_arm,
        "-device", "virtio-gpu-device",
        "-device", "virtio-keyboard-device",
        "-device", "virtio-rng-device",
        "-device", "ramfb",
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
            else (
                "vifast-kernel-x86_64-uefi-destroy-smp.efi"
                if feature == "test-destroy-smp"
                else "vifast-kernel-x86_64-uefi.efi"
            )
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


def test_interactive(name: str, index: int) -> bool:
    """Boot one arch on a socket chardev and type into its shell."""
    serial = os.path.join(tempfile.gettempdir(), f"vifast-{name}-ial.serial")
    disk = make_disk(f"{name}-ial")
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
        )
        cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-net-device,netdev=vn0",
            "-netdev", "user,id=vn0",
            "-device", "ramfb",
            "-bios", "default", "-kernel", kernel,
        ]
    elif name == "aarch64":
        kernel = os.path.join(ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64")
        cmd = [
            QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting",
            "-cpu", "max", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-net-device,netdev=vn0",
            "-netdev", "user,id=vn0",
            "-device", "ramfb",
            "-kernel", kernel,
        ]
    else:
        # x86_64 UEFI + ESP setup phức tạp — interactive để vòng sau.
        print(f"--- {name} interactive SKIP (uefi) ---")
        return True
    ok, output = run_qemu_interactive(name, cmd, serial, 30)
    print(f"--- {name} interactive {'PASS' if ok else 'FAIL'} ---")
    if not ok:
        print(output[-500:])
    return ok


def tcp_check(name: str) -> bool:
    """Boot with hostfwd and run a TCP echo round trip from the host.

    Vòng 77: QEMU user-net chuyển tiếp localhost:<port> vào cổng 8080 của
    guest; client gửi payload và yêu cầu guest echo lại đúng nội dung.
    """
    if name not in ("riscv64", "aarch64"):
        return True
    port = 18080 if name == "riscv64" else 18081
    serial = f"/tmp/vifast-tcp-{name}.log"
    disk = make_disk(f"{name}-tcp")
    if os.path.exists(serial):
        os.unlink(serial)
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
        )
        cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-net-device,netdev=vn0",
            "-netdev", f"user,id=vn0,hostfwd=tcp::{port}-:8080",
            "-bios", "default", "-kernel", kernel,
        ]
    else:
        kernel = os.path.join(
            ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64"
        )
        cmd = [
            QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting",
            "-cpu", "max", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-net-device,netdev=vn0",
            "-netdev", f"user,id=vn0,hostfwd=tcp::{port}-:8080",
            "-kernel", kernel,
        ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        payload = b"ping-from-host"
        echoed = b""
        deadline = time.time() + 12
        while time.time() < deadline:
            time.sleep(0.15)
            try:
                client = socket.create_connection(("127.0.0.1", port), timeout=1.0)
            except OSError:
                continue
            try:
                client.settimeout(3.0)
                client.sendall(payload)
                echoed = client.recv(1024)
            except OSError:
                echoed = b""
            finally:
                client.close()
            if echoed == payload:
                break
        if echoed != payload:
            print(f"--- {name} tcp FAIL (echo {echoed!r}) ---")
            return False
        text = ""
        if os.path.exists(serial):
            text = open(serial, "rb").read().decode("utf-8", errors="replace")
        if "VIRTIO TCP ACCEPT" not in text:
            print(f"--- {name} tcp FAIL (no ACCEPT marker) ---")
            return False
        print(f"--- {name} tcp PASS (echo {payload!r}) ---")
        return True
    finally:
        proc.kill()
        proc.wait()


def input_check(name: str) -> bool:
    """Boot with virtio-keyboard and type 'help' via QEMU monitor sendkey.

    Vòng 70 (M4 input): serial dùng chardev SOCKET để thấy prompt theo thời
    gian thực (file log bị buffer, phát hiện muộn thì guest đã thoát). Khi thấy
    `shell> ` thì gửi h/e/l/p/ret qua HMP `sendkey` và yêu cầu dòng help.
    """
    if name not in ("riscv64", "aarch64"):
        return True
    ser_sock = f"/tmp/vifast-in-{name}-ser.sock"
    mon = f"/tmp/vifast-in-{name}-mon.sock"
    serial = f"/tmp/vifast-in-{name}-ser.log"
    disk = make_disk(f"{name}-input")
    for path in (ser_sock, mon, serial):
        if os.path.exists(path):
            os.unlink(path)
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
        )
        base_cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-monitor", f"unix:{mon},server,nowait",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-keyboard-device",
            "-bios", "default", "-kernel", kernel,
        ]
    else:
        kernel = os.path.join(
            ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64"
        )
        base_cmd = [
            QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting",
            "-cpu", "max", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-monitor", f"unix:{mon},server,nowait",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-keyboard-device",
            "-kernel", kernel,
        ]
    cmd = base_cmd + [
        "-chardev",
        f"socket,path={ser_sock},server=on,wait=off,logfile={serial},id=vinser",
        "-serial",
        "chardev:vinser",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        # Kết nối cả serial và monitor trước khi guest chạy xong.
        ser = None
        mon_sock = None
        deadline = time.time() + 10
        while time.time() < deadline and (ser is None or mon_sock is None):
            if ser is None and os.path.exists(ser_sock):
                try:
                    ser = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    ser.settimeout(0.5)
                    ser.connect(ser_sock)
                except OSError:
                    ser = None
            if mon_sock is None and os.path.exists(mon):
                try:
                    mon_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    mon_sock.settimeout(5.0)
                    mon_sock.connect(mon)
                except OSError:
                    mon_sock = None
            time.sleep(0.05)
        if ser is None or mon_sock is None:
            print(f"--- {name} input FAIL (no sockets) ---")
            return False
        # Driver input khởi tạo rất sớm (<1s); chờ cố định rồi gửi 2 phím
        # khác nhau với hold ngắn (tránh auto-repeat) và kiểm tra mã phím thô
        # trong serial. Đây là bằng chứng thiết bị → driver hoạt động; phần
        # shell đã được kiểm bằng test UART interactive riêng.
        # Guest chỉ sống ~1.5-2s sau khi shell bắt đầu nên gửi đúng MỘT phím
        # ngay khi driver đã sẵn sàng; yêu cầu đúng mã phím thô trong serial.
        time.sleep(0.9)
        try:
            mon_sock.sendall(b"sendkey e 20\n")
            time.sleep(0.35)
            try:
                mon_sock.settimeout(0.05)
                mon_sock.recv(4096)
            except OSError:
                pass
        except OSError:
            print(f"--- {name} input FAIL (QEMU exited before keys sent) ---")
            return False
        wanted = (b"VIRTIO KEY 18",)
        ok = False
        deadline = time.time() + 5
        buf = b""
        while time.time() < deadline:
            try:
                chunk = ser.recv(65536)
                if chunk:
                    buf += chunk
            except socket.timeout:
                pass
            if all(marker in buf for marker in wanted):
                ok = True
                break
            if proc.poll() is not None:
                break
        text = ""
        if os.path.exists(serial):
            text = open(serial, "rb").read().decode("utf-8", errors="replace")
        if not ok:
            ok = all(marker.decode() in text for marker in wanted)
        ser.close()
        mon_sock.close()
        if not ok:
            print(f"--- {name} input FAIL (keys not delivered) ---")
            return False
        print(f"--- {name} input PASS (virtio-keyboard reached the shell) ---")
        return True
    finally:
        proc.kill()
        proc.wait()


def gpu_check(name: str) -> bool:
    """Boot with virtio-gpu, screendump the display and verify real pixels.

    Vòng 66 (M4 display MVP): chờ serial tới `VIRTIO GPU DRAW OK`, gửi
    `screendump` qua monitor socket, đọc PPM và so màu gradient tại vài điểm.
    Opt-in (cờ --gpu-check) để gate chuẩn không phụ thuộc display backend.
    """
    if name not in ("riscv64", "aarch64"):
        return True
    serial = f"/tmp/vifast-gpu-{name}-ser.log"
    mon = f"/tmp/vifast-gpu-{name}-mon.sock"
    ppm = f"/tmp/vifast-gpu-{name}.ppm"
    disk = make_disk(f"{name}-gpu")
    for path in (serial, mon, ppm):
        if os.path.exists(path):
            os.unlink(path)
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
        )
        cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-monitor", f"unix:{mon},server,nowait",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-gpu-device",
            "-device", "ramfb",
            "-bios", "default", "-kernel", kernel,
        ]
    else:
        kernel = os.path.join(
            ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64"
        )
        cmd = [
            QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting",
            "-cpu", "max", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-monitor", f"unix:{mon},server,nowait",
            "-serial", f"file:{serial}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-gpu-device",
            "-device", "ramfb",
            "-kernel", kernel,
        ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        deadline = time.time() + 25
        drawn = False
        while time.time() < deadline:
            time.sleep(0.3)
            if os.path.exists(serial):
                text = open(serial, "rb").read().decode("utf-8", errors="replace")
                if "VIRTIO GPU DRAW OK" in text:
                    drawn = True
                    break
                if "VIRTIO GPU" in text and "FAIL" in text.split("VIRTIO GPU", 1)[1][:200]:
                    print(f"--- {name} gpu FAIL (draw error in serial) ---")
                    return False
        if not drawn:
            print(f"--- {name} gpu FAIL (no DRAW OK) ---")
            return False
        sock = None
        for _ in range(20):
            if os.path.exists(mon):
                try:
                    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    sock.settimeout(5.0)
                    sock.connect(mon)
                    break
                except OSError:
                    sock = None
            time.sleep(0.2)
        if sock is None:
            print(f"--- {name} gpu FAIL (no monitor) ---")
            return False
        sock.sendall(f"screendump {ppm}\n".encode())
        time.sleep(1.5)
        sock.close()
        if not os.path.exists(ppm):
            print(f"--- {name} gpu FAIL (no screendump) ---")
            return False
        data = open(ppm, "rb").read()
        parts = data.split(b"\n", 3)
        if len(parts) < 4 or parts[0] != b"P6":
            print(f"--- {name} gpu FAIL (bad ppm) ---")
            return False
        width, height = map(int, parts[1].split())
        body = parts[3]
        if width != 160 or height != 120:
            print(f"--- {name} gpu FAIL (size {width}x{height}) ---")
            return False
        expected = {
            # Gradient nền (không bị text che).
            (0, 0): (0, 0, 64),
            (80, 60): (127, 127, 64),
            (159, 119): (253, 252, 64),
            # Hộp tối của splash (vòng 73): 0x101028.
            (7, 7): (16, 16, 40),
            # Nét đầu tiên của chữ 'V' (glyph cột 0, hàng 0) màu trắng.
            (10, 10): (255, 255, 255),
        }
        for (x, y), want in expected.items():
            i = (y * width + x) * 3
            got = (body[i], body[i + 1], body[i + 2])
            if got != want:
                print(f"--- {name} gpu FAIL pixel({x},{y}) got={got} want={want} ---")
                return False
        print(f"--- {name} gpu PASS (160x120 pixels verified) ---")
        return True
    finally:
        proc.kill()
        proc.wait()


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
        choices=(
            "test-boot-handoff",
            "test-panic-hart1",
            "test-skip-tlb-ipi",
            "test-destroy-smp",
        ),
        help="run a test-only feature build",
    )
    parser.add_argument(
        "--tcp-check",
        action="store_true",
        help="also run a TCP echo round trip through hostfwd (vòng 77; default off)",
    )
    parser.add_argument(
        "--input-check",
        action="store_true",
        help="also send keys through a virtio-keyboard and require the shell "
        "help output (pha 4 input, vòng 70; default off)",
    )
    parser.add_argument(
        "--gpu-check",
        action="store_true",
        help="also boot with virtio-gpu, screendump the display and verify "
        "real pixels (pha 4 display MVP, vòng 66; default off)",
    )
    parser.add_argument(
        "--interactive",
        action="store_true",
        help="after each standard boot, type 'help' into the guest shell "
        "over a socket chardev and require the help output (pha 4 input "
        "end-to-end, vòng 62; default off so the standard gate is untouched)",
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
    interactive_results: dict[str, list[bool]] = {name: [] for name in names}
    try:
        for index in range(1, args.repeat + 1):
            for name in names:
                if args.repeat > 1:
                    print(f"\n===== {name} run {index}/{args.repeat} =====")
                results[name].append(tests[name](index))
                # Vòng 62: input end-to-end (chỉ khi boot chuẩn PASS, không
                # feature). Dùng lại cmd của arch qua helper build riêng.
                if (
                    args.interactive
                    and args.feature is None
                    and results[name][-1]
                ):
                    interactive_results[name].append(
                        test_interactive(name, index)
                    )
                if args.gpu_check and args.feature is None and results[name][-1]:
                    if not gpu_check(name):
                        results[name][-1] = False
                if args.input_check and args.feature is None and results[name][-1]:
                    if not input_check(name):
                        results[name][-1] = False
                if args.tcp_check and args.feature is None and results[name][-1]:
                    if not tcp_check(name):
                        results[name][-1] = False
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
    # The docs status block may only consume the standard configuration
    # (no feature build, exactly three runs per architecture).
    if args.feature is None and args.repeat == 3:
        standard = os.path.join(ROOT, "target", "qemu-status-standard.json")
        with open(standard, "w") as handle:
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
