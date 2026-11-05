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
    summary = SUMMARY.read_text()
    handoff = HANDOFF.read_text()
    text = summary + "\n" + handoff

    required = [
        "20 TcbSetAffinity",
        "SMP WORKERS OK (a=50, b=50)",
        "SMP IDLE OK (hart1_wakeups=K)",
        "SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)",
        "PERCPU CLOBBER OK",
        "sched_harts: u32",
        "WFI",
    ]
    for value in required:
        if value not in text:
            fail(f"missing required string: {value}")

    forbidden = [
        "hart phụ idle",
        "hart phụ chạy scheduler",
        "chỉ idle",
        "chưa vào scheduler",
        "chưa chọn task",
    ]
    for value in forbidden:
        if value in summary or value in handoff:
            fail(f"stale string present: {value}")

    # Every non-docs commit since the multi-arch checkpoint must be listed.
    log = subprocess.check_output(
        ["git", "log", "--format=%h %s", "456d5ef..HEAD"], cwd=ROOT, text=True
    )
    missing: list[str] = []
    for line in log.splitlines():
        commit, _, subject = line.partition(" ")
        if subject.startswith("docs:"):
            continue
        if commit not in summary:
            missing.append(line)
    if missing:
        fail("summary commit list missing: " + ", ".join(missing))

    # Counts are checked as positive anchors; build.sh prints the authoritative
    # live test result and is run before make-bundle.
    for count in ("87/87", "5/5", "11/11", "17/17"):
        if count not in summary:
            fail(f"missing host-test count: {count}")

    # The harness strings are deliberately duplicated here: if an assertion is
    # added to test-qemu.py without documenting it, this check must be updated.
    for harness in ("SMP WORKERS OK", "SMP SCHED OK", "SMP IDLE OK", "SMP OK"):
        if harness not in summary:
            fail(f"harness assertion not documented: {harness}")

    print("docs check: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
