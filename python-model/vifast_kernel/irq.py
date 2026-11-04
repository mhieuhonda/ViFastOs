"""Chuyển tiếp ngắt phần cứng thành notification cho driver ở user space.

Ngắt bị che (mask) sau khi giao cho driver, đến khi driver IrqAck.
Ngắt đến lúc đang che được ghi nhận pending, giao ngay khi ack.
Rust: phần nhận trap và điều khiển PLIC nằm trong crate lõi unsafe.
"""
from __future__ import annotations

import heapq
from dataclasses import dataclass

from .abi import IrqAck, IrqGet, IrqSetNotification, SysResult
from .ktypes import KError, ObjType, Rights, fail
from .objects import Cap, IrqHandler, ObjRef

MAX_IRQ = 256


@dataclass
class IrqState:
    handler: ObjRef
    ntfn: ObjRef | None = None
    badge: int = 1
    masked: bool = False
    pending: bool = False


class IrqMixin:
    # ---------- "phần cứng" ----------
    def schedule_irq(self, irq: int, at: int, period: int = 0) -> None:
        """Nguồn ngắt ngoài giả lập (ví dụ cảm biến phát ngắt định kỳ)."""
        self._irq_seq += 1
        heapq.heappush(self.irq_events, (at, self._irq_seq, irq, period))

    def poll_hardware(self) -> None:
        while self.irq_events and self.irq_events[0][0] <= self.ticks:
            at, _, irq, period = heapq.heappop(self.irq_events)
            self.deliver_irq(irq)
            if period:
                self.schedule_irq(irq, at + period, period)
        for dev in self.devices:
            dev.tick(self)

    def deliver_irq(self, irq: int) -> None:
        st = self.irqs.get(irq)
        if st is None:
            self.stats["spurious_irq"] += 1
            return
        if st.masked or st.ntfn is None or self.objs.try_get(st.ntfn) is None:
            st.pending = True
            return
        st.masked = True
        self.ntfn_signal(st.ntfn, st.badge)

    # ---------- syscall ----------
    def create_irq_handler(self, parent, irq: int, dest) -> None:
        if not 0 <= irq < MAX_IRQ:
            fail(KError.INVALID_ARG)
        if irq in self.irqs:
            fail(KError.IRQ_IN_USE)
        h_ref = self.objs.insert(IrqHandler(irq=irq))
        self.irqs[irq] = IrqState(h_ref)
        self.insert_cap(dest, Cap(ObjType.IRQ_HANDLER, h_ref, Rights.ALL, irq=irq), parent)

    def bind_irq(self, irq: int, ntfn_cap: Cap) -> None:
        st = self.irqs[irq]
        st.ntfn, st.badge = ntfn_cap.obj, ntfn_cap.badge or 1
        if st.pending and not st.masked:
            st.pending = False
            self.deliver_irq(irq)

    def sys_irq_get(self, ref, t, op: IrqGet):
        ca, _ = self.lookup_cap(t, op.control, ObjType.IRQ_CONTROL)
        dest = self.dest_slot(t, op.dest_cnode, op.dest_index)
        self.create_irq_handler(ca, op.irq, dest)
        return SysResult()

    def sys_irq_set_notification(self, ref, t, op: IrqSetNotification):
        _, hcap = self.lookup_cap(t, op.handler, ObjType.IRQ_HANDLER)
        _, ncap = self.lookup_cap(t, op.ntfn, ObjType.NOTIFICATION, Rights.WRITE)
        self.bind_irq(hcap.irq, ncap)
        return SysResult()

    def sys_irq_ack(self, ref, t, op: IrqAck):
        _, hcap = self.lookup_cap(t, op.handler, ObjType.IRQ_HANDLER)
        st = self.irqs[hcap.irq]
        st.masked = False
        if st.pending:
            st.pending = False
            self.deliver_irq(hcap.irq)
        return SysResult()
