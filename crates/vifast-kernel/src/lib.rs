//! Portable ViFastOs kernel core.
//!
//! This crate contains no architecture-specific instructions, no `std`, no
//! heap and no `unsafe`.  It implements a small capability-based object model,
//! synchronous IPC, notifications and a priority scheduler over fixed-capacity
//! arrays.
//!
//! The architecture ports call [`boot`] with a concrete [`Arch`]
//! implementation.  The same core is compiled for x86_64, aarch64 and
//! riscv64, and is also exercised by host tests.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod console;
pub mod fs;
pub mod ipc;
pub mod object;
pub mod selftest;

use vifast_abi::*;
use vifast_arch::{Arch, BootInfo};

pub use object::*;

/// Result of a syscall executed on behalf of a user thread.  The architecture
/// trap handler turns `Debug` into console output and `Exit` into a machine
/// shutdown; the portable core never touches user memory directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserAction {
    Return {
        value: u64,
        error: KError,
    },
    /// A received IPC message.  `error/value` are the normal return registers;
    /// `label` and `data` carry the message.
    Message {
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    },
    Debug {
        ptr: u64,
        len: u64,
    },
    /// Map `pa` at `va` in a user address space.  The architecture handler
    /// updates the page table, flushes the TLB and returns the chosen VA in
    /// the normal result registers.
    MapFrame {
        vspace: ObjRef,
        root: u64,
        va: u64,
        pa: u64,
        rights: Rights,
    },
    /// Remove the user mapping at `va` and flush the TLB.
    UnmapFrame {
        vspace: ObjRef,
        root: u64,
        va: u64,
    },
    /// Copy a user path into the ramfs VFS and return an fd.
    Open {
        ptr: u64,
        len: u64,
        flags: u64,
    },
    /// Read from a ramfs fd into a user buffer.
    Read {
        fd: u64,
        ptr: u64,
        len: u64,
    },
    /// Write a user buffer into a ramfs fd.
    Write {
        fd: u64,
        ptr: u64,
        len: u64,
    },
    /// The syscall blocked the current TCB.  The architecture trap handler must
    /// save its context and schedule another TCB.
    Blocked,
    Exit,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KernelStats {
    pub ipc_sent: u64,
    pub ipc_received: u64,
    pub notifications: u64,
    pub context_switches: u64,
    pub ticks: u64,
}

pub struct Kernel {
    pub(crate) objs: [Object; MAX_OBJECTS],
    pub(crate) runq: [Option<ObjRef>; MAX_TCBS],
    pub(crate) runq_len: usize,
    pub(crate) current: Option<ObjRef>,
    pub(crate) seq: u64,
    pub(crate) ticks: u64,
    pub(crate) root: Option<ObjRef>,
    pub(crate) fs: fs::RamFs,
    pub(crate) stats: KernelStats,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub fn new() -> Kernel {
        let mut kernel = Kernel {
            objs: [Object::free(); MAX_OBJECTS],
            runq: [None; MAX_TCBS],
            runq_len: 0,
            current: None,
            seq: 0,
            ticks: 0,
            root: None,
            fs: fs::RamFs::new(),
            stats: KernelStats::default(),
        };
        kernel.init_fs();
        if let Some(root) = kernel.create_tcb("root", 1) {
            kernel.root = Some(root);
        }
        kernel
    }

    pub fn root(&self) -> Option<ObjRef> {
        self.root
    }

    pub fn stats(&self) -> KernelStats {
        self.stats
    }

    fn alloc(&mut self, kind: Kind) -> Option<ObjRef> {
        for i in 0..MAX_OBJECTS {
            if self.objs[i].is_free() {
                let gen = self.objs[i].gen.wrapping_add(1);
                let gen = if gen == 0 { 1 } else { gen };
                self.objs[i].gen = gen;
                self.objs[i].kind = kind;
                return Some(ObjRef {
                    index: i as u8,
                    gen,
                });
            }
        }
        None
    }

    pub fn create_endpoint(&mut self) -> Option<ObjRef> {
        self.alloc(Kind::Endpoint(Endpoint::default()))
    }

    pub fn create_notification(&mut self) -> Option<ObjRef> {
        self.alloc(Kind::Notification(Notification::default()))
    }

    pub fn create_tcb(&mut self, name: &'static str, priority: u8) -> Option<ObjRef> {
        self.alloc(Kind::Tcb(Tcb::new(name, priority)))
    }

    pub fn create_untyped(&mut self, base: usize, size: usize) -> Option<ObjRef> {
        self.create_untyped_with_page_size(base, size, DEFAULT_PAGE_SIZE as usize)
    }

    pub fn create_untyped_with_page_size(
        &mut self,
        base: usize,
        size: usize,
        page_size: usize,
    ) -> Option<ObjRef> {
        self.alloc(Kind::Untyped(Untyped {
            base,
            size,
            bump: 0,
            page_size,
        }))
    }

    pub fn create_frame(&mut self, pa: usize, size: usize) -> Option<ObjRef> {
        self.alloc(Kind::Frame(Frame { pa, size }))
    }

    pub fn create_vspace(&mut self, root: u64, base: u64, size: u64) -> Option<ObjRef> {
        self.create_vspace_with_page_size(root, base, size, DEFAULT_PAGE_SIZE)
    }

    pub fn create_vspace_with_page_size(
        &mut self,
        root: u64,
        base: u64,
        size: u64,
        page_size: u64,
    ) -> Option<ObjRef> {
        self.alloc(Kind::VSpace(VSpace::with_page_size(
            root, base, size, page_size,
        )))
    }

    pub fn create_page_table(&mut self, root: u64, level: u8) -> Option<ObjRef> {
        self.alloc(Kind::PageTable(PageTable { root, level }))
    }

    pub fn get(&self, r: ObjRef) -> Option<&Object> {
        let object = self.objs.get(r.index as usize)?;
        if object.gen == r.gen && !object.is_free() {
            Some(object)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, r: ObjRef) -> Option<&mut Object> {
        let object = self.objs.get_mut(r.index as usize)?;
        if object.gen == r.gen && !object.is_free() {
            Some(object)
        } else {
            None
        }
    }

    pub fn tcb(&self, r: ObjRef) -> Option<&Tcb> {
        match self.get(r)?.kind {
            Kind::Tcb(ref tcb) => Some(tcb),
            _ => None,
        }
    }

    pub fn tcb_mut(&mut self, r: ObjRef) -> Option<&mut Tcb> {
        match self.get_mut(r)?.kind {
            Kind::Tcb(ref mut tcb) => Some(tcb),
            _ => None,
        }
    }

    pub fn endpoint(&self, r: ObjRef) -> Option<&Endpoint> {
        match self.get(r)?.kind {
            Kind::Endpoint(ref endpoint) => Some(endpoint),
            _ => None,
        }
    }

    pub fn endpoint_mut(&mut self, r: ObjRef) -> Option<&mut Endpoint> {
        match self.get_mut(r)?.kind {
            Kind::Endpoint(ref mut endpoint) => Some(endpoint),
            _ => None,
        }
    }

    pub fn notification(&self, r: ObjRef) -> Option<&Notification> {
        match self.get(r)?.kind {
            Kind::Notification(ref notification) => Some(notification),
            _ => None,
        }
    }

    pub fn notification_mut(&mut self, r: ObjRef) -> Option<&mut Notification> {
        match self.get_mut(r)?.kind {
            Kind::Notification(ref mut notification) => Some(notification),
            _ => None,
        }
    }

    pub fn frame(&self, r: ObjRef) -> Option<&Frame> {
        match self.get(r)?.kind {
            Kind::Frame(ref frame) => Some(frame),
            _ => None,
        }
    }

    pub fn vspace(&self, r: ObjRef) -> Option<&VSpace> {
        match self.get(r)?.kind {
            Kind::VSpace(ref vspace) => Some(vspace),
            _ => None,
        }
    }

    pub fn vspace_mut(&mut self, r: ObjRef) -> Option<&mut VSpace> {
        match self.get_mut(r)?.kind {
            Kind::VSpace(ref mut vspace) => Some(vspace),
            _ => None,
        }
    }

    pub fn page_table(&self, r: ObjRef) -> Option<&PageTable> {
        match self.get(r)?.kind {
            Kind::PageTable(ref table) => Some(table),
            _ => None,
        }
    }

    pub fn untyped(&self, r: ObjRef) -> Option<&Untyped> {
        match self.get(r)?.kind {
            Kind::Untyped(ref untyped) => Some(untyped),
            _ => None,
        }
    }

    pub fn untyped_mut(&mut self, r: ObjRef) -> Option<&mut Untyped> {
        match self.get_mut(r)?.kind {
            Kind::Untyped(ref mut untyped) => Some(untyped),
            _ => None,
        }
    }

    /// Carve one object out of an untyped memory region.
    ///
    /// Frames and page tables consume one aligned physical page.  Endpoint,
    /// notification and TCB metadata do not consume user physical pages in
    /// this reference model; they are allocated from the fixed object table.
    pub fn retype(&mut self, untyped: ObjRef, ty: ObjType) -> KResult<ObjRef> {
        let (base, size, bump, page) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (region.base, region.size, region.bump, region.page_size)
        };
        if page == 0 || !page.is_power_of_two() {
            return Err(KError::Alignment);
        }
        let (kind, new_bump) = match ty {
            ObjType::Endpoint => (Kind::Endpoint(Endpoint::default()), bump),
            ObjType::Notification => (Kind::Notification(Notification::default()), bump),
            ObjType::Tcb => (Kind::Tcb(Tcb::new("retyped", 1)), bump),
            ObjType::Frame | ObjType::PageTable => {
                let unaligned = base.checked_add(bump).ok_or(KError::OutOfMemory)?;
                let pa = (unaligned + page - 1) & !(page - 1);
                let end = base.checked_add(size).ok_or(KError::OutOfMemory)?;
                if pa.checked_add(page).ok_or(KError::OutOfMemory)? > end {
                    return Err(KError::OutOfMemory);
                }
                let consumed = pa.checked_add(page).ok_or(KError::OutOfMemory)? - base;
                if ty == ObjType::Frame {
                    (Kind::Frame(Frame { pa, size: page }), consumed)
                } else {
                    (
                        Kind::PageTable(PageTable {
                            root: pa as u64,
                            level: 3,
                        }),
                        consumed,
                    )
                }
            }
            _ => return Err(KError::NotImplemented),
        };
        let object = self.alloc(kind).ok_or(KError::OutOfMemory)?;
        if new_bump != bump {
            let region = self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)?;
            region.bump = new_bump;
        }
        Ok(object)
    }

    /// Record a user mapping and validate W^X/alignment/overlap.  The caller
    /// (architecture trap handler) performs the real page-table update after
    /// this returns `UserAction::MapFrame`.
    pub fn map_frame(
        &mut self,
        vspace: ObjRef,
        frame: ObjRef,
        hint: u64,
        rights: Rights,
    ) -> KResult<Mapping> {
        let allowed = Rights::READ | Rights::WRITE | Rights::EXEC;
        if rights.is_empty() || !allowed.contains(rights) {
            return Err(KError::InvalidArg);
        }
        if rights.contains(Rights::WRITE) && rights.contains(Rights::EXEC) {
            return Err(KError::NoRights); // W^X
        }
        let frame = *self.frame(frame).ok_or(KError::WrongType)?;
        let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
        let page_size = space.page_size;
        if page_size == 0
            || page_size.is_power_of_two() == false
            || frame.pa as u64 % page_size != 0
            || (frame.size as u64) < page_size
        {
            return Err(KError::Alignment);
        }
        let va = if hint == 0 { space.next_va } else { hint };
        if va % page_size != 0 {
            return Err(KError::Alignment);
        }
        if va < space.base || va.saturating_add(page_size) > space.end {
            return Err(KError::InvalidArg);
        }
        if space.contains(va, page_size) {
            return Err(KError::AlreadyMapped);
        }
        let mapping = Mapping {
            va,
            pa: frame.pa as u64,
            len: page_size,
            rights,
        };
        space.insert(mapping)?;
        Ok(mapping)
    }

    /// Remove a mapping from the portable shadow table.  The architecture
    /// handler clears the actual entry and flushes its TLB.
    pub fn unmap_frame(&mut self, vspace: ObjRef, va: u64) -> KResult<Mapping> {
        let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
        let index = space.mapping_index(va).ok_or(KError::NotMapped)?;
        let mapping = space.mappings[index].take().ok_or(KError::NotMapped)?;
        Ok(mapping)
    }

    /// True if `va` is currently mapped in the shadow table.
    pub fn mapping_at(&self, vspace: ObjRef, va: u64) -> Option<Mapping> {
        self.vspace(vspace)?.find(va)
    }

    /// Check the core object/capability/scheduler/VSpace invariants.  This is
    /// intentionally deterministic and allocation-free so it can run in the
    /// self-test and in a future watchdog.
    pub fn check_invariants(&self) -> KResult<()> {
        if self.runq_len > MAX_TCBS {
            return Err(KError::IllegalOperation);
        }
        for i in 0..MAX_OBJECTS {
            let object = &self.objs[i];
            if object.is_free() {
                continue;
            }
            let reference = ObjRef {
                index: i as u8,
                gen: object.gen,
            };
            if self.get(reference).is_none() {
                return Err(KError::IllegalOperation);
            }
            match object.kind {
                Kind::Tcb(ref tcb) => {
                    for cap in tcb.cspace.iter().flatten() {
                        let Some(object) = self.get(cap.obj) else {
                            return Err(KError::ObjectDestroyed);
                        };
                        if object.kind.obj_type() != Some(cap.ty) {
                            return Err(KError::WrongType);
                        }
                    }
                    for fd in tcb.fds.iter().flatten() {
                        if self.fs.file(fd.file as usize).is_none() {
                            return Err(KError::IllegalOperation);
                        }
                    }
                    if tcb.queued {
                        let mut found = false;
                        for queued in self.runq.iter().flatten() {
                            if *queued == reference {
                                found = true;
                            }
                        }
                        if !found {
                            return Err(KError::IllegalOperation);
                        }
                    }
                }
                Kind::Endpoint(_) => {}
                Kind::Notification(ref notification) => {
                    if let Some(waiter) = notification.waiter {
                        if self.tcb(waiter).is_none() {
                            return Err(KError::ObjectDestroyed);
                        }
                    }
                }
                Kind::Frame(ref frame) => {
                    if frame.size == 0
                        || !frame.size.is_power_of_two()
                        || frame.pa % frame.size != 0
                    {
                        return Err(KError::Alignment);
                    }
                }
                Kind::VSpace(ref space) => {
                    let page = space.page_size;
                    for (i, mapping) in space.mappings.iter().enumerate() {
                        let Some(mapping) = mapping else { continue };
                        if page == 0
                            || page.is_power_of_two() == false
                            || mapping.va % page != 0
                            || mapping.pa % page != 0
                            || mapping.len != page
                            || mapping.rights.is_empty()
                            || (mapping.rights.contains(Rights::WRITE)
                                && mapping.rights.contains(Rights::EXEC))
                        {
                            return Err(KError::IllegalOperation);
                        }
                        if mapping.va < space.base
                            || mapping.va.saturating_add(mapping.len) > space.end
                        {
                            return Err(KError::InvalidArg);
                        }
                        for other in space.mappings.iter().skip(i + 1).flatten() {
                            if mapping.va < other.va.saturating_add(other.len)
                                && other.va < mapping.va.saturating_add(mapping.len)
                            {
                                return Err(KError::AlreadyMapped);
                            }
                        }
                    }
                }
                Kind::Untyped(ref untyped) => {
                    if untyped.bump > untyped.size {
                        return Err(KError::IllegalOperation);
                    }
                }
                Kind::PageTable(ref table) => {
                    if table.root % DEFAULT_PAGE_SIZE != 0 {
                        return Err(KError::Alignment);
                    }
                }
                Kind::Free => {}
            }
        }
        if let Some(current) = self.current {
            let Some(tcb) = self.tcb(current) else {
                return Err(KError::ObjectDestroyed);
            };
            if tcb.state != ThreadState::Running || tcb.queued {
                return Err(KError::IllegalOperation);
            }
        }
        for (i, queued) in self.runq.iter().flatten().enumerate() {
            let Some(tcb) = self.tcb(*queued) else {
                return Err(KError::ObjectDestroyed);
            };
            if tcb.state != ThreadState::Ready || !tcb.queued {
                return Err(KError::IllegalOperation);
            }
            for other in self.runq.iter().skip(i + 1).flatten() {
                if queued == other {
                    return Err(KError::IllegalOperation);
                }
            }
        }
        Ok(())
    }

    pub fn install_cap(&mut self, tcb: ObjRef, slot: u8, cap: Cap) -> KResult<()> {
        if slot == 0 || slot as usize >= MAX_CAPS {
            return Err(KError::InvalidArg);
        }
        if self.get(cap.obj).is_none() {
            return Err(KError::ObjectDestroyed);
        }
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        if table.cspace[slot as usize].is_some() {
            return Err(KError::SlotOccupied);
        }
        table.cspace[slot as usize] = Some(cap);
        Ok(())
    }

    pub fn lookup_cap(
        &self,
        tcb: ObjRef,
        slot: u8,
        want: Option<ObjType>,
        need: Rights,
    ) -> KResult<Cap> {
        let table = self.tcb(tcb).ok_or(KError::InvalidCap)?;
        let cap = table
            .cspace
            .get(slot as usize)
            .and_then(|slot| *slot)
            .ok_or(KError::InvalidCap)?;
        if let Some(want) = want {
            if cap.ty != want {
                return Err(KError::WrongType);
            }
        }
        if !cap.rights.contains(need) {
            return Err(KError::NoRights);
        }
        if self.get(cap.obj).is_none() {
            return Err(KError::ObjectDestroyed);
        }
        Ok(cap)
    }

    pub fn cap_at(&self, addr: SlotAddr) -> Option<Cap> {
        self.tcb(addr.tcb)?
            .cspace
            .get(addr.slot as usize)
            .and_then(|cap| *cap)
    }

    pub fn delete_cap(&mut self, tcb: ObjRef, slot: u8) -> Option<Cap> {
        self.tcb_mut(tcb)?.cspace.get_mut(slot as usize)?.take()
    }

    pub fn copy_cap(
        &mut self,
        src_tcb: ObjRef,
        src_slot: u8,
        dst_tcb: ObjRef,
        dst_slot: u8,
        rights: Rights,
        badge: u64,
    ) -> KResult<()> {
        let source = self.lookup_cap(src_tcb, src_slot, None, Rights::NONE)?;
        let mut derived = source.derive(rights, badge)?;
        derived.parent = Some(SlotAddr {
            tcb: src_tcb,
            slot: src_slot,
        });
        self.install_cap(dst_tcb, dst_slot, derived)
    }

    fn is_descendant(&self, cap: &Cap, target: SlotAddr) -> bool {
        let mut parent = cap.parent;
        let mut depth = 0usize;
        while let Some(addr) = parent {
            if addr == target {
                return true;
            }
            depth += 1;
            if depth > MAX_OBJECTS * MAX_CAPS {
                return false;
            }
            parent = self.cap_at(addr).and_then(|cap| cap.parent);
        }
        false
    }

    /// Remove all capabilities derived from `slot`.  The root capability is
    /// left in place.  Returns the number removed.
    pub fn revoke_cap(&mut self, tcb: ObjRef, slot: u8) -> KResult<usize> {
        let target = SlotAddr { tcb, slot };
        let mut victims = [SlotAddr::default(); MAX_OBJECTS * MAX_CAPS];
        let mut count = 0usize;
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(table) = self.tcb(r) {
                for s in 1..MAX_CAPS {
                    if let Some(cap) = table.cspace[s] {
                        if self.is_descendant(&cap, target) {
                            victims[count] = SlotAddr {
                                tcb: r,
                                slot: s as u8,
                            };
                            count += 1;
                            if count == victims.len() {
                                break;
                            }
                        }
                    }
                }
            }
        }
        for victim in victims.iter().take(count) {
            let _ = self.delete_cap(victim.tcb, victim.slot);
        }
        Ok(count)
    }

    pub fn set_cap_badge(&mut self, tcb: ObjRef, slot: u8, badge: u64) -> KResult<()> {
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        let cap = table
            .cspace
            .get_mut(slot as usize)
            .and_then(|cap| cap.as_mut())
            .ok_or(KError::InvalidCap)?;
        if !matches!(cap.ty, ObjType::Endpoint | ObjType::Notification) {
            return Err(KError::InvalidArg);
        }
        if cap.badge != 0 && badge != 0 {
            return Err(KError::NoRights);
        }
        cap.badge = badge;
        Ok(())
    }

    pub fn enqueue(&mut self, tcb: ObjRef) {
        if self.runq_len >= MAX_TCBS {
            return;
        }
        if self.tcb(tcb).is_none_or(|t| t.queued) {
            return;
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        if let Some(table) = self.tcb_mut(tcb) {
            table.queued = true;
            table.queue_seq = seq;
            if table.state == ThreadState::Inactive {
                table.state = ThreadState::Ready;
            }
        } else {
            return;
        }
        self.runq[self.runq_len] = Some(tcb);
        self.runq_len += 1;
    }

    pub fn pick(&mut self) -> Option<ObjRef> {
        let mut best: Option<(usize, u8, u64)> = None;
        for i in 0..self.runq_len {
            let Some(r) = self.runq[i] else { continue };
            let Some(tcb) = self.tcb(r) else { continue };
            let candidate = (i, tcb.priority, tcb.queue_seq);
            best = match best {
                None => Some(candidate),
                Some((_, best_prio, best_seq)) => {
                    if candidate.1 > best_prio
                        || (candidate.1 == best_prio && candidate.2 < best_seq)
                    {
                        Some(candidate)
                    } else {
                        best
                    }
                }
            };
        }
        let (index, _, _) = best?;
        let chosen = self.runq[index]?;
        let mut i = index;
        while i + 1 < self.runq_len {
            self.runq[i] = self.runq[i + 1];
            i += 1;
        }
        self.runq_len -= 1;
        self.runq[self.runq_len] = None;
        if let Some(tcb) = self.tcb_mut(chosen) {
            tcb.queued = false;
            tcb.state = ThreadState::Running;
        }
        self.current = Some(chosen);
        self.stats.context_switches += 1;
        Some(chosen)
    }

    pub fn yield_current(&mut self) -> Option<ObjRef> {
        let current = self.current.take()?;
        if let Some(tcb) = self.tcb_mut(current) {
            if tcb.state == ThreadState::Running {
                tcb.state = ThreadState::Ready;
            }
        }
        self.enqueue(current);
        Some(current)
    }

    pub fn current(&self) -> Option<ObjRef> {
        self.current
    }

    /// Take a pending syscall result from a woken TCB.  Architecture ports use
    /// this when they resume a blocked user thread.
    pub fn take_resume(&mut self, tcb: ObjRef) -> Option<Message> {
        self.tcb_mut(tcb).and_then(|table| table.resume.take())
    }

    pub fn tick(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
        self.stats.ticks = self.ticks;
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// User-visible syscall surface for the bring-up programs.
    ///
    /// This covers the capability-based IPC subset (`Send`, `Call`, `Recv`,
    /// `Reply`, `Signal`, `Wait`, `Poll`) plus `DebugWrite`, `GetTime` and
    /// `Exit`.  Memory-bearing operations are returned as a `Debug` action so
    /// the architecture trap handler validates the user pointer.
    pub fn user_syscall(
        &mut self,
        tcb: ObjRef,
        number: u64,
        a0: u64,
        a1: u64,
        a2: u64,
        a3: u64,
        a4: u64,
        a5: u64,
    ) -> UserAction {
        if self.tcb(tcb).is_none() {
            return UserAction::Return {
                value: 0,
                error: KError::InvalidCap,
            };
        }
        if number == SyscallId::Yield as u64 {
            return UserAction::Return {
                value: 0,
                error: KError::Ok,
            };
        }
        if number == SyscallId::GetTime as u64 {
            return UserAction::Return {
                value: self.ticks,
                error: KError::Ok,
            };
        }
        if number == SyscallId::DebugWrite as u64 {
            return UserAction::Debug { ptr: a0, len: a1 };
        }
        if number == SyscallId::Exit as u64 {
            return UserAction::Exit;
        }

        match number {
            n if n == SyscallId::Send as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Endpoint),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let msg = match message_from_cap(a1, a2, a3, a4, a5) {
                    Ok(msg) => msg,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let endpoint = cap.obj;
                match self.send(tcb, endpoint, cap.badge, msg, false) {
                    Ok(true) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Ok(false) => UserAction::Blocked,
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Call as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Endpoint),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let msg = match message_from_cap(a1, a2, a3, a4, a5) {
                    Ok(msg) => msg,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let endpoint = cap.obj;
                match self.send(tcb, endpoint, cap.badge, msg, true) {
                    Ok(_) => UserAction::Blocked,
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Recv as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Endpoint),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let endpoint = cap.obj;
                let recv_slot = slot_arg(a1).unwrap_or(0);
                match self.recv(tcb, endpoint, recv_slot) {
                    Ok(Some(msg)) => message_action(KError::Ok, 0, &msg),
                    Ok(None) => UserAction::Blocked,
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Reply as u64 => {
                let msg = match message_from(a1, a2, a3, a4, a5) {
                    Ok(msg) => msg,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.reply(tcb, msg) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Signal as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Notification),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let ntfn = cap.obj;
                match self.signal(ntfn, if cap.badge != 0 { cap.badge } else { 1 }) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Wait as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Notification),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let ntfn = cap.obj;
                match self.wait_as(tcb, ntfn) {
                    Ok(Some(word)) => UserAction::Return {
                        value: word,
                        error: KError::Ok,
                    },
                    Ok(None) => UserAction::Blocked,
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Poll as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Notification),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let ntfn = cap.obj;
                match self.poll(ntfn) {
                    Ok(word) => UserAction::Return {
                        value: word,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Retype as u64 => {
                let untyped = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Untyped),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let ty = match a1 {
                    n if n == ObjType::Frame as u64 => ObjType::Frame,
                    n if n == ObjType::PageTable as u64 => ObjType::PageTable,
                    _ => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                let dest = match slot_arg(a2) {
                    Ok(dest) if dest != 0 && (dest as usize) < MAX_CAPS => dest,
                    _ => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                if self
                    .tcb(tcb)
                    .and_then(|table| table.cspace.get(dest as usize))
                    .is_none()
                    || self
                        .tcb(tcb)
                        .and_then(|table| table.cspace[dest as usize])
                        .is_some()
                {
                    return UserAction::Return {
                        value: 0,
                        error: KError::SlotOccupied,
                    };
                }
                let object = match self.retype(untyped.obj, ty) {
                    Ok(object) => object,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let cap = Cap::new(object, ty, Rights::READ | Rights::WRITE);
                match self.install_cap(tcb, dest, cap) {
                    Ok(()) => UserAction::Return {
                        value: dest as u64,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::FrameMap as u64 => {
                let space = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::VSpace),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let frame = match self.lookup_cap(
                    tcb,
                    slot_arg(a1).unwrap_or(0),
                    Some(ObjType::Frame),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let rights = Rights((a3 as u8) & Rights::ALL.0);
                if !frame.rights.contains(rights) {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NoRights,
                    };
                }
                let mapping = match self.map_frame(space.obj, frame.obj, a2, rights) {
                    Ok(mapping) => mapping,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let Some(root) = self.vspace(space.obj).map(|space| space.root) else {
                    return UserAction::Return {
                        value: 0,
                        error: KError::ObjectDestroyed,
                    };
                };
                UserAction::MapFrame {
                    vspace: space.obj,
                    root,
                    va: mapping.va,
                    pa: mapping.pa,
                    rights: mapping.rights,
                }
            }
            n if n == SyscallId::FrameUnmap as u64 => {
                let space = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::VSpace),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let mapping = match self.unmap_frame(space.obj, a1) {
                    Ok(mapping) => mapping,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let Some(root) = self.vspace(space.obj).map(|space| space.root) else {
                    return UserAction::Return {
                        value: 0,
                        error: KError::ObjectDestroyed,
                    };
                };
                UserAction::UnmapFrame {
                    vspace: space.obj,
                    root,
                    va: mapping.va,
                }
            }
            n if n == SyscallId::Open as u64 => UserAction::Open {
                ptr: a0,
                len: a1,
                flags: a2,
            },
            n if n == SyscallId::Read as u64 => UserAction::Read {
                fd: a0,
                ptr: a1,
                len: a2,
            },
            n if n == SyscallId::Write as u64 => UserAction::Write {
                fd: a0,
                ptr: a1,
                len: a2,
            },
            n if n == SyscallId::Stat as u64 => match self.stat_fd(tcb, a0) {
                Ok(size) => UserAction::Return {
                    value: size as u64,
                    error: KError::Ok,
                },
                Err(error) => UserAction::Return { value: 0, error },
            },
            n if n == SyscallId::Seek as u64 => match self.seek_fd(tcb, a0, a1) {
                Ok(offset) => UserAction::Return {
                    value: offset as u64,
                    error: KError::Ok,
                },
                Err(error) => UserAction::Return { value: 0, error },
            },
            n if n == SyscallId::Close as u64 => match self.close_fd(tcb, a0) {
                Ok(()) => UserAction::Return {
                    value: 0,
                    error: KError::Ok,
                },
                Err(error) => UserAction::Return { value: 0, error },
            },
            _ => UserAction::Return {
                value: 0,
                error: KError::NotImplemented,
            },
        }
    }
}

fn slot_arg(value: u64) -> KResult<u8> {
    u8::try_from(value).map_err(|_| KError::InvalidArg)
}

fn message_from(label: u64, w0: u64, w1: u64, w2: u64, w3: u64) -> KResult<Message> {
    Message::new(label, &[w0, w1, w2, w3])
}

fn message_from_cap(label: u64, w0: u64, w1: u64, w2: u64, cap: u64) -> KResult<Message> {
    let mut msg = Message::new(label, &[w0, w1, w2])?;
    if cap != CAP_NONE {
        msg.cap = cap;
    }
    Ok(msg)
}

fn message_action(error: KError, value: u64, msg: &Message) -> UserAction {
    let mut data = [0u64; 4];
    let words = msg.words();
    let mut i = 0;
    while i < data.len() && i < words.len() {
        data[i] = words[i];
        i += 1;
    }
    UserAction::Message {
        error,
        value,
        label: msg.label,
        data,
    }
}

/// Entry point shared by all architecture ports.
pub fn boot<A: Arch>(info: BootInfo) -> ! {
    A::init();
    klog!(A, "ViFastOS booting on {}", A::NAME);
    klog!(
        A,
        "boot: hart={} dtb={:#x} ram={:#x}+{:#x}",
        info.hartid,
        info.dtb,
        info.memory_base,
        info.memory_size
    );
    let mut kernel = Kernel::new();
    let ok = selftest::run::<A>(&mut kernel);
    if ok {
        let stats = kernel.stats();
        klog!(
            A,
            "SELFTEST PASS (ipc_tx={} ipc_rx={} ntfn={} ctx={} ticks={})",
            stats.ipc_sent,
            stats.ipc_received,
            stats.notifications,
            stats.context_switches,
            stats.ticks
        );
        A::exit(0)
    } else {
        klog!(A, "SELFTEST FAIL");
        A::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestArch;

    impl Arch for TestArch {
        const NAME: &'static str = "test";

        fn init() {}
        fn putc(byte: u8) {
            use std::io::Write;
            let _ = std::io::stdout().write_all(&[byte]);
        }
        fn now() -> u64 {
            0
        }
        fn irq_disable() {}
        fn irq_enable() {}
        fn send_ipi(_: usize) {}
        fn halt() -> ! {
            loop {
                core::hint::spin_loop();
            }
        }
        fn exit(_: u8) -> ! {
            loop {
                core::hint::spin_loop();
            }
        }
    }

    #[test]
    fn kernel_size_is_reasonable() {
        println!("Kernel size = {} bytes", core::mem::size_of::<Kernel>());
        println!("Object size = {} bytes", core::mem::size_of::<Object>());
        println!("Tcb size = {} bytes", core::mem::size_of::<Tcb>());
    }

    #[test]
    fn selftest_passes() {
        let mut kernel = Kernel::new();
        assert!(selftest::run::<TestArch>(&mut kernel));
    }

    #[test]
    fn capability_rights_are_monotonic() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(
                    endpoint,
                    ObjType::Endpoint,
                    Rights::READ | Rights::WRITE | Rights::GRANT,
                ),
            )
            .unwrap();
        kernel.copy_cap(root, 1, root, 2, Rights::READ, 0).unwrap();
        let cap = kernel
            .lookup_cap(root, 2, Some(ObjType::Endpoint), Rights::NONE)
            .unwrap();
        assert_eq!(cap.rights, Rights::READ);
        assert_eq!(
            kernel.lookup_cap(root, 2, Some(ObjType::Endpoint), Rights::WRITE),
            Err(KError::NoRights)
        );
    }

    #[test]
    fn vspace_maps_enforce_wx_and_track_unmap() {
        let mut kernel = Kernel::new();
        let space = kernel.create_vspace(0x1000, 0x4000_a000, 0x2000).unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let mapping = kernel
            .map_frame(space, frame, 0, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(mapping.va, 0x4000_a000);
        assert_eq!(kernel.mapping_at(space, mapping.va), Some(mapping));
        assert_eq!(
            kernel.map_frame(space, frame, mapping.va, Rights::READ | Rights::WRITE),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(
            kernel.map_frame(space, frame, 0, Rights::WRITE | Rights::EXEC),
            Err(KError::NoRights)
        );
        assert_eq!(
            kernel.map_frame(space, frame, 0x4000_a000, Rights::READ | Rights::EXEC),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(kernel.unmap_frame(space, mapping.va), Ok(mapping));
        assert_eq!(
            kernel.unmap_frame(space, mapping.va),
            Err(KError::NotMapped)
        );
    }

    #[test]
    fn syscall_rejects_bad_capability_types_and_wx() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Send as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::WrongType
            }
        );
        assert!(kernel.delete_cap(root, 1).is_some());
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Send as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );

        let vspace = kernel.create_vspace(0x1000, 0x4000_a000, 4096).unwrap();
        kernel
            .install_cap(root, 2, Cap::new(vspace, ObjType::VSpace, Rights::WRITE))
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                2,
                3,
                0,
                (Rights::WRITE | Rights::EXEC).0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights
            }
        );
    }

    #[test]
    fn cap_transfer_requires_an_explicit_empty_slot() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let worker = kernel.create_tcb("worker", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let notification = kernel.create_notification().unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(notification, ObjType::Notification, Rights::ALL),
            )
            .unwrap();
        assert_eq!(kernel.recv(worker, endpoint, 6), Ok(None));
        let message = Message::new(0xCAFE, &[1]).unwrap().with_cap(1);
        assert_eq!(kernel.send(root, endpoint, 0, message, false), Ok(true));
        assert_eq!(kernel.recv(worker, endpoint, 6), Ok(None));
        let second = Message::new(0xBEEF, &[2]).unwrap().with_cap(1);
        assert_eq!(
            kernel.send(root, endpoint, 0, second, false),
            Err(KError::SlotOccupied)
        );
    }

    #[test]
    fn frame_alignment_is_checked() {
        let mut kernel = Kernel::new();
        let space = kernel.create_vspace(1, 0x1000, 0x1000).unwrap();
        let frame = kernel.create_frame(0x9001, 4096).unwrap();
        assert_eq!(
            kernel.map_frame(space, frame, 0, Rights::READ),
            Err(KError::Alignment)
        );
    }

    #[test]
    fn revoke_removes_descendants_only() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel.copy_cap(root, 1, root, 2, Rights::ALL, 0).unwrap();
        kernel.copy_cap(root, 2, root, 3, Rights::ALL, 0).unwrap();
        assert_eq!(kernel.revoke_cap(root, 1).unwrap(), 2);
        assert!(kernel.lookup_cap(root, 1, None, Rights::NONE).is_ok());
        assert!(kernel.lookup_cap(root, 2, None, Rights::NONE).is_err());
        assert!(kernel.lookup_cap(root, 3, None, Rights::NONE).is_err());
    }
}
