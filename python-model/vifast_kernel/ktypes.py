"""Kiểu dữ liệu nền của kernel ViFastOs.

Rust: crate `vifast-abi` (no_std), dùng chung cho kernel và user space.
- IntEnum  -> #[repr(u32)] enum
- IntFlag  -> bitflags!
- KernelError (exception) -> Result<T, KError>, `raise` -> `return Err(..)`, lan truyền bằng `?`
"""
from __future__ import annotations

from dataclasses import dataclass
from enum import IntEnum, IntFlag
from typing import NoReturn

PAGE_SIZE = 4096
PAGE_SHIFT = 12
WORD_BITS = 64
WORD_MAX = (1 << WORD_BITS) - 1
MSG_MAX_WORDS = 8          # số word tối đa trong một message (đi qua thanh ghi)
MSG_MAX_CAPS = 1           # số capability tối đa chuyển trong một message
NUM_PRIORITIES = 256
DEFAULT_TIMESLICE = 5      # số tick trước khi timer ngắt để xoay vòng
REVOKE_BATCH = 8           # số cap tối đa xoá trong một lần chạy revoke (preemption point)
REAP_BUDGET = 16           # số slot của CNode zombie được dọn mỗi tick
MAX_ASID = 1 << 16

LABEL_FAULT = 0xFFFF_0001         # message kernel gửi tới fault handler
LABEL_NOTIFICATION = 0xFFFF_0002  # message do bound notification sinh ra


class KError(IntEnum):
    OK = 0
    INVALID_CAP = 1        # cptr không phân giải được hoặc slot rỗng
    WRONG_TYPE = 2
    NO_RIGHTS = 3
    SLOT_OCCUPIED = 4
    OUT_OF_MEMORY = 5      # untyped/ASID không đủ (chỉ ảnh hưởng người gọi)
    INVALID_ARG = 6
    ALIGNMENT = 7
    ALREADY_MAPPED = 8
    NOT_MAPPED = 9
    NO_CALLER = 10
    OBJECT_DESTROYED = 11
    IRQ_IN_USE = 12
    PRIO_TOO_HIGH = 13     # vượt MCP của người gọi
    CANCELLED = 14         # đối tác bị restart/suspend/huỷ
    MISSING_PAGE_TABLE = 15
    ILLEGAL_OPERATION = 16  # ví dụ trang vừa ghi vừa thực thi (vi phạm W^X)


class KernelError(Exception):
    def __init__(self, code: KError):
        super().__init__(code.name)
        self.code = code


def fail(code: KError) -> NoReturn:
    raise KernelError(code)


class VmFault(Exception):
    """Page fault. Rust: trap từ phần cứng (scause 12/13/15 trên RISC-V)."""

    def __init__(self, addr: int):
        super().__init__(hex(addr))
        self.addr = addr


class _Sentinel:
    def __init__(self, name: str) -> None:
        self.name = name

    def __repr__(self) -> str:
        return self.name


BLOCKED = _Sentinel("BLOCKED")      # syscall làm thread bị chặn
PREEMPTED = _Sentinel("PREEMPTED")  # syscall làm dở, sẽ chạy lại ở lượt sau (restart syscall)


class Rights(IntFlag):
    NONE = 0
    READ = 1     # endpoint: recv, notification: wait, frame: đọc
    WRITE = 2    # endpoint: send/call, notification: signal, frame: ghi
    EXEC = 4
    GRANT = 8    # được chuyển capability qua IPC
    ALL = 15

    @classmethod
    def parse(cls, text: str) -> "Rights":
        table = {"R": cls.READ, "W": cls.WRITE, "X": cls.EXEC, "G": cls.GRANT}
        r = cls.NONE
        for ch in text.upper():
            if ch not in table:
                raise ValueError(f"quyền không hợp lệ: {ch}")
            r |= table[ch]
        return r


class ObjType(IntEnum):
    UNTYPED = 0
    TCB = 1
    ENDPOINT = 2
    NOTIFICATION = 3
    CNODE = 4
    FRAME = 5
    VSPACE = 6
    IRQ_CONTROL = 7
    IRQ_HANDLER = 8
    PAGE_TABLE = 9
    IO_SPACE = 10        # bảng dịch IOMMU của một thiết bị (cấp lúc boot)
    SCHED_CONTEXT = 11   # ngân sách CPU budget/period (kiểu seL4 MCS)
    SCHED_CONTROL = 12   # quyền cấu hình sched context


RETYPEABLE = {ObjType.UNTYPED, ObjType.TCB, ObjType.ENDPOINT, ObjType.NOTIFICATION,
              ObjType.CNODE, ObjType.FRAME, ObjType.VSPACE, ObjType.PAGE_TABLE,
              ObjType.SCHED_CONTEXT}

_FIXED_SIZE = {
    ObjType.TCB: 1024,
    ObjType.ENDPOINT: 16,
    ObjType.NOTIFICATION: 32,
    ObjType.FRAME: PAGE_SIZE,
    ObjType.VSPACE: PAGE_SIZE,       # chính là page table gốc Sv39
    ObjType.PAGE_TABLE: PAGE_SIZE,
    ObjType.SCHED_CONTEXT: 256,
}
CNODE_SLOT_SIZE = 32


def object_size(obj_type: ObjType, size_bits: int) -> int:
    """Kích thước object khi retype từ untyped (byte)."""
    if obj_type not in RETYPEABLE:
        fail(KError.INVALID_ARG)
    if obj_type in _FIXED_SIZE:
        return _FIXED_SIZE[obj_type]
    if obj_type == ObjType.CNODE:
        if not 1 <= size_bits <= 12:
            fail(KError.INVALID_ARG)
        return (1 << size_bits) * CNODE_SLOT_SIZE
    if not 4 <= size_bits <= 40:         # UNTYPED
        fail(KError.INVALID_ARG)
    return 1 << size_bits


class ThreadState(IntEnum):
    INACTIVE = 0
    READY = 1
    RUNNING = 2
    BLOCKED_SEND = 3
    BLOCKED_RECV = 4
    BLOCKED_REPLY = 5
    BLOCKED_NOTIFICATION = 6
    FAULTED = 7          # lỗi mà không có fault handler, chờ restart


class FaultKind(IntEnum):
    VM_FAULT = 1         # truy cập bộ nhớ không hợp lệ (sửa được rồi chạy tiếp)
    USER_EXCEPTION = 2   # panic / lệnh không hợp lệ (phải restart)
    EXITED = 3           # chương trình kết thúc
    BAD_SYSCALL = 4


@dataclass(frozen=True)
class Message:
    """Message IPC: label + tối đa 8 word + tối đa 1 capability."""

    label: int = 0
    data: tuple = ()
    caps: tuple = ()   # phía gửi: cptr cần chuyển; phía nhận: slot đã nhận

    def __post_init__(self) -> None:
        if len(self.data) > MSG_MAX_WORDS or len(self.caps) > MSG_MAX_CAPS:
            raise ValueError("message quá lớn")
        for w in (self.label, *self.data, *self.caps):
            if not isinstance(w, int) or not 0 <= w <= WORD_MAX:
                raise ValueError(f"word không hợp lệ: {w!r}")
