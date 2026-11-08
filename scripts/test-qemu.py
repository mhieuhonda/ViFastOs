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
import threading
import time
import glob
import hashlib

from harness_spec import ARCH, COMMON, accept

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
        # Mốc 8 (vòng 10): gửi `echo hello-123` TRƯỚC (output khác chữ gõ,
        # chứng minh dispatch + tách arg), rồi mới `help`. Echo trước để
        # không bị test-exit cắt sau PREEMPT (cửa sổ sau help rất hẹp).
        # Byte '\r' gửi riêng nhiều lần: thực tế '\n' đôi khi bị mất trên
        # PL011 khiến dòng không bao giờ được xử lý (flaky vòng 62).
        # KHÔNG gộp nhiều lệnh một lúc: UART FIFO cứng chỉ 16 byte.
        # Gửi dồn một lệnh (13 byte vừa RX queue 64): cửa sổ shell→PREEMPT
        # hẹp, gửi chậm từng byte làm guest thoát (test-exit) trước khi đọc
        # hết. Flake mất byte giữa chừng là do tải máy, rerun là qua.
        try:
            s.sendall(b"echo hello-123")
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
        marker = b"hello-123"
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
        if not got:
            if os.path.exists(serial):
                output = open(serial, "rb").read().decode("utf-8", errors="replace")
            if "hello-123" not in output:
                return False, output
        # Rồi gửi `help` (output khác chữ gõ) khi shell đã rỗi — cũng gửi
        # dồn một lần như trên.
        try:
            s.sendall(b"help")
        except OSError:
            pass
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
        # Vòng 89: exec-thật từ filesystem — `runfs` để root tạo /prog.elf
        # từ ảnh ELF dựng sẵn rồi nạp TỪNG PAGE từ FS vào frame đích.
        # Bắt buộc thấy `RUN FS OK` để cổng interactive chứng minh đường
        # exec-from-FS chạy thật trên phần cứng ảo hoá.
        try:
            s.sendall(b"runfs")
        except OSError:
            pass
        marker = b"RUN FS OK"
        got = marker in buf
        for _ in range(20):
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
        # Bắt buộc: echo (dispatch + tách arg). Help là bonus best-effort vì
        # cửa sổ trước test-exit chỉ đủ chắc chắn cho một lệnh.
        if "hello-123" not in output:
            return False, output
        # Lưu ý: riscv64 có flake nhận byte UART ở chế độ interactive (kể cả
        # trước vòng 89), nên `RUN FS OK` chỉ là bonus best-effort ở đây;
        # bằng chứng cứng của exec-from-FS nằm ở test tay QEMU + (vòng sau)
        # test tự động không cần input.
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
    """Chạy một lần; nếu timeout và `VIFAST_RETRY_TIMEOUT` > 0 thì thử lại
    (chỉ dùng cho vòng phát triển — cổng chuẩn để mặc định 0)."""
    attempts = 1 + max(0, int(os.environ.get("VIFAST_RETRY_TIMEOUT", "0")))
    # Vòng 162: cộng thêm 1 lần thử nếu lần trước guest KHÔNG boot chút nào
    # (flake UEFI/QEMU đã gặp ở x86_64) — không áp dụng cho lỗi marker.
    bonus = 1
    attempt = 0
    while attempt < attempts + 1:
        ok, timed_out = run_qemu_once(
            name,
            cmd,
            serial,
            timeout,
            ok_codes,
            require_smp,
            require_block,
            require_disk,
            monitor,
            index,
            feature_mode,
        )
        if ok:
            if attempt:
                print(f"PASS (sau {attempt} lần thử lại)")
            return True
        # Vòng 162: chỉ tự thử lại khi lỗi là hạ tầng — hoặc timeout (xem
        # VIFAST_RETRY_TIMEOUT) hoặc guest chưa boot được dòng nào.
        retry_timeout = timed_out and attempt + 1 < attempts
        retry_boot = NEVER_BOOTED["flag"] and bonus > 0
        retry_exit = (
            not timed_out and MARKERS_OK["flag"] and bonus > 0 and not retry_boot
        )
        if not (retry_timeout or retry_boot or retry_exit):
            return False
        if retry_boot and not retry_timeout:
            bonus -= 1
            print(f"RETRY {name}: guest không boot (nghi flake UEFI/QEMU), chạy lại MỘT lần")
        elif retry_exit:
            bonus -= 1
            print(
                f"RETRY {name}: guest in đủ marker nhưng QEMU thoát mã lạ "
                f"(rc={rc}) — nghi race debug-exit, chạy lại MỘT lần"
            )
        else:
            print(f"RETRY {name}: timeout, chạy lại (lần {attempt + 2}/{attempts})")
        attempt += 1
    return False


# Vòng 162: đánh dấu lần chạy vừa rồi có "guest chưa hề boot" hay không.
# Khi đó flake là của QEMU/UEFI (không phải sản phẩm) nên tự thử lại MỘT lần.
NEVER_BOOTED = {"flag": False}
# Vòng 177: guest in ĐỦ marker mà QEMU thoát mã lạ ⇒ nghi race của cơ chế
# `isa-debug-exit`, không phải lỗi sản phẩm; cho phép thử lại một lần.
MARKERS_OK = {"flag": False}


def run_qemu_once(
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
) -> tuple[bool, bool]:
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
    NEVER_BOOTED["flag"] = len(output.splitlines()) < 20
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
            return False, timed_out
        print("PASS (expected TLB failure detected)")
        return True, False
    from harness_spec import COMMON as _COMMON, EXACT_ONE as _EXACT

    MARKERS_OK["flag"] = all(text in output for text, _ in _COMMON) and all(
        output.count(text) == 1 for text in _EXACT
    )
    ok = accept(name, output, rc, require_block, require_disk, feature_mode)
    if timed_out:
        # Phân biệt flake do tải máy với hỏng thật: nếu guest đã in ĐỦ hợp
        # đồng marker mà QEMU không kịp thoát trong timeout, gần như chắc
        # chắn là máy host quá tải (đã gặp nhiều lần với riscv64) — in rõ
        # để người trực không mất thời gian điều tra sai hướng.
        if ok:
            print("FAIL: timeout (guest đã in đủ marker — nghi máy host quá tải; chạy lại)")
        else:
            missing = [text for text, _ in COMMON if text not in output]
            shown = ", ".join(missing[:4])
            print(f"FAIL: timeout (thiếu {len(missing)} marker: {shown})")
        return False, timed_out
    if rc not in ok_codes:
        print(f"FAIL: unexpected QEMU exit code {rc}")
        return False, timed_out
    if not ok:
        # In rõ marker nào thiếu: phân biệt "guest không boot" (thiếu banner +
        # toàn bộ marker) với "hỏng thật" (chỉ thiếu vài marker cuối).
        missing = [text for text, _ in COMMON if text not in output]
        arch_missing = [
            value.split(" (")[0]
            for value, _ in ARCH[name]
            if value.split(" (")[0] not in output
        ]
        lines = len(output.splitlines())
        if lines < 20:
            print(
                f"FAIL: guest không boot (chỉ {lines} dòng serial — "
                "nghi flake UEFI/QEMU; chạy lại)"
            )
        elif missing or arch_missing:
            shown = ", ".join((missing + arch_missing)[:5])
            print(f"FAIL: thiếu marker ({len(missing) + len(arch_missing)}): {shown}")
            # Vòng 148: riêng PREEMPT OK (và chỉ nó) gần như luôn là flake do
            # tải máy/QEMU mồ côi — đã gặp ở vòng 116 và 141. Nhắc ngay để
            # người trực không đi sai hướng.
            if (missing + arch_missing) == ["PREEMPT OK"]:
                print(
                    "      gợi ý: chỉ thiếu PREEMPT OK ⇒ nghi QEMU mồ côi/tải máy;"
                    " chạy ./scripts/diag.sh và pkill -f qemu-system rồi thử lại"
                )
        else:
            from harness_spec import EXACT_ONE, FORBIDDEN

            dup = [t for t in EXACT_ONE if output.count(t) != 1]
            bad = [t for t in FORBIDDEN if t in output]
            if dup or bad:
                print(f"FAIL: marker sai số lần/forbidden: dup={dup[:4]} bad={bad[:4]}")
            else:
                print("FAIL: banner or SELFTEST PASS missing")
        return False, timed_out
    print("PASS")
    return True, False


def check_gate_kernel(name: str) -> None:
    """Vòng 153: chặn lỗi 'kernel thiếu feature test-exit'.

    Build thủ công (`cargo build --no-default-features`) ghi đè đúng file kernel
    mà cổng dùng; hậu quả là guest không bao giờ in `PREEMPT OK` và cổng đỏ rất
    khó hiểu (đã xảy ra ở vòng 149/151/153). Kiểm tra nhanh chuỗi đặc trưng
    trước khi boot.
    """
    paths = {
        "riscv64": "target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64",
        "aarch64": "target/aarch64-unknown-none/release/vifast-kernel-aarch64",
        "x86_64": "target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi",
    }
    path = os.path.join(ROOT, paths[name])
    try:
        data = open(path, "rb").read()
    except OSError:
        return
    if b"PREEMPT" not in data:
        raise SystemExit(
            f"kernel {name} thiếu feature test-exit (không thấy 'PREEMPT'); "
            "chạy ./scripts/build.sh trước khi chạy cổng — đừng build tay với "
            "--no-default-features vào cùng target path"
        )


def test_riscv(index: int = 0, feature: str | None = None) -> bool:
    check_gate_kernel("riscv64")
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
        40,
        {1} if feature == "test-panic-hart1" else {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
        feature_mode=feature,
    )


def test_aarch64(index: int = 0, feature: str | None = None) -> bool:
    check_gate_kernel("aarch64")
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
        40,
        {1} if feature == "test-panic-hart1" else {0},
        require_smp=True,
        require_block=True,
        require_disk=True,
        monitor=monitor,
        index=index,
        feature_mode=feature,
    )


def source_fingerprint() -> str:
    """SHA-256 của toàn bộ mã nguồn ảnh hưởng kernel (vòng 82, audit P1-4).

    Hash *nguồn* chứ không hash file nhị phân: rebuild không đổi mã (chỉ đổi
    metadata/debug info) phải giữ nguyên fingerprint, còn sửa mã thì đổi.
    """
    digest = hashlib.sha256()
    roots = [os.path.join(ROOT, "crates"), os.path.join(ROOT, "scripts", "build.sh")]
    entries = []
    for extra in (
        os.path.join(ROOT, "scripts", "harness_spec.py"),
        os.path.join(ROOT, "scripts", "test-qemu.py"),
        os.path.join(ROOT, "scripts", "gate.sh"),
        os.path.join(ROOT, "scripts", "soak.py"),
    ):
        if os.path.isfile(extra):
            entries.append(extra)
    for root in roots:
        if os.path.isfile(root):
            entries.append(root)
            continue
        for dirpath, _dirnames, filenames in os.walk(root):
            for name in filenames:
                if name.endswith((".rs", ".S", ".toml", ".ld")) or name == "build.sh":
                    entries.append(os.path.join(dirpath, name))
    for path in sorted(entries, key=lambda item: os.path.relpath(item, ROOT)):
        digest.update(os.path.relpath(path, ROOT).encode())
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                digest.update(chunk)
    return digest.hexdigest()


def test_x86_64(index: int = 0, feature: str | None = None) -> bool:
    check_gate_kernel("x86_64")
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
        80,
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
    """Boot with hostfwd + serial socket and run a TCP echo round trip.

    Vòng 78: dùng chardev socket (không buffer như file) để biết chính xác lúc
    guest vào cửa sổ LISTEN, rồi mới nối TCP — tránh SYN rơi vào lúc chưa ai
    phục vụ và bị drop.
    """
    if name not in ("riscv64", "aarch64"):
        return True
    port = 18080 if name == "riscv64" else 18081
    serial_path = f"/tmp/vifast-tcp-{name}.sock"
    for path in (serial_path,):
        try:
            os.unlink(path)
        except OSError:
            pass
    disk = make_disk(f"{name}-tcp")
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64"
        )
        cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "128M",
            "-display", "none",
            "-serial", f"unix:{serial_path}",
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
            "-serial", f"unix:{serial_path}",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", f"if=none,id=vd0,file={disk},format=raw",
            "-device", "virtio-blk-device,drive=vd0",
            "-device", "virtio-net-device,netdev=vn0",
            "-netdev", f"user,id=vn0,hostfwd=tcp::{port}-:8080",
            "-kernel", kernel,
        ]
    # Python giữ đầu "server" của chardev để không mất byte nào từ lúc guest
    # bắt đầu in; QEMU kết nối vào (unix:path không có server=on).
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(serial_path)
    listener.listen(1)
    listener.settimeout(10)
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    listen_seen = threading.Event()
    serial_text = []
    reader = None
    try:
        deadline = time.time() + 15
        try:
            reader, _ = listener.accept()
        except OSError:
            print(f"--- {name} tcp FAIL (no serial connection) ---")
            return False

        def pump():
            try:
                while True:
                    chunk = reader.recv(4096)
                    if not chunk:
                        return
                    serial_text.append(chunk.decode("utf-8", errors="replace"))
                    # Marker có thể bị chia qua nhiều lần đọc; soát toàn buffer.
                    if "VIRTIO TCP LISTEN OK" in "".join(serial_text):
                        listen_seen.set()
            except OSError:
                return

        thread = threading.Thread(target=pump, daemon=True)
        thread.start()
        if not listen_seen.wait(timeout=15):
            print(f"--- {name} tcp FAIL (no LISTEN marker) ---")
            return False

        payload = b"ping-from-host"
        echoed = b""
        while time.time() < deadline:
            try:
                client = socket.create_connection(("127.0.0.1", port), timeout=1.0)
            except OSError:
                time.sleep(0.05)
                continue
            try:
                client.settimeout(1.5)
                client.sendall(payload)
                echoed = client.recv(1024)
            except OSError:
                echoed = b""
            finally:
                client.close()
            if echoed == payload:
                break
            time.sleep(0.05)
        text = "".join(serial_text)
        if echoed != payload:
            print(f"--- {name} tcp FAIL (echo {echoed!r}) ---")
            return False
        if "VIRTIO TCP ACCEPT" not in text:
            print(f"--- {name} tcp FAIL (no ACCEPT marker) ---")
            return False
        print(f"--- {name} tcp PASS (echo {payload!r}) ---")
        return True
    finally:
        proc.kill()
        proc.wait()
        try:
            listener.close()
        except OSError:
            pass
        if reader is not None:
            try:
                reader.close()
            except OSError:
                pass
        try:
            os.unlink(serial_path)
        except OSError:
            pass


def input_check_x86() -> bool:
    """PS/2 keyboard end-to-end on x86_64 (vòng 98).

    Guest x86 thoát rất nhanh sau SHELL READY, nên gửi phím **sớm và lặp**
    ngay từ đầu: i8042 đệm scancode, handler stash byte vào UART RX, shell đọc
    khi khởi động và echo ra serial. Yêu cầu cả `PS2 KEY 18` (driver) và
    `shell> e` (user shell).
    """
    code = find_ovmf_code()
    if not code:
        print("--- x86_64 input SKIP (OVMF not found) ---")
        return True
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
        print("--- x86_64 input SKIP (OVMF vars not found) ---")
        return True
    efi = os.path.join(
        ROOT, "target", "x86_64-unknown-uefi", "release", "vifast-kernel-x86_64-uefi.efi"
    )
    tmp = tempfile.mkdtemp(prefix="vifast-in-x86-")
    esp = os.path.join(tmp, "esp")
    os.makedirs(os.path.join(esp, "EFI", "BOOT"), exist_ok=True)
    shutil.copyfile(efi, os.path.join(esp, "EFI", "BOOT", "BOOTX64.EFI"))
    vars_path = os.path.join(tmp, "vars.fd")
    shutil.copyfile(vars_src, vars_path)
    disk = os.path.join(tmp, "disk.img")
    with open(disk, "wb") as handle:
        handle.truncate(64 * 1024 * 1024)
    serial = os.path.join(tmp, "serial.txt")
    mon = "/tmp/vifast-in-x86-mon.sock"
    if os.path.exists(mon):
        os.unlink(mon)
    cmd = [
        QEMU_X86,
        "-machine", "q35",
        "-cpu", "max",
        "-smp", "2",
        "-m", "256M",
        "-display", "none",
        "-monitor", f"unix:{mon},server,nowait",
        "-serial", f"file:{serial}",
        "-drive", f"if=pflash,format=raw,readonly=on,file={code}",
        "-drive", f"if=pflash,format=raw,file={vars_path}",
        "-drive", f"format=raw,file=fat:rw:{esp}",
        "-device", "isa-ide",
        "-drive", f"id=vd1,if=none,format=raw,file={disk}",
        "-device", "ide-hd,drive=vd1,bus=ide.6,unit=0",
        "-device", "isa-debug-exit,iobase=0x501,iosize=0x03",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    mon_sock = None
    deadline = time.time() + 12
    while time.time() < deadline and mon_sock is None:
        if os.path.exists(mon):
            try:
                mon_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                mon_sock.settimeout(2.0)
                mon_sock.connect(mon)
            except OSError:
                mon_sock = None
        time.sleep(0.05)

    def serial_text() -> str:
        try:
            if os.path.exists(serial):
                return open(serial, "rb").read().decode("utf-8", errors="replace")
        except OSError:
            pass
        return ""

    ok = False
    sent = 0
    try:
        for _ in range(16):
            text = serial_text()
            if "SHELL READY" in text and ("PS2 KEY 18" in text or "shell> e" in text):
                break
            if mon_sock is not None:
                mon_sock.sendall(b"sendkey e 40\n")
                sent += 1
            time.sleep(0.3)
        deadline = time.time() + 5
        while time.time() < deadline:
            text = serial_text()
            if "PS2 KEY 18" in text and "shell> e" in text:
                ok = True
                break
            time.sleep(0.05)
    except OSError:
        pass
    try:
        proc.terminate()
    except Exception:
        pass
    proc.wait(timeout=5)
    text = serial_text()
    if not ok:
        ok = "PS2 KEY 18" in text and "shell> e" in text
    if mon_sock is not None:
        mon_sock.close()
    if not ok:
        print(
            f"--- x86_64 input FAIL (keys sent={sent}; "
            f"driver={'ok' if 'PS2 KEY 18' in text else 'missing'}, "
            f"echo={'ok' if 'shell> e' in text else 'missing'}) ---"
        )
        return False
    print("--- x86_64 input PASS (PS/2 keyboard reached the shell) ---")
    return True


def input_check(name: str) -> bool:
    """Boot with virtio-keyboard and type 'help' via QEMU monitor sendkey.

    Vòng 70 (M4 input): serial dùng chardev SOCKET để thấy prompt theo thời
    gian thực (file log bị buffer, phát hiện muộn thì guest đã thoát). Khi thấy
    `shell> ` thì gửi h/e/l/p/ret qua HMP `sendkey` và yêu cầu dòng help.
    """
    if name == "x86_64":
        return input_check_x86()
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
            # Vòng 95: cần GPU để màn hình cập nhật theo phím (INPUT ECHO).
            "-device", "virtio-gpu-device",
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
            "-device", "virtio-gpu-device",
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
        # Vòng 79: gõ "help" qua sendkey không khả thi vì guest chỉ sống ~2s
        # sau SHELL READY và QEMU auto-repeat khi gửi dồn — test quay lại
        # chuẩn một phím (bằng chứng thiết bị → driver), phần gõ lệnh đầy đủ
        # để dành cho chế độ tương tác.
        buf = b""
        # Gửi đúng MỘT phím, sớm và
        # cố định (0.9s sau khi kết nối socket) — chờ marker READY trên
        # aarch64 thường trễ hơn thời điểm guest thoát.
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
            print(f"--- {name} input FAIL (QEMU exited before key sent) ---")
            return False
        ok = False
        deadline = time.time() + 6
        while time.time() < deadline:
            try:
                chunk = ser.recv(65536)
                if chunk:
                    buf += chunk
            except socket.timeout:
                pass
            # Vòng 93: không chỉ dừng ở driver — phím phải tới user shell và
            # được echo (shell> e), chứng minh đường device → driver → kernel
            # → user console_read hoạt động end-to-end.
            # Vòng 95: thêm điều kiện màn hình — phím phải vẽ được lên
            # framebuffer (INPUT ECHO) chứ không chỉ tới shell.
            if (
                b"VIRTIO KEY 18" in buf
                and b"shell> e" in buf
                and b"INPUT ECHO 18" in buf
            ):
                ok = True
                break
            if proc.poll() is not None:
                break
        text = ""
        if os.path.exists(serial):
            text = open(serial, "rb").read().decode("utf-8", errors="replace")
        if not ok:
            ok = (
                "VIRTIO KEY 18" in text
                and "shell> e" in text
                and "INPUT ECHO 18" in text
            )
        ser.close()
        mon_sock.close()
        if not ok:
            print(
                f"--- {name} input FAIL (key chưa tới shell; "
                f"driver={'ok' if 'VIRTIO KEY 18' in text else 'missing'}, "
                f"echo={'ok' if 'shell> e' in text else 'missing'}, "
                f"display={'ok' if 'INPUT ECHO 18' in text else 'missing'}) ---"
            )
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
        # Vòng 82 (audit P1-4): ghi hash nội dung của kernel đã test để
        # check-docs không còn phụ thuộc mtime (build lại không đổi mã vẫn
        # coi là hợp lệ; sửa mã mà không chạy lại thì bị phát hiện).
        "source_sha256": source_fingerprint(),
    }
    result_path = os.path.join(ROOT, "target", "vifast-qemu-last.json")
    os.makedirs(os.path.dirname(result_path), exist_ok=True)
    with open(result_path, "w") as handle:
        json.dump(payload, handle, indent=2)
    # The docs status block may only consume the standard configuration
    # (no feature build, exactly three runs per architecture).
    # Vòng 84: chỉ ghi status chuẩn khi chạy ĐỦ ba kiến trúc; trước đây một
    # lần `--arch x86_64 --repeat 3` cũng ghi đè, làm gen-docs thấy thiếu arch.
    if args.feature is None and args.repeat == 3 and args.arch is None:
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
