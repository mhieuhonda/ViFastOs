#!/usr/bin/env python3
"""Build the single-file handoff bundle for one ViFastOs round.

Layout:
  1. docs/agent2-report-NN.md
  2. quick status (commits, tests, headroom, summary SHA)
  3. full ViFastOs-architecture-summary.txt in a text block
  final line: SHA-256 of part 1 and part 3
"""

from __future__ import annotations

import argparse
import hashlib
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPORT_DIR = ROOT / "docs"
BUNDLE_DIR = REPORT_DIR / "bundles"
SUMMARY = ROOT / "ViFastOs-architecture-summary.txt"
LOG = REPORT_DIR / "overnight-log.md"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_value(*args: str, default: str = "unknown") -> str:
    try:
        return subprocess.check_output(
            ["git", *args], cwd=ROOT, text=True, stderr=subprocess.DEVNULL
        ).strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return default

def latest_code_commit() -> str:
    try:
        log = subprocess.check_output(
            ["git", "log", "--format=%h %s", "-n", "80"],
            cwd=ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        )
        for line in log.splitlines():
            subject = line.split(" ", 1)[1] if " " in line else ""
            if not subject.startswith("docs:"):
                return line.split(" ", 1)[0]
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--round", required=True, help="round number, e.g. 06")
    parser.add_argument("--docs-commit", default="included in this docs commit")
    parser.add_argument("--test-summary", default="")
    parser.add_argument("--headroom", default="")
    args = parser.parse_args()

    check = ROOT / "scripts" / "check-docs.py"
    subprocess.run([sys.executable, str(check)], cwd=ROOT, check=True)

    report = REPORT_DIR / f"agent2-report-{args.round}.md"
    if not report.exists():
        print(f"missing report: {report}", file=sys.stderr)
        return 1
    if not SUMMARY.exists():
        print(f"missing summary: {SUMMARY}", file=sys.stderr)
        return 1

    report_bytes = report.read_bytes()
    summary_bytes = SUMMARY.read_bytes()
    report_hash = hashlib.sha256(report_bytes).hexdigest()
    summary_hash = hashlib.sha256(summary_bytes).hexdigest()

    log = LOG.read_text()
    match = re.search(
        r"SHA-256 `ViFastOs-architecture-summary\.txt`: `([0-9a-f]{64})`", log
    )
    if not match:
        print("overnight log has no summary SHA-256", file=sys.stderr)
        return 1
    if match.group(1) != summary_hash:
        print(
            "summary SHA-256 mismatch: "
            f"log={match.group(1)} file={summary_hash}",
            file=sys.stderr,
        )
        return 1

    code_commit = latest_code_commit()
    test_summary = args.test_summary or "see the report above"
    headroom = args.headroom or "see the report above"

    parts: list[str] = []
    parts.append(f"# ViFastOs round {args.round} bundle\n\n")
    parts.append("## Part 1 - Agent 2 report\n\n")
    parts.append(report_bytes.decode("utf-8"))
    if not parts[-1].endswith("\n"):
        parts[-1] += "\n"
    parts.append("\n## Part 2 - Quick status\n\n")
    parts.append("```text\n")
    parts.append(f"code commit: {code_commit}\n")
    parts.append(f"docs commit: {args.docs_commit}\n")
    parts.append(f"tests: {test_summary}\n")
    parts.append(f"RISC-V headroom: {headroom}\n")
    parts.append(f"summary SHA-256: {summary_hash}\n")
    parts.append("```\n\n")
    parts.append("## Part 3 - Full ViFastOs-architecture-summary.txt\n\n")
    parts.append("```text\n")
    parts.append(summary_bytes.decode("utf-8"))
    if not parts[-1].endswith("\n"):
        parts[-1] += "\n"
    parts.append("```\n\n")
    parts.append(
        f"SHA-256 part1={report_hash} part3={summary_hash}\n"
    )

    BUNDLE_DIR.mkdir(parents=True, exist_ok=True)
    out = BUNDLE_DIR / f"ViFastOs-round-{args.round}.md"
    out.write_bytes("".join(parts).encode("utf-8"))
    print(out.relative_to(ROOT))
    print(f"report={report_hash} summary={summary_hash}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
