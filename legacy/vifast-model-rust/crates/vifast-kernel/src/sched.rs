//! Lập lịch: hàng đợi ưu tiên theo từng hart, timer xoay vòng, ngân sách CPU (MCS).
//!
//! - Mỗi hart có hàng đợi riêng (bitmap 256 mức, chọn O(1)). Thread gắn với một hart.
//! - Timer (`mtimecmp`) ngắt khi hết timeslice, xoay vòng trong cùng mức ưu tiên.
//! - Thread có SchedContext bị giới hạn `budget` tick mỗi `period` tick. Hết thì bị
//!   throttle tới chu kỳ sau, kể cả khi ưu tiên cao nhất (cô lập thời gian).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::cmp::Reverse;

use vifast_abi::*;

use crate::kernel::{done, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

pub struct RunQueue {
    queues: Vec<VecDeque<ObjRef>>,
    bitmap: [u64; 4],
    location: BTreeMap<ObjRef, u8>,
}

impl Default for RunQueue {
    fn default() -> Self {
        RunQueue::new()
    }
}

impl RunQueue {
    pub fn new() -> RunQueue {
        RunQueue {
            queues: (0..NUM_PRIORITIES).map(|_| VecDeque::new()).collect(),
            bitmap: [0; 4],
            location: BTreeMap::new(),
        }
    }

    pub fn contains(&self, r: ObjRef) -> bool {
        self.location.contains_key(&r)
    }

    pub fn len(&self) -> usize {
        self.location.len()
    }

    pub fn is_empty(&self) -> bool {
        self.location.is_empty()
    }

    fn set_bit(&mut self, prio: u8) {
        self.bitmap[prio as usize / 64] |= 1u64 << (prio as usize % 64);
    }

    fn clear_bit(&mut self, prio: u8) {
        self.bitmap[prio as usize / 64] &= !(1u64 << (prio as usize % 64));
    }

    pub fn enqueue(&mut self, r: ObjRef, prio: u8, head: bool) {
        if self.contains(r) {
            return;
        }
        let q = &mut self.queues[prio as usize];
        if head {
            q.push_front(r);
        } else {
            q.push_back(r);
        }
        self.location.insert(r, prio);
        self.set_bit(prio);
    }

    pub fn remove(&mut self, r: ObjRef) {
        let Some(prio) = self.location.remove(&r) else { return };
        let empty = {
            let q = &mut self.queues[prio as usize];
            if let Some(pos) = q.iter().position(|x| *x == r) {
                q.remove(pos);
            }
            q.is_empty()
        };
        if empty {
            self.clear_bit(prio);
        }
    }

    /// Mức ưu tiên cao nhất đang có thread sẵn sàng.
    pub fn best_prio(&self) -> Option<u8> {
        for w in (0..4).rev() {
            if self.bitmap[w] != 0 {
                return Some((w * 64 + 63 - self.bitmap[w].leading_zeros() as usize) as u8);
            }
        }
        None
    }

    pub fn pick(&mut self) -> Option<ObjRef> {
        let prio = self.best_prio()?;
        let (r, empty) = {
            let q = &mut self.queues[prio as usize];
            let r = q.pop_front()?;
            (r, q.is_empty())
        };
        self.location.remove(&r);
        if empty {
            self.clear_bit(prio);
        }
        Some(r)
    }
}

/// Một lõi CPU.
pub struct Hart {
    pub id: usize,
    pub runq: RunQueue,
    pub current: Option<ObjRef>,
    pub resched: bool,
    pub mtimecmp: u64,
    pub ctx_switches: u64,
    pub traps: u64,
    pub timer_irqs: u64,
    pub idle: u64,
    pub busy: u64,
}

impl Hart {
    pub fn new(id: usize) -> Hart {
        Hart {
            id,
            runq: RunQueue::new(),
            current: None,
            resched: false,
            mtimecmp: 0,
            ctx_switches: 0,
            traps: 0,
            timer_irqs: 0,
            idle: 0,
            busy: 0,
        }
    }
}

impl<P: Platform> Kernel<P> {
    // ------------------------------------------------------------------
    // hàng đợi
    // ------------------------------------------------------------------

    pub fn sched_enqueue(&mut self, r: ObjRef, head: bool) {
        let (throttled, aff, prio) = match self.objs.tcb(r) {
            Ok(t) => (t.throttled, t.affinity, t.priority),
            Err(_) => return,
        };
        if throttled || aff >= self.harts.len() {
            return;
        }
        self.harts[aff].runq.enqueue(r, prio, head);
        if let Some(cur) = self.cur_hart {
            if cur != aff {
                let preempt = match self.harts[aff].current {
                    Some(c) => self.objs.tcb(c).map_or(true, |t| prio > t.priority),
                    None => true,
                };
                if preempt {
                    self.stats.ipi += 1; // báo hart kia lập lịch lại
                }
            }
        }
    }

    pub fn sched_remove(&mut self, r: ObjRef) {
        for h in self.harts.iter_mut() {
            h.runq.remove(r);
        }
    }

    pub fn sched_queued(&self, r: ObjRef) -> bool {
        self.harts.iter().any(|h| h.runq.contains(r))
    }

    // ------------------------------------------------------------------
    // chọn thread
    // ------------------------------------------------------------------

    pub(crate) fn schedule_inner(&mut self, h: usize) -> Option<ObjRef> {
        let ticks = self.ticks;
        let mut cur = self.harts[h].current;

        // Thread hiện tại không còn chạy được ở hart này?
        if let Some(c) = cur {
            match self.objs.tcb(c).map(|t| (t.state, t.affinity)) {
                Ok((state, aff)) if state == ThreadState::Running && aff == h => {}
                Ok((state, _)) => {
                    if state == ThreadState::Running {
                        // đổi affinity khi đang chạy: chuyển sang hàng đợi hart mới
                        if let Ok(t) = self.objs.tcb_mut(c) {
                            t.state = ThreadState::Ready;
                        }
                        self.sched_enqueue(c, false);
                    }
                    self.harts[h].current = None;
                    cur = None;
                }
                Err(_) => {
                    self.harts[h].current = None;
                    cur = None;
                }
            }
        }

        // Ngắt timer: hết timeslice.
        if let Some(c) = cur {
            if ticks >= self.harts[h].mtimecmp {
                self.harts[h].timer_irqs += 1;
                if let Ok(t) = self.objs.tcb_mut(c) {
                    t.timeslice = 0;
                }
                self.harts[h].resched = true;
            }
        }

        let best = self.harts[h].runq.best_prio();
        if let Some(c) = cur {
            let cprio = self.objs.tcb(c).map(|t| t.priority).unwrap_or(0);
            if !self.harts[h].resched && best.map_or(true, |b| b <= cprio) {
                return Some(c);
            }
            let ts = match self.objs.tcb_mut(c) {
                Ok(t) => {
                    t.state = ThreadState::Ready;
                    t.timeslice
                }
                Err(_) => 0,
            };
            self.harts[h].current = None;
            // còn lượt thì về đầu hàng (bị chen ngang), hết lượt thì xuống cuối hàng
            self.sched_enqueue(c, ts > 0);
        }

        self.harts[h].resched = false;
        let r = self.harts[h].runq.pick()?;
        let ts = match self.objs.tcb_mut(r) {
            Ok(t) => {
                t.state = ThreadState::Running;
                if t.timeslice <= 0 {
                    t.timeslice = DEFAULT_TIMESLICE;
                }
                t.timeslice
            }
            Err(_) => return None,
        };
        let hart = &mut self.harts[h];
        hart.mtimecmp = ticks + ts as u64;
        hart.current = Some(r);
        hart.ctx_switches += 1;
        Some(r)
    }

    /// Tính một tick CPU cho thread vừa chạy trên hart `h`.
    pub(crate) fn charge(&mut self, h: usize, r: ObjRef) {
        let sc = match self.objs.tcb_mut(r) {
            Ok(t) => {
                t.timeslice -= 1;
                t.sc
            }
            Err(_) => return,
        };
        let Some(sc) = sc else { return };
        let exhausted = match self.objs.sched_context_mut(sc) {
            Ok(s) if s.configured => {
                s.remaining -= 1;
                s.remaining <= 0
            }
            _ => false,
        };
        if exhausted {
            if let Ok(t) = self.objs.tcb_mut(r) {
                if t.state == ThreadState::Running {
                    t.throttled = true;
                    t.state = ThreadState::Ready;
                    self.harts[h].current = None;
                    self.stats.throttle += 1;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // ngân sách CPU (MCS)
    // ------------------------------------------------------------------

    pub(crate) fn push_release(&mut self, at: u64, sc: ObjRef, epoch: u64) {
        self.release_seq += 1;
        self.releases.push(Reverse((at, self.release_seq, sc, epoch)));
    }

    pub(crate) fn process_releases(&mut self) {
        while let Some(Reverse((at, _, sc_ref, epoch))) = self.releases.peek().copied() {
            if at > self.ticks {
                break;
            }
            self.releases.pop();
            let (period, bound) = match self.objs.sched_context_mut(sc_ref) {
                Ok(sc) if sc.epoch == epoch && sc.configured => {
                    sc.remaining = sc.budget as i64;
                    (sc.period, sc.bound_tcb)
                }
                _ => continue,
            };
            self.push_release(at + period, sc_ref, epoch);
            let Some(t) = bound else { continue };
            let ready = match self.objs.tcb_mut(t) {
                Ok(x) if x.throttled => {
                    x.throttled = false;
                    x.state == ThreadState::Ready
                }
                _ => false,
            };
            if ready {
                self.sched_enqueue(t, false);
            }
        }
    }

    pub fn configure_sc(&mut self, sc_ref: ObjRef, budget: u64, period: u64) -> KResult<()> {
        if budget == 0 || budget > period {
            return Err(KError::InvalidArg);
        }
        let epoch = {
            let sc = self.objs.sched_context_mut(sc_ref)?;
            sc.epoch += 1;
            sc.budget = budget;
            sc.period = period;
            sc.remaining = budget as i64;
            sc.configured = true;
            sc.epoch
        };
        let at = self.ticks + period;
        self.push_release(at, sc_ref, epoch);
        Ok(())
    }

    pub fn bind_sc(&mut self, sc_ref: ObjRef, t: ObjRef) -> KResult<()> {
        let sc_bound = self.objs.sched_context(sc_ref)?.bound_tcb.is_some();
        let t_bound = self.objs.tcb(t)?.sc.is_some();
        if sc_bound || t_bound {
            return Err(KError::InvalidArg);
        }
        self.objs.sched_context_mut(sc_ref)?.bound_tcb = Some(t);
        self.objs.tcb_mut(t)?.sc = Some(sc_ref);
        Ok(())
    }

    pub(crate) fn unbind_sc(&mut self, t: ObjRef) {
        let sc = match self.objs.tcb_mut(t) {
            Ok(x) => {
                x.throttled = false;
                x.sc.take()
            }
            Err(_) => return,
        };
        if let Some(sc) = sc {
            if let Ok(s) = self.objs.sched_context_mut(sc) {
                s.bound_tcb = None;
            }
        }
    }

    // ------------------------------------------------------------------
    // syscall
    // ------------------------------------------------------------------

    pub(crate) fn sys_sc_configure(&mut self, t: ObjRef, sched_control: u64, sc: u64, budget: u64, period: u64) -> KResult<Ret> {
        self.lookup_cap(t, sched_control, Some(ObjType::SchedControl), Rights::NONE)?;
        let (_, sccap) = self.lookup_cap(t, sc, Some(ObjType::SchedContext), Rights::WRITE)?;
        self.configure_sc(sccap.obj.ok_or(KError::InvalidCap)?, budget, period)?;
        done()
    }

    pub(crate) fn sys_sc_bind(&mut self, t: ObjRef, sc: u64, tcb: u64) -> KResult<Ret> {
        let (_, sccap) = self.lookup_cap(t, sc, Some(ObjType::SchedContext), Rights::WRITE)?;
        let (_, tcap) = self.lookup_cap(t, tcb, Some(ObjType::Tcb), Rights::WRITE)?;
        self.bind_sc(sccap.obj.ok_or(KError::InvalidCap)?, tcap.obj.ok_or(KError::InvalidCap)?)?;
        done()
    }
}
