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

from harness_spec import ARCH, COMMON, accept, tcp_echo_verdict

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
    """Create a fresh disk in a unique per-run directory.

    r240 (T5/D10): nếu có chương trình ngoài đã build, dựng ảnh FS ngay trên
    host bằng `vifast-fsutil` rồi chèn `/bin/vf` + các hard link. Chương
    trình nằm trên ĐĨA nên không tốn frame Untyped lúc boot (bài học r238:
    nhúng vào ảnh root làm pool Untyped cạn 79→84/86 page).

    r248: `name` có thể mang hậu tố (`riscv64-ial`, `x86_64-tcp`…). Trước đây
    đưa nguyên tên đó cho `populate_disk` ⇒ `TARGETS.get()` trả None ⇒ đĩa
    RỖNG ở mọi cấu hình không chuẩn (interactive/tcp/input/gpu), nên chế độ
    interactive không thể chạy chương trình ngoài nào.
    """
    directory = tempfile.mkdtemp(prefix=f"vifast-{name}-disk-")
    if os.environ.get("VIFAST_KEEP_DISK") != "1":
        DISK_DIRS.append(directory)
    path = os.path.join(directory, "disk.img")
    with open(path, "wb") as disk:
        disk.truncate(4 * 1024 * 1024)
    populate_disk(path, name.split("-")[0])
    return path


# r254 (T17): kịch bản gõ vào shell cho `--interactive`. Mỗi cặp là
# (lệnh gửi đi, marker PHẢI thấy trên serial sau lệnh đó). Marker phải là chuỗi
# chỉ xuất hiện do lệnh sinh ra (D20: chờ theo sự kiện, không theo thời gian).
INTERACTIVE_SCENARIO = [
    # (lệnh, marker, số lần TỐI THIỂU phải thấy).
    # - Marker luôn gồm phần shell echo ("<lệnh>\n<output>") để không khớp nhầm
    #   với marker của self-test ở boot.
    # - `BIN OK uname` đã xuất hiện MỘT lần ở self-test boot, nên bản tương tác
    #   phải là lần thứ HAI (đo ở r254: nếu chỉ tìm chuỗi thì test vacuous).
    (b"help", b"help\nhelp time echo ls", 1),
    (b"cd /bin\rpwd", b"pwd\n/bin", 1),
    (b"uname", b"BIN OK uname", 2),
    (b"ps", b"BIN OK ps", 1),
    # r268 (T27/G2 Phần B): `df` nay là APPLET `/bin/df`, không còn builtin —
    # bước này chứng minh chương trình con exec từ FS thật sự nhận được cap
    # VFS (slot 1, badge 1) do root uỷ quyền, nếu không nó in "df: error".
    (b"df", b"blocks", 1),
    # `!!` để CUỐI: r254 đo được nó làm shell client fault (store page fault
    # vào vùng .text của client) nên guest chết ngay sau đó.
    (b"history\r!!", b"!!\n!!: ", 1),
]
# Mỗi bước tối đa chừng này giây; tổng cửa sổ shell của guest ngắn nên giữ nhỏ.
INTERACTIVE_STEP_TIMEOUT = 8.0
# Tổng thời gian chờ kịch bản chạy xong trong guest.
INTERACTIVE_TOTAL_TIMEOUT = 12.0

TARGETS = {
    "riscv64": "riscv64imac-unknown-none-elf",
    "aarch64": "aarch64-unknown-none-softfloat",
    "x86_64": "x86_64-unknown-none",
}


def populate_disk(path: str, arch: str) -> None:
    """Chèn chương trình ngoài vào ảnh đĩa (r240). Bỏ qua êm nếu chưa build.

    Chương trình nằm trên ĐĨA nên không tốn frame Untyped lúc boot; xem bài
    học r238 trong docs/reviews/progress-g1.md.
    """
    fsutil = os.path.join(ROOT, "target", "debug", "vifast-fsutil")
    target = TARGETS.get(arch)
    if target is None or not os.path.exists(fsutil):
        return
    host = os.path.join(ROOT, "target", target, "root-task",
                        "vifast-bin.stripped")
    if not os.path.exists(host):
        return
    # D14 (r247): một ảnh multicall + hard link, không còn 3 ảnh riêng.
    # r268 (T27/G2): `df` là applet ĐẦU TIÊN cần cap VFS — chương trình con
    # nhận endpoint ở slot 1 (badge 1) từ root lúc exec.
    links = ("uname", "uptime", "free", "ps", "jobs", "args", "df")
    subprocess.run([fsutil, "mkfs", path], stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL)
    subprocess.run([fsutil, "mkdir", path, "/bin"], stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL)
    subprocess.run([fsutil, "put-file", path, "/bin/vf", host],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for name in links:
        subprocess.run([fsutil, "link", path, "/bin/vf", f"/bin/{name}"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def run_qemu_interactive(
    name: str,
    cmd_base: list[str],
    serial: str,
    timeout: int,
    scenario: list[tuple[bytes, bytes]],
) -> tuple[bool, list[tuple[bytes, bytes, bool]], str]:
    """Boot một lần, gõ CẢ kịch bản, chờ marker từng lệnh rồi mới gửi lệnh kế.

    r254 (T17/D20): bản kernel `*-shell` (feature `test-shell-linger`) đặt
    `TIMER_TICKS` lớn nên guest sống đủ lâu để gõ nhiều lệnh. Mỗi lệnh chỉ được
    gửi khi đã thấy marker của lệnh trước — đồng bộ theo SỰ KIỆN, không theo
    thời gian. Không gửi dồn: FIFO UART của guest chỉ 16 B (đo ở r254).

    Trả về (ok, kết quả từng bước, serial log).
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
            skip_next = True
            continue
        if arg == f"file:{serial}":
            continue
        cmd.append(arg)
    cmd += [
        "-chardev",
        f"socket,path={sock},server=on,wait=off,logfile={serial},id=visser",
        "-serial",
        "chardev:visser",
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    results: list[tuple[bytes, bytes, bool]] = []
    try:
        sock_client = None
        for _ in range(100):
            time.sleep(0.2)
            if os.path.exists(sock):
                try:
                    sock_client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    sock_client.settimeout(0.2)
                    sock_client.connect(sock)
                    break
                except OSError:
                    pass
        if sock_client is None:
            proc.kill()
            proc.wait()
            return False, [], "input: no chardev connect"
        # Gửi lệnh đầu NGAY: guest đệm byte vào RX buffer và xử lý khi shell
        # tới lượt, nên lệnh không bị mất dù guest còn đang chạy demo boot.
        try:
            sock_client.sendall(scenario[0][0] + b"\r")
        except OSError:
            pass
        buf = b""
        log_pos = 0
        for index, (command, marker, need) in enumerate(scenario):
            deadline = time.time() + INTERACTIVE_STEP_TIMEOUT
            found = False
            while time.time() < deadline:
                # r254: CHỈ đọc file log. Đọc thêm socket sẽ nhận ĐÚNG cùng
                # luồng serial lần thứ hai và làm buf bị nhân đôi ký tự
                # ("hheellpp"), khiến marker không bao giờ khớp (đo được).
                time.sleep(0.02)
                if os.path.exists(serial):
                    with open(serial, "rb") as handle:
                        handle.seek(log_pos)
                        chunk = handle.read()
                        log_pos = handle.tell()
                    buf += chunk
                if buf.count(marker) >= need:
                    found = True
                    break
            if os.environ.get("VIFAST_INTERACTIVE_DEBUG") == "1":
                print(f"    [dbg] {name} buf={len(buf)}B marker={marker!r} found={found}")
                print(f"    [dbg] tail={buf[-120:]!r}")
            results.append((command, marker, found))
            if not found:
                break
            if index + 1 < len(scenario):
                try:
                    sock_client.sendall(scenario[index + 1][0] + b"\r")
                except OSError:
                    break
        try:
            sock_client.close()
        except OSError:
            pass
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        output = ""
        if os.path.exists(serial):
            output = open(serial, "rb").read().decode("utf-8", errors="replace")
        ok = len(results) == len(scenario) and all(found for _, _, found in results)
        return ok, results, output
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


def write_artifacts(
    name: str,
    index: int,
    attempt: int,
    output: str,
    reason: str,
    monitor: str | None = None,
) -> str:
    """Ghi artifact cho MỘT lần boot hỏng — ĐƯỜNG DUY NHẤT (B1', r294.B).

    Trước đây có hai đường ghi khác nhau (`dump_timeout_artifacts` cho timeout
    có monitor, `save_flake_artifacts` cho lỗi marker) và cả hai đều có thể bỏ
    sót `serial.txt`. Hệ quả: dòng `ROOT PANIC` + `file:dòng` — thứ quyết định
    phân loại T-ROOT — bị vứt đi, nên mọi kết luận sau đó đều đoán.

    Ghi: `serial.txt` (toàn bộ), `status.txt` (dòng quan trọng + khối panic),
    `tail.txt` (80 dòng cuối), `registers.txt` (nếu có monitor).
    """
    directory = os.path.join(ARTIFACT_ROOT, f"flake-{name}-{index}-a{attempt}")
    os.makedirs(directory, exist_ok=True)

    with open(os.path.join(directory, "serial.txt"), "w") as handle:
        handle.write(output)

    lines = output.splitlines()
    important = [
        line
        for line in lines
        if re.search(
            r"STALL|PANIC|SMP |USER FAULT|KERNEL EXCEPTION|BKL|T-ROOT|TICK ", line
        )
    ]
    # B2': khối panic phải vào status.txt NGUYÊN VẸN — dòng `ROOT PANIC` và
    # dòng kế tiếp (thường là `crates/.../main.rs<dòng>`) là thứ phân loại
    # T-ROOT <file:dòng>. Bản cũ lọc theo regex nên dòng file:dòng bị rơi.
    panic_block: list[str] = []
    for position, line in enumerate(lines):
        if "ROOT PANIC" in line or "PANIC" in line and "ROOT" in line:
            panic_block.append(line)
            if position + 1 < len(lines):
                panic_block.append(lines[position + 1])
    with open(os.path.join(directory, "status.txt"), "w") as handle:
        handle.write(f"reason: {reason}\n")
        handle.write("\n".join(important[-40:]) + "\n")
        if panic_block:
            handle.write("--- panic ---\n")
            handle.write("\n".join(panic_block) + "\n")

    with open(os.path.join(directory, "tail.txt"), "w") as handle:
        handle.write("\n".join(lines[-80:]) + "\n")

    registers = ""
    if monitor and os.path.exists(monitor):
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(2.0)
                sock.connect(monitor)
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
    if registers:
        with open(os.path.join(directory, "registers.txt"), "w") as handle:
            handle.write(registers)
    # Khối A (kế hoạch qua đêm): khi CẢ HAI hart đều không chạy được (hart 0 quay
    # user với IRQ0 đang chờ, hart 1 `hlt` IF=0) thì không hook nào đổ được vòng
    # đệm — phải đọc bộ nhớ khách qua monitor. Chỉ làm khi serial có RING ADDR.
    ring_address = None
    for line in output.splitlines():
        if line.startswith("RING ADDR "):
            try:
                ring_address = int(line.split()[2], 16)
            except (IndexError, ValueError):
                ring_address = None
    if ring_address is not None and monitor and os.path.exists(monitor):
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.settimeout(2.0)
                sock.connect(monitor)
                banner = b""
                while b"(qemu)" not in banner:
                    try:
                        chunk = sock.recv(65536)
                    except socket.timeout:
                        break
                    if not chunk:
                        break
                    banner += chunk
                sock.sendall(f"xp /96gx {ring_address:#x}\n".encode())
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
                with open(os.path.join(directory, "ring.txt"), "wb") as handle:
                    handle.write(b"".join(chunks))
        except OSError:
            pass

    print(f"artifacts: {directory}")
    return directory


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


def read_settled_serial(path: str, settle: float = 3.0) -> str:
    """Đọc serial SAU khi QEMU đã chết, chờ tới khi file ngừng lớn.

    T33 (r276): gặp lần soak báo "thiếu 50 marker" nhưng file serial — đọc lại
    sau đó — lại CÓ ĐỦ mọi marker, kể cả `PREEMPT OK`. Nghĩa là QEMU còn flush
    chardev sau khi bị kill, và harness kết luận quá sớm. Hàm này chờ (tối đa
    `settle` giây) tới khi kích thước file ổn định rồi mới trả nội dung.
    """
    last = -1
    data = ""
    deadline = time.time() + settle
    while time.time() < deadline:
        if os.path.exists(path):
            with open(path, "rb") as fh:
                data = fh.read().decode("utf-8", errors="replace")
        size = len(data)
        if size == last:
            break
        last = size
        time.sleep(0.2)
    return data

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
            attempt,
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
        retry_shell = (not SHELL_READY["flag"] and bonus > 0 and not retry_boot)
        # r293.1 + B1': mọi lần hỏng đều được đếm + phân loại + lưu artifact
        # (kể cả lần sẽ được retry che), qua MỘT hàm ghi duy nhất.
        RUN_STATS["retries"] += 1
        kind = record_failure(
            name, index, attempt, LAST_OUTPUT["text"], f"attempt {attempt} hỏng"
        )
        if not (retry_timeout or retry_boot or retry_exit or retry_shell):
            return False
        if retry_boot and not retry_timeout:
            bonus -= 1
            print(f"RETRY {name}: [{kind}] guest không boot (nghi flake UEFI/QEMU), chạy lại MỘT lần")
        elif retry_shell:
            bonus -= 1
            print(
                f"RETRY {name}: [{kind}] guest dừng trước SHELL READY — nghi flake boot,"
                " chạy lại MỘT lần"
            )
        elif retry_exit:
            bonus -= 1
            print(
                f"RETRY {name}: [{kind}] guest in đủ marker nhưng QEMU thoát mã lạ "
                "— nghi race debug-exit, chạy lại MỘT lần"
            )
        else:
            print(f"RETRY {name}: [{kind}] timeout, chạy lại (lần {attempt + 2}/{attempts})")
        attempt += 1
    return False


# r293.1 (tệp quyết định r293-r298, mục 2): đếm RETRY và PHÂN LOẠI lần hỏng.
# Lý do: harness có retry nên "PASS" đang che flake — mọi phép đo FS/scheduler
# trước r293 đều bị nhiễu. Ba loại, đếm riêng:
#   T-FS    — thiếu marker `FS V2 OK` hoặc readdir đọc ảnh cũ;
#   T-SCHED — ROOT PANIC, thiếu PREEMPT OK, `USER FAULT tcb=None`,
#             "no runnable thread", `SMP NO READY`;
#   T-KHAC  — mọi thứ khác (phải phân loại bằng tay trong log).
# r293.1: serial của lần chạy vừa rồi, để `run_qemu` phân loại được lần hỏng
# mà không phải đổi chữ ký hàm `run_qemu_once`.
LAST_OUTPUT = {"text": ""}

RUN_STATS = {
    "boots": 0,
    "retries": 0,
    "classes": {},
    "events": [],
}


def format_retry_line(stats: dict) -> str:
    """Dòng `RETRY n/N (T-FS=.., T-SCHED=.., T-KHAC=..)` (r293.1).

    Tách thành hàm thuần để test-harness-spec kiểm được mà không phải chạy
    QEMU: case âm (bỏ đếm) làm test đỏ vì hàm/khóa không còn.
    """
    classes = stats["classes"]
    parts = ", ".join(f"{name}={count}" for name, count in sorted(classes.items()))
    detail = parts if parts else "không lớp nào"
    return f"RETRY {stats['retries']}/{max(1, stats['boots'])} ({detail})"


def panic_site(output: str) -> str:
    """Lấy `file:dòng` của dòng `ROOT PANIC` (D3, kế hoạch qua đêm).

    Chấp nhận CẢ hai dạng: bản mới in `main.rs:480`, bản cũ (artifact trước D3)
    in `main.rs480` — regex `\.rs:?(\d+)` đọc được cả hai.
    """
    lines = output.splitlines()
    for position, line in enumerate(lines):
        if "ROOT PANIC" not in line:
            continue
        for candidate in lines[position : position + 3]:
            match = re.search(r"([A-Za-z0-9_/.-]+\.rs):?(\d+)", candidate)
            if match:
                return f"{match.group(1)}:{match.group(2)}"
    return ""


def classify_failure(output: str) -> str:
    """Phân loại một lần boot hỏng theo bảng của kế hoạch qua đêm (294.C + D3).

    Thứ tự xét (gặp lớp nào trước thì dừng):
      T-BOOT (kernel không in dòng nào) > T-SCHED "mất current"
      > T-ROOT <file:dòng> > T-FS > T-SCHED chung > T-KHAC.

    Vì sao T-SCHED "mất current" đứng TRƯỚC T-ROOT: kế hoạch qua đêm §4 xếp
    `SMP NO CURRENT` trước — nếu mất current thì panic của root chỉ là hệ quả.
    """
    # (0) T-BOOT: kernel chưa hề in dòng khởi động nào.
    if "booting on" not in output and "SELFTEST PASS" not in output:
        return "T-BOOT"
    # (1) Mất current — chữ ký riêng, xét trước panic.
    if "SMP NO CURRENT" in output:
        return "T-SCHED"
    # (2) Các chữ ký scheduler rõ ràng khác.
    for signature in (
        "USER FAULT tcb=None",
        "no runnable thread",
        "SMP NO READY",
        "KERNEL FAULT",
        "no current thread",
    ):
        if signature in output:
            return "T-SCHED"
    # (3) T-ROOT kèm lớp con `<file>:<dòng>`.
    if "ROOT PANIC" in output:
        site = panic_site(output)
        return f"T-ROOT {site}" if site else "T-ROOT"
    # (4) Hỏng đúng tầng FS (chỉ khi guest đã tới SHELL READY).
    if "FS V2 BAD" in output:
        return "T-FS"
    if "FS V2 OK" not in output and "SHELL READY" in output:
        return "T-FS"
    # (5) Thiếu PREEMPT OK mà không có dấu hiệu nào ở trên: vẫn là scheduler.
    if "PREEMPT OK" not in output:
        return "T-SCHED"
    return "T-KHAC"


def record_failure(name: str, index: int, attempt: int, output: str, reason: str) -> str:
    """Ghi nhận một lần hỏng + lưu artifact đầy đủ (r293.1, r293.2, r294.B B1')."""
    kind = classify_failure(output)
    RUN_STATS["classes"][kind] = RUN_STATS["classes"].get(kind, 0) + 1
    RUN_STATS["events"].append(
        {
            "arch": name,
            "run": index,
            "attempt": attempt,
            "class": kind,
            "reason": reason,
        }
    )
    write_artifacts(name, index, attempt, output, reason)
    return kind


# Vòng 162: đánh dấu lần chạy vừa rồi có "guest chưa hề boot" hay không.
# Khi đó flake là của QEMU/UEFI (không phải sản phẩm) nên tự thử lại MỘT lần.
NEVER_BOOTED = {"flag": False}
# Vòng 177: guest in ĐỦ marker mà QEMU thoát mã lạ ⇒ nghi race của cơ chế
# `isa-debug-exit`, không phải lỗi sản phẩm; cho phép thử lại một lần.
MARKERS_OK = {"flag": False}
# Vòng 182: soak x86 5 lần bắt được 1 lần guest dừng giữa boot (sau `USER EXIT`,
# chưa tới `SHELL READY`). Đây là flake của giai đoạn boot, không phải lỗi
# marker; cho phép thử lại MỘT lần và in lý do rõ ràng.
SHELL_READY = {"flag": False}


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
    attempt: int = 0,
) -> tuple[bool, bool]:
    # r293.1: N = số lần BOOT THẬT (kể cả lần phải chạy lại), nên đếm ở đây
    # chứ không ở `run_qemu` (một lần gọi có thể boot nhiều lần).
    RUN_STATS["boots"] += 1
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
            write_artifacts(
                name,
                index,
                attempt,
                output_at_timeout,
                "timeout (guest chưa thoát)",
                monitor,
            )
        proc.kill()
        rc = proc.wait()
    elapsed = time.time() - started
    output = ""
    if os.path.exists(serial):
        if timed_out:
            # T33 (r276): timeout thì chờ serial ổn định rồi mới đọc — tránh
            # kết luận "thiếu marker" trong khi QEMU còn đang flush.
            output = read_settled_serial(serial)
        else:
            output = open(serial, "rb").read().decode("utf-8", errors="replace")
    if os.environ.get("VIFAST_KEEP_SERIAL") == "1" and os.path.exists(serial):
        # Giữ lại serial của từng lần chạy để điều tra flake (vòng 67).
        # r288: thêm số LẦN THỬ vào tên — trước đây mọi lần thử lại ghi đè cùng
        # một file nên serial của lần HỎNG bị lần thử lại (thường xanh) xoá mất,
        # làm mọi cuộc điều tra flake đọc nhầm serial.
        shutil.copyfile(
            serial,
            f"{tempfile.gettempdir()}/vifast-keep-{name}-{index}-a{attempt}.serial",
        )
    LAST_OUTPUT["text"] = output
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

    SHELL_READY["flag"] = "SHELL READY" in output
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
            # r262: nêu NGAY marker bị cấm nếu có. Trước đây một lần boot in
            # `FS V2 BAD` (bị cấm) chỉ được báo là "thiếu FS V2 OK", và câu
            # đó đã làm điều tra đi sai hướng suốt một vòng: thiếu marker là
            # TRIỆU CHỨNG, marker bị cấm mới là NGUYÊN NHÂN.
            from harness_spec import FORBIDDEN as _FORBIDDEN

            forbidden_seen = [token for token in _FORBIDDEN if token in output]
            if forbidden_seen:
                print(
                    "      kèm marker BỊ CẤM: "
                    f"{forbidden_seen[:4]} — đây là lỗi thật, không phải flake"
                )
        else:
            from harness_spec import BOOT_STACK_RE, EXACT_ONE, FORBIDDEN

            dup = [t for t in EXACT_ONE if output.count(t) != 1]
            bad = [t for t in FORBIDDEN if t in output]
            if dup or bad:
                print(f"FAIL: marker sai số lần/forbidden: dup={dup[:4]} bad={bad[:4]}")
            else:
                # r227 (T2, D11): TRƯỚC ĐÂY in "banner or SELFTEST PASS missing"
                # cho mọi trường hợp còn lại. Khi boot-handoff đo sai HWM (100%)
                # câu này đổ lỗi sang banner và làm điều tra sai hướng 3 vòng.
                # Nay nêu đúng điều kiện số học đang trượt.
                stack = BOOT_STACK_RE.search(output)
                hint = "không khớp điều kiện nào trong accept() (xem log đầy đủ)"
                if stack:
                    used, size = int(stack.group(1)), int(stack.group(2))
                    if size < 0x20000:
                        hint = f"BOOT STACK size={size} < 0x20000"
                    elif used < 1:
                        hint = "BOOT STACK used=0 (số đo không có)"
                    elif used * 2 > size:
                        hint = (
                            f"BOOT STACK HWM {used}/{size} vượt 50% "
                            "(nghi tràn stack hoặc chưa tô 0xA5)"
                        )
                print(f"FAIL: {hint}")
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


class TcpProbeListener:
    """Vòng 178: listener TCP trên host để chứng minh guest bắt tay TCP thật.

    Slirp ánh xạ `10.0.2.2:<port>` về loopback của host, nên guest gửi SYN tới
    đó sẽ tới listener này. Chỉ nhận **một** kết nối rồi đóng.
    """

    def __init__(self, port: int) -> None:
        import socket
        import threading

        self.accepted = 0
        self.payload_seen = False
        self.error: str | None = None
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            self._sock.bind(("127.0.0.1", port))
            self._sock.listen(1)
            self._sock.settimeout(30)
        except OSError as exc:  # cổng bận ⇒ guest sẽ in SKIP, cổng vẫn xanh
            self.error = str(exc)
            self._sock.close()
            return

        def serve() -> None:
            try:
                conn, _ = self._sock.accept()
                self.accepted += 1
                conn.settimeout(10)
                # Vòng 180: phải NHẬN được dữ liệu ứng dụng do guest gửi.
                try:
                    data = conn.recv(64)
                except OSError:
                    data = b""
                if b"PING-FROM-GUEST" in data:
                    self.payload_seen = True
                # Rồi gửi dữ liệu về để chứng minh chiều ngược lại.
                try:
                    conn.sendall(b"VIFAST-ECHO")
                except OSError:
                    pass
                conn.close()
            except OSError:
                pass

        self._thread = threading.Thread(target=serve, daemon=True)
        self._thread.start()

    def close(self) -> None:
        try:
            self._sock.close()
        except OSError:
            pass


def test_riscv(index: int = 0, feature: str | None = None) -> bool:
    check_gate_kernel("riscv64")
    # Vòng 178: listener TCP cho phép thử bắt tay guest -> host.
    listener = TcpProbeListener(8099) if feature is None else None
    if feature == "test-boot-handoff":
        kernel_name = "vifast-kernel-riscv64-handoff"
    elif feature == "test-panic-hart1":
        kernel_name = "vifast-kernel-riscv64-panic"
    elif feature == "test-skip-tlb-ipi":
        kernel_name = "vifast-kernel-riscv64-skip-tlb"
    elif feature == "test-destroy-smp":
        kernel_name = "vifast-kernel-riscv64-destroy-smp"
    elif feature == "test-check-boundary":
        kernel_name = "vifast-kernel-riscv64-boundary"
    elif feature == "test-check-boundary-mutation":
        kernel_name = "vifast-kernel-riscv64-boundary-mut"
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
    ok = run_qemu(
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
    # Vòng 179 / r226 (T1, D11): marker echo chỉ có nghĩa khi harness CÓ listener.
    # Feature-mode không tạo TcpProbeListener nên marker vắng mặt là đúng; trước
    # đây check vô điều kiện làm MỌI feature test riscv64 đỏ giả từ v179.
    if ok:
        try:
            text = open(serial, "rb").read().decode("utf-8", errors="replace")
        except OSError:
            text = ""
        ok, message = tcp_echo_verdict(listener is not None, text)
        print(message)
    if listener is not None:
        accepted = listener.accepted
        listener.close()
        if accepted:
            print("TCP PROBE: host accepted the guest connection")
            if listener.payload_seen:
                print("TCP PROBE: host received PING-FROM-GUEST from guest")
            else:
                print("FAIL: host KHÔNG nhận được payload ứng dụng từ guest")
                ok = False
        elif listener.error:
            print(f"TCP PROBE: không mở được listener ({listener.error}) — guest in SKIP")
        else:
            # Vòng 178: guest PHẢI chủ động kết nối TCP tới host — đây là yêu
            # cầu của cổng (không phải chỉ thông báo), vì listener chắc chắn
            # đang chờ trên đúng cổng đó.
            print("FAIL: host KHÔNG nhận được kết nối TCP từ guest")
            ok = False
    return ok

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
                else (
                    "vifast-kernel-x86_64-uefi-boundary.efi"
                    if feature == "test-check-boundary"
                    else (
                        "vifast-kernel-x86_64-uefi-boundary-mut.efi"
                        if feature == "test-check-boundary-mutation"
                        else "vifast-kernel-x86_64-uefi.efi"
                    )
                )
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
    """Boot bản kernel `*-shell` và gõ kịch bản shell (T17)."""
    serial = os.path.join(tempfile.gettempdir(), f"vifast-{name}-ial.serial")
    disk = make_disk(f"{name}-ial")
    if name == "riscv64":
        kernel = os.path.join(
            ROOT, "target", "riscv64gc-unknown-none-elf", "release", "vifast-kernel-riscv64-shell"
        )
        cmd = [
            QEMU_RV, "-machine", "virt", "-smp", "2", "-m", "256M",
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
        kernel = os.path.join(ROOT, "target", "aarch64-unknown-none", "release", "vifast-kernel-aarch64-shell")
        cmd = [
            QEMU_ARM, "-machine", "virt,gic-version=2", "-semihosting",
            "-cpu", "max", "-smp", "2", "-m", "256M",
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
        print(f"--- {name} interactive SKIP (uefi) ---")
        return True
    ok, results, output = run_qemu_interactive(name, cmd, serial, 30, INTERACTIVE_SCENARIO)
    for command, marker, found in results:
        print(
            f"    [{name}] {command.decode(errors='replace'):<14} "
            f"-> {marker.decode(errors='replace')!r}: {'OK' if found else 'MISSING'}"
        )
    print(f"--- {name} interactive {'PASS' if ok else 'FAIL'} ---")
    if not ok:
        print(output[-2000:])
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
            "test-check-boundary",
            "test-check-boundary-mutation",
        ),
        help="run a test-only feature build",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="RETRY > 0 là ĐỎ (chế độ nghiêm, theo tệp quyết định r293-r298 mục 2.4; "
        "gate chỉ bật từ r296)",
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
    # r259 (T21): biến thể bất biến ranh giới chỉ dựng cho riscv64/x86_64 —
    # cùng phạm vi với test-skip-tlb-ipi; phần logic dùng chung đã có host test.
    if args.feature in ("test-check-boundary", "test-check-boundary-mutation") and args.arch == "aarch64":
        parser.error(
            f"--feature {args.feature} only applies to riscv64/x86_64"
        )
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
    elif args.feature in (
        "test-skip-tlb-ipi",
        "test-check-boundary",
        "test-check-boundary-mutation",
    ):
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
    retry_total = RUN_STATS["retries"]
    boots_total = max(1, RUN_STATS["boots"])
    classes = RUN_STATS["classes"]
    print(format_retry_line(RUN_STATS))
    payload["retry"] = {
        "count": retry_total,
        "boots": boots_total,
        "classes": dict(classes),
        "events": list(RUN_STATS["events"]),
    }
    payload["strict"] = bool(getattr(args, "strict", False))
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
    # r293.3: chế độ NGHIÊM — chưa bật trong gate (bật từ r296 theo tệp quyết
    # định). Ở chế độ này RETRY > 0 là ĐỎ, vì mọi phép đo "xanh" nhờ retry đều
    # là xanh giả.
    if getattr(args, "strict", False) and retry_total > 0:
        print(
            f"STRICT: ĐỎ — {retry_total}/{boots_total} lần boot phải chạy lại "
            f"(T-FS={classes['T-FS']}, T-SCHED={classes['T-SCHED']}, T-KHAC={classes['T-KHAC']})"
        )
        failed = True
    # r248 (D11): trước đây `--interactive` chạy nhưng kết quả bị bỏ — cổng
    # xanh dù input/flake hỏng. Giờ mọi lần interactive thất bại đều làm đỏ.
    for name, runs in interactive_results.items():
        if runs and not all(runs):
            print(f"{name:8} interactive {sum(runs)}/{len(runs)} PASS")
            failed = True
    return 1 if failed else 0

if __name__ == "__main__":
    sys.exit(main())
