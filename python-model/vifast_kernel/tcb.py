"""Quản lý thread (TCB): cấu hình, chạy, dừng, khởi động lại, huỷ, đổi hart.

Chính sách (restart bao nhiêu lần, khi nào vào safe mode) KHÔNG nằm ở đây
mà ở supervisor chạy trong user space. Kernel chỉ cung cấp cơ chế.
"""
from __future__ import annotations

from .abi import (SysResult, TcbBindNotification, TcbConfigure, TcbRestart, TcbResume,
                  TcbSetAffinity, TcbSetPriority, TcbSuspend)
from .ktypes import (DEFAULT_TIMESLICE, NUM_PRIORITIES, WORD_BITS, KError, ObjType,
                     Rights, ThreadState, fail)
from .objects import CNode, Endpoint, Notification, ObjRef, Tcb, TrapFrame

_BLOCKED_STATES = (ThreadState.BLOCKED_SEND, ThreadState.BLOCKED_RECV,
                   ThreadState.BLOCKED_REPLY, ThreadState.BLOCKED_NOTIFICATION)


class TcbMixin:
    # ---------- chuyển trạng thái ----------
    def make_ready(self, ref: ObjRef, t: Tcb, value) -> None:
        t.resume_value = value
        self.write_result_regs(t, value)
        t.state, t.blocked_on = ThreadState.READY, None
        self.sched_enqueue(ref, t)

    def cancel_ipc(self, ref: ObjRef, t: Tcb) -> bool:
        """Gỡ thread khỏi mọi hàng đợi. Trả về True nếu nó đang bị chặn."""
        was_blocked = t.state in _BLOCKED_STATES
        obj = self.objs.try_get(t.blocked_on)
        if t.state in (ThreadState.BLOCKED_SEND, ThreadState.BLOCKED_RECV) and isinstance(obj, Endpoint):
            for q in (obj.send_queue, obj.recv_queue):
                if ref in q:
                    q.remove(ref)
        elif t.state == ThreadState.BLOCKED_NOTIFICATION and isinstance(obj, Notification):
            if ref in obj.waiters:
                obj.waiters.remove(ref)
        t.pending, t.blocked_on = None, None
        t.reply_epoch += 1            # mọi "quyền reply" cũ tới thread này mất hiệu lực
        self.sched_remove(ref)
        return was_blocked

    def abort_blocked(self, ref: ObjRef, err: KError) -> None:
        t = self.objs.try_get(ref)
        if t is None:
            return
        t.pending, t.blocked_on = None, None
        if t.fault_kind is not None:
            t.state = ThreadState.FAULTED
        else:
            self.make_ready(ref, t, SysResult(err=err))

    def release_caller(self, t: Tcb) -> None:
        """Server bị restart/huỷ khi đang giữ client -> client nhận CANCELLED thay vì treo mãi."""
        if t.caller is None:
            return
        c_ref, epoch = t.caller
        t.caller = None
        c = self.objs.try_get(c_ref)
        if c is None or c.state != ThreadState.BLOCKED_REPLY or c.reply_epoch != epoch:
            return
        if c.fault_kind is not None:
            c.state = ThreadState.FAULTED
        else:
            self.make_ready(c_ref, c, SysResult(err=KError.CANCELLED))

    def reset_context(self, t: Tcb) -> None:
        if t.program is not None:
            t.program.close()
        t.program = t.resume_value = t.retry_op = t.fault_kind = None
        t.regs = TrapFrame()
        t.timeslice = DEFAULT_TIMESLICE

    def tcb_teardown(self, ref: ObjRef, t: Tcb) -> None:
        self.release_caller(t)
        self.cancel_ipc(ref, t)
        self.reset_context(t)
        n = self.objs.try_get(t.bound_ntfn)
        if isinstance(n, Notification):
            n.bound_tcb = None
        self.unbind_sc(t)
        t.state = ThreadState.INACTIVE

    def configure_tcb(self, t: Tcb, *, name: str, image: str, priority: int, mcp: int,
                      cspace: ObjRef, guard: int, guard_bits: int, vspace: ObjRef,
                      fault_ep: ObjRef | None, fault_badge: int, env: dict,
                      affinity: int = 0) -> None:
        if image not in self.images:
            fail(KError.INVALID_ARG)
        if not (0 <= priority < NUM_PRIORITIES and 0 <= mcp < NUM_PRIORITIES):
            fail(KError.INVALID_ARG)
        if not 0 <= affinity < len(self.harts):
            fail(KError.INVALID_ARG)
        cn = self.objs.get(cspace)
        if not isinstance(cn, CNode) or guard_bits + cn.radix > WORD_BITS:
            fail(KError.INVALID_ARG)
        if not all(isinstance(k, str) and isinstance(v, int) for k, v in env.items()):
            fail(KError.INVALID_ARG)
        self._tcb_counter += 1
        t.name = name or f"{image}#{self._tcb_counter}"
        t.image, t.priority, t.mcp, t.affinity = image, priority, mcp, affinity
        t.cspace, t.cspace_guard, t.cspace_guard_bits = cspace, guard, guard_bits
        t.vspace = vspace
        t.fault_ep, t.fault_badge = fault_ep, fault_badge
        t.env = dict(env)

    def _target(self, t: Tcb, cptr: int) -> tuple[ObjRef, Tcb]:
        _, cap = self.lookup_cap(t, cptr, ObjType.TCB, Rights.WRITE)
        return cap.obj, self.objs.get(cap.obj)

    # ---------- syscall ----------
    def sys_tcb_configure(self, ref, t, op: TcbConfigure):
        tref, target = self._target(t, op.tcb)
        if op.priority > t.mcp or op.mcp > t.mcp:
            fail(KError.PRIO_TOO_HIGH)
        if target.state not in (ThreadState.INACTIVE, ThreadState.FAULTED):
            fail(KError.INVALID_ARG)
        _, ccap = self.lookup_cap(t, op.cspace, ObjType.CNODE)
        _, vcap = self.lookup_cap(t, op.vspace, ObjType.VSPACE)
        fault_ep, fault_badge = None, 0
        if op.fault_ep:
            _, fcap = self.lookup_cap(t, op.fault_ep, ObjType.ENDPOINT, Rights.WRITE)
            fault_ep, fault_badge = fcap.obj, fcap.badge
        self.configure_tcb(target, name="", image=op.image, priority=op.priority, mcp=op.mcp,
                           cspace=ccap.obj, guard=ccap.guard, guard_bits=ccap.guard_bits,
                           vspace=vcap.obj, fault_ep=fault_ep, fault_badge=fault_badge,
                           env=op.env, affinity=t.affinity)
        return SysResult()

    def sys_tcb_resume(self, ref, t, op: TcbResume):
        tref, target = self._target(t, op.tcb)
        if target.image is None or target.state == ThreadState.FAULTED:
            fail(KError.INVALID_ARG)
        if target.state == ThreadState.INACTIVE:
            self.make_ready(tref, target, target.resume_value)
        return SysResult()

    def sys_tcb_suspend(self, ref, t, op: TcbSuspend):
        tref, target = self._target(t, op.tcb)
        if self.cancel_ipc(tref, target):
            target.resume_value = SysResult(err=KError.CANCELLED)
        target.state = ThreadState.INACTIVE
        return SysResult()

    def sys_tcb_restart(self, ref, t, op: TcbRestart):
        tref, target = self._target(t, op.tcb)
        if target.image is None:
            fail(KError.INVALID_ARG)
        self.release_caller(target)
        self.cancel_ipc(tref, target)
        self.reset_context(target)
        self.make_ready(tref, target, None)
        return SysResult()

    def _requeue(self, tref: ObjRef, target: Tcb, change) -> None:
        queued = self.sched_queued(tref)
        self.sched_remove(tref)
        change()
        if queued:
            self.sched_enqueue(tref, target)

    def sys_tcb_set_priority(self, ref, t, op: TcbSetPriority):
        tref, target = self._target(t, op.tcb)
        if not 0 <= op.priority < NUM_PRIORITIES:
            fail(KError.INVALID_ARG)
        if op.priority > t.mcp:
            fail(KError.PRIO_TOO_HIGH)
        self._requeue(tref, target, lambda: setattr(target, "priority", op.priority))
        return SysResult()

    def sys_tcb_set_affinity(self, ref, t, op: TcbSetAffinity):
        tref, target = self._target(t, op.tcb)
        if not 0 <= op.hart < len(self.harts):
            fail(KError.INVALID_ARG)
        self._requeue(tref, target, lambda: setattr(target, "affinity", op.hart))
        return SysResult()

    def sys_tcb_bind_notification(self, ref, t, op: TcbBindNotification):
        tref, target = self._target(t, op.tcb)
        _, ncap = self.lookup_cap(t, op.ntfn, ObjType.NOTIFICATION, Rights.READ)
        self.bind_notification(tref, target, ncap.obj)
        return SysResult()

    def bind_notification(self, tref: ObjRef, target: Tcb, n_ref: ObjRef) -> None:
        n: Notification = self.objs.get(n_ref)
        if target.bound_ntfn is not None or n.bound_tcb is not None:
            fail(KError.INVALID_ARG)
        target.bound_ntfn, n.bound_tcb = n_ref, tref
