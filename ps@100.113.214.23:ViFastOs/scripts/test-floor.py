#!/usr/bin/env python3
"""289.5 (r289): sàn test host — gate ĐỎ nếu bất kỳ crate nào giảm số test.

Vì sao cần: `scripts/gate.sh` từng chốt cứng `-ge 138` trong khi thực tế đã
245 — xoá 107 test vẫn xanh, tức lưới an toàn có lỗ. Nay sàn là số ĐO ĐƯỢC,
lưu trong `scripts/test-floor.json`:

    python3 scripts/test-floor.py --write   # chốt sàn theo số hiện tại
    python3 scripts/test-floor.py --check   # gate dùng: đỏ nếu giảm

Quy ước:
* **Giảm** (xoá/đổi tên test) ⇒ exit 1. Đây là thứ gate phải chặn.
* **Tăng** ⇒ exit 0 nhưng in nhắc chốt lại sàn, để con số luôn phản ánh thật.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FLOOR = ROOT / "scripts" / "test-floor.json"

# Các crate có test host (bỏ crate chỉ có test biên dịch-chéo).
CRATES = (
    "vifast-kernel",
    "vifast-runtime",
    "vifast-user",
    "vifast-abi",
    "vifast-pte",
    "vifast-elf",
    "vifast-fs",
    "vifast-virtio",
    "vifast-arch",
    "vifast-acpi",
    "vifast-maint",
)


def count_tests(crate: str) -> int | None:
    """Đếm test của một crate; `None` nếu crate không build được trên host."""
    proc = subprocess.run(
        ["cargo", "test", "-p", crate, "--", "--list"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if proc.returncode != 0:
        return None
    lines = proc.stdout.decode("utf-8", errors="replace").splitlines()
    return sum(1 for line in lines if line.endswith(": test"))


def measure() -> dict[str, int]:
    counts: dict[str, int] = {}
    for crate in CRATES:
        value = count_tests(crate)
        if value is not None:
            counts[crate] = value
    return counts


def load_floor() -> dict[str, int]:
    if not FLOOR.exists():
        return {}
    return json.loads(FLOOR.read_text(encoding="utf-8"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="chốt sàn theo số đo hiện tại")
    parser.add_argument("--check", action="store_true", help="kiểm sàn (gate dùng)")
    args = parser.parse_args()
    if not args.write and not args.check:
        parser.error("chọn --write hoặc --check")

    measured = measure()
    if not measured:
        print("sàn test: KHÔNG đo được crate nào (cargo test lỗi?)")
        return 1

    if args.write:
        FLOOR.write_text(json.dumps(measured, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        total = sum(measured.values())
        print(f"sàn test: đã chốt {len(measured)} crate, tổng {total} test")
        return 0

    floor = load_floor()
    if not floor:
        print("sàn test: chưa có scripts/test-floor.json — chạy --write trước")
        return 1

    failures: list[str] = []
    grew: list[str] = []
    for crate, expected in sorted(floor.items()):
        actual = measured.get(crate)
        if actual is None:
            failures.append(f"{crate}: không đo được (crate hỏng?) — sàn {expected}")
        elif actual < expected:
            failures.append(f"{crate}: {actual} test < sàn {expected}")
        elif actual > expected:
            grew.append(f"{crate}: {actual} > sàn {expected}")

    if failures:
        print("sàn test: ĐỎ — có test bị mất:")
        for line in failures:
            print(f"  - {line}")
        return 1

    total = sum(measured.get(crate, 0) for crate in floor)
    print(f"sàn test: PASS — {len(floor)} crate, tổng {total} test (không crate nào giảm)")
    for line in grew:
        print(f"  lưu ý (tăng): {line} — chạy --write để chốt lại")
    return 0


if __name__ == "__main__":
    sys.exit(main())
