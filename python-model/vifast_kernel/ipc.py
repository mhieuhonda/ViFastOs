"""IPC đồng bộ qua Endpoint và tín hiệu bất đồng bộ qua Notification.

Endpoint: rendezvous, kernel chuyển message trực tiếp từ người gửi sang người nhận (không đệm).
Call = Send + chờ Reply; người nhận giữ "quyền trả lời một lần" (caller + reply_epoch).
Notification: một word bit, Signal OR vào, Wait lấy ra và xoá. Dùng cho ngắt và báo hiệu.
"""
from __future__ import annotations

from .abi import Call, Poll, Recv, Reply, ReplyRecv, Send, Signal, SysResult, Wait
from .ktypes import (BLOCKED, LABEL_NOTIFICATION, FaultKind, KError, KernelError,
                     Message, ObjType, Rights, ThreadState, fail)
from .objects import (CNode, Endpoint, Notification, ObjRef, PendingSend, SlotAddr, Tcb)


class IpcMixin:
    def transfer(self, sender: Tcb, msg: Message, can_grant: bool,
                 receiver: Tcb, recv_slot: int) -> Message:
        """Sao chép message; nếu được phép thì chuyển cap vào slot nhận (CNode gốc).
        Cap không chuyển được thì bị bỏ, message vẫn đi."""
        caps_out: tuple = ()
        if msg.caps and can_grant and recv_slot:
            try:
                src_addr, src_cap = self.lookup_cap(sender, msg.caps[0])
                cn = self.objs.try_get(receiver.cspace)
                if (isinstance(cn, CNode) and not cn.dying and 0 < recv_slot < len(cn.slots)
                        and cn.slots[recv_slot].cap is None):
                    self.insert_cap(SlotAddr(receiver.cspace, recv_slot),
                                    src_cap.derive(Rights.ALL), parent=src_addr)
                    caps_out = (recv_slot,)
            except KernelError:
                pass
        return Message(msg.label, msg.data, caps_out)

    def ipc_send(self, ref: ObjRef, t: Tcb, ep_ref: ObjRef, badge: int, msg: Message,
                 is_call: bool, can_grant: bool):
        ep: Endpoint = self.objs.get(ep_ref)
        if ep.recv_queue:
            r_ref = ep.recv_queue.popleft()
            r = self.objs.get(r_ref)
            delivered = self.transfer(t, msg, can_grant, r, r.recv_slot)
            if is_call:
                r.caller = (ref, t.reply_epoch)
                t.state, t.blocked_on = ThreadState.BLOCKED_REPLY, None
            self.make_ready(r_ref, r, SysResult(badge=badge, msg=delivered))
            return BLOCKED if is_call else SysResult()
        t.pending = PendingSend(badge, msg, is_call, can_grant)
        t.state, t.blocked_on = ThreadState.BLOCKED_SEND, ep_ref
        ep.send_queue.append(ref)
        return BLOCKED

    def ipc_recv(self, ref: ObjRef, t: Tcb, ep_ref: ObjRef, recv_slot: int):
        n = self.objs.try_get(t.bound_ntfn)
        if isinstance(n, Notification) and n.word:
            v, n.word = n.word, 0
            return SysResult(badge=v, msg=Message(LABEL_NOTIFICATION, (v,)))
        ep: Endpoint = self.objs.get(ep_ref)
        if ep.send_queue:
            s_ref = ep.send_queue.popleft()
            s = self.objs.get(s_ref)
            p, s.pending = s.pending, None
            delivered = self.transfer(s, p.msg, p.can_grant, t, recv_slot)
            if p.is_call:
                t.caller = (s_ref, s.reply_epoch)
                s.state, s.blocked_on = ThreadState.BLOCKED_REPLY, None
            else:
                self.make_ready(s_ref, s, SysResult())
            return SysResult(badge=p.badge, msg=delivered)
        t.recv_slot = recv_slot
        t.state, t.blocked_on = ThreadState.BLOCKED_RECV, ep_ref
        ep.recv_queue.append(ref)
        return BLOCKED

    def do_reply(self, t: Tcb, msg: Message) -> None:
        if t.caller is None:
            fail(KError.NO_CALLER)
        c_ref, epoch = t.caller
        t.caller = None
        c = self.objs.try_get(c_ref)
        if c is None or c.state != ThreadState.BLOCKED_REPLY or c.reply_epoch != epoch:
            fail(KError.NO_CALLER)
        if c.fault_kind is not None:
            # Trả lời fault: VM fault thì chạy lại lệnh lỗi, loại khác thì thread phải restart.
            kind, c.fault_kind = c.fault_kind, None
            if kind == FaultKind.VM_FAULT and c.program is not None:
                self.make_ready(c_ref, c, None)
            else:
                c.state, c.retry_op = ThreadState.INACTIVE, None
            return
        delivered = self.transfer(t, msg, True, c, c.recv_slot)
        self.make_ready(c_ref, c, SysResult(msg=delivered))

    def ntfn_signal(self, n_ref: ObjRef, bits: int) -> None:
        n: Notification = self.objs.get(n_ref)
        if n.waiters:
            w_ref = n.waiters.popleft()
            v, n.word = n.word | bits, 0
            self.make_ready(w_ref, self.objs.get(w_ref), SysResult(value=v))
            return
        b = self.objs.try_get(n.bound_tcb)
        if isinstance(b, Tcb) and b.state == ThreadState.BLOCKED_RECV:
            self.cancel_ipc(n.bound_tcb, b)
            v, n.word = n.word | bits, 0
            self.make_ready(n.bound_tcb, b,
                            SysResult(badge=v, msg=Message(LABEL_NOTIFICATION, (v,))))
            return
        n.word |= bits

    # ---------- syscall ----------
    def sys_send(self, ref, t, op: Send):
        _, cap = self.lookup_cap(t, op.ep, ObjType.ENDPOINT, Rights.WRITE)
        return self.ipc_send(ref, t, cap.obj, cap.badge, op.msg, False,
                             bool(cap.rights & Rights.GRANT))

    def sys_call(self, ref, t, op: Call):
        _, cap = self.lookup_cap(t, op.ep, ObjType.ENDPOINT, Rights.WRITE)
        t.recv_slot = op.recv_slot
        return self.ipc_send(ref, t, cap.obj, cap.badge, op.msg, True,
                             bool(cap.rights & Rights.GRANT))

    def sys_recv(self, ref, t, op: Recv):
        _, cap = self.lookup_cap(t, op.ep, ObjType.ENDPOINT, Rights.READ)
        return self.ipc_recv(ref, t, cap.obj, op.recv_slot)

    def sys_reply(self, ref, t, op: Reply):
        self.do_reply(t, op.msg)
        return SysResult()

    def sys_reply_recv(self, ref, t, op: ReplyRecv):
        _, cap = self.lookup_cap(t, op.ep, ObjType.ENDPOINT, Rights.READ)
        try:
            self.do_reply(t, op.msg)
        except KernelError:
            pass                     # giống seL4: reply hỏng thì bỏ qua, vẫn recv
        return self.ipc_recv(ref, t, cap.obj, op.recv_slot)

    def sys_signal(self, ref, t, op: Signal):
        _, cap = self.lookup_cap(t, op.ntfn, ObjType.NOTIFICATION, Rights.WRITE)
        self.ntfn_signal(cap.obj, cap.badge or 1)
        return SysResult()

    def sys_wait(self, ref, t, op: Wait):
        _, cap = self.lookup_cap(t, op.ntfn, ObjType.NOTIFICATION, Rights.READ)
        n: Notification = self.objs.get(cap.obj)
        if n.word:
            v, n.word = n.word, 0
            return SysResult(value=v)
        t.state, t.blocked_on = ThreadState.BLOCKED_NOTIFICATION, cap.obj
        n.waiters.append(ref)
        return BLOCKED

    def sys_poll(self, ref, t, op: Poll):
        _, cap = self.lookup_cap(t, op.ntfn, ObjType.NOTIFICATION, Rights.READ)
        n: Notification = self.objs.get(cap.obj)
        v, n.word = n.word, 0
        return SysResult(value=v)
