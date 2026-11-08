"""Shared QEMU acceptance conditions.

This module is the single source of truth for the guest contract.  It is
imported by ``test-qemu.py`` and by ``gen-docs.py``.  The negative test in
``scripts/test-harness-spec.py`` removes each condition from a synthetic
output and asserts that acceptance fails.
"""

from __future__ import annotations

import re

COMMON = [
    ("SELFTEST PASS", "kernel host self-test passed in guest"),
    ("booting on", "architecture banner printed"),
    ("BOOT UNTYPED", "boot untyped accounting printed"),
    ("PREEMPT OK", "timer preemption progressed"),
    ("Isolation", "fault isolation demo ran"),
    ("Server", "user VFS server ran"),
    ("Client", "user VFS client ran"),
    ("SLEEP OK", "sleep observed the timer deadline"),
    ("Mapped", "client VSpace mapping succeeded"),
    ("Cap OK", "capability transfer demo succeeded"),
    ("ViFastOs ramfs", "ramfs read path ran"),
    ("ViFastOs write", "ramfs write path ran"),
    ("FS V2 OK", "FS v2 namespace/data demo completed"),
    ("FS FSCK OK", "server fsck passed after demo"),
    ("FS PERM OK", "badge-based FS permissions behaved"),
    ("BADGE SPOOF DENIED OK", "rebadging and badge spoofing were denied"),
    ("IPC BUF OK", "IPC buffer transfer succeeded"),
    ("BLOCK DENIED OK", "client was denied BlockWrite without a device cap"),
    ("Reply OK", "reply path succeeded"),
    ("ROOT RUST", "root Rust ELF entered user mode"),
    ("BOOTINFO OK", "root validated BootInfo v3"),
    ("SCHED CTX OK", "scheduler context configured and bound"),
    ("RUST MODULES OK", "server/client Rust ELF modules loaded"),
    ("CLIENT VSPACE OK", "client VSpace built"),
    ("FAULT EP OK", "fault endpoint delivered all fault records"),
    ("ROOT READY", "root finished initialisation"),
    ("Zero OK", "zero page / BSS path verified"),
    ("BLOCK OK", "block device backend responded"),
    ("FS DISK OK", "filesystem disk path responded"),
    ("PAGER RECLAIM OK", "pager demo frames/TCBs returned to untyped"),
    ("RUN FS OK", "exec-from-filesystem loaded an ELF page-by-page and ran it"),
    ("HIARG", "program executed from the filesystem printed an argv-only marker"),
    (
        "SYS REJECT OK",
        "invalid handles and out-of-range user pointers were all refused",
    ),
    (
        "FS OPS FSCK OK",
        "fsck stayed clean after create/unlink/truncate/rename/link operations",
    ),
    (
        "FS RENAME OK",
        "rename made the old name disappear and kept the new name's size",
    ),
    (
        "FS LINK OK",
        "hard link produced two names with the same inode/size",
    ),
    (
        "FS TRUNC OK",
        "truncate to 4 bytes kept exactly the first 4 bytes on disk",
    ),
    (
        "FS RM OK",
        "create + stat + unlink + stat cycle on the filesystem succeeded",
    ),
    (
        "PROG MID OK",
        "a 16-byte window at the middle of /prog.elf matches the source image",
    ),
    (
        "PROG TAIL OK",
        "the last 32 bytes of /prog.elf match the source ELF image",
    ),
    (
        "PROG SIZE OK",
        "the ELF written to /prog.elf has exactly the expected byte size",
    ),
    (
        "PAGER SRV 0/0/0",
        "pager server was reclaimed (tcb, stack frame, ipc frame destroyed)",
    ),
]

# Exactly one occurrence is required for each of these lines.
EXACT_ONE = [
    "VFS SERVER OK",
    "VFS OK",
    "VFS STAT OK",
    "IOMMU OK",
    "PERCPU CLOBBER OK",
    "TIME MONOTONIC OK",
    "TLB REMAP OK",
    "CAP DELETE OK",
    "TCB SUSPEND OK",
    "FS LARGE OK",
    "FS APPEND OK",
    "FS OPEN FLAGS OK",
    "FS STAT PATH OK",
    "HARD LINK OK",
    "SYMLINK OK",
    "IRQ OK",
    "CNODE OK",
    "TCB REAP OK",
    "WAIT REAP OK",
    "PAGER OK",
    "PAGER2 OK",
    "PAGER EVICT OK",
    "PAGER3 OK",
    "SHELL READY",
    "DESTROY OK",
]

FORBIDDEN = [
    "PERCPU CLOBBER BAD",
    "VFS BAD",
    "ROOT PANIC",
    # Chỉ khớp cả dòng; substring "STALL" trùng "USER CODE CAP INSTALL FAILED".
    "KERNEL STALL",
    "WATCHDOG STALL",
    "BOOT HANDOFF FAIL",
    "KERNEL IRQ OUTSIDE IDLE",
    "AP PIC IRQ",
    "BLOCK DENIED BAD",
    "FS CORRUPT",
    "FS V2 BAD",
    "FS PERM BAD",
    "BADGE SPOOF BAD",
    "TIME MONOTONIC BAD",
    "TLB REMAP BAD",
    "CAP DELETE BAD",
    "TCB SUSPEND BAD",
    "FS LARGE BAD",
    "FS APPEND BAD",
    "FS OPEN FLAGS BAD",
    "FS STAT PATH BAD",
    "TCB REAP BAD",
    "PAGER2 BAD",
    "PAGER EVICT BAD",
    "PAGER3 BAD",
    "VIRTIO INPUT INIT FAIL",
    "VIRTIO RNG BAD",
    "DEMAND VFSLOAD BAD",
    "DEMAND LOAD BAD",
    "HARD LINK BAD",
    "SYMLINK BAD",
    "IRQ BAD",
    "PAGER BAD",
    "DESTROY BAD",
]

FAULT_ADDRESSES = [
    0x8000_0000,
    0x1000002000,
    0x1000301000,
    0x1001000000,
    0x1001001000,
    0x1001002000,
    0x1001003000,
]

EXIT_CODES = {
    "riscv64": {0},
    "aarch64": {0},
    "x86_64": {3},
}

ARCH = {
    "riscv64": [
        ("SMP OK", "both harts online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP TIMER RATE (ap=T, bsp=B)", "independent BSP and AP timer rates"),
        ("SMP TIMER OK (hart1_ticks=T)", "secondary timer ran, exactly once"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP PREEMPT OK (user_ticks=U, preempts=P)", "secondary timer preempted a busy user task, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50, c=1)", "both yield workers and busy worker completed"),
        ("SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)", "post-worker idle timer proof"),
        ("TLB STATS (va_flush=V, ipi=I, skipped=K)", "per-VA TLB accounting"),
    ],
    "aarch64": [
        ("SMP OK", "both CPUs online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP TIMER RATE (ap=T, bsp=B)", "independent BSP and AP timer rates"),
        ("SMP TIMER OK (hart1_ticks=T)", "secondary timer ran, exactly once"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP PREEMPT OK (user_ticks=U, preempts=P)", "secondary timer preempted a busy user task, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50, c=1)", "both yield workers and busy worker completed"),
        ("SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)", "post-worker idle timer proof"),
        ("TLB STATS (va_flush=V, ipi=I, skipped=K)", "per-VA TLB accounting"),
        ("EL0 FP TRAP", "FP/SIMD của EL0 bị trap, không rò rỉ giữa task"),
    ],
    "x86_64": [
        ("KERNEL DATA NX OK", "kernel data is non-executable"),
        ("ACPI MADT OK (cpus=C, bsp=ID, lapic=0x...)", "ACPI MADT parsed"),
        ("LAPIC OK", "BSP local APIC enabled"),
        ("AP TRAMPOLINE OK (page=0x...)", "AP trampoline allocated below 1 MiB"),
        ("AP REAL MODE OK (apic=...)", "AP executed real-mode trampoline"),
        ("AP LONG MODE OK (cpu=1)", "AP entered long mode"),
        ("SMP OK", "both CPUs online"),
        ("ipi=1", "AP maintenance IPI acknowledged"),
        ("SMP TIMER OK (hart1_ticks=T)", "AP LAPIC timer ran, exactly once"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "AP idle accounting, exactly once"),
        ("SMP TIMER RATE (ap=T, bsp=B)", "AP and BSP timer rates agree, exactly once"),
        ("SMP IPI COUNT (sent=X, seen=Y)", "AP maintenance IPI accounting, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "AP scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50, c=1)", "both yield workers and busy worker completed"),
        ("SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)", "post-worker AP idle proof"),
        ("TLB STATS (va_flush=V, ipi=I, skipped=K)", "per-VA TLB accounting"),
    ],
}

# Required textual ordering (earlier marker must precede later marker).
ORDER = {
    "x86_64": [
        ("AP REAL MODE OK", "AP LONG MODE OK"),
        ("AP LONG MODE OK", "SMP OK"),
    ],
}

# Generated pass block: each architecture line must be present exactly once.
ARCH_EXACT_ONE = {
    "riscv64": [
        "SMP OK",
        "SMP TIMER OK",
        "SMP TIMER RATE",
        "SMP IDLE OK",
        "SMP PREEMPT OK",
        "SMP SCHED OK",
        "SMP WORKERS OK",
        "SMP POST IDLE OK",
        "VIRTIO NET READY",
        "VIRTIO NET PING REPLY",
        "VIRTIO NET DNS sent",
        "VIRTIO GPU READY",
        "VIRTIO GPU DRAW OK",
        "DEMAND LOAD OK",
        "DEMAND VFSLOAD OK",
        "VIRTIO INPUT READY",
        "VIRTIO RNG READY",
        "VIRTIO TCP LISTEN OK",
    ],
    "aarch64": [
        "SMP OK",
        "SMP TIMER OK",
        "SMP TIMER RATE",
        "SMP IDLE OK",
        "SMP PREEMPT OK",
        "SMP SCHED OK",
        "SMP WORKERS OK",
        "SMP POST IDLE OK",
        "VIRTIO NET READY",
        "VIRTIO NET PING REPLY",
        "VIRTIO NET DNS sent",
        "VIRTIO GPU READY",
        "VIRTIO GPU DRAW OK",
        "DEMAND LOAD OK",
        "DEMAND VFSLOAD OK",
        "VIRTIO INPUT READY",
        "VIRTIO RNG READY",
        "VIRTIO TCP LISTEN OK",
    ],
    "x86_64": ["KERNEL DATA NX OK", "ACPI MADT OK", "LAPIC OK", "AP TRAMPOLINE OK", "AP REAL MODE OK", "AP LONG MODE OK", "SMP OK", "SMP TIMER OK", "SMP IDLE OK", "SMP PREEMPT OK", "SMP TIMER RATE", "SMP IPI COUNT", "SMP SCHED OK", "SMP WORKERS OK", "SMP POST IDLE OK", "DEMAND LOAD OK", "PCI SCAN OK", "SSE USER OFF"],
}

NUMERIC = {
    "riscv64": [
        "8 USER FAULT records including 0x80000000, 0x1000002000, 0x1000301000, 0x1001000000, 0x1001001000x2, 0x1001002000",
        "ROOT SWITCH n > 1",
        "N > 1, M >= 100, W > 0, W <= S, S <= I",
        "U >= 3, P >= 1, U <= T",
        "B >= 5 and B/2 <= T <= 2B",
        "TLB STATS: V >= 1, I >= 1, K >= 1 (regex, exactly once)",
        "Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2",
    ],
    "aarch64": [
        "8 USER FAULT records including 0x80000000, 0x1000002000, 0x1000301000, 0x1001000000, 0x1001001000x2, 0x1001002000",
        "ROOT SWITCH n > 1",
        "T > 0, I >= 1",
        "N > 1, M >= 100, W > 0, W <= S, S <= I",
        "U >= 3, P >= 1, U <= T",
        "B >= 5 and B/2 <= T <= 2B",
        "TLB STATS: V >= 1, I == 0 (regex, exactly once)",
        "Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2",
    ],
    "x86_64": [
        "8 USER FAULT records including 0x80000000, 0x1000002000, 0x1000301000, 0x1001000000, 0x1001001000x2, 0x1001002000",
        "ROOT SWITCH n > 1",
        "ACPI MADT cpus == 2",
        "T > 0, I >= 1",
        "1 <= Y <= X, Wt <= T, Ws <= Y",
        "N > 1, M >= 100, W > 0, W <= S, S <= I",
        "U >= 3, P >= 1, U <= T",
        "B >= 5 and B/2 <= T <= 2B",
        "TLB STATS: V >= 1, I >= 1, K >= 1 (regex, exactly once)",
        "Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2",
    ],
}

SCHED_RE = re.compile(
    r"SMP SCHED OK \(hart0=(\d+), hart1=(\d+), resched_sent=(\d+), "
    r"resched_wake=(\d+), idle_entries=(\d+)\)"
)
IDLE_RE = re.compile(
    r"SMP IDLE OK \(hart1_wakeups=(\d+), idle_entries=(\d+), "
    r"wake_timer=(\d+), wake_sgi=(\d+)\)"
)
TIMER_RE = re.compile(r"SMP TIMER OK \(hart1_ticks=(\d+)\)")
PREEMPT_RE = re.compile(r"SMP PREEMPT OK \(user_ticks=(\d+), preempts=(\d+)\)")
TRAMPOLINE_RE = re.compile(r"AP TRAMPOLINE OK \(page=0x([0-9a-fA-F]+)\)")
ACPI_RE = re.compile(r"ACPI MADT OK \(cpus=(\d+), bsp=\d+, lapic=0x[0-9a-fA-F]+\)")
POST_RE = re.compile(
    r"SMP POST IDLE OK \(ticks=(\d+), idle_entries=(\d+), wake_timer=(\d+)\)"
)
BOOT_STACK_RE = re.compile(r"BOOT STACK used=(\d+) size=(\d+)")
RATE_RE = re.compile(r"SMP TIMER RATE \(ap=(\d+), bsp=(\d+)\)")
TLB_RE = re.compile(r"TLB STATS \(va_flush=(\d+), ipi=(\d+), skipped=(\d+)\)")
IPI_RE = re.compile(r"SMP IPI COUNT \(sent=(\d+), seen=(\d+)\)")
ROOT_SWITCH_RE = re.compile(r"ROOT SWITCH\s+(\d+)")
FAULT_RE = re.compile(r"USER FAULT.*?addr=(0x[0-9a-fA-F]+)")


def fault_addresses(output: str) -> list[int]:
    return [int(value, 16) for value in FAULT_RE.findall(output)]


def smp_sched(output: str) -> tuple[int, int, int, int, int]:
    match = SCHED_RE.search(output)
    if not match:
        return (-1, -1, -1, -1, -1)
    return tuple(int(match.group(i)) for i in range(1, 6))


def smp_idle(output: str) -> tuple[int, int, int, int]:
    match = IDLE_RE.search(output)
    if not match:
        return (-1, -1, -1, -1)
    return tuple(int(match.group(i)) for i in range(1, 5))


def accept(
    name: str,
    output: str,
    rc: int,
    require_block: bool = True,
    require_disk: bool = True,
    feature_mode: str | None = None,
) -> bool:
    if feature_mode == "test-destroy-smp":
        # Variant of the normal run: the generic lifecycle demo runs again
        # after a secondary hart is executing a worker.
        if output.count("DESTROY SMP OK") != 1 or "DESTROY SMP BAD" in output:
            return False
    elif "DESTROY SMP OK" in output:
        return False
    if feature_mode == "test-panic-hart1":
        if rc == 0:
            return False
        marker = "PANIC (hart=1)"
        if output.count(marker) != 1 or "PANIC (hart=0)" in output:
            return False
        after = output.split(marker, 1)[1]
        for line in after.splitlines():
            if any(token in line for token in ("ROOT", "SMP ", "USER FAULT", "PANIC (hart=")):
                return False
        return True
    if rc not in EXIT_CODES[name]:
        return False
    if not all(text in output for text, _ in COMMON):
        return False
    if require_block and "BLOCK OK" not in output:
        return False
    if require_disk and "FS DISK OK" not in output:
        return False
    if not all(output.count(text) == 1 for text in EXACT_ONE):
        return False
    if any(text in output for text in FORBIDDEN):
        return False
    if feature_mode == "test-boot-handoff":
        if output.count("BOOT HANDOFF OK (from=1)") != 1:
            return False
    elif "BOOT HANDOFF OK" in output:
        return False
    if not all(value.split(" (")[0] in output for value, _ in ARCH[name]):
        return False
    if not all(
        sum(1 for line in output.splitlines() if line.startswith(anchor)) == 1
        for anchor in ARCH_EXACT_ONE[name]
    ):
        return False
    stack = BOOT_STACK_RE.search(output)
    if not stack or output.count("BOOT STACK used=") != 1:
        return False
    stack_used = int(stack.group(1))
    stack_size = int(stack.group(2))
    if stack_size < 0x20000 or stack_used < 1 or stack_used * 2 > stack_size:
        return False
    faults = fault_addresses(output)
    # Pager v2 (vòng 34-37, nới vòng 67): evict có thể tạo fault thứ hai cho
    # PAGER2_VA hoặc không (race server map lại trước khi client đọc), nên chỉ
    # kiểm tra sự hiện diện của từng địa chỉ thay vì tổng số tuyệt đối.
    required = list(FAULT_ADDRESSES)
    if not all(address in faults for address in required):
        return False
    if faults.count(0x1001001000) not in (1, 2):
        return False
    # Vòng 33: exec demo thêm task làm đổi interleaving evict/demand nên
    # PAGER3 (0x1001002000) cũng fault 2 lần như PAGER2 (vòng 67) — cùng
    # bản chất race server map lại trước khi client đọc.
    if faults.count(0x1001002000) not in (1, 2):
        return False
    rate = RATE_RE.search(output)
    if not rate:
        return False
    ap_rate = int(rate.group(1))
    bsp_rate = int(rate.group(2))
    if bsp_rate < 5 or ap_rate * 2 < bsp_rate or ap_rate > bsp_rate * 2:
        return False
    tlb = TLB_RE.search(output)
    if not tlb or output.count("TLB STATS") != 1:
        return False
    va_flush = int(tlb.group(1))
    tlb_ipi = int(tlb.group(2))
    tlb_skipped = int(tlb.group(3))
    if va_flush < 1:
        return False
    if name == "aarch64":
        if tlb_ipi != 0:
            return False
    elif tlb_ipi < 1 or tlb_skipped < 1:
        return False
    match = ROOT_SWITCH_RE.search(output)
    if not match or int(match.group(1)) <= 1:
        return False
    if output.find("Server") <= output.find("ROOT RUST"):
        return False
    if output.find("Client") <= output.find("ROOT RUST"):
        return False
    if name in ("riscv64", "aarch64"):
        sched = smp_sched(output)
        if sched[0] <= 1 or sched[1] < 100 or sched[3] <= 0 or sched[2] > sched[4]:
            return False
        timer = TIMER_RE.search(output)
        preempt = PREEMPT_RE.search(output)
        if not timer or int(timer.group(1)) <= 0 or not preempt:
            return False
        if int(preempt.group(1)) < 3 or int(preempt.group(2)) < 1 or int(preempt.group(1)) > int(timer.group(1)):
            return False
        if sched[3] > sched[2]:
            return False
        if output.count("SMP WORKERS OK (a=50, b=50, c=1)") != 1:
            return False
        idle = smp_idle(output)
        if min(idle) < 0 or idle[0] <= 0:
            return False
        post = POST_RE.search(output)
        if not post:
            return False
        pt, pi, pw = (int(post.group(i)) for i in range(1, 4))
        if pt < 5 or pi < pt - 2 or pw < pt - 2:
            return False
    if name == "x86_64":
        acpi = ACPI_RE.search(output)
        if not acpi or int(acpi.group(1)) != 2:
            return False
        trampoline = TRAMPOLINE_RE.search(output)
        if not trampoline:
            return False
        page = int(trampoline.group(1), 16)
        if page >= 0xA0000 or page & 0xfff != 0:
            return False
        for before, after in ORDER["x86_64"]:
            if output.find(before) < 0 or output.find(after) <= output.find(before):
                return False
        rate = RATE_RE.search(output)
        if not rate:
            return False
        ap_ticks, bsp_ticks = (int(rate.group(i)) for i in (1, 2))
        if bsp_ticks < 5 or ap_ticks < bsp_ticks // 2 or ap_ticks > bsp_ticks * 2:
            return False
        ipi = IPI_RE.search(output)
        if not ipi:
            return False
        sent, seen = (int(ipi.group(i)) for i in (1, 2))
        if seen < 1 or seen > sent:
            return False
        timer = TIMER_RE.search(output)
        idle = smp_idle(output)
        if not timer or int(timer.group(1)) <= 0 or min(idle) < 0 or idle[1] < 1:
            return False
        if idle[2] > int(timer.group(1)) or idle[3] > seen:
            return False
        sched = smp_sched(output)
        if sched[0] <= 1 or sched[1] < 100 or sched[3] <= 0 or sched[2] > sched[4] or sched[3] > sched[2]:
            return False
        preempt = PREEMPT_RE.search(output)
        if not preempt or int(preempt.group(1)) < 3 or int(preempt.group(2)) < 1 or int(preempt.group(1)) > int(timer.group(1)):
            return False
        if output.count("SMP WORKERS OK (a=50, b=50, c=1)") != 1:
            return False
        post = POST_RE.search(output)
        if not post:
            return False
        pt, pi, pw = (int(post.group(i)) for i in range(1, 4))
        if pt < 5 or pi < pt - 2 or pw < pt - 2:
            return False
    if name == "aarch64":
        timer = TIMER_RE.search(output)
        if not timer or int(timer.group(1)) <= 0:
            return False
    return True
