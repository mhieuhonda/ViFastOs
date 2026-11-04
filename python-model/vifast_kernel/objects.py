"""Kernel object và bảng object.

Nguyên tắc để port Rust:
- Không object nào giữ tham chiếu Python tới object khác; chỉ giữ ObjRef (index + generation).
  Rust: arena `Vec<Entry>` + generation, tránh lifetime phức tạp và use-after-free.
- Kernel không có heap chung: object sinh ra từ Untyped, có địa chỉ vật lý thật (paddr)
  trong PhysMem. Chỉ vài object lúc boot (IRQ control, IOSpace, device frame) là ngoại lệ.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from typing import Any

from .ktypes import (DEFAULT_TIMESLICE, FaultKind, KError, Message, ObjType,
                     Rights, ThreadState, fail)


@dataclass(frozen=True)
class ObjRef:
    index: int
    gen: int


@dataclass(frozen=True)
class SlotAddr:
    cnode: ObjRef
    index: int


@dataclass
class Cap:
    obj_type: ObjType
    obj: ObjRef | None
    rights: Rights
    badge: int = 0
    irq: int | None = None
    mapped: tuple | None = None      # frame: (vspace, vaddr); page table: (vspace, vaddr, level)
    io_mapped: tuple | None = None   # frame: (iospace, iova)
    guard: int = 0                   # CNode cap: guard dùng khi phân giải cptr
    guard_bits: int = 0

    def derive(self, rights: Rights, badge: int | None = None) -> "Cap":
        """Tạo cap con: quyền chỉ giảm, badge chỉ gán được một lần, không mang theo mapping."""
        new_badge = self.badge
        if badge:
            if self.obj_type not in (ObjType.ENDPOINT, ObjType.NOTIFICATION):
                fail(KError.INVALID_ARG)
            if self.badge:
                fail(KError.NO_RIGHTS)
            new_badge = badge
        return Cap(self.obj_type, self.obj, self.rights & rights, new_badge, self.irq,
                   guard=self.guard, guard_bits=self.guard_bits)


@dataclass
class KObject:
    refs: int = 0                         # số cap trỏ tới object
    parent_untyped: ObjRef | None = None
    dying: bool = False
    paddr: int = 0                        # địa chỉ vật lý của object


@dataclass
class Untyped(KObject):
    size: int = 0
    watermark: int = 0
    children: set = field(default_factory=set)


@dataclass
class Endpoint(KObject):
    send_queue: deque = field(default_factory=deque)
    recv_queue: deque = field(default_factory=deque)


@dataclass
class Notification(KObject):
    word: int = 0
    waiters: deque = field(default_factory=deque)
    bound_tcb: ObjRef | None = None


@dataclass
class Slot:
    """Một ô capability + vị trí trong cây dẫn xuất (CDT)."""
    cap: Cap | None = None
    parent: SlotAddr | None = None
    children: list = field(default_factory=list)


@dataclass
class CNode(KObject):
    slots: list = field(default_factory=list)
    reap_cursor: int = 0                  # tiến độ dọn khi CNode thành zombie

    @property
    def radix(self) -> int:
        return len(self.slots).bit_length() - 1


@dataclass
class Frame(KObject):
    device: bool = False                  # frame MMIO của thiết bị (không thuộc RAM)


@dataclass
class PageTable(KObject):
    pass


@dataclass
class VSpace(KObject):
    """Page table gốc Sv39 nằm ở paddr. `frames`/`tables` chỉ là bản đồ ngược để huỷ cho đúng."""
    asid: int = 0
    frames: dict = field(default_factory=dict)   # vpn -> SlotAddr của frame cap
    tables: dict = field(default_factory=dict)   # (vaddr, level) -> SlotAddr của PT cap


@dataclass
class IoMapping:
    paddr: int
    rights: Rights
    slot: SlotAddr


@dataclass
class IoSpace(KObject):
    """Bảng dịch IOMMU của một thiết bị: iova_page -> IoMapping."""
    device: str = ""
    pages: dict = field(default_factory=dict)


@dataclass
class IrqControl(KObject):
    pass


@dataclass
class IrqHandler(KObject):
    irq: int = 0


@dataclass
class SchedControl(KObject):
    pass


@dataclass
class SchedContext(KObject):
    budget: int = 0
    period: int = 0
    remaining: int = 0
    configured: bool = False
    bound_tcb: ObjRef | None = None
    epoch: int = 0                        # đổi cấu hình -> sự kiện nạp lại cũ bị bỏ


@dataclass
class PendingSend:
    badge: int
    msg: Message
    is_call: bool
    can_grant: bool


@dataclass
class TrapFrame:
    """Thanh ghi user được lưu khi trap. Rust: struct #[repr(C)] mà assembly lưu/khôi phục."""
    pc: int = 0
    a: list = field(default_factory=lambda: [0] * 8)


@dataclass
class Tcb(KObject):
    name: str = ""
    state: ThreadState = ThreadState.INACTIVE
    priority: int = 0
    mcp: int = 0
    affinity: int = 0                     # hart chạy thread này
    cspace: ObjRef | None = None
    cspace_guard: int = 0
    cspace_guard_bits: int = 0
    vspace: ObjRef | None = None
    fault_ep: ObjRef | None = None
    fault_badge: int = 0
    image: str | None = None
    env: dict = field(default_factory=dict)
    regs: TrapFrame = field(default_factory=TrapFrame)
    # Ngữ cảnh thực thi. Rust: TrapFrame + kernel stack thay cho generator.
    program: Any = None
    resume_value: Any = None
    retry_op: Any = None
    fault_kind: FaultKind | None = None
    # IPC
    pending: PendingSend | None = None
    blocked_on: ObjRef | None = None
    recv_slot: int = 0
    caller: tuple | None = None       # (ObjRef, reply_epoch) của thread đang chờ reply
    reply_epoch: int = 0
    bound_ntfn: ObjRef | None = None
    # Lập lịch
    timeslice: int = DEFAULT_TIMESLICE
    sc: ObjRef | None = None
    throttled: bool = False           # hết budget, chờ chu kỳ sau


_CTORS = {
    ObjType.TCB: Tcb, ObjType.ENDPOINT: Endpoint, ObjType.NOTIFICATION: Notification,
    ObjType.FRAME: Frame, ObjType.VSPACE: VSpace, ObjType.PAGE_TABLE: PageTable,
    ObjType.SCHED_CONTEXT: SchedContext,
}


def new_object(obj_type: ObjType, size_bits: int) -> KObject:
    if obj_type == ObjType.UNTYPED:
        return Untyped(size=1 << size_bits)
    if obj_type == ObjType.CNODE:
        return CNode(slots=[Slot() for _ in range(1 << size_bits)])
    if obj_type in _CTORS:
        return _CTORS[obj_type]()
    fail(KError.INVALID_ARG)


class ObjectTable:
    """Arena có generation. Rust: mảng cố định lúc boot + free list."""

    def __init__(self) -> None:
        self._entries: list[list] = []   # [generation, object | None]
        self._free: list[int] = []

    def insert(self, obj: KObject) -> ObjRef:
        if self._free:
            i = self._free.pop()
            self._entries[i][1] = obj
        else:
            i = len(self._entries)
            self._entries.append([0, obj])
        return ObjRef(i, self._entries[i][0])

    def try_get(self, ref: ObjRef | None) -> Any:
        if ref is None or ref.index >= len(self._entries):
            return None
        gen, obj = self._entries[ref.index]
        return obj if gen == ref.gen else None

    def get(self, ref: ObjRef | None) -> Any:
        obj = self.try_get(ref)
        if obj is None:
            fail(KError.OBJECT_DESTROYED)
        return obj

    def remove(self, ref: ObjRef) -> None:
        entry = self._entries[ref.index]
        if entry[0] != ref.gen:
            return
        entry[0] += 1
        entry[1] = None
        self._free.append(ref.index)

    def items(self):
        for i, (gen, obj) in enumerate(self._entries):
            if obj is not None:
                yield ObjRef(i, gen), obj
