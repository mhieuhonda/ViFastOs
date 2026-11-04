"""Lập lịch: hàng đợi ưu tiên theo từng hart, timer xoay vòng, ngân sách CPU (MCS).

- Mỗi hart có hàng đợi riêng (bitmap 256 mức, chọn O(1)). Thread gắn với một hart (affinity).
- Timer (CLINT mtimecmp) ngắt khi hết timeslice -> xoay vòng trong cùng mức ưu tiên.
- Thread có SchedContext bị giới hạn `budget` tick mỗi `period` tick; hết thì bị
  "throttle" đến chu kỳ sau, kể cả khi ưu tiên cao nhất -> cô lập thời gian (temporal isolation).
Rust: bitmap [u64; 4] + leading_zeros, hàng đợi intrusive trong TCB, release queue là heap.
"""
from __future__ import annotations

import heapq
from collections import deque
from dataclasses import dataclass, field

from .abi import SchedContextBind, SchedContextConfigure, SysResult
from .hw import Tlb
from .ktypes import (DEFAULT_TIMESLICE, NUM_PRIORITIES, KError, ObjType, Rights,
                     ThreadState, fail)
from .objects import ObjRef, SchedContext, Tcb


class RunQueue:
    def __init__(self) -> None:
        self.queues = [deque() for _ in range(NUM_PRIORITIES)]
        self.bitmap = 0
        self.where: dict[ObjRef, int] = {}

    def enqueue(self, ref: ObjRef, prio: int, head: bool = False) -> None:
        if ref in self.where:
            return
        q = self.queues[prio]
        if head:
            q.appendleft(ref)
        else:
            q.append(ref)
        self.where[ref] = prio
        self.bitmap |= 1 << prio

    def remove(self, ref: ObjRef) -> None:
        prio = self.where.pop(ref, None)
        if prio is None:
            return
        q = self.queues[prio]
        q.remove(ref)
        if not q:
            self.bitmap &= ~(1 << prio)

    def best_prio(self) -> int | None:
        return self.bitmap.bit_length() - 1 if self.bitmap else None

    def pick(self) -> ObjRef | None:
        prio = self.best_prio()
        if prio is None:
            return None
        q = self.queues[prio]
        ref = q.popleft()
        del self.where[ref]
        if not q:
            self.bitmap &= ~(1 << prio)
        return ref


@dataclass
class Hart:
    """Một lõi CPU."""
    id: int
    runq: RunQueue = field(default_factory=RunQueue)
    tlb: Tlb = field(default_factory=Tlb)
    current: ObjRef | None = None
    resched: bool = False
    mtimecmp: int = 0
    mode: str = "S"
    ctx_switches: int = 0
    traps: int = 0
    timer_irqs: int = 0
    idle: int = 0
    busy: int = 0


class SchedMixin:
    # ---------- hàng đợi ----------
    def sched_enqueue(self, ref: ObjRef, t: Tcb, head: bool = False) -> None:
        if t.throttled:
            return
        hart = self.harts[t.affinity]
        hart.runq.enqueue(ref, t.priority, head)
        if self.cur_hart is not None and hart is not self.cur_hart:
            cur = self.objs.try_get(hart.current)
            if cur is None or t.priority > cur.priority:
                self.stats["ipi"] += 1        # báo hart kia lập lịch lại

    def sched_remove(self, ref: ObjRef) -> None:
        for h in self.harts:
            h.runq.remove(ref)

    def sched_queued(self, ref: ObjRef) -> bool:
        return any(ref in h.runq.where for h in self.harts)

    # ---------- chọn thread cho một hart ----------
    def hart_schedule(self, h: Hart) -> ObjRef | None:
        cur = self.objs.try_get(h.current)
        if cur is not None and (cur.state != ThreadState.RUNNING or cur.affinity != h.id):
            if cur.state == ThreadState.RUNNING:          # đổi affinity khi đang chạy
                cur.state = ThreadState.READY
                self.sched_enqueue(h.current, cur)
            h.current, cur = None, None
        if cur is not None and self.ticks >= h.mtimecmp:
            h.timer_irqs += 1                             # ngắt timer: hết timeslice
            cur.timeslice = 0
            h.resched = True
        best = h.runq.best_prio()
        if cur is not None and not h.resched and (best is None or best <= cur.priority):
            return h.current
        if cur is not None:
            cur.state = ThreadState.READY
            self.sched_enqueue(h.current, cur, head=cur.timeslice > 0)
            h.current = None
        h.resched = False
        ref = h.runq.pick()
        if ref is None:
            return None
        t = self.objs.get(ref)
        t.state = ThreadState.RUNNING
        if t.timeslice <= 0:
            t.timeslice = DEFAULT_TIMESLICE
        h.mtimecmp = self.ticks + t.timeslice
        h.current = ref
        h.ctx_switches += 1
        return ref

    def charge(self, h: Hart, ref: ObjRef, t: Tcb) -> None:
        """Tính một tick CPU cho thread vừa chạy."""
        t.timeslice -= 1
        sc = self.objs.try_get(t.sc)
        if isinstance(sc, SchedContext) and sc.configured:
            sc.remaining -= 1
            if sc.remaining <= 0 and t.state == ThreadState.RUNNING:
                t.throttled = True
                t.state = ThreadState.READY
                h.current = None
                self.stats["throttle"] += 1

    # ---------- nạp lại ngân sách ----------
    def push_release(self, at: int, sc_ref: ObjRef, epoch: int) -> None:
        self._release_seq += 1
        heapq.heappush(self.releases, (at, self._release_seq, sc_ref, epoch))

    def process_releases(self) -> None:
        while self.releases and self.releases[0][0] <= self.ticks:
            at, _, sc_ref, epoch = heapq.heappop(self.releases)
            sc = self.objs.try_get(sc_ref)
            if not isinstance(sc, SchedContext) or sc.epoch != epoch or not sc.configured:
                continue
            sc.remaining = sc.budget
            self.push_release(at + sc.period, sc_ref, epoch)
            t = self.objs.try_get(sc.bound_tcb)
            if isinstance(t, Tcb) and t.throttled:
                t.throttled = False
                if t.state == ThreadState.READY:
                    self.sched_enqueue(sc.bound_tcb, t)

    def configure_sc(self, sc_ref: ObjRef, budget: int, period: int) -> None:
        if not 0 < budget <= period:
            fail(KError.INVALID_ARG)
        sc: SchedContext = self.objs.get(sc_ref)
        sc.epoch += 1
        sc.budget, sc.period, sc.remaining, sc.configured = budget, period, budget, True
        self.push_release(self.ticks + period, sc_ref, sc.epoch)

    def bind_sc(self, sc_ref: ObjRef, tref: ObjRef) -> None:
        sc: SchedContext = self.objs.get(sc_ref)
        t: Tcb = self.objs.get(tref)
        if sc.bound_tcb is not None or t.sc is not None:
            fail(KError.INVALID_ARG)
        sc.bound_tcb, t.sc = tref, sc_ref

    def unbind_sc(self, t: Tcb) -> None:
        sc = self.objs.try_get(t.sc)
        if isinstance(sc, SchedContext):
            sc.bound_tcb = None
        t.sc, t.throttled = None, False

    # ---------- syscall ----------
    def sys_sc_configure(self, ref, t, op: SchedContextConfigure):
        self.lookup_cap(t, op.sched_control, ObjType.SCHED_CONTROL)
        _, sccap = self.lookup_cap(t, op.sc, ObjType.SCHED_CONTEXT, Rights.WRITE)
        self.configure_sc(sccap.obj, op.budget, op.period)
        return SysResult()

    def sys_sc_bind(self, ref, t, op: SchedContextBind):
        _, sccap = self.lookup_cap(t, op.sc, ObjType.SCHED_CONTEXT, Rights.WRITE)
        _, tcap = self.lookup_cap(t, op.tcb, ObjType.TCB, Rights.WRITE)
        self.bind_sc(sccap.obj, tcap.obj)
        return SysResult()
