//! IPC đồng bộ qua Endpoint và tín hiệu bất đồng bộ qua Notification.
//!
//! - Endpoint: rendezvous. Kernel chuyển message trực tiếp từ người gửi sang người
//!   nhận, không đệm. Bên nào tới trước thì xếp hàng chờ bên kia.
//! - Call = Send + chờ Reply. Người nhận giữ "quyền trả lời một lần"
//!   (`caller` + `reply_epoch`); restart/huỷ đối tác làm quyền này mất hiệu lực.
//! - Notification: một word bit. Signal OR vào, Wait lấy ra và xoá. Dùng cho ngắt.

use vifast_abi::*;

use crate::kernel::{done, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

impl<P: Platform> Kernel<P> {
    /// Sao chép message; nếu được phép thì chuyển cap vào `recv_slot` (CNode gốc
    /// của người nhận). Cap không chuyển được thì bị bỏ, message vẫn đi.
    pub(crate) fn transfer(&mut self, sender: ObjRef, msg: &Message, can_grant: bool, receiver: ObjRef, recv_slot: u64) -> Message {
        let mut out = *msg;
        out.cap = None;
        let Some(cptr) = msg.cap else { return out };
        if !can_grant || recv_slot == 0 {
            return out;
        }
        let Ok((src_addr, src_cap)) = self.lookup_cap(sender, cptr, None, Rights::NONE) else { return out };
        let Some(rc) = self.objs.tcb(receiver).ok().and_then(|t| t.cspace) else { return out };
        let free = match self.live_cnode(rc) {
            Ok(cn) => {
                let i = recv_slot as usize;
                i < cn.slots.len() && cn.slots[i].cap.is_none()
            }
            Err(_) => false,
        };
        if !free {
            return out;
        }
        let Ok(derived) = src_cap.derive(Rights::ALL, 0) else { return out };
        let dest = SlotAddr { cnode: rc, index: recv_slot as usize };
        if self.insert_cap(dest, derived, Some(src_addr)).is_ok() {
            out.cap = Some(recv_slot);
        }
        out
    }

    pub(crate) fn ipc_send(&mut self, t: ObjRef, ep: ObjRef, badge: u64, msg: Message, is_call: bool, can_grant: bool) -> KResult<Ret> {
        let receiver = self.objs.endpoint_mut(ep)?.recv_queue.pop_front();
        if let Some(r) = receiver {
            let rslot = self.objs.tcb(r)?.recv_slot;
            let delivered = self.transfer(t, &msg, can_grant, r, rslot);
            if is_call {
                let epoch = self.objs.tcb(t)?.reply_epoch;
                self.objs.tcb_mut(r)?.caller = Some((t, epoch));
                let me = self.objs.tcb_mut(t)?;
                me.state = ThreadState::BlockedReply;
                me.blocked_on = None;
            }
            let res = SysResult { badge, msg: Some(delivered), ..SysResult::ok() };
            self.make_ready(r, Some(res));
            return if is_call { Ok(Ret::Blocked) } else { done() };
        }
        {
            let me = self.objs.tcb_mut(t)?;
            me.pending = Some(PendingSend { badge, msg, is_call, can_grant });
            me.state = ThreadState::BlockedSend;
            me.blocked_on = Some(ep);
        }
        self.objs.endpoint_mut(ep)?.send_queue.push_back(t);
        Ok(Ret::Blocked)
    }

    pub(crate) fn ipc_recv(&mut self, t: ObjRef, ep: ObjRef, recv_slot: u64) -> KResult<Ret> {
        // Bound notification có tín hiệu thì trả về ngay.
        if let Some(n) = self.objs.tcb(t)?.bound_ntfn {
            if let Ok(nt) = self.objs.notification_mut(n) {
                if nt.word != 0 {
                    let v = nt.word;
                    nt.word = 0;
                    let msg = Message::new(LABEL_NOTIFICATION, &[v]);
                    return Ok(Ret::Done(SysResult { badge: v, msg: Some(msg), ..SysResult::ok() }));
                }
            }
        }
        let sender = self.objs.endpoint_mut(ep)?.send_queue.pop_front();
        if let Some(s) = sender {
            let p = self.objs.tcb_mut(s)?.pending.take().ok_or(KError::InvalidArg)?;
            let delivered = self.transfer(s, &p.msg, p.can_grant, t, recv_slot);
            if p.is_call {
                let epoch = self.objs.tcb(s)?.reply_epoch;
                self.objs.tcb_mut(t)?.caller = Some((s, epoch));
                let st = self.objs.tcb_mut(s)?;
                st.state = ThreadState::BlockedReply;
                st.blocked_on = None;
            } else {
                self.make_ready(s, Some(SysResult::ok()));
            }
            return Ok(Ret::Done(SysResult { badge: p.badge, msg: Some(delivered), ..SysResult::ok() }));
        }
        {
            let me = self.objs.tcb_mut(t)?;
            me.recv_slot = recv_slot;
            me.state = ThreadState::BlockedRecv;
            me.blocked_on = Some(ep);
        }
        self.objs.endpoint_mut(ep)?.recv_queue.push_back(t);
        Ok(Ret::Blocked)
    }

    pub(crate) fn do_reply(&mut self, t: ObjRef, msg: &Message) -> KResult<()> {
        let (c, epoch) = self.objs.tcb_mut(t)?.caller.take().ok_or(KError::NoCaller)?;
        let valid = self
            .objs
            .tcb(c)
            .map_or(false, |ct| ct.state == ThreadState::BlockedReply && ct.reply_epoch == epoch);
        if !valid {
            return Err(KError::NoCaller);
        }
        let fault = self.objs.tcb_mut(c)?.fault_kind.take();
        if let Some(kind) = fault {
            // Trả lời fault: VM fault thì chạy lại lệnh lỗi, loại khác thì phải restart.
            if kind == FaultKind::VmFault {
                self.make_ready(c, None);
            } else {
                self.objs.tcb_mut(c)?.state = ThreadState::Inactive;
            }
            return Ok(());
        }
        let rslot = self.objs.tcb(c)?.recv_slot;
        let delivered = self.transfer(t, msg, true, c, rslot);
        self.make_ready(c, Some(SysResult { msg: Some(delivered), ..SysResult::ok() }));
        Ok(())
    }

    pub fn ntfn_signal(&mut self, n: ObjRef, bits: u64) -> KResult<()> {
        let waiter = self.objs.notification_mut(n)?.waiters.pop_front();
        if let Some(w) = waiter {
            let nt = self.objs.notification_mut(n)?;
            let v = nt.word | bits;
            nt.word = 0;
            self.make_ready(w, Some(SysResult::value(v)));
            return Ok(());
        }
        let bound = self.objs.notification(n)?.bound_tcb;
        if let Some(b) = bound {
            let recv_blocked = self.objs.tcb(b).map_or(false, |t| t.state == ThreadState::BlockedRecv);
            if recv_blocked {
                self.cancel_ipc(b);
                let nt = self.objs.notification_mut(n)?;
                let v = nt.word | bits;
                nt.word = 0;
                let msg = Message::new(LABEL_NOTIFICATION, &[v]);
                self.make_ready(b, Some(SysResult { badge: v, msg: Some(msg), ..SysResult::ok() }));
                return Ok(());
            }
        }
        self.objs.notification_mut(n)?.word |= bits;
        Ok(())
    }

    // ------------------------------------------------------------------
    // syscall
    // ------------------------------------------------------------------

    pub(crate) fn sys_send(&mut self, t: ObjRef, ep: u64, msg: Message) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ep, Some(ObjType::Endpoint), Rights::WRITE)?;
        let ep_ref = cap.obj.ok_or(KError::InvalidCap)?;
        self.ipc_send(t, ep_ref, cap.badge, msg, false, cap.rights.contains(Rights::GRANT))
    }

    pub(crate) fn sys_call(&mut self, t: ObjRef, ep: u64, msg: Message, recv_slot: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ep, Some(ObjType::Endpoint), Rights::WRITE)?;
        let ep_ref = cap.obj.ok_or(KError::InvalidCap)?;
        self.objs.tcb_mut(t)?.recv_slot = recv_slot;
        self.ipc_send(t, ep_ref, cap.badge, msg, true, cap.rights.contains(Rights::GRANT))
    }

    pub(crate) fn sys_recv(&mut self, t: ObjRef, ep: u64, recv_slot: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ep, Some(ObjType::Endpoint), Rights::READ)?;
        self.ipc_recv(t, cap.obj.ok_or(KError::InvalidCap)?, recv_slot)
    }

    pub(crate) fn sys_reply_recv(&mut self, t: ObjRef, ep: u64, msg: Message, recv_slot: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ep, Some(ObjType::Endpoint), Rights::READ)?;
        // Giống seL4: reply hỏng thì bỏ qua, vẫn recv.
        let _ = self.do_reply(t, &msg);
        self.ipc_recv(t, cap.obj.ok_or(KError::InvalidCap)?, recv_slot)
    }

    pub(crate) fn sys_signal(&mut self, t: ObjRef, ntfn: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ntfn, Some(ObjType::Notification), Rights::WRITE)?;
        let bits = if cap.badge != 0 { cap.badge } else { 1 };
        self.ntfn_signal(cap.obj.ok_or(KError::InvalidCap)?, bits)?;
        done()
    }

    pub(crate) fn sys_wait(&mut self, t: ObjRef, ntfn: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ntfn, Some(ObjType::Notification), Rights::READ)?;
        let n = cap.obj.ok_or(KError::InvalidCap)?;
        {
            let nt = self.objs.notification_mut(n)?;
            if nt.word != 0 {
                let v = nt.word;
                nt.word = 0;
                return Ok(Ret::Done(SysResult::value(v)));
            }
        }
        {
            let me = self.objs.tcb_mut(t)?;
            me.state = ThreadState::BlockedNotification;
            me.blocked_on = Some(n);
        }
        self.objs.notification_mut(n)?.waiters.push_back(t);
        Ok(Ret::Blocked)
    }

    pub(crate) fn sys_poll(&mut self, t: ObjRef, ntfn: u64) -> KResult<Ret> {
        let (_, cap) = self.lookup_cap(t, ntfn, Some(ObjType::Notification), Rights::READ)?;
        let nt = self.objs.notification_mut(cap.obj.ok_or(KError::InvalidCap)?)?;
        let v = nt.word;
        nt.word = 0;
        Ok(Ret::Done(SysResult::value(v)))
    }
}
