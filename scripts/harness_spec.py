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
    ("Mapped", "client VSpace mapping succeeded"),
    ("Cap OK", "capability transfer demo succeeded"),
    ("ViFastOS ramfs", "ramfs read path ran"),
    ("ViFastOS write", "ramfs write path ran"),
    ("IPC BUF OK", "IPC buffer transfer succeeded"),
    ("Reply OK", "reply path succeeded"),
    ("ROOT RUST", "root Rust ELF entered user mode"),
    ("BOOTINFO OK", "root validated BootInfo v2"),
    ("RUST MODULES OK", "server/client Rust ELF modules loaded"),
    ("CLIENT VSPACE OK", "client VSpace built"),
    ("FAULT EP OK", "fault endpoint delivered all fault records"),
    ("ROOT READY", "root finished initialisation"),
    ("Zero OK", "zero page / BSS path verified"),
    ("BLOCK OK", "block device backend responded"),
    ("FS DISK OK", "filesystem disk path responded"),
]

# Exactly one occurrence is required for each of these lines.
EXACT_ONE = [
    "VFS SERVER OK",
    "VFS OK",
    "VFS STAT OK",
    "IOMMU OK",
    "PERCPU CLOBBER OK",
]

FORBIDDEN = [
    "PERCPU CLOBBER BAD",
    "VFS BAD",
    "ROOT PANIC",
    "STALL",
    "BOOT HANDOFF FAIL",
]

FAULT_ADDRESSES = [0x8000_0000, 0x1000002000, 0x1000301000]

EXIT_CODES = {
    "riscv64": {0},
    "aarch64": {0},
    "x86_64": {3},
}

ARCH = {
    "riscv64": [
        ("SMP OK", "both harts online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50)", "both secondary workers completed"),
        ("SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)", "post-worker idle timer proof"),
    ],
    "aarch64": [
        ("SMP OK", "both CPUs online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP TIMER OK (hart1_ticks=T)", "secondary timer ran, exactly once"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50)", "both secondary workers completed"),
        ("SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)", "post-worker idle timer proof"),
    ],
    "x86_64": [
        ("KERNEL DATA NX OK", "kernel data is non-executable"),
        ("ACPI MADT OK (cpus=C, bsp=ID, lapic=0x...)", "ACPI MADT parsed"),
        ("LAPIC OK", "BSP local APIC enabled"),
        ("AP TRAMPOLINE OK (page=0x...)", "AP trampoline allocated below 1 MiB"),
        ("AP REAL MODE OK (apic=...)", "AP executed real-mode trampoline"),
        ("AP LONG MODE OK (cpu=1)", "AP entered long mode"),
        ("SMP SKIP", "x86 AP bootstrap pending"),
    ],
}

# Generated pass block: each architecture line must be present exactly once.
ARCH_EXACT_ONE = {
    "riscv64": [
        "SMP OK",
        "SMP IDLE OK",
        "SMP SCHED OK",
        "SMP WORKERS OK",
        "SMP POST IDLE OK",
    ],
    "aarch64": [
        "SMP OK",
        "SMP TIMER OK",
        "SMP IDLE OK",
        "SMP SCHED OK",
        "SMP WORKERS OK",
        "SMP POST IDLE OK",
    ],
    "x86_64": ["KERNEL DATA NX OK", "ACPI MADT OK", "LAPIC OK", "AP TRAMPOLINE OK", "AP REAL MODE OK", "AP LONG MODE OK", "SMP SKIP"],
}

NUMERIC = {
    "riscv64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
        "N > 1, M >= 100, W > 0, S <= I",
        "Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2",
    ],
    "aarch64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
        "T > 0, I >= 1",
        "N > 1, M >= 100, W > 0, S <= I",
        "Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2",
    ],
    "x86_64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
        "ACPI MADT cpus == 2",
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
TRAMPOLINE_RE = re.compile(r"AP TRAMPOLINE OK \(page=0x([0-9a-fA-F]+)\)")
ACPI_RE = re.compile(r"ACPI MADT OK \(cpus=(\d+), bsp=\d+, lapic=0x[0-9a-fA-F]+\)")
POST_RE = re.compile(
    r"SMP POST IDLE OK \(ticks=(\d+), idle_entries=(\d+), wake_timer=(\d+)\)"
)
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
    if not all(output.count(anchor) == 1 for anchor in ARCH_EXACT_ONE[name]):
        return False
    faults = fault_addresses(output)
    if len(faults) != 4:
        return False
    if not all(address in faults for address in FAULT_ADDRESSES):
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
        if output.count("SMP WORKERS OK (a=50, b=50)") != 1:
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
    if name == "aarch64":
        timer = TIMER_RE.search(output)
        if not timer or int(timer.group(1)) <= 0:
            return False
    return True
