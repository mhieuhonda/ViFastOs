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
            "SMP TIMER OK (hart1_ticks=20)",
            "SMP TIMER RATE (ap=20, bsp=20)",
            "SMP IDLE OK (hart1_wakeups=5, idle_entries=5, wake_timer=0, wake_sgi=5)",
            "SMP PREEMPT OK (user_ticks=4, preempts=2)",
            "SMP SCHED OK (hart0=100, hart1=200, resched_sent=1, resched_wake=1, idle_entries=5)",
            "SMP WORKERS OK (a=50, b=50, c=1)",
            "SMP POST IDLE OK (ticks=6, idle_entries=6, wake_timer=5)",
            "TLB STATS (va_flush=2, ipi=1, skipped=1)",
            "EL0 FP TRAP",
            "VIRTIO NET READY base=0x10007000 accepted=0x10020",
            "VIRTIO NET PING REPLY from 10.0.2.2",
            "VIRTIO NET DNS sent len=0",
            "VIRTIO GPU READY",
            "VIRTIO GPU DRAW OK fb=0x80368400 160x120",
            "DEMAND LOAD OK",
            "VIRTIO INPUT READY base=0x10005000",
            "DEMAND VFSLOAD OK",
            "VIRTIO RNG READY base=0x10004000",
            "VIRTIO TCP LISTEN OK port=8080",
        ]
    elif arch == "aarch64":
        lines += [
            "SMP OK (harts=2, psci=0, ipi=1)",
            "SMP TIMER OK (hart1_ticks=20)",
            "SMP TIMER RATE (ap=20, bsp=20)",
            "SMP IDLE OK (hart1_wakeups=2, idle_entries=2, wake_timer=0, wake_sgi=2)",
            "SMP PREEMPT OK (user_ticks=4, preempts=2)",
            "SMP SCHED OK (hart0=100, hart1=200, resched_sent=1, resched_wake=1, idle_entries=2)",
            "SMP WORKERS OK (a=50, b=50, c=1)",
            "SMP POST IDLE OK (ticks=6, idle_entries=6, wake_timer=5)",
            "TLB STATS (va_flush=2, ipi=1, skipped=1)",
            "EL0 FP TRAP",
            "VIRTIO NET READY base=0xa003c00 accepted=0x10020",
            "VIRTIO NET PING REPLY from 10.0.2.2",
            "VIRTIO NET DNS sent len=0",
            "VIRTIO GPU READY",
            "VIRTIO GPU DRAW OK fb=0x80368400 160x120",
            "DEMAND LOAD OK",
            "VIRTIO INPUT READY base=0xa003800",
            "DEMAND VFSLOAD OK",
            "VIRTIO RNG READY base=0xa003600",
            "VIRTIO TCP LISTEN OK port=8080",
        ]
    else:
        lines += [
            "KERNEL DATA NX OK",
            "PCI SCAN OK devices=4",
            "SSE USER OFF",
            "ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000)",
            "LAPIC OK (id=0)",
            "AP TRAMPOLINE OK (page=0x90000)",
            "AP REAL MODE OK (apic=1)",
            "AP LONG MODE OK (cpu=1)",
            "SMP OK (harts=2, ipi=1)",
            "SMP TIMER OK (hart1_ticks=20)",
            "SMP TIMER RATE (ap=20, bsp=20)",
            "SMP IDLE OK (hart1_wakeups=3, idle_entries=4, wake_timer=2, wake_sgi=1)",
            "SMP IPI COUNT (sent=2, seen=2)",
            "SMP PREEMPT OK (user_ticks=4, preempts=2)",
            "SMP SCHED OK (hart0=100, hart1=200, resched_sent=1, resched_wake=1, idle_entries=5)",
            "SMP WORKERS OK (a=50, b=50, c=1)",
            "SMP POST IDLE OK (ticks=6, idle_entries=6, wake_timer=5)",
            "TLB STATS (va_flush=2, ipi=1, skipped=1)",
            "EL0 FP TRAP",
            "DEMAND LOAD OK",
        ]
    # Vòng 179: bổ sung mọi marker ARCH mà fixture chưa liệt kê — nếu không,
    # thêm marker mới vào `harness_spec` sẽ làm fixture "invalid" một cách khó
    # hiểu (đã xảy ra với NET TCP CONNECT OK / NET TCP ECHO OK).
    for text, _ in spec.ARCH[arch]:
        token = text.split(" (")[0]
        if not any(token in line for line in lines):
            lines.append(text)
    addresses = list(spec.FAULT_ADDRESSES)
    for address in [0] + addresses:
        lines.append(f"USER FAULT tcb=None cause=0x0 addr=0x{address:x}")
    # Pager v2 evict demo: PAGER2_VA faults twice.
    lines.append("USER FAULT tcb=None cause=0x0 addr=0x1001001000")
    lines.append("BOOT STACK used=1000 size=196608")
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
                "workers": base.replace("SMP WORKERS OK (a=50, b=50, c=1)\n", ""),
                "preempt-u": base.replace("user_ticks=4", "user_ticks=2"),
                "preempt-p": base.replace("preempts=2", "preempts=0"),
                "preempt-over": base.replace("user_ticks=4", "user_ticks=21"),
                "w-gt-s": base.replace("resched_sent=1", "resched_sent=0"),
                "timer-zero": base.replace("SMP TIMER OK (hart1_ticks=20)", "SMP TIMER OK (hart1_ticks=0)"),
                "post-ticks": base.replace("ticks=6", "ticks=4"),
                "post-entries": base.replace("idle_entries=6", "idle_entries=1"),
                "post-wake": base.replace("wake_timer=5", "wake_timer=0"),
                "block": base.replace("BLOCK OK", ""),
                "disk": base.replace("FS DISK OK", ""),
            }
            for label, output in variants.items():
                expect_fail(f"{arch}: numeric {label}", arch, output, rc)
            if arch != "aarch64":
                expect_fail("tlb-v", arch, base.replace("TLB STATS (va_flush=2", "TLB STATS (va_flush=0"), rc)
                expect_fail("tlb-ipi", arch, base.replace("TLB STATS (va_flush=2, ipi=1", "TLB STATS (va_flush=2, ipi=0"), rc)
                expect_fail("tlb-skip", arch, base.replace("skipped=1)", "skipped=0)"), rc)
        if arch == "aarch64":
            expect_fail(
                "aarch64: tlb-ipi",
                arch,
                base.replace("TLB STATS (va_flush=2, ipi=1", "TLB STATS (va_flush=2, ipi=0"),
                rc,
            )
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
            variants = {
                "rate-high": base.replace("ap=20, bsp=20", "ap=1000, bsp=20"),
                "rate-low-b": base.replace("ap=20, bsp=20", "ap=20, bsp=4"),
                "ipi-zero": base.replace("seen=2", "seen=0"),
                "ipi-over": base.replace("seen=2", "seen=3"),
                "timer-wake": base.replace("wake_timer=2", "wake_timer=21"),
                "ipi-wake": base.replace("wake_sgi=1", "wake_sgi=3"),
                "sched-hart0": base.replace("hart0=100", "hart0=1"),
                "sched-hart1": base.replace("hart1=200", "hart1=99"),
                "sched-wake": base.replace("resched_wake=1", "resched_wake=0"),
                "sched-sent": base.replace("resched_sent=1", "resched_sent=6"),
                "post-ticks": base.replace("ticks=6", "ticks=4"),
                "post-entries": base.replace("idle_entries=6", "idle_entries=1"),
                "post-wake": base.replace("wake_timer=5", "wake_timer=0"),
                "workers": base.replace("SMP WORKERS OK (a=50, b=50, c=1)\n", ""),
                "preempt-u": base.replace("user_ticks=4", "user_ticks=2"),
                "preempt-p": base.replace("preempts=2", "preempts=0"),
                "preempt-over": base.replace("user_ticks=4", "user_ticks=21"),
                "w-gt-s": base.replace("resched_sent=1", "resched_sent=0"),
                "order-real-long": base.replace(
                    "AP REAL MODE OK (apic=1)\nAP LONG MODE OK (cpu=1)",
                    "AP LONG MODE OK (cpu=1)\nAP REAL MODE OK (apic=1)",
                ),
                "order-long-smp": base.replace(
                    "AP LONG MODE OK (cpu=1)\nSMP OK (harts=2, ipi=1)",
                    "SMP OK (harts=2, ipi=1)\nAP LONG MODE OK (cpu=1)",
                ),
            }
            for label, output in variants.items():
                expect_fail(f"x86_64: numeric {label}", arch, output, rc)
            expect_fail("x86_64: tlb-v", arch, base.replace("TLB STATS (va_flush=2", "TLB STATS (va_flush=0"), rc)
            expect_fail("x86_64: tlb-ipi", arch, base.replace("TLB STATS (va_flush=2, ipi=1", "TLB STATS (va_flush=2, ipi=0"), rc)
            expect_fail("x86_64: tlb-skip", arch, base.replace("skipped=1)", "skipped=0)"), rc)

    panic_base = "booting on x86_64\nPANIC (hart=1)\n"
    for arch in ("riscv64", "aarch64", "x86_64"):
        cases += 1
        if not spec.accept(arch, panic_base, 1, feature_mode="test-panic-hart1"):
            failures.append(f"{arch}: panic feature accepted")
        cases += 1
        if spec.accept(arch, panic_base, 0, feature_mode="test-panic-hart1"):
            failures.append(f"{arch}: panic zero exit accepted")
        for label, output in {
            "missing": "booting on x86_64\n",
            "hart0": "PANIC (hart=0)\n",
            "duplicate": panic_base + "PANIC (hart=1)\n",
            "after": panic_base + "ROOT SWITCH 2\n",
        }.items():
            cases += 1
            if spec.accept(arch, output, 0, feature_mode="test-panic-hart1"):
                failures.append(f"{arch}: panic {label} accepted")

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

    for arch in ("riscv64", "aarch64", "x86_64"):
        base = fixture(arch)
        rc = next(iter(spec.EXIT_CODES[arch]))
        cases += 1
        if spec.accept(arch, base.replace("BOOT STACK used=1000 size=196608\n", ""), rc):
            failures.append(f"{arch}: boot stack missing accepted")
        cases += 1
        if spec.accept(
            arch,
            base.replace("BOOT STACK used=1000", "BOOT STACK used=100000"),
            rc,
        ):
            failures.append(f"{arch}: boot stack over half accepted")
        cases += 1
        if spec.accept(
            arch, base.replace("BOOT STACK used=1000", "BOOT STACK used=0"), rc
        ):
            failures.append(f"{arch}: boot stack zero accepted")
        cases += 1
        if spec.accept(
            arch, base + "BOOT STACK used=1000 size=196608\n", rc
        ):
            failures.append(f"{arch}: boot stack duplicate accepted")

    for arch in ("riscv64", "aarch64", "x86_64"):
        destroy_base = fixture(arch) + "DESTROY SMP OK (frames=20, vspaces=2)\n"
        rc = next(iter(spec.EXIT_CODES[arch]))
        cases += 1
        if not spec.accept(arch, destroy_base, rc, feature_mode="test-destroy-smp"):
            failures.append(f"{arch}: destroy-smp feature rejected")
        cases += 1
        if spec.accept(arch, fixture(arch), rc, feature_mode="test-destroy-smp"):
            failures.append(f"{arch}: destroy-smp missing accepted")
        cases += 1
        if spec.accept(
            arch,
            destroy_base + "DESTROY SMP BAD (code=1)\n",
            rc,
            feature_mode="test-destroy-smp",
        ):
            failures.append(f"{arch}: destroy-smp bad marker accepted")
        cases += 1
        if spec.accept(
            arch,
            destroy_base + "DESTROY SMP OK (frames=20, vspaces=2)\n",
            rc,
            feature_mode="test-destroy-smp",
        ):
            failures.append(f"{arch}: destroy-smp duplicate accepted")
        cases += 1
        if spec.accept(arch, destroy_base, rc):
            failures.append(f"{arch}: normal run accepted destroy-smp marker")

    # r226 (T1/D11): marker `NET TCP ECHO OK` có điều kiện tiên quyết là harness
    # PHẢI có TcpProbeListener. Bốn tổ hợp dưới đây khóa đúng hành vi: thiếu
    # listener thì SKIP (không FAIL giả), có listener mà thiếu marker thì FAIL.
    echo_cases = [
        (False, "", True, "skip", "feature-mode không listener, không marker"),
        (False, "NET TCP ECHO OK\n", True, "skip", "feature-mode không listener, có marker"),
        (True, "", False, "fail", "có listener nhưng thiếu marker"),
        (True, "NET TCP ECHO OK\n", True, "ok", "có listener và có marker"),
    ]
    for listener, output, want_ok, label, desc in echo_cases:
        cases += 1
        ok, message = spec.tcp_echo_verdict(listener, output)
        if ok != want_ok:
            failures.append(f"tcp-echo: {desc} -> ok={ok}, cần {want_ok}")
        if label == "skip" and "SKIP" not in message:
            failures.append(f"tcp-echo: {desc} -> thông điệp thiếu SKIP: {message}")
        if label == "fail" and not message.startswith("FAIL"):
            failures.append(f"tcp-echo: {desc} -> thông điệp không phải FAIL: {message}")

    # r293.1 (tệp quyết định r293-r298 mục 2): harness phải ĐẾM retry và PHÂN
    # LOẠI lần hỏng. Case dương: một log giả có retry phải ra đúng dòng
    # `RETRY 1/N` kèm đúng loại. Case âm: nếu bỏ bộ đếm (hàm/khóa không còn)
    # thì phần này đỏ ngay — đúng yêu cầu "bỏ đếm thì test đỏ".
    import importlib.util

    qemu_path = Path(__file__).resolve().parent / "test-qemu.py"
    spec_qemu = importlib.util.spec_from_file_location("vifast_qemu", qemu_path)
    qemu = importlib.util.module_from_spec(spec_qemu)
    try:
        spec_qemu.loader.exec_module(qemu)
    except Exception as error:  # pragma: no cover - chỉ để báo lỗi rõ ràng
        failures.append(f"retry-count: không import được test-qemu.py: {error}")
        qemu = None

    if qemu is not None:
        # Phân loại: thiếu FS V2 OK ⇒ T-FS; thiếu PREEMPT OK ⇒ T-SCHED;
        # thiếu cả hai ⇒ T-FS (ưu tiên đúng thứ tự mục 2.5); lỗi lạ ⇒ T-KHAC.
        classify_cases = [
            ("booting on\nFS V2 OK\nPREEMPT OK\n", "T-KHAC", "đủ marker nhưng gọi phân loại"),
            # 294.C + D3: T-BOOT khi kernel chưa in dòng nào; T-ROOT kèm
            # `<file>:<dòng>`; cả hai dạng in (có/không dấu hai chấm) đều đọc được.
            ("", "T-BOOT", "không có dòng kernel nào"),
            (
                "booting on\nROOT PANIC\ncrates/vifast-root/src/main.rs:480\n",
                "T-ROOT crates/vifast-root/src/main.rs:480",
                "ROOT PANIC + file:có dấu hai chấm",
            ),
            (
                "booting on\nROOT PANIC\ncrates/vifast-root/src/main.rs480\n",
                "T-ROOT crates/vifast-root/src/main.rs:480",
                "ROOT PANIC + file KHÔNG dấu hai chấm (artifact cũ)",
            ),
            (
                "booting on\nSMP NO CURRENT hart=1 nr=5\nROOT PANIC\nmain.rs:9\n",
                "T-SCHED",
                "mất current được xét TRƯỚC panic (kế hoạch §4)",
            ),
            ("booting on\nPREEMPT OK\nSHELL READY\n", "T-FS", "thiếu FS V2 OK sau SHELL READY"),
            ("booting on\nFS V2 OK\n", "T-SCHED", "thiếu PREEMPT OK"),
            # Chết SỚM: thiếu cả FS V2 OK lẫn PREEMPT OK, nhưng có chữ ký
            # scheduler ⇒ phải là T-SCHED, KHÔNG được dán nhãn T-FS (lỗi phân
            # loại đã mắc ở phép đo nền đầu r293).
            (
                "booting on\nSHELL READY\nUSER FAULT tcb=None cause=0x0\nno runnable thread after fault\n",
                "T-SCHED",
                "chết sớm có chữ ký scheduler — không phải T-FS",
            ),
            (
                "booting on\nSHELL READY\nMEM OOM BAD\n",
                "T-FS",
                "thiếu FS V2 OK sau SHELL READY, không chữ ký ⇒ T-FS",
            ),

            ("booting on\nFS V2 OK\nPREEMPT OK\nUSER FAULT tcb=None \n", "T-SCHED", "USER FAULT tcb=None"),
            ("booting on\nFS V2 OK\nPREEMPT OK\nSMP NO READY (hart=0)\n", "T-SCHED", "SMP NO READY"),
            (
                "booting on\nFS V2 OK\nPREEMPT OK\nKERNEL FAULT (hart=1, cause=0xd) — bỏ qua\n",
                "T-SCHED",
                "KERNEL FAULT (Agent 0: phải đếm, không được nuốt)",
            ),
            ("booting on\nFS V2 OK\nPREEMPT OK\nchuyện lạ\n", "T-KHAC", "không khớp chữ ký nào"),
        ]
        for output, want, desc in classify_cases:
            cases += 1
            got = qemu.classify_failure(output)
            if got != want:
                failures.append(f"retry-count: classify {desc} -> {got}, cần {want}")

        # Dòng tổng kết: 1 retry trên 3 lần boot phải in đúng `RETRY 1/3`.
        cases += 1
        stats = {"retries": 1, "boots": 3, "classes": {"T-FS": 0, "T-SCHED": 1, "T-KHAC": 0}}
        line = qemu.format_retry_line(stats)
        if not line.startswith("RETRY 1/3 ") or "T-SCHED=1" not in line:
            failures.append(f"retry-count: dòng tổng kết sai: {line!r}")

        # 0 retry phải in `RETRY 0/N` (trung tính, không che flake bằng im lặng).
        cases += 1
        zero = qemu.format_retry_line(
            {"retries": 0, "boots": 7, "classes": {"T-FS": 0, "T-SCHED": 0, "T-KHAC": 0}}
        )
        if not zero.startswith("RETRY 0/7 "):
            failures.append(f"retry-count: dòng 0 retry sai: {zero!r}")

        # Artifact: mọi lần hỏng phải để lại serial/status/tail (r293.2).
        cases += 1
        for name in ("write_artifacts", "classify_failure", "format_retry_line"):
            if not hasattr(qemu, name):
                failures.append(f"retry-count: thiếu {name} (đã bị gỡ?)")

        source = qemu_path.read_text()
        # B1' (r294.B): MỌI đường hỏng phải đi qua MỘT hàm ghi artifact, và
        # hàm đó phải ghi serial.txt đầy đủ. Case âm: viết lại một hàm riêng
        # (như `dump_timeout_artifacts` cũ) là đỏ ngay.
        cases += 1
        if "def write_artifacts(" not in source:
            failures.append("B1': thiếu write_artifacts (đường ghi artifact dùng chung)")
        if "dump_timeout_artifacts(" in source or "save_flake_artifacts(" in source:
            failures.append("B1': còn đường ghi artifact riêng (phải gom về write_artifacts)")
        if "serial.txt" not in source:
            failures.append("B1': write_artifacts không ghi serial.txt")

        # B2': status.txt phải giữ dòng `ROOT PANIC` VÀ dòng kế tiếp
        # (`crates/.../main.rs<dòng>`) — đó là thứ phân loại T-ROOT.
        cases += 1
        if "panic_block" not in source:
            failures.append("B2': status.txt không có khối panic (mất file:dòng)")

        # --strict phải là cờ của harness (r293.3).
        cases += 1
        if '"--strict"' not in source:
            failures.append("retry-count: thiếu cờ --strict")

        # CASE ÂM (điều kiện nghiệm thu 293.1): bỏ chỗ ĐẾM trong `run_qemu` thì
        # phải đỏ. Chỉ kiểm hàm thuần là chưa đủ — gỡ lời gọi đếm vẫn để hàm
        # sống, nên phải soi thân hàm `run_qemu` có thật sự cộng bộ đếm không.
        cases += 1
        run_qemu_body = source.split("def run_qemu(", 1)[1].split("\ndef ", 1)[0]
        if 'RUN_STATS["retries"] += 1' not in run_qemu_body:
            failures.append(
                "retry-count: `run_qemu` không cộng RUN_STATS['retries'] (bỏ đếm là che flake)"
            )
        if "record_failure(" not in run_qemu_body:
            failures.append("retry-count: `run_qemu` không phân loại/ghi nhận lần hỏng")
        if source.count("write_artifacts(") < 2:
            failures.append("retry-count: không có chỗ GỌI write_artifacts (B1')")

    if failures:
        print("harness negative tests FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print(f"harness negative tests PASS ({cases} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
