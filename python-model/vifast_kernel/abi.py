"""Giao diện syscall (ABI) giữa user space và kernel.

Chương trình user là generator: `r = yield Call(ep, msg)`.
`yield` mô phỏng lệnh `ecall`: CPU trap vào kernel, kernel xử lý rồi trả kết quả.
Load/Store/Fetch là lệnh máy bình thường đi qua MMU, KHÔNG trap (trừ khi page fault).
Mọi cptr được phân giải qua CSpace nhiều cấp có guard (như seL4), độ sâu 64 bit.
"""
from __future__ import annotations

from dataclasses import dataclass, field

from .ktypes import KError, Message, ObjType, Rights


@dataclass(frozen=True)
class SysResult:
    err: KError = KError.OK
    value: int = 0
    badge: int = 0
    msg: Message | None = None
    data: bytes | None = None

    @property
    def ok(self) -> bool:
        return self.err == KError.OK


# ---------- IPC ----------
@dataclass(frozen=True)
class Send:
    ep: int
    msg: Message


@dataclass(frozen=True)
class Call:
    ep: int
    msg: Message
    recv_slot: int = 0      # slot (trong CNode gốc) nhận capability trong reply


@dataclass(frozen=True)
class Recv:
    ep: int
    recv_slot: int = 0


@dataclass(frozen=True)
class Reply:
    msg: Message


@dataclass(frozen=True)
class ReplyRecv:
    ep: int
    msg: Message
    recv_slot: int = 0


@dataclass(frozen=True)
class Signal:
    ntfn: int


@dataclass(frozen=True)
class Wait:
    ntfn: int


@dataclass(frozen=True)
class Poll:
    ntfn: int


@dataclass(frozen=True)
class Yield:
    pass


# ---------- Lệnh máy đi qua MMU (không phải syscall) ----------
@dataclass(frozen=True)
class Load:
    vaddr: int
    length: int


@dataclass(frozen=True)
class Store:
    vaddr: int
    data: bytes


@dataclass(frozen=True)
class Fetch:
    """Nạp lệnh để thực thi: cần quyền X."""
    vaddr: int


USER_INSTRUCTIONS = (Load, Store, Fetch)


# ---------- Capability ----------
@dataclass(frozen=True)
class Retype:
    untyped: int
    obj_type: ObjType
    size_bits: int
    dest_index: int
    count: int = 1
    dest_cnode: int = 0     # 0 = CNode gốc của chính mình


@dataclass(frozen=True)
class CapCopy:
    src: int
    dest_index: int
    rights: Rights = Rights.ALL
    dest_cnode: int = 0


@dataclass(frozen=True)
class CapMint:
    src: int
    dest_index: int
    rights: Rights
    badge: int
    dest_cnode: int = 0


@dataclass(frozen=True)
class CNodeMint:
    """Tạo cap CNode có guard (để dựng CSpace nhiều cấp)."""
    src: int
    dest_index: int
    guard: int
    guard_bits: int
    dest_cnode: int = 0


@dataclass(frozen=True)
class CapDelete:
    slot: int


@dataclass(frozen=True)
class CapRevoke:
    slot: int


# ---------- TCB ----------
@dataclass(frozen=True)
class TcbConfigure:
    tcb: int
    cspace: int
    vspace: int
    priority: int
    mcp: int
    image: str
    env: dict = field(default_factory=dict)
    fault_ep: int = 0       # badge lỗi = badge của cap này


@dataclass(frozen=True)
class TcbResume:
    tcb: int


@dataclass(frozen=True)
class TcbSuspend:
    tcb: int


@dataclass(frozen=True)
class TcbRestart:
    tcb: int


@dataclass(frozen=True)
class TcbSetPriority:
    tcb: int
    priority: int


@dataclass(frozen=True)
class TcbSetAffinity:
    tcb: int
    hart: int


@dataclass(frozen=True)
class TcbBindNotification:
    tcb: int
    ntfn: int


# ---------- Lập lịch theo ngân sách (MCS) ----------
@dataclass(frozen=True)
class SchedContextConfigure:
    sched_control: int
    sc: int
    budget: int
    period: int


@dataclass(frozen=True)
class SchedContextBind:
    sc: int
    tcb: int


# ---------- Bộ nhớ ảo ----------
@dataclass(frozen=True)
class PageTableMap:
    page_table: int
    vspace: int
    vaddr: int


@dataclass(frozen=True)
class FrameMap:
    frame: int
    vspace: int
    vaddr: int
    rights: Rights = Rights.READ | Rights.WRITE


@dataclass(frozen=True)
class FrameUnmap:
    frame: int


# ---------- IOMMU ----------
@dataclass(frozen=True)
class IoMap:
    frame: int
    iospace: int
    iova: int
    rights: Rights = Rights.READ | Rights.WRITE


@dataclass(frozen=True)
class IoUnmap:
    frame: int


# ---------- Ngắt ----------
@dataclass(frozen=True)
class IrqGet:
    control: int
    irq: int
    dest_index: int
    dest_cnode: int = 0


@dataclass(frozen=True)
class IrqSetNotification:
    handler: int
    ntfn: int


@dataclass(frozen=True)
class IrqAck:
    handler: int


# ---------- Khác ----------
@dataclass(frozen=True)
class DebugPrint:
    text: str


@dataclass(frozen=True)
class GetTime:
    pass


# Số hiệu syscall (giá trị thanh ghi a7). Rust: hằng số trong vifast-abi.
SYSCALLS = (Send, Call, Recv, Reply, ReplyRecv, Signal, Wait, Poll, Yield,
            Retype, CapCopy, CapMint, CNodeMint, CapDelete, CapRevoke,
            TcbConfigure, TcbResume, TcbSuspend, TcbRestart, TcbSetPriority, TcbSetAffinity,
            TcbBindNotification, SchedContextConfigure, SchedContextBind,
            PageTableMap, FrameMap, FrameUnmap, IoMap, IoUnmap,
            IrqGet, IrqSetNotification, IrqAck, DebugPrint, GetTime)
SYSCALL_NUMBER = {cls: i for i, cls in enumerate(SYSCALLS)}
