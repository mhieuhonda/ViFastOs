//! Struct `Kernel`, điểm vào trap (syscall, fault) và vòng đời một tick.
//!
//! Bên thực thi user (CPU thật, hoặc `vifast-sim` trên host) gọi kernel theo trình tự:
//! ```text
//! mỗi tick:  tick_start() -> (phần cứng: ngắt, DMA) -> tick_housekeeping()
//! mỗi hart:  schedule(h) -> chạy MỘT lệnh của thread -> after_run(h, t)
//!            lệnh là syscall    -> syscall(h, t, &sc)
//!            lệnh bị page fault -> vm_fault(h, t, addr)
//!            chương trình thoát -> user_fault(h, t, Exited, ..)
//! ```
//! Khoá kernel lớn (BKL): trong một tick chỉ một hart được vào `syscall`.
//! Hart khác nhận `Outcome::Contended` và phải chạy lại đúng lệnh đó (spin).

use alloc::collections::{BTreeMap, BinaryHeap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Reverse;

use vifast_abi::*;

use crate::irq::IrqState;
use crate::object::*;
use crate::platform::Platform;
use crate::sched::Hart;

pub struct KernelConfig {
    pub harts: usize,
    pub iommu: bool,
    /// Tên các image hợp lệ. Chỉ số trong danh sách = mã image trong TcbConfigure.
    pub image_names: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub ipi: u64,
    pub throttle: u64,
    pub revoke_preempt: u64,
    pub zombie_reaped: u64,
    pub shootdown: u64,
    pub iommu_fault: u64,
    pub spurious_irq: u64,
    pub lock_contention: u64,
}

/// Kết quả một lần trap syscall, nhìn từ phía bên thực thi user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Xong, kết quả đã lưu trong `Tcb::resume` và thanh ghi a0/a1.
    Done(SysResult),
    /// Thread bị chặn, sẽ được đánh thức kèm kết quả sau.
    Blocked,
    /// Syscall làm dở (preemption point). Chạy lại đúng syscall đó lượt sau.
    Preempted,
    /// Hart khác đang giữ khoá kernel. Chạy lại đúng syscall đó lượt sau.
    Contended,
}

/// Kết quả nội bộ của một hàm `sys_*`.
pub(crate) enum Ret {
    Done(SysResult),
    Blocked,
    Preempted,
}

pub(crate) fn done() -> KResult<Ret> {
    Ok(Ret::Done(SysResult::ok()))
}

pub struct Kernel<P: Platform> {
    pub plat: P,
    pub objs: ObjectTable,
    pub harts: Vec<Hart>,
    /// Hart đang thực thi trong kernel (dùng để tính IPI).
    pub(crate) cur_hart: Option<usize>,
    pub ticks: u64,
    pub irqs: BTreeMap<u32, IrqState>,
    /// Sự kiện nạp lại ngân sách: (thời điểm, seq, sched context, epoch).
    pub(crate) releases: BinaryHeap<Reverse<(u64, u64, ObjRef, u64)>>,
    pub(crate) release_seq: u64,
    pub reap_queue: VecDeque<ObjRef>,
    pub(crate) asid_free: Vec<u32>,
    pub next_asid: u32,
    pub(crate) lock_holder: Option<usize>,
    pub(crate) tcb_counter: u64,
    pub stats: Stats,
    pub iommu_enabled: bool,
    /// Chỉ để minh hoạ lỗi khi quên shootdown. Luôn bật trong hệ thật.
    pub tlb_shootdown_enabled: bool,
    pub image_names: Vec<String>,
}

impl<P: Platform> Kernel<P> {
    pub fn new(plat: P, cfg: KernelConfig) -> Kernel<P> {
        Kernel {
            plat,
            objs: ObjectTable::new(),
            harts: (0..cfg.harts.max(1)).map(Hart::new).collect(),
            cur_hart: None,
            ticks: 0,
            irqs: BTreeMap::new(),
            releases: BinaryHeap::new(),
            release_seq: 0,
            reap_queue: VecDeque::new(),
            asid_free: Vec::new(),
            next_asid: 0,
            lock_holder: None,
            tcb_counter: 0,
            stats: Stats::default(),
            iommu_enabled: cfg.iommu,
            tlb_shootdown_enabled: true,
            image_names: cfg.image_names,
        }
    }

    // ------------------------------------------------------------------
    // tiện ích
    // ------------------------------------------------------------------

    pub fn klog(&mut self, source: &str, text: &str) {
        let t = self.ticks;
        self.plat.log(t, source, text);
    }

    pub(crate) fn read_u64(&self, pa: u64) -> u64 {
        let mut b = [0u8; 8];
        if self.plat.phys_read(pa, &mut b) {
            u64::from_le_bytes(b)
        } else {
            0
        }
    }

    pub(crate) fn write_u64(&mut self, pa: u64, v: u64) {
        self.plat.phys_write(pa, &v.to_le_bytes());
    }

    pub fn find_tcb(&self, name: &str) -> Option<(ObjRef, &Tcb)> {
        self.objs.iter().find_map(|(r, o)| match &o.kind {
            Kind::Tcb(t) if t.name == name => Some((r, t)),
            _ => None,
        })
    }

    /// Lấy (và xoá) kết quả đang chờ giao cho thread.
    pub fn take_resume(&mut self, t: ObjRef) -> Option<SysResult> {
        self.objs.tcb_mut(t).ok().and_then(|x| x.resume.take())
    }

    /// Còn việc nền (dọn zombie, chờ nạp ngân sách cho thread bị throttle)?
    pub fn has_pending_work(&self) -> bool {
        if !self.reap_queue.is_empty() {
            return true;
        }
        !self.releases.is_empty()
            && self.objs.iter().any(|(_, o)| matches!(&o.kind, Kind::Tcb(t) if t.throttled))
    }

    // ------------------------------------------------------------------
    // vòng đời một tick
    // ------------------------------------------------------------------

    /// Đầu tick: tăng đồng hồ. Sau đó bên ngoài xử lý ngắt và thiết bị.
    pub fn tick_start(&mut self) {
        self.ticks += 1;
    }

    /// Việc nền của kernel trong tick: nạp ngân sách, dọn zombie, nhả khoá.
    pub fn tick_housekeeping(&mut self) {
        self.process_releases();
        self.reap(REAP_BUDGET);
        self.lock_holder = None;
    }

    /// Chọn thread cho hart `h`. `None` = hart rảnh.
    pub fn schedule(&mut self, h: usize) -> Option<ObjRef> {
        let r = self.schedule_inner(h);
        if r.is_some() {
            self.harts[h].busy += 1;
        } else {
            self.harts[h].idle += 1;
        }
        r
    }

    /// Sau khi thread chạy xong một lệnh: tính tick CPU hoặc nhả hart.
    pub fn after_run(&mut self, h: usize, t: ObjRef) {
        let state = match self.objs.tcb(t) {
            Ok(x) => x.state,
            Err(_) => {
                self.harts[h].current = None;
                return;
            }
        };
        if state == ThreadState::Running {
            self.charge(h, t);
        } else if self.harts[h].current == Some(t) {
            self.harts[h].current = None;
        }
    }

    // ------------------------------------------------------------------
    // trap
    // ------------------------------------------------------------------

    /// Trap `ecall`: xử lý một syscall của thread `t` đang chạy trên hart `h`.
    pub fn syscall(&mut self, h: usize, t: ObjRef, sc: &Syscall) -> Outcome {
        if let Some(holder) = self.lock_holder {
            if holder != h {
                self.stats.lock_contention += 1;
                return Outcome::Contended;
            }
        }
        self.lock_holder = Some(h);
        self.harts[h].traps += 1;
        self.cur_hart = Some(h);
        if let Ok(x) = self.objs.tcb_mut(t) {
            x.regs.a[7] = sc.number();
        }
        let res = self.dispatch(t, sc);
        let out = match res {
            Ok(Ret::Done(r)) => Outcome::Done(r),
            Ok(Ret::Blocked) => Outcome::Blocked,
            Ok(Ret::Preempted) => Outcome::Preempted,
            Err(e) => Outcome::Done(SysResult::error(e)),
        };
        if out != Outcome::Preempted {
            if let Ok(x) = self.objs.tcb_mut(t) {
                x.regs.pc += 4;
                if let Outcome::Done(r) = &out {
                    x.resume = Some(*r);
                    write_result_regs(&mut x.regs, r);
                }
            }
        }
        self.stats_preempt(&out);
        self.cur_hart = None;
        out
    }

    fn stats_preempt(&mut self, out: &Outcome) {
        if *out == Outcome::Preempted {
            self.stats.revoke_preempt += 1;
        }
    }

    /// Page fault khi thread `t` truy cập `addr`.
    pub fn vm_fault(&mut self, h: usize, t: ObjRef, addr: u64) {
        self.cur_hart = Some(h);
        self.raise_fault(t, FaultKind::VmFault, addr, "");
        self.cur_hart = None;
    }

    /// Chương trình kết thúc (`Exited`) hoặc panic (`UserException`).
    pub fn user_fault(&mut self, h: usize, t: ObjRef, kind: FaultKind, detail: &str) {
        self.cur_hart = Some(h);
        self.raise_fault(t, kind, 0, detail);
        self.cur_hart = None;
    }

    /// Gửi fault tới fault handler như một Call. Không có handler thì thread dừng ở Faulted.
    pub fn raise_fault(&mut self, t: ObjRef, kind: FaultKind, addr: u64, detail: &str) {
        let (name, fault_ep, fault_badge) = match self.objs.tcb(t) {
            Ok(x) => (x.name.clone(), x.fault_ep, x.fault_badge),
            Err(_) => return,
        };
        let text = if kind == FaultKind::Exited {
            format!("'{}' kết thúc", name)
        } else {
            let s = format!("fault {} ở '{}' addr={:#x} {}", kind.name(), name, addr, detail);
            String::from(s.trim_end())
        };
        self.klog("kernel", &text);
        if let Ok(x) = self.objs.tcb_mut(t) {
            x.fault_kind = Some(kind);
        }
        let ep_alive = fault_ep.map_or(false, |e| self.objs.endpoint(e).is_ok());
        match fault_ep {
            Some(ep) if ep_alive => {
                let msg = Message::new(LABEL_FAULT, &[kind as u64, addr]);
                let _ = self.ipc_send(t, ep, fault_badge, msg, true, false);
            }
            _ => {
                if let Ok(x) = self.objs.tcb_mut(t) {
                    x.state = ThreadState::Faulted;
                }
            }
        }
    }

    fn dispatch(&mut self, t: ObjRef, sc: &Syscall) -> KResult<Ret> {
        use Syscall as S;
        match sc {
            S::Send { ep, msg } => self.sys_send(t, *ep, *msg),
            S::Call { ep, msg, recv_slot } => self.sys_call(t, *ep, *msg, *recv_slot),
            S::Recv { ep, recv_slot } => self.sys_recv(t, *ep, *recv_slot),
            S::Reply { msg } => {
                self.do_reply(t, msg)?;
                done()
            }
            S::ReplyRecv { ep, msg, recv_slot } => self.sys_reply_recv(t, *ep, *msg, *recv_slot),
            S::Signal { ntfn } => self.sys_signal(t, *ntfn),
            S::Wait { ntfn } => self.sys_wait(t, *ntfn),
            S::Poll { ntfn } => self.sys_poll(t, *ntfn),
            S::Yield => self.sys_yield(t),
            S::Retype { untyped, obj_type, size_bits, dest_index, count, dest_cnode } => {
                self.sys_retype(t, *untyped, *obj_type, *size_bits, *dest_index, *count, *dest_cnode)
            }
            S::CapCopy { src, dest_index, rights, dest_cnode } => {
                self.sys_cap_copy(t, *src, *dest_index, *rights, *dest_cnode)
            }
            S::CapMint { src, dest_index, rights, badge, dest_cnode } => {
                self.sys_cap_mint(t, *src, *dest_index, *rights, *badge, *dest_cnode)
            }
            S::CNodeMint { src, dest_index, guard, guard_bits, dest_cnode } => {
                self.sys_cnode_mint(t, *src, *dest_index, *guard, *guard_bits, *dest_cnode)
            }
            S::CapDelete { slot } => self.sys_cap_delete(t, *slot),
            S::CapRevoke { slot } => self.sys_cap_revoke(t, *slot),
            S::TcbConfigure { tcb, cspace, vspace, priority, mcp, image, args, fault_ep } => {
                self.sys_tcb_configure(t, *tcb, *cspace, *vspace, *priority, *mcp, *image, *args, *fault_ep)
            }
            S::TcbResume { tcb } => self.sys_tcb_resume(t, *tcb),
            S::TcbSuspend { tcb } => self.sys_tcb_suspend(t, *tcb),
            S::TcbRestart { tcb } => self.sys_tcb_restart(t, *tcb),
            S::TcbSetPriority { tcb, priority } => self.sys_tcb_set_priority(t, *tcb, *priority),
            S::TcbSetAffinity { tcb, hart } => self.sys_tcb_set_affinity(t, *tcb, *hart),
            S::TcbBindNotification { tcb, ntfn } => self.sys_tcb_bind_notification(t, *tcb, *ntfn),
            S::SchedContextConfigure { sched_control, sc, budget, period } => {
                self.sys_sc_configure(t, *sched_control, *sc, *budget, *period)
            }
            S::SchedContextBind { sc, tcb } => self.sys_sc_bind(t, *sc, *tcb),
            S::PageTableMap { page_table, vspace, vaddr } => {
                self.sys_page_table_map(t, *page_table, *vspace, *vaddr)
            }
            S::FrameMap { frame, vspace, vaddr, rights } => {
                self.sys_frame_map(t, *frame, *vspace, *vaddr, *rights)
            }
            S::FrameUnmap { frame } => self.sys_frame_unmap(t, *frame),
            S::IoMap { frame, iospace, iova, rights } => self.sys_io_map(t, *frame, *iospace, *iova, *rights),
            S::IoUnmap { frame } => self.sys_io_unmap(t, *frame),
            S::IrqGet { control, irq, dest_index, dest_cnode } => {
                self.sys_irq_get(t, *control, *irq, *dest_index, *dest_cnode)
            }
            S::IrqSetNotification { handler, ntfn } => self.sys_irq_set_notification(t, *handler, *ntfn),
            S::IrqAck { handler } => self.sys_irq_ack(t, *handler),
            S::DebugPrint { text } => {
                let name = self.objs.tcb(t)?.name.clone();
                self.klog(&name, text);
                done()
            }
            S::GetTime => Ok(Ret::Done(SysResult::value(self.ticks))),
        }
    }

    fn sys_yield(&mut self, t: ObjRef) -> KResult<Ret> {
        self.objs.tcb_mut(t)?.timeslice = 0;
        if let Some(h) = self.cur_hart {
            self.harts[h].resched = true;
        }
        done()
    }
}

/// Quy ước trả về: a0 = mã lỗi, a1 = value (hoặc badge nếu value = 0).
pub fn write_result_regs(regs: &mut TrapFrame, r: &SysResult) {
    regs.a[0] = r.err as u64;
    regs.a[1] = if r.value != 0 { r.value } else { r.badge };
}
