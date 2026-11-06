#!/usr/bin/env python3
"""Soak runner: boot the QEMU gate repeatedly and report pass rates.

Dùng cho các đợt chạy dài (cuối tuần): mỗi vòng chạy `test-qemu.py --repeat 1`
cho cả 3 kiến trúc, đếm PASS/FAIL, và giữ lại serial + log của lần hỏng dưới
`artifacts/soak/` để điều tra flake. Không thay thế gate chuẩn (QEMU 3/3×3,
persist 7×3, harness) — đây là lớp kiểm tra độ ổn định bổ sung.

Ví dụ:
    python3 scripts/soak.py --iterations 20
    python3 scripts/soak.py --iterations 50 --archs riscv64,aarch64
    python3 scripts/soak.py --iterations 10 --with-persist
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ARTIFACTS = os.path.join(ROOT, "artifacts", "soak")


def run(cmd: list[str], env: dict[str, str]) -> tuple[int, str]:
    proc = subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return proc.returncode, proc.stdout.decode("utf-8", errors="replace")


def parse_summary(output: str) -> dict[str, tuple[int, int]]:
    """Đọc khối '=== summary ===' của test-qemu.py."""
    result: dict[str, tuple[int, int]] = {}
    lines = output.splitlines()
    in_summary = False
    for line in lines:
        if line.strip() == "=== summary ===":
            in_summary = True
            continue
        if not in_summary:
            continue
        stripped = line.strip()
        if not stripped:
            continue
        parts = stripped.split()
        # Dạng: "riscv64  3/3 PASS" (tên, tỉ lệ, PASS).
        ratio = next((p for p in parts if "/" in p), None)
        if ratio is not None:
            passed, total = ratio.split("/")
            if passed.isdigit() and total.isdigit():
                result[parts[0]] = (int(passed), int(total))
                continue
        # Dòng khác (ví dụ lỗi) kết thúc phần summary.
        if not stripped.startswith(("===", "---")):
            in_summary = False
    return result


def save_failure(iteration: int, output: str, env: dict[str, str]) -> str:
    directory = os.path.join(ARTIFACTS, f"iter-{iteration:03d}")
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, "run.log"), "w") as handle:
        handle.write(output)
    tmp = env.get("TMPDIR") or "/tmp"
    for name in ("vifast-riscv64.serial", "vifast-aarch64.serial"):
        source = os.path.join(tmp, name)
        if os.path.exists(source):
            shutil.copyfile(source, os.path.join(directory, name))
    for kept in os.listdir(tmp) if os.path.isdir(tmp) else []:
        if kept.startswith("vifast-keep-"):
            shutil.copyfile(os.path.join(tmp, kept), os.path.join(directory, kept))
    return directory


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument(
        "--archs",
        default="riscv64,aarch64,x86_64",
        help="comma separated architectures (default all three)",
    )
    parser.add_argument(
        "--with-persist",
        action="store_true",
        help="also run test-persist.py each iteration (slower)",
    )
    parser.add_argument(
        "--with-harness",
        action="store_true",
        help="also run test-harness-spec.py each iteration (fast)",
    )
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be at least 1")
    archs = [a.strip() for a in args.archs.split(",") if a.strip()]

    env = dict(os.environ)
    env["VIFAST_KEEP_SERIAL"] = "1"

    totals: dict[str, tuple[int, int]] = {arch: (0, 0) for arch in archs}
    failures: list[tuple[int, list[str], str]] = []
    started = time.time()
    for iteration in range(1, args.iterations + 1):
        arch_failures: list[str] = []
        output_all = ""
        for arch in archs:
            code, output = run(
                [
                    sys.executable,
                    os.path.join(ROOT, "scripts", "test-qemu.py"),
                    "--arch",
                    arch,
                    "--repeat",
                    "1",
                ],
                env,
            )
            output_all += f"\n===== iteration {iteration} {arch} =====\n{output}"
            summary = parse_summary(output)
            passed, total = summary.get(arch, (0, 1))
            prev_passed, prev_total = totals[arch]
            totals[arch] = (prev_passed + passed, prev_total + total)
            if code != 0 or passed != total:
                arch_failures.append(arch)
        if args.with_persist:
            code, output = run(
                [sys.executable, os.path.join(ROOT, "scripts", "test-persist.py")], env
            )
            output_all += f"\n===== iteration {iteration} persist =====\n{output}"
            if code != 0:
                arch_failures.append("persist")
        if args.with_harness:
            code, output = run(
                [
                    sys.executable,
                    os.path.join(ROOT, "scripts", "test-harness-spec.py"),
                ],
                env,
            )
            output_all += f"\n===== iteration {iteration} harness =====\n{output}"
            if code != 0:
                arch_failures.append("harness")
        status = "PASS" if not arch_failures else "FAIL " + ",".join(arch_failures)
        print(f"iteration {iteration:03d}/{args.iterations:03d}: {status}", flush=True)
        if arch_failures:
            directory = save_failure(iteration, output_all, env)
            failures.append((iteration, arch_failures, directory))
            print(f"  artifacts saved to {directory}", flush=True)

    elapsed = time.time() - started
    print("\n=== soak summary ===")
    for arch in archs:
        passed, total = totals[arch]
        rate = (100.0 * passed / total) if total else 0.0
        print(f"{arch:8} {passed}/{total} PASS ({rate:.1f}%)")
    if args.with_persist:
        print("persist  checked each iteration")
    if args.with_harness:
        print("harness  checked each iteration")
    print(f"iterations: {args.iterations}, elapsed: {elapsed:.1f}s")
    if failures:
        print("\nfailures:")
        for iteration, arch_failures, directory in failures:
            print(f"  iteration {iteration}: {','.join(arch_failures)} -> {directory}")
        return 1
    print("soak: all iterations passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
