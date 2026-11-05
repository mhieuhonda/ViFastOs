#!/usr/bin/env python3
"""Positive documentation consistency checks used before bundling."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUMMARY = ROOT / "ViFastOs-architecture-summary.txt"
HANDOFF = ROOT / "docs" / "ViFastOs-agent-handoff.md"


def fail(message: str) -> None:
    print(f"docs check: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> int:
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "gen_docs", ROOT / "scripts" / "gen-docs.py"
    )
    if spec is None or spec.loader is None:
        fail("cannot load scripts/gen-docs.py")
    gen_docs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gen_docs)
    import harness_spec

    summary = SUMMARY.read_text()
    handoff = HANDOFF.read_text()
    text = summary + "\n" + handoff

    required_markers = [
        "### BEGIN GENERATED commits",
        "### BEGIN GENERATED syscalls",
        "### BEGIN GENERATED tests",
        "### BEGIN GENERATED pass",
        "### BEGIN GENERATED status",
    ]
    for marker in required_markers:
        if marker not in summary:
            fail(f"missing generated marker: {marker}")

    expected = gen_docs.render()
    if expected != summary:
        fail("summary generated sections are stale; run scripts/gen-docs.py")

    forbidden = [
        "hart phụ idle",
        "hart phụ chạy scheduler",
        "chỉ idle",
        "chưa vào scheduler",
        "chưa chọn task",
        "maintenance-only",
        "resched=R",
        "AArch64/x86 hart phụ",
        "AArch64 hart phụ scheduler",
        "AArch64/x86 chưa có scheduler",
        "AArch64 chưa",
        "(hart0, hart1, resched)",
        "458.336",
    ]
    for value in forbidden:
        if value in summary or value in handoff:
            fail(f"stale string present: {value}")

    if "SMP TIMER OK (hart1_ticks=T)" not in summary:
        fail("missing AArch64 timer contract in summary")

    generated = gen_docs.pass_conditions().splitlines()
    generated_count = sum(
        1
        for line in generated
        if (line.startswith("- ") or line.startswith("  - "))
        and line not in ("- riscv64:", "- aarch64:", "- x86_64:")
    )
    expected_count = (
        len(harness_spec.COMMON)
        + len(harness_spec.EXACT_ONE)
        + len(harness_spec.FORBIDDEN)
        + sum(len(values) for values in harness_spec.ARCH.values())
        + sum(len(values) for values in harness_spec.NUMERIC.values())
        + len(harness_spec.EXIT_CODES)
    )
    if generated_count != expected_count:
        fail(
            f"generated pass condition count {generated_count} != spec {expected_count}"
        )

    print("docs check: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
