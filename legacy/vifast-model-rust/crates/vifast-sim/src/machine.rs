//! Máy giả lập nhiều hart: CPU (MMU + TLB), thiết bị, nguồn ngắt, và vòng lặp tick.
//!
//! Mỗi tick:
//! 1. `kernel.tick_start()`
//! 2. phần cứng: ngắt ngoài đến hạn, thiết bị DMA hoàn tất
//! 3. `kernel.tick_housekeeping()`: nạp ngân sách, dọn zombie, nhả khoá kernel
//! 4. mỗi hart (xoay vòng thứ tự để công bằng) chạy đúng MỘT lệnh của thread hiện tại

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::rc::Rc;
use std::task::Poll;

use vifast_abi::{FaultKind, PAGE_SHIFT, PAGE_SIZE};
use vifast_kernel::vm::{self, MAX_ACCESS, PTE_R, PTE_U, PTE_W, PTE_X, USER_VA_LIMIT};
use vifast_kernel::{Kernel, KernelConfig, ObjRef, Outcome, Tcb};

use crate::hw::{DmaSensor, Hw, PhysMem, Tlb};
use crate::user::{poll_program, Chan, Env, Images, Instr, Program, Ret, Sys};

/// Ngữ cảnh thực thi của một thread phía simulator (tương đương thanh ghi + PC thật).
struct ThreadCtx {
    epoch: u64,
    program: Option<Program>,
    /// Lệnh phải chạy lại (PC chưa tăng): page fault, preempted, tranh khoá.
    held: Option<Instr>,
    /// Kết quả Load/Store/Fetch chờ giao cho chương trình.
    pending_data: Option<Vec<u8>>,
    chan: Rc<RefCell<Chan>>,
}

impl ThreadCtx {
    fn new(epoch: u64) -> ThreadCtx {
        ThreadCtx { epoch, program: None, held: None, pending_data: None, chan: Rc::new(RefCell::new(Chan::default())) }
    }
}

pub struct Machine {
    pub kernel: Kernel<Hw>,
    pub devices: Vec<DmaSensor>,
    pub images: Images,
    /// Env của các task tạo lúc boot.
    pub boot_env: BTreeMap<ObjRef, Env>,
    irq_events: BinaryHeap<Reverse<(u64, u64, u32, u64)>>,
    irq_seq: u64,
    threads: BTreeMap<ObjRef, ThreadCtx>,
}

impl Machine {
    pub fn new(images: Images, ram_size: u64, harts: usize, iommu: bool, verbose: bool) -> Machine {
        let harts = harts.max(1);
        let hw = Hw {
            mem: PhysMem::new(ram_size),
            tlbs: (0..harts).map(|_| Tlb::new(64)).collect(),
            log: Vec::new(),
            verbose,
        };
        let cfg = KernelConfig { harts, iommu, image_names: images.names() };
        Machine {
            kernel: Kernel::new(hw, cfg),
            devices: Vec::new(),
            images,
            boot_env: BTreeMap::new(),
            irq_events: BinaryHeap::new(),
            irq_seq: 0,
            threads: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // tiện ích quan sát
    // ------------------------------------------------------------------

    pub fn ticks(&self) -> u64 {
        self.kernel.ticks
    }

    /// Mọi dòng log của một nguồn (tên task, "kernel", "iommu"...).
    pub fn lines(&self, source: &str) -> Vec<String> {
        self.kernel.plat.log.iter().filter(|l| l.source == source).map(|l| l.text.clone()).collect()
    }

    pub fn all_lines(&self) -> Vec<String> {
        self.kernel.plat.log.iter().map(|l| l.text.clone()).collect()
    }

    pub fn find_tcb(&self, name: &str) -> Option<(ObjRef, &Tcb)> {
        self.kernel.find_tcb(name)
    }

    pub fn kernel_canary_intact(&self) -> bool {
        self.kernel.plat.mem.kernel_canary_intact()
    }

    pub fn report(&self) -> Vec<(String, String)> {
        let s = &self.kernel.stats;
        let mut out = vec![
            ("ticks".to_string(), self.kernel.ticks.to_string()),
            ("ipi".to_string(), s.ipi.to_string()),
            ("lock_contention".to_string(), s.lock_contention.to_string()),
            ("throttle".to_string(), s.throttle.to_string()),
            ("shootdown".to_string(), s.shootdown.to_string()),
            ("iommu_fault".to_string(), s.iommu_fault.to_string()),
            ("revoke_preempt".to_string(), s.revoke_preempt.to_string()),
            ("zombie_reaped".to_string(), s.zombie_reaped.to_string()),
        ];
        for h in &self.kernel.harts {
            let tlb = &self.kernel.plat.tlbs[h.id];
            out.push((
                format!("hart{}", h.id),
                format!(
                    "busy={} idle={} ctx_switch={} traps={} timer_irq={} tlb_hit={} tlb_miss={}",
                    h.busy, h.idle, h.ctx_switches, h.traps, h.timer_irqs, tlb.hits, tlb.misses
                ),
            ));
        }
        out
    }

    // ------------------------------------------------------------------
    // nguồn ngắt và thiết bị
    // ------------------------------------------------------------------

    /// Ngắt ngoài giả lập (ví dụ cảm biến phát ngắt định kỳ). `period = 0`: một lần.
    pub fn schedule_irq(&mut self, irq: u32, at: u64, period: u64) {
        self.irq_seq += 1;
        self.irq_events.push(Reverse((at, self.irq_seq, irq, period)));
    }

    fn poll_hardware(&mut self) {
        let now = self.kernel.ticks;
        while let Some(Reverse((at, _, irq, period))) = self.irq_events.peek().copied() {
            if at > now {
                break;
            }
            self.irq_events.pop();
            self.kernel.deliver_irq(irq);
            if period > 0 {
                self.schedule_irq(irq, at + period, period);
            }
        }
        for i in 0..self.devices.len() {
            self.device_tick(i);
        }
    }

    fn device_tick(&mut self, i: usize) {
        let now = self.kernel.ticks;
        let (addr, payload, iospace, name, irq) = {
            let d = &mut self.devices[i];
            match d.done_at {
                Some(at) if now >= at => {}
                _ => return,
            }
            d.done_at = None;
            d.counter += 1;
            let len = d.regs[1].min(64) as usize;
            let mut payload = format!("SENSOR-{}", d.counter).into_bytes();
            payload.truncate(len);
            payload.resize(len, 0);
            (d.regs[0], payload, d.iospace, d.name.clone(), d.irq)
        };
        let ok = match self.kernel.dma_plan(iospace, &name, addr, payload.len(), true) {
            Some(plan) => plan.iter().all(|&(pa, off, n)| self.kernel.plat.mem.write(pa, &payload[off..off + n])),
            None => false,
        };
        self.devices[i].regs[3] = if ok { DmaSensor::DONE } else { DmaSensor::ERROR };
        self.kernel.deliver_irq(irq);
    }

    fn hardware_pending(&self) -> bool {
        !self.irq_events.is_empty() || self.devices.iter().any(|d| d.busy()) || self.kernel.has_pending_work()
    }

    // ------------------------------------------------------------------
    // vòng lặp
    // ------------------------------------------------------------------

    /// Một tick. `false` nếu mọi hart rảnh.
    pub fn step(&mut self) -> bool {
        self.kernel.tick_start();
        self.poll_hardware();
        self.kernel.tick_housekeeping();
        let n = self.kernel.harts.len();
        let base = self.kernel.ticks as usize;
        let mut busy = false;
        for i in 0..n {
            busy |= self.run_hart((base + i) % n);
        }
        busy
    }

    pub fn run(&mut self, max_ticks: u64) -> &mut Machine {
        for _ in 0..max_ticks {
            if !self.step() && !self.hardware_pending() {
                break;
            }
        }
        self
    }

    fn run_hart(&mut self, h: usize) -> bool {
        let Some(t) = self.kernel.schedule(h) else { return false };
        let epoch = match self.kernel.objs.tcb(t) {
            Ok(x) => x.context_epoch,
            Err(_) => {
                self.kernel.after_run(h, t);
                return true;
            }
        };
        let mut ctx = match self.threads.remove(&t) {
            Some(c) if c.epoch == epoch => c,
            _ => ThreadCtx::new(epoch),
        };
        let instr = match ctx.held.take() {
            Some(i) => Some(i),
            None => self.advance(h, t, &mut ctx),
        };
        if let Some(i) = instr {
            self.execute(h, t, i, &mut ctx);
        }
        self.threads.insert(t, ctx);
        self.kernel.after_run(h, t);
        true
    }

    /// Cho chương trình chạy tới lệnh kế tiếp. `None` nếu chương trình đã kết thúc.
    fn advance(&mut self, h: usize, t: ObjRef, ctx: &mut ThreadCtx) -> Option<Instr> {
        let resume = self.kernel.take_resume(t);
        if ctx.program.is_none() {
            let (image, args) = match self.kernel.objs.tcb(t) {
                Ok(x) => (x.image, x.args),
                Err(_) => return None,
            };
            let Some(def) = image.and_then(|i| self.images.get(i)) else {
                self.kernel.user_fault(h, t, FaultKind::BadSyscall, "image không tồn tại");
                return None;
            };
            let env: Env = match self.boot_env.get(&t) {
                Some(e) => e.clone(),
                None => def.args.iter().cloned().zip(args.iter().copied()).collect(),
            };
            ctx.chan = Rc::new(RefCell::new(Chan::default()));
            ctx.pending_data = None;
            ctx.program = Some((def.ctor)(Sys::new(ctx.chan.clone(), env)));
        } else {
            let ret = match ctx.pending_data.take() {
                Some(d) => Ret::Data(d),
                None => match resume {
                    Some(r) => Ret::Sys(r),
                    None => Ret::None,
                },
            };
            ctx.chan.borrow_mut().inp = Some(ret);
        }
        let poll = match ctx.program.as_mut() {
            Some(p) => poll_program(p),
            None => return None,
        };
        match poll {
            Poll::Ready(Ok(())) => {
                ctx.program = None;
                self.kernel.user_fault(h, t, FaultKind::Exited, "");
                None
            }
            Poll::Ready(Err(e)) => {
                ctx.program = None;
                self.kernel.user_fault(h, t, FaultKind::UserException, &e);
                None
            }
            Poll::Pending => {
                let out = ctx.chan.borrow_mut().out.take();
                if out.is_none() {
                    ctx.program = None;
                    self.kernel.user_fault(h, t, FaultKind::UserException, "chương trình chờ thứ không phải lệnh máy");
                }
                out
            }
        }
    }

    fn execute(&mut self, h: usize, t: ObjRef, instr: Instr, ctx: &mut ThreadCtx) {
        match instr {
            Instr::Sys(sc) => match self.kernel.syscall(h, t, &sc) {
                Outcome::Preempted | Outcome::Contended => ctx.held = Some(Instr::Sys(sc)),
                Outcome::Done(_) | Outcome::Blocked => {}
            },
            Instr::Load { vaddr, len } => {
                if len > MAX_ACCESS {
                    ctx.pending_data = Some(Vec::new());
                    return;
                }
                match self.access(h, t, vaddr, len as usize, PTE_R, None) {
                    Ok(d) => ctx.pending_data = Some(d),
                    Err(addr) => {
                        ctx.held = Some(Instr::Load { vaddr, len });
                        self.kernel.vm_fault(h, t, addr);
                    }
                }
            }
            Instr::Store { vaddr, data } => {
                if data.len() as u64 > MAX_ACCESS {
                    ctx.pending_data = Some(Vec::new());
                    return;
                }
                match self.access(h, t, vaddr, data.len(), PTE_W, Some(&data)) {
                    Ok(_) => ctx.pending_data = Some(Vec::new()),
                    Err(addr) => {
                        ctx.held = Some(Instr::Store { vaddr, data });
                        self.kernel.vm_fault(h, t, addr);
                    }
                }
            }
            Instr::Fetch { vaddr } => match self.access(h, t, vaddr, 4, PTE_X, None) {
                Ok(d) => ctx.pending_data = Some(d),
                Err(addr) => {
                    ctx.held = Some(Instr::Fetch { vaddr });
                    self.kernel.vm_fault(h, t, addr);
                }
            },
        }
    }

    // ------------------------------------------------------------------
    // MMU
    // ------------------------------------------------------------------

    /// Dịch địa chỉ như phần cứng: TLB trước, miss thì đi bảng Sv39.
    fn translate(&mut self, h: usize, t: ObjRef, va: u64, need: u64) -> Result<u64, u64> {
        let (root, asid) = self.kernel.address_space(t).ok_or(va)?;
        if va >= USER_VA_LIMIT {
            return Err(va);
        }
        let vpn = va >> PAGE_SHIFT;
        let hw = &mut self.kernel.plat;
        let pte = match hw.tlbs[h].get(asid, vpn) {
            Some(p) => p,
            None => {
                let mem = &hw.mem;
                let p = vm::walk(|pa: u64| mem.read_u64(pa), root, va).ok_or(va)?;
                hw.tlbs[h].put(asid, vpn, p);
                p
            }
        };
        if pte & PTE_U == 0 || pte & need == 0 {
            return Err(va);
        }
        Ok(vm::pte_pa(pte) | (va & (PAGE_SIZE - 1)))
    }

    /// Dịch toàn bộ vùng trước, sau đó mới đọc/ghi (lỗi thì không ghi gì).
    fn access(&mut self, h: usize, t: ObjRef, va: u64, len: usize, need: u64, write: Option<&[u8]>) -> Result<Vec<u8>, u64> {
        let end = va.saturating_add(len as u64);
        let mut chunks = Vec::new();
        let mut addr = va;
        while addr < end {
            let n = (PAGE_SIZE - (addr & (PAGE_SIZE - 1))).min(end - addr);
            let pa = self.translate(h, t, addr, need)?;
            chunks.push((pa, (addr - va) as usize, n as usize));
            addr += n;
        }
        let mut out = Vec::new();
        for (pa, off, n) in chunks {
            match write {
                Some(d) => {
                    if !self.mem_write(pa, &d[off..off + n]) {
                        return Err(va + off as u64);
                    }
                }
                None => match self.mem_read(pa, n) {
                    Some(b) => out.extend(b),
                    None => return Err(va + off as u64),
                },
            }
        }
        Ok(out)
    }

    fn mem_read(&self, pa: u64, n: usize) -> Option<Vec<u8>> {
        if let Some(d) = self.devices.iter().find(|d| d.contains(pa, n)) {
            return Some(d.mmio_read(pa - d.mmio_base, n));
        }
        let mut buf = vec![0u8; n];
        if self.kernel.plat.mem.read(pa, &mut buf) {
            Some(buf)
        } else {
            None
        }
    }

    fn mem_write(&mut self, pa: u64, data: &[u8]) -> bool {
        let now = self.kernel.ticks;
        if let Some(d) = self.devices.iter_mut().find(|d| d.contains(pa, data.len())) {
            let off = pa - d.mmio_base;
            d.mmio_write(off, data, now);
            return true;
        }
        self.kernel.plat.mem.write(pa, data)
    }
}
