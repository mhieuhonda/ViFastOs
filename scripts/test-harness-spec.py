#!/usr/bin/env python3
"""Negative tests for every condition in harness_spec.accept()."""

from __future__ import annotations

import copy
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness_spec as spec


def fixture(arch: str) -> str:
    lines = ["ROOT RUST"]
    lines += [text for text, _ in spec.COMMON if text != "ROOT RUST"]
    lines += list(spec.EXACT_ONE)
    if arch == "riscv64":
        lines += [
            "SMP OK (harts=2, ipi=1)",
            "SMP IDLE OK (hart1_wakeups=5, idle_entries=5, wake_timer=0, wake_sgi=5)",
            "SMP SCHED OK (hart0=100, hart1=200, resched_sent=1, resched_wake=1, idle_entries=5)",
            "SMP WORKERS OK (a=50, b=50)",
            "SMP POST IDLE OK (ticks=6, idle_entries=6, wake_timer=5)",
        ]
    elif arch == "aarch64":
        lines += [
            "SMP OK (harts=2, psci=0, ipi=1)",
            "SMP TIMER OK (hart1_ticks=20)",
            "SMP IDLE OK (hart1_wakeups=2, idle_entries=2, wake_timer=0, wake_sgi=2)",
            "SMP SCHED OK (hart0=100, hart1=200, resched_sent=1, resched_wake=1, idle_entries=2)",
            "SMP WORKERS OK (a=50, b=50)",
            "SMP POST IDLE OK (ticks=6, idle_entries=6, wake_timer=5)",
        ]
    else:
        lines += [
            "KERNEL DATA NX OK",
            "ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000)",
            "LAPIC OK (id=0)",
            "AP TRAMPOLINE OK (page=0x90000)",
            "AP REAL MODE OK (apic=1)",
            "SMP SKIP (x86 AP bootstrap pending)",
        ]
    for address in [0] + spec.FAULT_ADDRESSES:
        lines.append(f"USER FAULT tcb=None cause=0x0 addr=0x{address:x}")
    lines.append("ROOT SWITCH 2")
    return "\n".join(lines) + "\n"


def main() -> int:
    cases = 0
    failures: list[str] = []

    def expect_fail(name: str, arch: str, output: str, rc: int) -> None:
        nonlocal cases
        cases += 1
        if spec.accept(arch, output, rc):
            failures.append(name)

    for arch in ("riscv64", "aarch64", "x86_64"):
        base = fixture(arch)
        rc = next(iter(spec.EXIT_CODES[arch]))
        assert spec.accept(arch, base, rc), f"fixture invalid for {arch}"

        for text, _ in spec.COMMON:
            expect_fail(f"{arch}: common {text!r}", arch, base.replace(text, ""), rc)
        for text in spec.EXACT_ONE:
            expect_fail(f"{arch}: exact {text!r}", arch, base.replace(text, ""), rc)
            expect_fail(f"{arch}: duplicate {text!r}", arch, base + text + "\n", rc)
        for text, _ in spec.ARCH[arch]:
            anchor = text.split(" (")[0]
            expect_fail(f"{arch}: arch {anchor!r}", arch, base.replace(anchor, ""), rc)
        for address in spec.FAULT_ADDRESSES:
            line = f"USER FAULT tcb=None cause=0x0 addr=0x{address:x}\n"
            expect_fail(f"{arch}: fault {address:#x}", arch, base.replace(line, ""), rc)
        expect_fail(f"{arch}: root switch", arch, base.replace("ROOT SWITCH 2", "ROOT SWITCH 1"), rc)
        for forbidden in spec.FORBIDDEN:
            expect_fail(f"{arch}: forbidden {forbidden!r}", arch, base + forbidden + "\n", rc)
        bad_rc = next(code for code in (0, 3) if code not in spec.EXIT_CODES[arch])
        expect_fail(f"{arch}: exit code", arch, base, bad_rc)

        if arch in ("riscv64", "aarch64"):
            variants = {
                "hart0": base.replace("hart0=100", "hart0=1"),
                "hart1": base.replace("hart1=200", "hart1=99"),
                "wake": base.replace("resched_wake=1", "resched_wake=0"),
                "sent": base.replace("resched_sent=1", "resched_sent=6"),
                "idle-zero": base.replace("hart1_wakeups=5", "hart1_wakeups=0")
                .replace("hart1_wakeups=2", "hart1_wakeups=0"),
                "workers": base.replace("SMP WORKERS OK (a=50, b=50)\n", ""),
                "post-ticks": base.replace("ticks=6", "ticks=4"),
                "post-entries": base.replace("idle_entries=6", "idle_entries=1"),
                "post-wake": base.replace("wake_timer=5", "wake_timer=0"),
                "block": base.replace("BLOCK OK", ""),
                "disk": base.replace("FS DISK OK", ""),
            }
            for label, output in variants.items():
                expect_fail(f"{arch}: numeric {label}", arch, output, rc)
        if arch == "aarch64":
            expect_fail(
                "aarch64: timer zero",
                arch,
                base.replace("SMP TIMER OK (hart1_ticks=20)", "SMP TIMER OK (hart1_ticks=0)"),
                rc,
            )
        if arch == "x86_64":
            expect_fail(
                "x86_64: acpi count",
                arch,
                base.replace("cpus=2", "cpus=1"),
                rc,
            )
            expect_fail(
                "x86_64: block",
                arch,
                base.replace("BLOCK OK", ""),
                rc,
            )
            expect_fail(
                "x86_64: disk",
                arch,
                base.replace("FS DISK OK", ""),
                rc,
            )

    feature_base = fixture("riscv64") + "BOOT HANDOFF OK (from=1)\n"
    cases += 1
    if not spec.accept(
        "riscv64", feature_base, 0, feature_mode="test-boot-handoff"
    ):
        failures.append("riscv64: feature handoff accepted")
    cases += 1
    if spec.accept("riscv64", fixture("riscv64"), 0, feature_mode="test-boot-handoff"):
        failures.append("riscv64: feature handoff missing")
    cases += 1
    if spec.accept("riscv64", feature_base, 0):
        failures.append("riscv64: normal run accepted handoff marker")

    if failures:
        print("harness negative tests FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print(f"harness negative tests PASS ({cases} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
