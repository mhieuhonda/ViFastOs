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
        "SMP SKIP (x86 AP bootstrap pending)",
        "start_smp chỉ in SMP SKIP",
        "timer đa hart",
        "x86 sẽ theo D2",
        "x86 AP scheduler/user",
    ]
    for value in forbidden:
        if value in summary or value in handoff:
            fail(f"stale string present: {value}")

    if "SMP TIMER OK (hart1_ticks=T)" not in summary:
        fail("missing AArch64 timer contract in summary")

    bundle_rounds = sorted(
        int(path.stem.rsplit("-", 1)[1])
        for path in (ROOT / "docs" / "bundles").glob("ViFastOs-round-*.md")
        if path.stem.rsplit("-", 1)[-1].isdigit()
    )
    for round_number in bundle_rounds:
        if re.search(rf"\n\d+\. Vòng {round_number:02d}", summary) is None and round_number >= 5:
            fail(f"section 18 missing round {round_number:02d}")

    # Prose must not duplicate numbers that the generated blocks own.
    clean = re.sub(
        r"### BEGIN GENERATED .*?### END GENERATED .*?\n", "", summary, flags=re.S
    )
    for pattern in (
        r"\b\d+\s+test\b",
        r"\b\d+/\d+\s+PASS\b",
        r"\bheadroom\s+\d",
        r"\b\d+/\d+\s+target\s+PASS\b",
    ):
        match = re.search(pattern, clean, re.I)
        if match:
            line = clean.count("\n", 0, match.start()) + 1
            fail(f"number outside generated block at summary line {line}: {match.group(0)!r}")

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
        + sum(len(values) for values in harness_spec.ORDER.values())
        + len(harness_spec.EXIT_CODES)
    )
    if generated_count != expected_count:
        fail(
            f"generated pass condition count {generated_count} != spec {expected_count}"
        )

    for stale in ("AP mới maintenance", "chỉ maintenance", "x86 hart phụ chưa có scheduler"):
        if stale in summary:
            fail(f"stale x86 prose: {stale}")

    # Every host-tested workspace crate must be listed and have a test command.
    for package in gen_docs.PACKAGES:
        if package not in summary:
            fail(f"section 19 missing crate: {package}")
        if f"cargo test -p {package}" not in summary:
            fail(f"section 14 missing cargo test for: {package}")

    print("docs check: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
