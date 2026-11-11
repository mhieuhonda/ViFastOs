#!/usr/bin/env python3
"""Bài test hồi quy D18 cho lỗi `FS V2 OK` trên x86_64 (T26).

Vì sao cần script này: lỗi gốc nằm ở đường FS server khi có ≥2 hart, chỉ lộ ra
qua NHIỀU lần boot QEMU — không thể viết thành unit test trên host. Đây là dạng
"test đỏ trước" khả thi cho lỗi đó: cùng một script, hai chế độ.

  * `--expect green` (mặc định) — điều kiện ĐÓNG lỗi: N lần boot x86_64 phải
    xanh hết. Đây là thứ phải giữ xanh trước khi ai đó nâng `MAX_IO`.
  * `--expect red` — chứng minh script CÓ RĂNG: với `MAX_IO = 256` (giá trị gây
    hồi quy ở `d3f2472`) phải thấy ít nhất một lần hỏng. Nếu chạy `red` mà
    toàn xanh thì bài test vô dụng và phải sửa trước khi tin nó.

Dùng:
    python3 scripts/fs-smp-regression.py --iterations 20
    python3 scripts/fs-smp-regression.py --iterations 20 --expect red

Ghi chú vận hành: script tự đọc `MAX_IO` từ `crates/vifast-abi/src/lib.rs` và in
ra để log biết đang đo ở giá trị nào. Nó KHÔNG tự rebuild — chạy
`./scripts/build.sh` trước (bài học r280: build lẻ một crate làm QEMU đỏ oan).
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ABI = os.path.join(ROOT, "crates", "vifast-abi", "src", "lib.rs")


def read_max_io() -> int:
    """Đọc `pub const MAX_IO: usize = <n>;` trong crate ABI."""
    src = open(ABI, encoding="utf-8").read()
    match = re.search(r"const MAX_IO:\s*usize\s*=\s*(\d+)", src)
    if match is None:
        raise SystemExit("không tìm thấy MAX_IO trong vifast-abi/src/lib.rs")
    return int(match.group(1))


def run_once() -> tuple[bool, str]:
    """Boot một lần x86_64 qua harness chuẩn; trả (pass, output)."""
    proc = subprocess.run(
        [
            sys.executable,
            os.path.join(ROOT, "scripts", "test-qemu.py"),
            "--arch",
            "x86_64",
            "--repeat",
            "1",
        ],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    output = proc.stdout.decode("utf-8", errors="replace")
    passed = proc.returncode == 0 and re.search(r"x86_64\s+1/1 PASS", output) is not None
    return passed, output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--iterations", type=int, default=20, help="số lần boot (mặc định 20)"
    )
    parser.add_argument(
        "--expect",
        choices=("green", "red"),
        default="green",
        help="green: phải xanh hết (điều kiện đóng lỗi); red: phải có ít nhất một lần hỏng",
    )
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations phải >= 1")

    max_io = read_max_io()
    expected_note = "xanh hết" if args.expect == "green" else "có ít nhất 1 lần hỏng"
    print(f"MAX_IO = {max_io} · {args.iterations} lần boot x86_64 · kỳ vọng: {expected_note}")

    failures = 0
    for attempt in range(1, args.iterations + 1):
        passed, output = run_once()
        if passed:
            print(f"  {attempt:03d}/{args.iterations:03d}: PASS", flush=True)
        else:
            failures += 1
            reason = next(
                (line for line in output.splitlines() if line.startswith("FAIL")),
                "không rõ lý do",
            )
            print(f"  {attempt:03d}/{args.iterations:03d}: FAIL — {reason}", flush=True)

    print()
    if args.expect == "green":
        if failures == 0:
            print(f"KẾT QUẢ: ĐẠT — {args.iterations}/{args.iterations} xanh (MAX_IO={max_io})")
            return 0
        print(
            f"KẾT QUẢ: HỎNG — {failures}/{args.iterations} lần đỏ. "
            "Đây chính là điều kiện cấm nâng MAX_IO (D18/T26)."
        )
        return 1
    if failures > 0:
        print(
            f"KẾT QUẢ: ĐẠT (chứng minh có răng) — {failures}/{args.iterations} lần đỏ "
            f"ở MAX_IO={max_io}"
        )
        return 0
    print(
        "KẾT QUẢ: VÔ DỤNG — toàn xanh, bài test không phân biệt được cấu hình hỏng. "
        "Phải tăng số lần boot hoặc xem lại cách đo."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
