"""Shared QEMU acceptance conditions.

This module is imported by ``test-qemu.py`` and by ``gen-docs.py`` so the
summary cannot silently drift away from the harness.  Adding a condition here
also adds it to the harness (and therefore only tightens acceptance).
"""

from __future__ import annotations

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
]

FAULT_ADDRESSES = [0x8000_0000, 0x1000002000, 0x1000301000]

ARCH = {
    "riscv64": [
        ("SMP OK", "both harts online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50)", "both secondary workers completed"),
    ],
    "aarch64": [
        ("SMP OK", "both CPUs online"),
        ("ipi=1", "maintenance IPI acknowledged"),
        ("SMP TIMER OK (hart1_ticks=T)", "secondary timer ran, exactly once"),
        ("SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)", "idle accounting, exactly once"),
        ("SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)", "secondary scheduling, exactly once"),
        ("SMP WORKERS OK (a=50, b=50)", "both secondary workers completed"),
    ],
    "x86_64": [
        ("KERNEL DATA NX OK", "kernel data is non-executable"),
    ],
}

NUMERIC = {
    "riscv64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
        "N > 1, M >= 100, W > 0, S <= I",
    ],
    "aarch64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
        "T > 0, I >= 1, K >= Wt + Ws",
        "N > 1, M >= 100, W > 0, S <= I",
    ],
    "x86_64": [
        "4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000",
        "ROOT SWITCH n > 1",
    ],
}
