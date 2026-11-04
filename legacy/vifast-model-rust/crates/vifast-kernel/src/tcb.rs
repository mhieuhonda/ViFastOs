//! Vòng đời thread: cấu hình, chạy, dừng, khởi động lại, huỷ, đổi hart.
//!
//! Chính sách (restart bao nhiêu lần, khi nào vào safe mode) KHÔNG nằm ở đây mà ở
//! supervisor chạy trong user space. Kernel chỉ cung cấp cơ chế.

use alloc::format;
use alloc::string::String;

use vifast_abi::*;

use crate::kernel::{done, write_result_regs, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

/// Tham số cấu hình thread (dùng chung cho boot và syscall TcbConfigure).
#[derive(Clone, Debug)]
pub struct TcbConfig {
    /// Rỗng: tự đặt tên "<image>#<số thứ tự>".
    pub name: String,
    pub image: u32,
    pub priority: u32,
    pub mcp: u32,
    pub cspace: ObjRef,
    pub guard: u64,
    pub guard_bits: u32,
    pub vspace: ObjRef,
    pub fault_ep: Option<ObjRef>,
    pub fault_badge: u64,
    pub args: [u64; NUM_ARGS],
    pub affinity: usize,
}

impl<P: Platform> Kernel<P> {
    // ------------------------------------------------------------------
    // chuyển trạng thái
    // ------------------------------------------------------------------

    /// Đưa thread vào hàng đợi, kèm kết quả sẽ giao cho nó khi chạy lại.
    pub fn make_ready(&mut self, r: ObjRef, value: Option<SysResult>) {
        let Ok(t) = self.objs.tcb_mut(r) else { return };
        t.resume = value;
        if let Some(v) = &value {
            write_result_regs(&mut t.regs, v);
        }
        t.state = ThreadState::Ready;
        t.blocked_on = None;
        self.sched_enqueue(r, false);
    }

    /// Gỡ thread khỏi mọi hàng đợi. Trả `true` nếu nó đang bị chặn.
    pub(crate) fn cancel_ipc(&mut self, r: ObjRef) -> bool {
        let (state, blocked_on) = match self.objs.tcb(r) {
            Ok(t) => (t.state, t.blocked_on),
            Err(_) => return false,
        };
        if let Some(o) = blocked_on {
            match state {
                ThreadState::BlockedSend | ThreadState::BlockedRecv => {
                    if let Ok(ep) = self.objs.endpoint_mut(o) {
                        ep.send_queue.retain(|x| *x != r);
                        ep.recv_queue.retain(|x| *x != r);
                    }
                }
                ThreadState::BlockedNotification => {
                    if let Ok(n) = self.objs.notification_mut(o) {
                        n.waiters.retain(|x| *x != r);
                    }
                }
                _ => {}
            }
        }
        if let Ok(t) = self.objs.tcb_mut(r) {
            t.pending = None;
            t.blocked_on = None;
            t.reply_epoch += 1; // mọi "quyền reply" cũ tới thread này mất hiệu lực
        }
        self.sched_remove(r);
        state.is_blocked()
    }

    /// Đối tượng mà thread đang chờ bị huỷ.
    pub(crate) fn abort_blocked(&mut self, r: ObjRef, err: KError) {
        let faulted = match self.objs.tcb_mut(r) {
            Ok(t) => {
                t.pending = None;
                t.blocked_on = None;
                t.fault_kind.is_some()
            }
            Err(_) => return,
        };
        if faulted {
            if let Ok(t) = self.objs.tcb_mut(r) {
                t.state = ThreadState::Faulted;
            }
        } else {
            self.make_ready(r, Some(SysResult::error(err)));
        }
    }

    /// Server bị restart/huỷ khi đang giữ client: client nhận CANCELLED thay vì treo mãi.
    pub(crate) fn release_caller(&mut self, r: ObjRef) {
        let caller = match self.objs.tcb_mut(r) {
            Ok(t) => t.caller.take(),
            Err(_) => None,
        };
        let Some((c, epoch)) = caller else { return };
        let (valid, faulted) = match self.objs.tcb(c) {
            Ok(ct) => (ct.state == ThreadState::BlockedReply && ct.reply_epoch == epoch, ct.fault_kind.is_some()),
            Err(_) => (false, false),
        };
        if !valid {
            return;
        }
        if faulted {
            if let Ok(ct) = self.objs.tcb_mut(c) {
                ct.state = ThreadState::Faulted;
            }
        } else {
            self.make_ready(c, Some(SysResult::error(KError::Cancelled)));
        }
    }

    /// Xoá ngữ cảnh thực thi: lần chạy sau bắt đầu lại từ điểm vào.
    pub(crate) fn reset_context(&mut self, r: ObjRef) {
        if let Ok(t) = self.objs.tcb_mut(r) {
            t.context_epoch += 1;
            t.resume = None;
            t.fault_kind = None;
            t.regs = TrapFrame::default();
            t.timeslice = DEFAULT_TIMESLICE;
        }
    }

    pub(crate) fn tcb_teardown(&mut self, r: ObjRef) {
        self.release_caller(r);
        self.cancel_ipc(r);
        self.reset_context(r);
        let bound = self.objs.tcb(r).ok().and_then(|t| t.bound_ntfn);
        if let Some(n) = bound {
            if let Ok(nt) = self.objs.notification_mut(n) {
                nt.bound_tcb = None;
            }
        }
        self.unbind_sc(r);
        if let Ok(t) = self.objs.tcb_mut(r) {
            t.state = ThreadState::Inactive;
        }
    }

    pub fn configure_tcb(&mut self, r: ObjRef, cfg: &TcbConfig) -> KResult<()> {
        if cfg.image as usize >= self.image_names.len() {
            return Err(KError::InvalidArg);
        }
        if cfg.priority as usize >= NUM_PRIORITIES || cfg.mcp as usize >= NUM_PRIORITIES {
            return Err(KError::InvalidArg);
        }
        if cfg.affinity >= self.harts.len() {
            return Err(KError::InvalidArg);
        }
        let radix = self.objs.cnode(cfg.cspace).map_err(|_| KError::InvalidArg)?.radix();
        if cfg.guard_bits + radix > WORD_BITS {
            return Err(KError::InvalidArg);
        }
        self.tcb_counter += 1;
        let name = if cfg.name.is_empty() {
            format!("{}#{}", self.image_names[cfg.image as usize], self.tcb_counter)
        } else {
            cfg.name.clone()
        };
        let t = self.objs.tcb_mut(r)?;
        t.name = name;
        t.image = Some(cfg.image);
        t.priority = cfg.priority as u8;
        t.mcp = cfg.mcp as u8;
        t.affinity = cfg.affinity;
        t.cspace = Some(cfg.cspace);
        t.cspace_guard = cfg.guard;
        t.cspace_guard_bits = cfg.guard_bits as u8;
        t.vspace = Some(cfg.vspace);
        t.fault_ep = cfg.fault_ep;
        t.fault_badge = cfg.fault_badge;
        t.args = cfg.args;
        Ok(())
    }

    pub fn bind_notification(&mut self, tr: ObjRef, n: ObjRef) -> KResult<()> {
        let t_bound = self.objs.tcb(tr)?.bound_ntfn.is_some();
        let n_bound = self.objs.notification(n)?.bound_tcb.is_some();
        if t_bound || n_bound {
            return Err(KError::InvalidArg);
        }
        self.objs.tcb_mut(tr)?.bound_ntfn = Some(n);
        self.objs.notification_mut(n)?.bound_tcb = Some(tr);
        Ok(())
    }

    fn target(&self, t: ObjRef, cptr: u64) -> KResult<ObjRef> {
        let (_, cap) = self.lookup_cap(t, cptr, Some(ObjType::Tcb), Rights::WRITE)?;
        cap.obj.ok_or(KError::InvalidCap)
    }

    fn requeue<F: FnOnce(&mut Tcb)>(&mut self, tr: ObjRef, change: F) -> KResult<()> {
        let queued = self.sched_queued(tr);
        self.sched_remove(tr);
        change(self.objs.tcb_mut(tr)?);
        if queued {
            self.sched_enqueue(tr, false);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // syscall
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sys_tcb_configure(&mut self, t: ObjRef, tcb: u64, cspace: u64, vspace: u64, priority: u32, mcp: u32, image: u32, args: [u64; NUM_ARGS], fault_ep: u64) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        let (caller_mcp, aff) = {
            let me = self.objs.tcb(t)?;
            (me.mcp as u32, me.affinity)
        };
        if priority > caller_mcp || mcp > caller_mcp {
            return Err(KError::PrioTooHigh);
        }
        let st = self.objs.tcb(tr)?.state;
        if !matches!(st, ThreadState::Inactive | ThreadState::Faulted) {
            return Err(KError::InvalidArg);
        }
        let (_, ccap) = self.lookup_cap(t, cspace, Some(ObjType::CNode), Rights::NONE)?;
        let (_, vcap) = self.lookup_cap(t, vspace, Some(ObjType::VSpace), Rights::NONE)?;
        let (fep, fbadge) = if fault_ep != 0 {
            let (_, fcap) = self.lookup_cap(t, fault_ep, Some(ObjType::Endpoint), Rights::WRITE)?;
            (fcap.obj, fcap.badge)
        } else {
            (None, 0)
        };
        let cfg = TcbConfig {
            name: String::new(),
            image,
            priority,
            mcp,
            cspace: ccap.obj.ok_or(KError::InvalidCap)?,
            guard: ccap.guard,
            guard_bits: ccap.guard_bits as u32,
            vspace: vcap.obj.ok_or(KError::InvalidCap)?,
            fault_ep: fep,
            fault_badge: fbadge,
            args,
            affinity: aff,
        };
        self.configure_tcb(tr, &cfg)?;
        done()
    }

    pub(crate) fn sys_tcb_resume(&mut self, t: ObjRef, tcb: u64) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        let (image, state, resume) = {
            let x = self.objs.tcb(tr)?;
            (x.image, x.state, x.resume)
        };
        if image.is_none() || state == ThreadState::Faulted {
            return Err(KError::InvalidArg);
        }
        if state == ThreadState::Inactive {
            self.make_ready(tr, resume);
        }
        done()
    }

    pub(crate) fn sys_tcb_suspend(&mut self, t: ObjRef, tcb: u64) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        let was_blocked = self.cancel_ipc(tr);
        let x = self.objs.tcb_mut(tr)?;
        if was_blocked {
            x.resume = Some(SysResult::error(KError::Cancelled));
        }
        x.state = ThreadState::Inactive;
        done()
    }

    pub(crate) fn sys_tcb_restart(&mut self, t: ObjRef, tcb: u64) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        if self.objs.tcb(tr)?.image.is_none() {
            return Err(KError::InvalidArg);
        }
        self.release_caller(tr);
        self.cancel_ipc(tr);
        self.reset_context(tr);
        self.make_ready(tr, None);
        done()
    }

    pub(crate) fn sys_tcb_set_priority(&mut self, t: ObjRef, tcb: u64, priority: u32) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        if priority as usize >= NUM_PRIORITIES {
            return Err(KError::InvalidArg);
        }
        if priority > self.objs.tcb(t)?.mcp as u32 {
            return Err(KError::PrioTooHigh);
        }
        self.requeue(tr, |x| x.priority = priority as u8)?;
        done()
    }

    pub(crate) fn sys_tcb_set_affinity(&mut self, t: ObjRef, tcb: u64, hart: u32) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        if hart as usize >= self.harts.len() {
            return Err(KError::InvalidArg);
        }
        self.requeue(tr, |x| x.affinity = hart as usize)?;
        done()
    }

    pub(crate) fn sys_tcb_bind_notification(&mut self, t: ObjRef, tcb: u64, ntfn: u64) -> KResult<Ret> {
        let tr = self.target(t, tcb)?;
        let (_, ncap) = self.lookup_cap(t, ntfn, Some(ObjType::Notification), Rights::READ)?;
        self.bind_notification(tr, ncap.obj.ok_or(KError::InvalidCap)?)?;
        done()
    }
}
