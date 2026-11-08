#!/usr/bin/env python3
"""Sinh bảng marker của cổng từ `scripts/harness_spec.py` (nguồn duy nhất).

Vòng 188: runbook từng có nhiều mục viết tay mô tả marker, dễ lệch như bảng test
host đã lệch (vòng 176). Bảng này sinh tự động, có `--check` và được `build.sh`
gọi, nên không thể lệch âm thầm.

Dùng:
    python3 scripts/gen-marker-table.py           # cập nhật tài liệu
    python3 scripts/gen-marker-table.py --check   # exit 1 nếu lệch
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness_spec as spec  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs/11-hien-tai/16-van-hanh.md"
BEGIN = "<!-- MARKER-TABLE:BEGIN -->"
END = "<!-- MARKER-TABLE:END -->"
ARCHES = ("riscv64", "aarch64", "x86_64")


def arch_markers() -> dict[str, list[str]]:
    """Marker riêng arch → danh sách arch yêu cầu nó."""
    table: dict[str, list[str]] = {}
    for arch in ARCHES:
        for text, _note in spec.ARCH[arch]:
            prefix = text.split(" (")[0]
            table.setdefault(prefix, [])
            if arch not in table[prefix]:
                table[prefix].append(arch)
    return table


def render() -> str:
    lines = [
        BEGIN,
        "### 5. Bảng marker của cổng (sinh tự động)",
        "",
        "Sinh bằng `python3 scripts/gen-marker-table.py` từ `scripts/harness_spec.py`",
        "— **đừng sửa tay**. Cột *Nhóm*: **bắt buộc** = phải có trên mọi arch;",
        "**đúng 1 lần** = không được lặp và không được thiếu; **riêng arch** = chỉ",
        "arch liệt kê mới cần.",
        "",
        "| Marker | Nhóm | Ý nghĩa |",
        "|---|---|---|",
    ]
    for text, note in spec.COMMON:
        lines.append(f"| `{text}` | bắt buộc | {note} |")
    for text in spec.EXACT_ONE:
        lines.append(f"| `{text}` | đúng 1 lần | không được thiếu hoặc lặp |")
    for text, arches in sorted(arch_markers().items()):
        joined = ", ".join(arches)
        lines.append(f"| `{text}` | riêng arch ({joined}) | chỉ arch này mới cần |")
    lines.append(END)
    return "\n".join(lines)


def main() -> int:
    check = "--check" in sys.argv
    doc = DOC.read_text()
    if BEGIN not in doc or END not in doc:
        sys.exit("thiếu mốc MARKER-TABLE trong 16-van-hanh.md")
    block = render()
    start = doc.index(BEGIN)
    end = doc.index(END) + len(END)
    updated = doc[:start] + block + doc[end:]
    if check:
        if updated != doc:
            print("bảng marker LỆCH so với harness_spec — chạy lại không --check")
            return 1
        print("bảng marker khớp harness_spec")
        return 0
    DOC.write_text(updated)
    total = len(spec.COMMON) + len(spec.EXACT_ONE) + len(arch_markers())
    print(f"đã cập nhật bảng marker ({total} dòng)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
