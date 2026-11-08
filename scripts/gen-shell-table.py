#!/usr/bin/env python3
"""Sinh bảng tra "lệnh shell -> syscall -> nơi sửa" từ MÃ NGUỒN.

Vòng 175: bảng viết tay sẽ lệch ngay sau vài vòng. Script này đọc chính
`crates/vifast-client/src/main.rs` (danh sách lệnh) và `crates/vifast-user/src/lib.rs`
(hàm wrapper -> SyscallId) để dựng bảng, rồi chèn vào `docs/11-hien-tai/16-van-hanh.md`
giữa hai mốc `<!-- SHELL-TABLE:BEGIN -->` và `<!-- SHELL-TABLE:END -->`.

Dùng:
    python3 scripts/gen-shell-table.py          # cập nhật tài liệu
    python3 scripts/gen-shell-table.py --check  # chỉ kiểm tra (exit 1 nếu lệch)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CLIENT = ROOT / "crates/vifast-client/src/main.rs"
USERLIB = ROOT / "crates/vifast-user/src/lib.rs"
DOC = ROOT / "docs/11-hien-tai/16-van-hanh.md"
BEGIN = "<!-- SHELL-TABLE:BEGIN -->"
END = "<!-- SHELL-TABLE:END -->"

# Lệnh nào gọi syscall nào (tên hàm wrapper trong vifast-user).
# Giữ ở đây thay vì suy luận tự động: client gọi wrapper gián tiếp qua helper
# nội bộ (vfs_open, write...), tự suy luận sẽ sai thầm.
COMMAND_SYSCALLS: dict[str, str] = {
    "cat": "vfs_open (Open) + vfs_read_at (Read) + vfs_close (Close)",
    "df": "vfs_statfs (StatFs)",
    "echo": "-",
    "exec": "send LABEL_RUN",
    "free": "mem_info (MemInfo)",
    "fsck": "vfs_fsck (Fsck)",
    "help": "-",
    "history": "-",
    "!!": "-",
    "jobs": "tcb_info (TcbInfo)",
    "kill": "send LABEL_KILL",
    "ln": "vfs_link (Link)",
    "ls": "vfs_open + vfs_read_at + vfs_close",
    "mkdir": "vfs_open (create dir)",
    "mv": "vfs_rename (Rename)",
    "ps": "tcb_info (TcbInfo)",
    "rm": "vfs_unlink (Unlink)",
    "run": "send LABEL_RUN",
    "runfs": "send LABEL_RUN",
    "stat": "vfs_stat_path (StatPath)",
    "sync": "vfs_fsync (Fsync)",
    "time": "get_time (GetTime)",
    "truncate": "vfs_truncate (Truncate)",
    "uname": "-",
    "uptime": "get_time (GetTime) + timer_hz (TimerInfo)",
}

COMMAND_SOURCE = {
    "cat": "vifast-client (vfs_read_at) + vifast-server (OPEN/READ)",
    "df": "vifast-client + vifast-server (STATFS) + vifast-fs (usage)",
    "exec": "vifast-client + vifast-root (spawn_elf_from_fs)",
    "free": "vifast-client + vifast-kernel (MemInfo)",
    "fsck": "vifast-client + vifast-server + vifast-fs (fsck)",
    "jobs": "vifast-client + vifast-kernel (TcbInfo)",
    "kill": "vifast-client + vifast-root (destroy_exec)",
    "ls": "vifast-client + vifast-server",
    "ps": "vifast-client + vifast-kernel (TcbInfo)",
    "run": "vifast-client + vifast-root",
    "runfs": "vifast-client + vifast-root",
    "stat": "vifast-client + vifast-server + vifast-fs",
    "sync": "vifast-client + vifast-server + vifast-fs (journal)",
    "time": "vifast-client + vifast-user",
    "uptime": "vifast-client + vifast-kernel (TimerInfo)",
}

# Lệnh nào có marker/gate liên quan (để biết sửa lệnh phải chạy cổng nào).
COMMAND_GATE = {
    "runfs": "RUN FS OK / HIARG (QEMU) + persist",
    "exec": "RUN OK / RUN FS OK",
    "sync": "FS PERSIST OK (persist)",
    "df": "FS USAGE OK",
    "fsck": "FS OPS FSCK OK",
    "free": "MEM FREE OK",
    "ps": "-",
    "jobs": "-",
    "kill": "KILL OK (self-test)",
}


def shell_commands() -> list[str]:
    src = CLIENT.read_text()
    found = set(re.findall(r'line == b"([a-z!]+)"', src))
    found |= set(re.findall(r'strip_prefix\(b"([a-z]+) "', src))
    return sorted(found)


def render(commands: list[str]) -> str:
    lines = [
        BEGIN,
        "### 2c. Bảng tra lệnh shell (sinh tự động từ mã nguồn)",
        "",
        "Sinh bằng `python3 scripts/gen-shell-table.py`; **đừng sửa tay** — sửa",
        "`COMMAND_SYSCALLS`/`COMMAND_SOURCE` trong script rồi chạy lại.",
        "",
        "| Lệnh | Syscall/đường IPC | Nơi sửa chính | Marker/cổng liên quan |",
        "|---|---|---|---|",
    ]
    for cmd in commands:
        calls = COMMAND_SYSCALLS.get(cmd, "*(chưa khai báo)*")
        where = COMMAND_SOURCE.get(cmd, "vifast-client")
        gate = COMMAND_GATE.get(cmd, "-")
        lines.append(f"| `{cmd}` | {calls} | {where} | {gate} |")
    lines.append(END)
    return "\n".join(lines)


def main() -> int:
    check = "--check" in sys.argv
    commands = shell_commands()
    block = render(commands)
    doc = DOC.read_text()
    if BEGIN in doc and END in doc:
        start = doc.index(BEGIN)
        end = doc.index(END) + len(END)
        updated = doc[:start] + block + doc[end:]
    else:
        sys.exit("thiếu mốc SHELL-TABLE trong 16-van-hanh.md")
    if check:
        if updated != doc:
            print("bảng lệnh shell LỆCH so với mã nguồn — chạy lại không --check")
            return 1
        print("bảng lệnh shell khớp mã nguồn")
        return 0
    DOC.write_text(updated)
    print(f"đã cập nhật bảng {len(commands)} lệnh")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
