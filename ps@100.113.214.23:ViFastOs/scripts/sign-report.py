#!/usr/bin/env python3
"""r262: tính và (tuỳ chọn) ghi khối SHA-256 cho một báo cáo ViFastOs.

Quy ước của dự án (giữ từ r253): hash phủ mọi byte TRƯỚC dấu `<!-- SHA-256`,
sau khi `rstrip()` và thêm đúng một `\n`. Nhờ vậy hash tái lập được: sửa phần
thân là hash đổi, sửa khối SHA ở cuối thì không.

    python3 scripts/sign-report.py docs/reviews/ViFastOs-report-r262.md        # chỉ in
    python3 scripts/sign-report.py --write docs/reviews/ViFastOs-report-r262.md
"""
import hashlib
import sys

MARKER = "<!-- SHA-256"


def body_bytes(text: str) -> bytes:
    at = text.index(MARKER)
    return (text[:at].rstrip() + "\n").encode()


def main() -> int:
    args = sys.argv[1:]
    write = "--write" in args
    paths = [a for a in args if not a.startswith("--")]
    if len(paths) != 1:
        print(__doc__)
        return 2
    path = paths[0]
    text = open(path, encoding="utf-8").read()
    if MARKER not in text:
        print(f"thiếu dấu {MARKER} trong {path}")
        return 1
    digest = hashlib.sha256(body_bytes(text)).hexdigest()
    if not write:
        print(f"{digest}  {path}")
        return 0
    head = text[: text.index(MARKER)].rstrip() + "\n"
    tail = (
        f"\n{MARKER}: phần dưới đây KHÔNG nằm trong hash -->\n\n"
        "## SHA-256\n\n"
        "```text\n"
        f"{digest}  {path}\n"
        "```\n\n"
        "Kiểm: băm mọi byte TRƯỚC dấu `<!-- SHA-256` (sau khi `rstrip()` và thêm\n"
        "đúng một `\\n`), phải ra đúng chuỗi trên. Lệnh:\n\n"
        "```sh\n"
        "python3 scripts/sign-report.py "
        f"{path}\n"
        "```\n"
    )
    open(path, "w", encoding="utf-8").write(head + tail)
    check = hashlib.sha256(body_bytes(open(path, encoding="utf-8").read())).hexdigest()
    print(f"{digest}  {path}  (kiểm lại: {'OK' if check == digest else 'LỆCH'})")
    return 0 if check == digest else 1


if __name__ == "__main__":
    sys.exit(main())
