#!/usr/bin/env python3
"""Sinh bảng "Host test" trong docs/11-hien-tai/09-testing.md từ kết quả thật.

Vòng 176: bảng này từng ghi `vifast-kernel 131/131` trong khi thực tế đã 195 —
tài liệu lệch âm thầm vì viết tay. Script chạy `cargo test -p <crate> -- --list`
(đã cache nên nhanh), đếm số test và cập nhật đúng các dòng bảng.

Dùng:
    python3 scripts/gen-test-counts.py           # cập nhật
    python3 scripts/gen-test-counts.py --check   # exit 1 nếu lệch
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs/11-hien-tai/09-testing.md"

# crate -> ghi chú ngắn (giữ nguyên như tài liệu hiện tại)
CRATES: dict[str, str] = {
    "vifast-abi": "protocol pager v2: label, VA window, encode/decode",
    "vifast-kernel": "+ 2 test fuzz dài `#[ignore]`, chạy `--ignored`",
    "vifast-arch": "",
    "vifast-pte": "",
    "vifast-elf": "",
    "vifast-acpi": "",
    "vifast-fs": "định dạng đĩa, biên block, fsck; nằm trong cổng từ vòng 166",
    "vifast-virtio": "checksum TCP/IP, keymap bàn phím; nằm trong cổng từ vòng 167",
}


def count_tests(crate: str) -> int | None:
    """Đếm test ĐÃ CHẠY của một crate (khớp đúng con số trong `build.sh`).

    Dùng `cargo test` rồi cộng các dòng `test result: ok. N passed` — cách này
    khớp với cổng, khác `--list` (bao gồm cả test `#[ignore]` chưa chạy).
    """
    proc = subprocess.run(
        ["cargo", "test", "-p", crate],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return None
    return sum(int(m) for m in re.findall(r"test result: ok\. (\d+) passed", proc.stdout))


def main() -> int:
    check = "--check" in sys.argv
    counts: dict[str, int] = {}
    for crate in CRATES:
        n = count_tests(crate)
        if n is None:
            print(f"bỏ qua {crate}: cargo test lỗi")
            continue
        counts[crate] = n

    doc = DOC.read_text()
    lines = doc.splitlines()
    changed = False
    for i, line in enumerate(lines):
        m = re.match(r"\| `([a-z0-9-]+)` \| \*{0,2}(\d+)(?:/\d+)?\*{0,2} \| ?(.*?) ?\|", line)
        if not m:
            continue
        crate = m.group(1)
        if crate not in counts:
            continue
        note = CRATES.get(crate) or m.group(3).strip()
        updated = f"| `{crate}` | **{counts[crate]}/{counts[crate]}** | {note} |"
        if updated != line:
            lines[i] = updated
            changed = True
        # ghi chú có thể trống: chuẩn hoá khoảng trắng
        lines[i] = updated
    # Bổ sung dòng cho crate đã có test nhưng chưa có trong bảng.
    for crate, n in counts.items():
        if not any(f"`{crate}`" in line and line.startswith("| `") for line in lines):
            for i, line in enumerate(lines):
                if line.startswith("| ") and "kiểm qua QEMU/build" in line:
                    lines.insert(i, f"| `{crate}` | **{n}/{n}** | {CRATES.get(crate, '')} |")
                    changed = True
                    break
    # Cập nhật dòng tổng.
    total = sum(counts.values())
    for i, line in enumerate(lines):
        if line.startswith("| **Tổng duy nhất**"):
            updated = f"| **Tổng duy nhất** | **{total}** | |"
            if updated != line:
                lines[i] = updated
                changed = True
            break
    new_doc = "\n".join(lines) + ("\n" if doc.endswith("\n") else "")
    if check:
        if changed:
            print("bảng host test LỆCH so với kết quả cargo test — chạy lại không --check")
            return 1
        print("bảng host test khớp kết quả thật")
        return 0
    DOC.write_text(new_doc)
    total = sum(counts.values())
    print(f"đã cập nhật {len(counts)} dòng, tổng {total} test")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
