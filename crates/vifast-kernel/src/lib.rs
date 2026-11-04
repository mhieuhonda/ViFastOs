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
    /// Program a user context after a successful `TcbConfigure`.
    ConfigureTcb {
        tcb: ObjRef,
        entry: u64,
        stack: u64,
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

pub use vifast_pte::Pte;
/// Platform page-table operations used by revoke/unmap, retype and the future
/// generic walker.  unsafe PTE/TLB access stays in the platform impl.
///
/// Writes are infallible; callers perform preflight checks before destroying
/// objects.  The default implementations keep host tests and simple platforms
/// usable; real ports override all methods they need.
pub trait PageTableOps {
    fn levels(&self) -> usize {
        0
    }
    fn index(&self, _va: u64, _level: usize) -> usize {
        0
    }
    fn read_pte(&self, _table_pa: u64, _idx: usize) -> u64 {
        0
    }
    fn write_pte(&self, _table_pa: u64, _idx: usize, _pte: u64) {}
    fn encode_leaf(&self, _pa: u64, _rights: Rights) -> u64 {
        0
    }
    fn encode_table(&self, _pa: u64) -> u64 {
        0
    }
    fn decode(&self, _pte: u64) -> Pte {
        Pte::Invalid
    }
    /// Level-aware decode for architectures where page and table descriptors
    /// share the same low bits (AArch64).
    fn decode_at(&self, pte: u64, _level: usize) -> Pte {
        self.decode(pte)
    }
    /// Number of physical pages needed by a retyped VSpace root.
    fn vspace_pages(&self) -> usize {
        1
    }
    /// Populate a fresh VSpace root (and optional private PDPT/L1) with the
    /// architecture's shared kernel entries.  User entries stay invalid.
    fn install_kernel(&self, _root_pa: u64, _aux_pa: u64) {}
    /// Physical range the kernel can access through its identity map.
    /// `(0, 0)` means the platform does not advertise a restriction.
    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0, 0)
    }
    fn user_va_range(&self) -> (u64, u64) {
        (0, 0)
    }
    fn flush_va(&self, _va: u64) {}
    fn flush_all_user(&self) {}
    fn zero_frame(&self, _pa: u64, _size: usize) {}
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
    pub(crate) page_table_ops: Option<&'static dyn PageTableOps>,
    pub(crate) next_cap_id: u64,
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub const fn new_const() -> Kernel {
        Kernel {
            objs: [Object::free(); MAX_OBJECTS],
            runq: [None; MAX_TCBS],
            runq_len: 0,
            current: None,
            seq: 0,
            ticks: 0,
            root: None,
            fs: fs::RamFs::new(),
            stats: KernelStats {
                ipc_sent: 0,
                ipc_received: 0,
                notifications: 0,
                context_switches: 0,
                ticks: 0,
            },
            page_table_ops: None,
            next_cap_id: 1,
        }
    }

    /// Reinitialize a statically allocated kernel in place.  This avoids
    /// constructing a large `Kernel` temporary on the boot stack.
    pub fn reset(&mut self) {
        for object in self.objs.iter_mut() {
            *object = Object::free();
        }
        self.runq = [None; MAX_TCBS];
        self.runq_len = 0;
        self.current = None;
        self.seq = 0;
        self.ticks = 0;
        self.root = None;
        self.fs = fs::RamFs::new();
        self.stats = KernelStats {
            ipc_sent: 0,
            ipc_received: 0,
            notifications: 0,
            context_switches: 0,
            ticks: 0,
        };
        self.page_table_ops = None;
        self.next_cap_id = 1;
        self.init_fs();
        if let Some(root) = self.create_tcb("root", 1) {
            self.root = Some(root);
        }
    }

    pub fn new() -> Kernel {
        let mut kernel = Self::new_const();
        kernel.reset();
        kernel
    }

    pub fn root(&self) -> Option<ObjRef> {
        self.root
    }

    pub fn set_page_table_ops(&mut self, ops: &'static dyn PageTableOps) {
        self.page_table_ops = Some(ops);
    }

    pub fn stats(&self) -> KernelStats {
        self.stats
    }

    fn alloc(&mut self, kind: Kind) -> Option<ObjRef> {
        self.alloc_owned(kind, None, None)
    }

    fn alloc_owned(
        &mut self,
        kind: Kind,
        owner: Option<ObjRef>,
        creator: Option<CapRef>,
    ) -> Option<ObjRef> {
        for i in 0..MAX_OBJECTS {
            if self.objs[i].is_free() {
                let gen = self.objs[i].gen.wrapping_add(1);
                let gen = if gen == 0 { 1 } else { gen };
                self.objs[i].gen = gen;
                self.objs[i].kind = kind;
                self.objs[i].owner = owner;
                self.objs[i].creator = creator;
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

    fn tcb_count(&self) -> usize {
        self.objs
            .iter()
            .filter(|object| matches!(object.kind, Kind::Tcb(_)))
            .count()
    }

    pub fn create_tcb(&mut self, name: &'static str, priority: u8) -> Option<ObjRef> {
        if self.tcb_count() >= MAX_TCBS {
            return None;
        }
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
        self.create_untyped_with_limit(base, size, page_size, 8)
    }

    pub fn create_untyped_with_limit(
        &mut self,
        base: usize,
        size: usize,
        page_size: usize,
        max_objects: usize,
    ) -> Option<ObjRef> {
        if let Some(ops) = self.page_table_ops {
            let (lo, hi) = ops.kernel_accessible_range();
            let end = base.checked_add(size)?;
            if lo < hi && ((base as u64) < lo || end as u64 > hi) {
                return None;
            }
        }
        self.alloc(Kind::Untyped(Untyped {
            base,
            size,
            bump: 0,
            page_size,
            max_objects,
            objects: 0,
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

    pub fn create_user_vspace(
        &mut self,
        root: u64,
        data_base: u64,
        page_size: u64,
        code_base: u64,
        code_len: u64,
        stack_base: u64,
        stack_len: u64,
    ) -> Option<ObjRef> {
        self.alloc(Kind::VSpace(VSpace::with_user_layout(
            root, data_base, page_size, page_size, code_base, code_len, stack_base, stack_len,
        )))
    }

    pub fn set_tcb_vspace(&mut self, tcb: ObjRef, vspace: ObjRef) -> KResult<()> {
        if self.vspace(vspace).is_none() {
            return Err(KError::WrongType);
        }
        let table = self.tcb_mut(tcb).ok_or(KError::WrongType)?;
        table.vspace = Some(vspace);
        Ok(())
    }

    pub fn configure_tcb(
        &mut self,
        tcb: ObjRef,
        entry: u64,
        stack: u64,
        vspace: ObjRef,
        priority: u8,
    ) -> KResult<()> {
        let space = self.vspace(vspace).ok_or(KError::WrongType)?;
        if space.code_len == 0
            || entry < space.code_base
            || entry >= space.code_base.saturating_add(space.code_len)
        {
            return Err(KError::InvalidArg);
        }
        if space.stack_len == 0
            || stack <= space.stack_base
            || stack > space.stack_base.saturating_add(space.stack_len)
        {
            return Err(KError::InvalidArg);
        }
        let table = self.tcb_mut(tcb).ok_or(KError::WrongType)?;
        table.entry = entry;
        table.stack = stack;
        table.vspace = Some(vspace);
        table.priority = priority.min(MAX_PRIORITY);
        Ok(())
    }

    pub fn resume_tcb(&mut self, tcb: ObjRef) -> KResult<()> {
        let table = self.tcb_mut(tcb).ok_or(KError::WrongType)?;
        if table.entry == 0 || table.vspace.is_none() {
            return Err(KError::IllegalOperation);
        }
        if table.state == ThreadState::Running {
            return Err(KError::AlreadyMapped);
        }
        table.state = ThreadState::Ready;
        self.enqueue(tcb);
        Ok(())
    }

    /// Validate a user pointer against the TCB's VSpace layout and shadow
    /// mappings.  Kernel addresses and unmapped pages are rejected before any
    /// architecture copy routine sees the pointer.
    pub fn validate_user_range(
        &self,
        tcb: ObjRef,
        va: u64,
        len: usize,
        need_write: bool,
    ) -> KResult<()> {
        let space_ref = self
            .tcb(tcb)
            .and_then(|table| table.vspace)
            .ok_or(KError::InvalidArg)?;
        let space = self.vspace(space_ref).ok_or(KError::InvalidArg)?;
        if space.validate_range(va, len, need_write) {
            Ok(())
        } else {
            Err(KError::InvalidArg)
        }
    }

    pub fn create_page_table(&mut self, root: u64, _level: u8) -> Option<ObjRef> {
        self.alloc(Kind::PageTable(PageTable { root, mapped: None }))
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

    pub fn page_table_mut(&mut self, r: ObjRef) -> Option<&mut PageTable> {
        match self.get_mut(r)?.kind {
            Kind::PageTable(ref mut table) => Some(table),
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
        self.retype_as(untyped, ty, None)
    }

    /// Retype with the capability that authorizes allocation.  Syscall
    /// Retype uses this so a different copy of the same Untyped cannot destroy
    /// objects created through this cap.
    pub fn retype_as(
        &mut self,
        untyped: ObjRef,
        ty: ObjType,
        creator: Option<SlotAddr>,
    ) -> KResult<ObjRef> {
        let creator = match creator {
            Some(addr) => {
                let cap = self.cap_at(addr).ok_or(KError::InvalidCap)?;
                Some(CapRef { addr, id: cap.id })
            }
            None => None,
        };
        let (base, size, bump, page, max_objects, objects) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (
                region.base,
                region.size,
                region.bump,
                region.page_size,
                region.max_objects,
                region.objects,
            )
        };
        if page == 0 || !page.is_power_of_two() {
            return Err(KError::Alignment);
        }
        if objects >= max_objects {
            return Err(KError::OutOfMemory);
        }
        if ty == ObjType::Tcb && self.tcb_count() >= MAX_TCBS {
            return Err(KError::OutOfMemory);
        }
        let vspace_pages = if ty == ObjType::VSpace {
            self.page_table_ops
                .map(|ops| ops.vspace_pages())
                .unwrap_or(1)
                .max(1)
        } else {
            0
        };
        let (kind, charge, align) = match ty {
            ObjType::Endpoint => (
                Kind::Endpoint(Endpoint::default()),
                OBJ_ENDPOINT_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::Notification => (
                Kind::Notification(Notification::default()),
                OBJ_NOTIFICATION_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::Tcb => (
                Kind::Tcb(Tcb::new("retyped", 1)),
                OBJ_TCB_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::Frame | ObjType::PageTable => (Kind::Free, page, page),
            ObjType::VSpace => {
                let charge = page.checked_mul(vspace_pages).ok_or(KError::OutOfMemory)?;
                (Kind::Free, charge, page)
            }
            _ => return Err(KError::NotImplemented),
        };
        let unaligned = base.checked_add(bump).ok_or(KError::OutOfMemory)?;
        let pa = (unaligned + align - 1) & !(align - 1);
        let end = base.checked_add(size).ok_or(KError::OutOfMemory)?;
        if pa.checked_add(charge).ok_or(KError::OutOfMemory)? > end {
            return Err(KError::OutOfMemory);
        }
        let consumed = pa.checked_add(charge).ok_or(KError::OutOfMemory)? - base;
        if matches!(ty, ObjType::Frame | ObjType::PageTable | ObjType::VSpace) {
            if let Some(ops) = self.page_table_ops {
                for i in 0..vspace_pages.max(1) {
                    ops.zero_frame(pa as u64 + i as u64 * page as u64, page);
                }
            }
        }
        let kind = if matches!(ty, ObjType::Frame | ObjType::PageTable) {
            if ty == ObjType::Frame {
                Kind::Frame(Frame { pa, size: page })
            } else {
                Kind::PageTable(PageTable {
                    root: pa as u64,
                    mapped: None,
                })
            }
        } else if ty == ObjType::VSpace {
            let aux_pa = (vspace_pages > 1).then_some(pa as u64 + page as u64);
            if let Some(ops) = self.page_table_ops {
                ops.install_kernel(pa as u64, aux_pa.unwrap_or(0));
            }
            let mut space = VSpace::with_page_size(
                pa as u64,
                vifast_abi::user_layout::USER_VA_START as u64,
                (vifast_abi::user_layout::USER_VA_END - vifast_abi::user_layout::USER_VA_START)
                    as u64,
                page as u64,
            );
            space.aux_pa = aux_pa;
            Kind::VSpace(space)
        } else {
            kind
        };
        let object = self
            .alloc_owned(kind, Some(untyped), creator)
            .ok_or(KError::OutOfMemory)?;
        let region = self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)?;
        region.bump = consumed;
        region.objects = objects + 1;
        Ok(object)
    }

    /// Attach a retyped PageTable object at the first empty entry on the path
    /// to `va`.  The kernel, not user space, chooses the level.  The whole
    /// range covered by the new table must fit inside the architecture user VA
    /// range, otherwise no PTE is written.
    pub fn map_page_table(&mut self, page_table: ObjRef, vspace: ObjRef, va: u64) -> KResult<u8> {
        let table = *self.page_table(page_table).ok_or(KError::WrongType)?;
        if table.mapped.is_some() {
            return Err(KError::AlreadyMapped);
        }
        let (root, page_size, base, end) = {
            let space = self.vspace(vspace).ok_or(KError::WrongType)?;
            (space.root, space.page_size, space.base, space.end)
        };
        if page_size == 0 || !page_size.is_power_of_two() || table.root % page_size != 0 {
            return Err(KError::Alignment);
        }
        if va % page_size != 0 {
            return Err(KError::Alignment);
        }
        if va < base || va >= end {
            return Err(KError::InvalidArg);
        }
        let ops = self.page_table_ops.ok_or(KError::MissingPageTable)?;
        let levels = ops.levels();
        if levels < 2 {
            return Err(KError::MissingPageTable);
        }
        let (user_start, user_end) = ops.user_va_range();
        if user_start >= user_end || va < user_start || va >= user_end {
            return Err(KError::InvalidArg);
        }
        if table.root == root {
            return Err(KError::InvalidArg);
        }

        // Walk from the root until the first invalid entry.  Nothing is
        // written during this phase, so all error paths leave the table
        // byte-for-byte unchanged.
        let mut current = root;
        for level in 0..levels - 1 {
            let idx = ops.index(va, level);
            let pte = ops.read_pte(current, idx);
            match ops.decode_at(pte, level) {
                Pte::Invalid => {
                    let child_level = level + 1;
                    let span = vifast_pte::table_span(levels, child_level, page_size);
                    if span == 0 {
                        return Err(KError::InvalidArg);
                    }
                    let va_base = va & !(span - 1);
                    let Some(va_end) = va_base.checked_add(span) else {
                        return Err(KError::InvalidArg);
                    };
                    if va_base < user_start || va_end > user_end {
                        return Err(KError::InvalidArg);
                    }
                    ops.write_pte(current, idx, ops.encode_table(table.root));
                    ops.flush_va(va);
                    let table = self.page_table_mut(page_table).ok_or(KError::WrongType)?;
                    table.mapped = Some(PageTableMapping {
                        vspace,
                        va_base,
                        level: child_level as u8,
                    });
                    return Ok(child_level as u8);
                }
                Pte::Table(next) => current = next,
                Pte::Leaf(_, _) => return Err(KError::AlreadyMapped),
            }
        }
        Err(KError::AlreadyMapped)
    }

    /// Validate and install a user mapping through the generic page-table
    /// walk.  No platform-specific map function is used.  Intermediate page
    /// tables must already exist; otherwise `MissingPageTable` is returned.
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
        if rights.contains(Rights::WRITE) && !rights.contains(Rights::READ) {
            return Err(KError::InvalidArg); // Sv39 reserves W without R
        }
        let frame = *self.frame(frame).ok_or(KError::WrongType)?;
        let (root, page_size, base, end, next_va) = {
            let space = self.vspace(vspace).ok_or(KError::WrongType)?;
            (
                space.root,
                space.page_size,
                space.base,
                space.end,
                space.next_va,
            )
        };
        if page_size == 0
            || !page_size.is_power_of_two()
            || frame.pa as u64 % page_size != 0
            || (frame.size as u64) < page_size
        {
            return Err(KError::Alignment);
        }
        let va = if hint == 0 { next_va } else { hint };
        if va % page_size != 0 {
            return Err(KError::Alignment);
        }
        if va < base || va.saturating_add(page_size) > end {
            return Err(KError::InvalidArg);
        }
        if self
            .vspace(vspace)
            .is_some_and(|space| space.contains(va, page_size))
        {
            return Err(KError::AlreadyMapped);
        }
        if self.page_table_ops.is_some() {
            self.map_leaf(root, va, frame.pa as u64, rights)?;
        }
        let mapping = Mapping {
            va,
            pa: frame.pa as u64,
            len: page_size,
            rights,
        };
        let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
        if let Err(error) = space.insert(mapping) {
            if self.page_table_ops.is_some() {
                let _ = self.unmap_page_table(root, va);
            }
            return Err(error);
        }
        Ok(mapping)
    }

    /// Remove a mapping from the portable shadow table.  The architecture
    /// handler clears the actual entry and flushes its TLB.
    pub fn unmap_frame(&mut self, vspace: ObjRef, va: u64) -> KResult<Mapping> {
        let root = self
            .vspace(vspace)
            .map(|space| space.root)
            .ok_or(KError::WrongType)?;
        let index = self
            .vspace(vspace)
            .and_then(|space| space.mapping_index(va))
            .ok_or(KError::NotMapped)?;
        if self.page_table_ops.is_some() {
            self.unmap_page_table(root, va)?;
        }
        let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
        space.mappings[index].take().ok_or(KError::NotMapped)
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
                    if let Some(vspace) = tcb.vspace {
                        if self.vspace(vspace).is_none() {
                            return Err(KError::ObjectDestroyed);
                        }
                    }
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
                    if untyped.bump > untyped.size || untyped.objects > untyped.max_objects {
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

    pub fn install_cap(&mut self, tcb: ObjRef, slot: u8, mut cap: Cap) -> KResult<()> {
        if slot == 0 || slot as usize >= MAX_CAPS {
            return Err(KError::InvalidArg);
        }
        if self.get(cap.obj).is_none() {
            return Err(KError::ObjectDestroyed);
        }
        cap.id = self.next_cap_id;
        cap.root = cap.parent.is_none();
        self.next_cap_id = self.next_cap_id.wrapping_add(1);
        if self.next_cap_id == 0 {
            self.next_cap_id = 1;
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

    pub fn delete_cap(&mut self, tcb: ObjRef, slot: u8) -> KResult<Cap> {
        let addr = SlotAddr { tcb, slot };
        let cap = self.cap_at(addr).ok_or(KError::InvalidCap)?;
        if cap.ty == ObjType::Untyped {
            let target = CapRef { addr, id: cap.id };
            let has_live_objects = self.objs.iter().any(|object| {
                object.owner == Some(cap.obj)
                    && object
                        .creator
                        .is_some_and(|creator| self.cap_ref_is_descendant(creator, target))
            });
            let has_child_caps =
                (0..MAX_OBJECTS).any(|i| {
                    let r = ObjRef {
                        index: i as u8,
                        gen: self.objs[i].gen,
                    };
                    self.tcb(r).is_some_and(|table| {
                        table.cspace.iter().flatten().any(|child| {
                            child.parent.is_some() && self.is_descendant(child, target)
                        })
                    })
                });
            if has_live_objects || has_child_caps {
                return Err(KError::IllegalOperation);
            }
        }
        self.tcb_mut(tcb)
            .and_then(|table| table.cspace.get_mut(slot as usize)?.take())
            .ok_or(KError::InvalidCap)
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
        derived.parent = Some(CapRef {
            addr: SlotAddr {
                tcb: src_tcb,
                slot: src_slot,
            },
            id: source.id,
        });
        self.install_cap(dst_tcb, dst_slot, derived)
    }

    fn cap_ref_is_descendant(&self, creator: CapRef, target: CapRef) -> bool {
        if creator == target {
            return true;
        }
        let Some(cap) = self.cap_at(creator.addr) else {
            return false;
        };
        if cap.id != creator.id {
            return false;
        }
        self.is_descendant(&cap, target)
    }

    fn is_descendant(&self, cap: &Cap, target: CapRef) -> bool {
        let mut parent = cap.parent;
        let mut depth = 0usize;
        while let Some(parent_ref) = parent {
            if parent_ref == target {
                return true;
            }
            let Some(parent_cap) = self.cap_at(parent_ref.addr) else {
                return false;
            };
            if parent_cap.id != parent_ref.id {
                return false;
            }
            depth += 1;
            if depth > MAX_OBJECTS * MAX_CAPS {
                return false;
            }
            parent = parent_cap.parent;
        }
        false
    }

    fn wake_error(&mut self, tcb: ObjRef, error: KError) {
        if self.tcb(tcb).is_none() {
            return;
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.state = ThreadState::Ready;
            table.pending = None;
            table.resume = Some(Message::empty(0));
            table.resume_error = Some(error);
        }
        self.enqueue(tcb);
    }

    fn survivor_for(&self, cap_ref: CapRef, dying: &[CapRef]) -> Option<CapRef> {
        let cap = self.cap_at(cap_ref.addr)?;
        if cap.id != cap_ref.id {
            return None;
        }
        let mut parent = cap.parent;
        let mut depth = 0usize;
        while let Some(parent_ref) = parent {
            if !dying.contains(&parent_ref) {
                return self
                    .cap_at(parent_ref.addr)
                    .is_some_and(|candidate| candidate.id == parent_ref.id)
                    .then_some(parent_ref);
            }
            let parent_cap = self.cap_at(parent_ref.addr)?;
            depth += 1;
            if depth > MAX_OBJECTS * MAX_CAPS {
                return None;
            }
            parent = parent_cap.parent;
        }
        None
    }

    /// A dying TCB must not take its capability subtree with it.  Reparent
    /// every object/cap that referenced one of its caps to the nearest
    /// surviving ancestor.  A cap whose root dies with no ancestor leaves
    /// creator/parent as None; only a root cap can collect those objects.
    fn reparent_tcb_caps(&mut self, obj: ObjRef) {
        let Some(table) = self.tcb(obj) else {
            return;
        };
        let mut dying = [CapRef::default(); MAX_CAPS];
        let mut survivors = [None; MAX_CAPS];
        let mut count = 0usize;
        for (slot, cap) in table.cspace.iter().enumerate() {
            if let Some(cap) = cap {
                dying[count] = CapRef {
                    addr: SlotAddr {
                        tcb: obj,
                        slot: slot as u8,
                    },
                    id: cap.id,
                };
                count += 1;
            }
        }
        for i in 0..count {
            survivors[i] = self.survivor_for(dying[i], &dying[..count]);
        }

        for object in self.objs.iter_mut() {
            let Some(creator) = object.creator else {
                continue;
            };
            if let Some(index) = dying[..count].iter().position(|c| *c == creator) {
                object.creator = survivors[index];
            }
        }

        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let Some(table) = self.tcb_mut(r) else {
                continue;
            };
            for cap in table.cspace.iter_mut().flatten() {
                let Some(parent) = cap.parent else {
                    continue;
                };
                if let Some(index) = dying[..count].iter().position(|c| *c == parent) {
                    cap.parent = survivors[index];
                }
            }
        }
    }

    fn detach_tcb(&mut self, obj: ObjRef) {
        for slot in self.runq.iter_mut() {
            if *slot == Some(obj) {
                *slot = None;
            }
        }
        if let Some((client, _)) = self.tcb(obj).and_then(|table| table.caller) {
            self.wake_error(client, KError::ObjectDestroyed);
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(endpoint) = self.endpoint_mut(r) {
                endpoint.send.remove(obj);
                endpoint.recv.remove(obj);
            }
            if let Some(table) = self.tcb_mut(r) {
                if table.caller.is_some_and(|(client, _)| client == obj) {
                    table.caller = None;
                }
            }
            if let Some(notification) = self.notification_mut(r) {
                if notification.waiter == Some(obj) {
                    notification.waiter = None;
                }
            }
        }
    }

    fn destroy_endpoint(&mut self, obj: ObjRef) {
        let mut senders = [ObjRef::invalid(); MAX_QUEUE];
        let mut receivers = [ObjRef::invalid(); MAX_QUEUE];
        let mut send_count = 0usize;
        let mut recv_count = 0usize;
        if let Some(endpoint) = self.endpoint_mut(obj) {
            while let Some(sender) = endpoint.send.pop() {
                if send_count < MAX_QUEUE {
                    senders[send_count] = sender;
                    send_count += 1;
                }
            }
            while let Some(receiver) = endpoint.recv.pop() {
                if recv_count < MAX_QUEUE {
                    receivers[recv_count] = receiver;
                    recv_count += 1;
                }
            }
        }
        for sender in senders.iter().take(send_count) {
            self.wake_error(*sender, KError::ObjectDestroyed);
        }
        for receiver in receivers.iter().take(recv_count) {
            self.wake_error(*receiver, KError::ObjectDestroyed);
        }
    }

    fn destroy_notification(&mut self, obj: ObjRef) {
        let waiter = if let Some(notification) = self.notification_mut(obj) {
            notification.word = 0;
            notification.waiter.take()
        } else {
            None
        };
        if let Some(waiter) = waiter {
            self.wake_error(waiter, KError::ObjectDestroyed);
        }
    }

    fn unmap_with_ops(ops: &dyn PageTableOps, root: u64, va: u64) -> KResult<()> {
        let (lo, hi) = ops.user_va_range();
        if va < lo || va >= hi {
            return Err(KError::InvalidArg);
        }
        let levels = ops.levels();
        if levels == 0 {
            return Err(KError::MissingPageTable);
        }
        let mut table = root;
        for level in 0..levels {
            let idx = ops.index(va, level);
            let pte = ops.read_pte(table, idx);
            match ops.decode_at(pte, level) {
                Pte::Invalid => return Err(KError::NotMapped),
                Pte::Table(pa) => table = pa,
                Pte::Leaf(_, _) => {
                    ops.write_pte(table, idx, 0);
                    ops.flush_va(va);
                    return Ok(());
                }
            }
        }
        Err(KError::NotMapped)
    }

    pub fn unmap_page_table(&self, root: u64, va: u64) -> KResult<()> {
        let Some(ops) = self.page_table_ops else {
            return Ok(());
        };
        Self::unmap_with_ops(ops, root, va)
    }

    fn map_leaf(&self, root: u64, va: u64, pa: u64, rights: Rights) -> KResult<()> {
        let ops = self.page_table_ops.ok_or(KError::MissingPageTable)?;
        let (lo, hi) = ops.user_va_range();
        let page = self.page_table_ops.map(|_| 0u64).unwrap_or(0);
        let _ = page;
        if va < lo || va.saturating_add(4096) > hi {
            return Err(KError::InvalidArg);
        }
        let levels = ops.levels();
        if levels == 0 {
            return Err(KError::MissingPageTable);
        }
        let mut table = root;
        for level in 0..levels {
            let idx = ops.index(va, level);
            let pte = ops.read_pte(table, idx);
            let last = level + 1 == levels;
            match ops.decode_at(pte, level) {
                Pte::Invalid if last => {
                    ops.write_pte(table, idx, ops.encode_leaf(pa, rights));
                    ops.flush_va(va);
                    return Ok(());
                }
                Pte::Invalid => return Err(KError::MissingPageTable),
                Pte::Table(next) if !last => table = next,
                Pte::Table(_) | Pte::Leaf(_, _) => return Err(KError::AlreadyMapped),
            }
        }
        Err(KError::MissingPageTable)
    }

    fn unmap_frame_everywhere(&mut self, frame: Frame) {
        let start = frame.pa as u64;
        let end = start.saturating_add(frame.size as u64);
        let ops = self.page_table_ops;
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let Some(space) = self.vspace_mut(r) else {
                continue;
            };
            for slot in space.mappings.iter_mut() {
                if let Some(mapping) = *slot {
                    if mapping.pa >= start && mapping.pa < end {
                        if let Some(ops) = ops {
                            let _ = Self::unmap_with_ops(ops, space.root, mapping.va);
                        }
                        *slot = None;
                    }
                }
            }
        }
    }

    fn unmap_space(&mut self, obj: ObjRef) {
        let ops = self.page_table_ops;
        let Some(space) = self.vspace_mut(obj) else {
            return;
        };
        for slot in space.mappings.iter_mut() {
            if let Some(mapping) = *slot {
                if let Some(ops) = ops {
                    let _ = Self::unmap_with_ops(ops, space.root, mapping.va);
                }
                *slot = None;
            }
        }
    }

    fn remove_caps_to(&mut self, obj: ObjRef) {
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(table) = self.tcb_mut(r) {
                for slot in table.cspace.iter_mut() {
                    if slot.is_some_and(|cap| cap.obj == obj) {
                        *slot = None;
                    }
                }
            }
        }
    }

    fn table_span_for(&self, vspace: ObjRef, level: u8) -> Option<(u64, u64)> {
        let space = self.vspace(vspace)?;
        let ops = self.page_table_ops?;
        let span = vifast_pte::table_span(ops.levels(), level as usize, space.page_size);
        (span != 0).then_some((space.page_size, span))
    }

    /// Drop reverse mappings that lie inside a page-table span.  The leaf
    /// unmaps happen before the parent entry is removed, so the generic walk
    /// can still reach them and flush their TLB entries.
    fn forget_mappings_in_span(&mut self, vspace: ObjRef, start: u64, end: u64) {
        let ops = self.page_table_ops;
        let Some(space) = self.vspace_mut(vspace) else {
            return;
        };
        for slot in space.mappings.iter_mut() {
            if let Some(mapping) = *slot {
                let mapping_end = mapping.va.saturating_add(mapping.len);
                if mapping.va < end && start < mapping_end {
                    if let Some(ops) = ops {
                        let _ = Self::unmap_with_ops(ops, space.root, mapping.va);
                    }
                    *slot = None;
                }
            }
        }
    }

    /// When a parent table dies, child tables below it are unreachable even if
    /// their objects live in another Untyped.  Detach and zero them so they
    /// cannot be reattached full of stale PTEs.
    fn orphan_child_tables(&mut self, vspace: ObjRef, start: u64, end: u64, parent_level: u8) {
        let page_size = self.vspace(vspace).map(|space| space.page_size);
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let (root, mapped) = match self.page_table(r) {
                Some(table) => (table.root, table.mapped),
                None => continue,
            };
            let Some(location) = mapped else {
                continue;
            };
            if location.vspace == vspace
                && location.va_base >= start
                && location.va_base < end
                && location.level > parent_level
            {
                if let Some(table) = self.page_table_mut(r) {
                    table.mapped = None;
                }
                if let (Some(ops), Some(size)) = (self.page_table_ops, page_size) {
                    ops.zero_frame(root, size as usize);
                }
            }
        }
    }

    /// Remove the parent entry that points at `obj`, drop reverse mappings in
    /// its span, and flush the TLB.  This never walks through the table being
    /// removed; child tables must be destroyed first, which is why revoke
    /// sorts tables deepest-first.
    fn detach_page_table(&mut self, obj: ObjRef) {
        let (root, mapped) = match self.page_table(obj) {
            Some(table) => (table.root, table.mapped),
            None => return,
        };
        let Some(location) = mapped else {
            return;
        };
        let page_and_span = self.table_span_for(location.vspace, location.level);
        if let Some((_, span)) = page_and_span {
            let Some(end) = location.va_base.checked_add(span) else {
                return;
            };
            self.forget_mappings_in_span(location.vspace, location.va_base, end);
            self.orphan_child_tables(location.vspace, location.va_base, end, location.level);
        }
        if location.level > 0 {
            if let Some(ops) = self.page_table_ops {
                if let Some(space) = self.vspace(location.vspace) {
                    let mut current = space.root;
                    let parent_level = location.level as usize - 1;
                    let mut walk_ok = true;
                    for level in 0..parent_level {
                        let idx = ops.index(location.va_base, level);
                        match ops.decode_at(ops.read_pte(current, idx), level) {
                            Pte::Table(next) => current = next,
                            _ => {
                                walk_ok = false;
                                break;
                            }
                        }
                    }
                    if walk_ok {
                        let idx = ops.index(location.va_base, parent_level);
                        let pte = ops.read_pte(current, idx);
                        if let Pte::Table(pa) = ops.decode_at(pte, parent_level) {
                            if pa == root {
                                ops.write_pte(current, idx, 0);
                                ops.flush_va(location.va_base);
                            }
                        }
                    }
                }
            }
        }
        if let Some((page_size, _)) = page_and_span {
            if let Some(ops) = self.page_table_ops {
                ops.zero_frame(root, page_size as usize);
                ops.flush_all_user();
            }
        }
        if let Some(table) = self.page_table_mut(obj) {
            table.mapped = None;
        }
    }

    fn destroy_vspace_page_tables(&mut self, obj: ObjRef) {
        let page_size = self.vspace(obj).map(|space| space.page_size);
        // The reverse map is still able to walk through every table here.
        self.unmap_space(obj);
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let (root, mapped) = match self.page_table(r) {
                Some(table) => (table.root, table.mapped),
                None => continue,
            };
            if mapped.is_some_and(|location| location.vspace == obj) {
                if let Some(table) = self.page_table_mut(r) {
                    table.mapped = None;
                }
                if let (Some(ops), Some(size)) = (self.page_table_ops, page_size) {
                    ops.zero_frame(root, size as usize);
                }
            }
        }
        if let Some(ops) = self.page_table_ops {
            ops.flush_all_user();
        }
    }

    fn destroy_rank(&self, obj: ObjRef) -> u8 {
        match self.get(obj).map(|object| object.kind) {
            Some(Kind::Frame(_)) => 0,
            Some(Kind::PageTable(_)) => 1,
            Some(Kind::VSpace(_)) => 3,
            _ => 2,
        }
    }

    /// True when `a` must be destroyed before `b`.  Frames go first so their
    /// leaves can still be reached; page tables are removed child-first.
    fn destroy_before(&self, a: ObjRef, b: ObjRef) -> bool {
        let rank_a = self.destroy_rank(a);
        let rank_b = self.destroy_rank(b);
        if rank_a != rank_b {
            return rank_a < rank_b;
        }
        if rank_a == 1 {
            let level_a = self
                .page_table(a)
                .and_then(|table| table.mapped)
                .map(|mapping| mapping.level)
                .unwrap_or(0);
            let level_b = self
                .page_table(b)
                .and_then(|table| table.mapped)
                .map(|mapping| mapping.level)
                .unwrap_or(0);
            return level_a > level_b;
        }
        false
    }

    fn destroy_object(&mut self, obj: ObjRef) -> KResult<()> {
        let Some(kind) = self.get(obj).map(|object| object.kind) else {
            return Ok(());
        };
        if self.current == Some(obj) {
            return Err(KError::IllegalOperation);
        }
        if matches!(kind, Kind::VSpace(_))
            && self
                .current
                .and_then(|current| self.tcb(current))
                .and_then(|tcb| tcb.vspace)
                == Some(obj)
        {
            return Err(KError::IllegalOperation);
        }
        match kind {
            Kind::Tcb(_) => {
                // TCB teardown is the one path allowed to bypass
                // delete_cap's refusal rule: reparent first, then drop cspace.
                self.reparent_tcb_caps(obj);
                self.detach_tcb(obj);
            }
            Kind::Endpoint(_) => self.destroy_endpoint(obj),
            Kind::Notification(_) => self.destroy_notification(obj),
            Kind::Frame(frame) => self.unmap_frame_everywhere(frame),
            Kind::PageTable(_) => self.detach_page_table(obj),
            Kind::VSpace(_) => {
                self.destroy_vspace_page_tables(obj);
                for i in 0..MAX_OBJECTS {
                    let r = ObjRef {
                        index: i as u8,
                        gen: self.objs[i].gen,
                    };
                    if let Some(table) = self.tcb_mut(r) {
                        if table.vspace == Some(obj) {
                            table.vspace = None;
                            table.state = ThreadState::Faulted;
                        }
                    }
                }
            }
            Kind::Untyped(_) | Kind::Free => {}
        }
        self.remove_caps_to(obj);
        if let Some(object) = self.get_mut(obj) {
            object.kind = Kind::Free;
            object.owner = None;
            object.creator = None;
            let gen = object.gen.wrapping_add(1);
            object.gen = if gen == 0 { 1 } else { gen };
        }
        Ok(())
    }

    /// Remove all capabilities derived from `slot`.  The root capability is
    /// left in place.  Returns the number of capabilities removed.
    pub fn revoke_cap(&mut self, tcb: ObjRef, slot: u8) -> KResult<usize> {
        let target = SlotAddr { tcb, slot };
        let target_cap = self.cap_at(target).ok_or(KError::InvalidCap)?;
        let target_ref = CapRef {
            addr: target,
            id: target_cap.id,
        };
        let mut objects = [ObjRef::invalid(); MAX_OBJECTS];
        let mut object_count = 0usize;

        if target_cap.ty == ObjType::Untyped {
            for i in 0..MAX_OBJECTS {
                if self.objs[i].owner != Some(target_cap.obj) {
                    continue;
                }
                let created_here = match self.objs[i].creator {
                    Some(creator) => self.cap_ref_is_descendant(creator, target_ref),
                    None => target_cap.root,
                };
                if created_here {
                    let r = ObjRef {
                        index: i as u8,
                        gen: self.objs[i].gen,
                    };
                    if !objects[..object_count].contains(&r) && object_count < objects.len() {
                        objects[object_count] = r;
                        object_count += 1;
                    }
                }
            }
            if objects[..object_count].contains(&self.current.unwrap_or(ObjRef::invalid())) {
                return Err(KError::IllegalOperation);
            }
            // Destroy in dependency order: Frame leaves first, then page
            // tables deepest-first, and finally VSpace roots.
            for i in 1..object_count {
                let mut j = i;
                while j > 0 && self.destroy_before(objects[j], objects[j - 1]) {
                    objects.swap(j, j - 1);
                    j -= 1;
                }
            }
        }

        const BITS: usize = (MAX_OBJECTS * MAX_CAPS + 63) / 64;
        let mut victim_bits = [0u64; BITS];
        let mut removed = 0usize;
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let Some(table) = self.tcb(r) else { continue };
            for s in 1..MAX_CAPS {
                let Some(cap) = table.cspace[s] else { continue };
                let remove = self.is_descendant(&cap, target_ref)
                    || (target_cap.ty == ObjType::Untyped
                        && objects[..object_count].contains(&cap.obj));
                if remove {
                    let bit = i * MAX_CAPS + s;
                    victim_bits[bit / 64] |= 1u64 << (bit % 64);
                    removed += 1;
                }
            }
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            for s in 1..MAX_CAPS {
                let bit = i * MAX_CAPS + s;
                if victim_bits[bit / 64] & (1u64 << (bit % 64)) != 0 {
                    if let Some(table) = self.tcb_mut(r) {
                        table.cspace[s] = None;
                    }
                }
            }
        }

        if target_cap.ty == ObjType::Untyped {
            for object in objects.iter().take(object_count) {
                self.destroy_object(*object)?;
            }
            let remaining = self
                .objs
                .iter()
                .filter(|object| object.owner == Some(target_cap.obj))
                .count();
            if let Some(region) = self.untyped_mut(target_cap.obj) {
                region.objects = remaining;
                if remaining == 0 {
                    region.bump = 0;
                }
            }
        }
        Ok(removed)
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
        // Keep `current` set until the architecture has saved the user context.
        // The runtime's switch_blocked() saves it, then pick() replaces it.
        let current = self.current?;
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

    pub fn take_resume_error(&mut self, tcb: ObjRef) -> Option<KError> {
        self.tcb_mut(tcb)
            .and_then(|table| table.resume_error.take())
    }

    /// Suspend the currently running TCB after a user fault and return the
    /// next runnable thread (if any).  The TCB is not destroyed so its fault
    /// can later be reported to a fault endpoint.
    pub fn fault_current(&mut self) -> Option<ObjRef> {
        let old = self.current.take()?;
        if let Some(table) = self.tcb_mut(old) {
            table.state = ThreadState::Faulted;
            table.queued = false;
        }
        self.pick()
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
            if self.yield_current().is_some() {
                return UserAction::Blocked;
            }
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
            let len = (a1 as usize).min(USER_IO_MAX);
            if len != 0 && self.validate_user_range(tcb, a0, len, false).is_err() {
                return UserAction::Return {
                    value: 0,
                    error: KError::InvalidArg,
                };
            }
            return UserAction::Debug {
                ptr: a0,
                len: len as u64,
            };
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
                let untyped_slot = slot_arg(a0).unwrap_or(0);
                let untyped =
                    match self.lookup_cap(tcb, untyped_slot, Some(ObjType::Untyped), Rights::WRITE)
                    {
                        Ok(cap) => cap,
                        Err(error) => return UserAction::Return { value: 0, error },
                    };
                let ty = match a1 {
                    n if n == ObjType::Endpoint as u64 => ObjType::Endpoint,
                    n if n == ObjType::Notification as u64 => ObjType::Notification,
                    n if n == ObjType::Tcb as u64 => ObjType::Tcb,
                    n if n == ObjType::Frame as u64 => ObjType::Frame,
                    n if n == ObjType::PageTable as u64 => ObjType::PageTable,
                    n if n == ObjType::VSpace as u64 => ObjType::VSpace,
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
                let object = match self.retype_as(
                    untyped.obj,
                    ty,
                    Some(SlotAddr {
                        tcb,
                        slot: untyped_slot,
                    }),
                ) {
                    Ok(object) => object,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let mut cap = Cap::new(object, ty, Rights::READ | Rights::WRITE | Rights::GRANT);
                cap.parent = Some(CapRef {
                    addr: SlotAddr {
                        tcb,
                        slot: untyped_slot,
                    },
                    id: untyped.id,
                });
                match self.install_cap(tcb, dest, cap) {
                    Ok(()) => UserAction::Return {
                        value: dest as u64,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::CapCopy as u64 => {
                let src_slot = slot_arg(a0).unwrap_or(0);
                let dst_tcb_slot = slot_arg(a1).unwrap_or(0);
                let dst_slot = slot_arg(a2).unwrap_or(0);
                if dst_slot == 0 {
                    return UserAction::Return {
                        value: 0,
                        error: KError::InvalidArg,
                    };
                }
                let dest =
                    match self.lookup_cap(tcb, dst_tcb_slot, Some(ObjType::Tcb), Rights::WRITE) {
                        Ok(cap) => cap,
                        Err(error) => return UserAction::Return { value: 0, error },
                    };
                let rights = Rights((a3 as u8) & Rights::ALL.0);
                match self.copy_cap(tcb, src_slot, dest.obj, dst_slot, rights, 0) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbConfigure as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let vspace = match self.lookup_cap(
                    tcb,
                    slot_arg(a3).unwrap_or(0),
                    Some(ObjType::VSpace),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.configure_tcb(cap.obj, a1, a2, vspace.obj, a4 as u8) {
                    Ok(()) => UserAction::ConfigureTcb {
                        tcb: cap.obj,
                        entry: a1,
                        stack: a2,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbResume as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.resume_tcb(cap.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::CapRevoke as u64 => {
                let slot = slot_arg(a0).unwrap_or(0);
                match self.revoke_cap(tcb, slot) {
                    Ok(count) => UserAction::Return {
                        value: count as u64,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::PageTableMap as u64 => {
                let table = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::PageTable),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let space = match self.lookup_cap(
                    tcb,
                    slot_arg(a1).unwrap_or(0),
                    Some(ObjType::VSpace),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.map_page_table(table.obj, space.obj, a2) {
                    Ok(level) => UserAction::Return {
                        value: level as u64,
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
                match self.map_frame(space.obj, frame.obj, a2, rights) {
                    Ok(mapping) => UserAction::Return {
                        value: mapping.va,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
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
                match self.unmap_frame(space.obj, a1) {
                    Ok(_) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Open as u64 => {
                let len = (a1 as usize).min(crate::fs::MAX_PATH);
                if self.validate_user_range(tcb, a0, len, false).is_err() {
                    return UserAction::Return {
                        value: 0,
                        error: KError::InvalidArg,
                    };
                }
                UserAction::Open {
                    ptr: a0,
                    len: len as u64,
                    flags: a2,
                }
            }
            n if n == SyscallId::Read as u64 => {
                let len = (a2 as usize).min(USER_IO_MAX);
                if len != 0 && self.validate_user_range(tcb, a1, len, true).is_err() {
                    return UserAction::Return {
                        value: 0,
                        error: KError::InvalidArg,
                    };
                }
                UserAction::Read {
                    fd: a0,
                    ptr: a1,
                    len: len as u64,
                }
            }
            n if n == SyscallId::Write as u64 => {
                let len = (a2 as usize).min(USER_IO_MAX);
                if len != 0 && self.validate_user_range(tcb, a1, len, false).is_err() {
                    return UserAction::Return {
                        value: 0,
                        error: KError::InvalidArg,
                    };
                }
                UserAction::Write {
                    fd: a0,
                    ptr: a1,
                    len: len as u64,
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestArch;

    impl crate::console::Console for TestArch {
        const NAME: &'static str = "test";

        fn write_byte(byte: u8) {
            use std::io::Write;
            let _ = std::io::stdout().write_all(&[byte]);
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
        assert!(kernel.delete_cap(root, 1).is_ok());
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
    fn user_pointer_validation_rejects_kernel_and_unmapped_ranges() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(
                0x1000,
                0x4000_a000,
                4096,
                0x4000_0000,
                4096,
                0x4000_1000,
                0x8000,
            )
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();

        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::DebugWrite as u64,
                0x8000_0000,
                8,
                0,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Open as u64, 0x8000_0000, 8, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Read as u64, 0, 0x8000_0000, 8, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Write as u64, 0, 0x8000_0000, 8, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::DebugWrite as u64,
                0x4000_0000,
                8,
                0,
                0,
                0,
                0
            ),
            UserAction::Debug {
                ptr: 0x4000_0000,
                len: 8
            }
        );
    }

    #[test]
    fn untyped_accounts_object_metadata() {
        let mut kernel = Kernel::new();
        let untyped = kernel.create_untyped(0x100_0000, 8192).unwrap();
        assert!(matches!(kernel.retype(untyped, ObjType::Endpoint), Ok(_)));
        assert!(matches!(kernel.retype(untyped, ObjType::Tcb), Ok(_)));
        assert_eq!(
            kernel.retype(untyped, ObjType::Tcb),
            Err(KError::OutOfMemory)
        );
    }

    #[test]
    fn revoke_untyped_refuses_to_destroy_the_current_tcb() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x500_0000, 4096, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Tcb as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        let child = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        kernel.enqueue(child);
        assert_eq!(kernel.pick(), Some(child));
        // The child TCB was created through root's cap; root cannot revoke
        // that group while the child itself is the current thread.
        assert_eq!(kernel.revoke_cap(root, 1), Err(KError::IllegalOperation));
        assert!(kernel.get(child).is_some());
    }

    #[test]
    fn cap_copy_preserves_untyped_parent_chain() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let worker = kernel.create_tcb("worker", 1).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x400_0000, 4096, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Endpoint as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        assert!(kernel.copy_cap(root, 2, worker, 1, Rights::ALL, 0).is_ok());
        assert!(kernel
            .lookup_cap(worker, 1, Some(ObjType::Endpoint), Rights::READ)
            .is_ok());
        assert_eq!(kernel.revoke_cap(root, 1), Ok(2));
        assert!(kernel.lookup_cap(worker, 1, None, Rights::NONE).is_err());
    }

    #[test]
    fn revoke_untyped_frees_frame_and_removes_old_mapping() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 4096, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Frame as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        let frame = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        let old_pa = kernel.frame(frame).unwrap().pa;
        let space = kernel
            .create_user_vspace(
                0x1000,
                0x4000_a000,
                4096,
                0x4000_0000,
                4096,
                0x4000_1000,
                0x8000,
            )
            .unwrap();
        let space2 = kernel
            .create_vspace_with_page_size(0x2000, 0x5000_a000, 0x1000, 0x1000)
            .unwrap();
        let mapping = kernel
            .map_frame(space, frame, 0, Rights::READ | Rights::WRITE)
            .unwrap();
        let mapping2 = kernel
            .map_frame(space2, frame, 0, Rights::READ | Rights::WRITE)
            .unwrap();

        assert_eq!(kernel.revoke_cap(root, 1), Ok(1));
        assert!(kernel.get(frame).is_none());
        assert!(kernel.mapping_at(space, mapping.va).is_none());
        assert!(kernel.mapping_at(space2, mapping2.va).is_none());

        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Frame as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        let replacement = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        assert_eq!(kernel.frame(replacement).unwrap().pa, old_pa);
        assert!(kernel.mapping_at(space, mapping.va).is_none());
    }

    #[test]
    fn revoke_untyped_detaches_a_blocked_tcb() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x200_0000, 4096, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Tcb as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        let child = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(child, endpoint, 4), Ok(None));
        assert!(kernel.endpoint(endpoint).unwrap().recv.contains(child));

        assert_eq!(kernel.revoke_cap(root, 1), Ok(1));
        assert!(kernel.get(child).is_none());
        assert!(kernel.endpoint(endpoint).unwrap().recv.is_empty());
    }

    #[test]
    fn user_range_rejects_overflow_and_cross_page_write() {
        let mut kernel = Kernel::new();
        let space = kernel
            .create_vspace_with_page_size(0x1000, 0x4000_a000, 0x2000, 0x1000)
            .unwrap();
        let first = kernel.create_frame(0x9000, 4096).unwrap();
        let second = kernel.create_frame(0xa000, 4096).unwrap();
        kernel
            .map_frame(space, first, 0, Rights::READ | Rights::WRITE)
            .unwrap();
        kernel.map_frame(space, second, 0, Rights::READ).unwrap();
        assert!(space_validate(space, &kernel, 0x4000_a000, 0x1800, false));
        assert!(!space_validate(space, &kernel, 0x4000_a000, 0x1800, true));
        assert!(!space_validate(space, &kernel, u64::MAX - 1, 4, false));
    }

    fn space_validate(space: ObjRef, kernel: &Kernel, va: u64, len: usize, write: bool) -> bool {
        kernel.vspace(space).unwrap().validate_range(va, len, write)
    }

    struct ZeroOps;

    static ZERO_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static ZERO_OPS: ZeroOps = ZeroOps;

    impl PageTableOps for ZeroOps {
        fn zero_frame(&self, _pa: u64, _size: usize) {
            ZERO_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn tcb_pool_limit_is_enforced() {
        let mut kernel = Kernel::new();
        while kernel.tcb_count() < MAX_TCBS {
            assert!(kernel.create_tcb("extra", 1).is_some());
        }
        assert!(kernel.create_tcb("overflow", 1).is_none());
    }

    #[test]
    fn retype_zeroes_frame_and_page_table_pages() {
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&ZERO_OPS);
        let untyped = kernel
            .create_untyped_with_limit(0x600_0000, 8192, 4096, 4)
            .unwrap();
        ZERO_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        assert!(kernel.retype(untyped, ObjType::Frame).is_ok());
        assert!(kernel.retype(untyped, ObjType::PageTable).is_ok());
        assert_eq!(ZERO_CALLS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    struct FakeTableOps;

    static FAKE_MEM: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
    static FAKE_FLUSHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static FAKE_OPS: FakeTableOps = FakeTableOps;

    const F_ROOT: u64 = 0x0000;
    const F_T1: u64 = 0x1000;
    const F_T2: u64 = 0x2000;
    const F_T3: u64 = 0x3000;
    const F_ROOT_B: u64 = 0x4000;

    impl PageTableOps for FakeTableOps {
        fn levels(&self) -> usize {
            4
        }
        fn index(&self, va: u64, level: usize) -> usize {
            ((va >> (12 + 9 * (3 - level))) & 0x1ff) as usize
        }
        fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
            let base = ((table_pa >> 12) as usize) * 512;
            FAKE_MEM
                .lock()
                .unwrap()
                .get(base + idx)
                .copied()
                .unwrap_or(0)
        }
        fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
            let base = ((table_pa >> 12) as usize) * 512;
            if let Some(slot) = FAKE_MEM.lock().unwrap().get_mut(base + idx) {
                *slot = pte;
            }
        }
        fn encode_leaf(&self, pa: u64, _rights: Rights) -> u64 {
            (pa & !0xfff) | 0b10
        }
        fn encode_table(&self, pa: u64) -> u64 {
            (pa & !0xfff) | 0b01
        }
        fn decode(&self, pte: u64) -> Pte {
            if pte == 0 {
                Pte::Invalid
            } else if pte & 0b10 != 0 {
                Pte::Leaf(pte & !0xfff, Rights::READ | Rights::WRITE)
            } else {
                Pte::Table(pte & !0xfff)
            }
        }
        fn user_va_range(&self) -> (u64, u64) {
            (0x1000, 0x3000)
        }
        fn flush_va(&self, _va: u64) {
            FAKE_FLUSHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn reset_fake() {
        let mut mem = FAKE_MEM.lock().unwrap();
        mem.clear();
        mem.resize(8192, 0);
        FAKE_FLUSHES.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    fn fake_chain(root: u64, t1: u64, t2: u64, t3: u64) {
        let mut mem = FAKE_MEM.lock().unwrap();
        mem[((root >> 12) as usize) * 512] = (t1 & !0xfff) | 0b01;
        mem[((t1 >> 12) as usize) * 512] = (t2 & !0xfff) | 0b01;
        mem[((t2 >> 12) as usize) * 512] = (t3 & !0xfff) | 0b01;
    }

    fn fake_snapshot() -> Vec<u64> {
        FAKE_MEM.lock().unwrap().clone()
    }

    static WALK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    static MAP_USER_START: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0x4000_0000);
    static MAP_USER_END: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0x8000_0000);
    static MAP_OPS: MapOps = MapOps;

    struct MapOps;

    impl PageTableOps for MapOps {
        fn levels(&self) -> usize {
            4
        }
        fn index(&self, va: u64, level: usize) -> usize {
            ((va >> (12 + 9 * (3 - level))) & 0x1ff) as usize
        }
        fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
            let base = ((table_pa >> 12) as usize) * 512;
            FAKE_MEM
                .lock()
                .unwrap()
                .get(base + idx)
                .copied()
                .unwrap_or(0)
        }
        fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
            let base = ((table_pa >> 12) as usize) * 512;
            if let Some(slot) = FAKE_MEM.lock().unwrap().get_mut(base + idx) {
                *slot = pte;
            }
        }
        fn encode_leaf(&self, pa: u64, _rights: Rights) -> u64 {
            (pa & !0xfff) | 0b10
        }
        fn encode_table(&self, pa: u64) -> u64 {
            (pa & !0xfff) | 0b01
        }
        fn decode(&self, pte: u64) -> Pte {
            if pte == 0 {
                Pte::Invalid
            } else if pte & 0b10 != 0 {
                Pte::Leaf(pte & !0xfff, Rights::READ | Rights::WRITE)
            } else {
                Pte::Table(pte & !0xfff)
            }
        }
        fn decode_at(&self, pte: u64, _level: usize) -> Pte {
            self.decode(pte)
        }
        fn user_va_range(&self) -> (u64, u64) {
            (
                MAP_USER_START.load(std::sync::atomic::Ordering::SeqCst),
                MAP_USER_END.load(std::sync::atomic::Ordering::SeqCst),
            )
        }
        fn vspace_pages(&self) -> usize {
            2
        }
        fn kernel_accessible_range(&self) -> (u64, u64) {
            (0x1000, 0x10000)
        }
        fn install_kernel(&self, root_pa: u64, aux_pa: u64) {
            let root_base = ((root_pa >> 12) as usize) * 512;
            let aux_base = ((aux_pa >> 12) as usize) * 512;
            let mut mem = FAKE_MEM.lock().unwrap();
            if let Some(slot) = mem.get_mut(root_base) {
                *slot = (aux_pa & !0xfff) | 0b01;
            }
            if let Some(slot) = mem.get_mut(aux_base) {
                *slot = 0x9001;
            }
        }
        fn flush_va(&self, _va: u64) {
            FAKE_FLUSHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn zero_frame(&self, pa: u64, size: usize) {
            let base = ((pa >> 12) as usize) * 512;
            let words = (size + 7) / 8;
            if let Some(slice) = FAKE_MEM.lock().unwrap().get_mut(base..base + words) {
                slice.fill(0);
            }
        }
    }

    fn set_map_range(start: u64, end: u64) {
        MAP_USER_START.store(start, std::sync::atomic::Ordering::SeqCst);
        MAP_USER_END.store(end, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn page_table_map_assigns_levels_and_maps_through_them() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let pt1 = kernel.create_page_table(0x1000, 0).unwrap();
        let pt2 = kernel.create_page_table(0x2000, 0).unwrap();
        let pt3 = kernel.create_page_table(0x3000, 0).unwrap();
        let frame = kernel.create_frame(0x4000, 4096).unwrap();
        let va = 0x4000_1000;

        assert_eq!(kernel.map_page_table(pt1, space, va), Ok(1));
        assert_eq!(kernel.map_page_table(pt2, space, va), Ok(2));
        assert_eq!(kernel.map_page_table(pt3, space, va), Ok(3));
        let table = kernel.page_table(pt1).unwrap();
        assert_eq!(
            table.mapped,
            Some(PageTableMapping {
                vspace: space,
                va_base: 0x4000_0000,
                level: 1
            })
        );
        assert_eq!(kernel.page_table(pt3).unwrap().mapped.unwrap().va_base, va);

        let mapping = kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(mapping.va, va);
        assert_eq!(FAKE_MEM.lock().unwrap()[0], 0x1001);
        assert_eq!(FAKE_MEM.lock().unwrap()[512 + 1], 0x2001);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0x3001);
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4002);

        let before = fake_snapshot();
        assert_eq!(
            kernel.map_page_table(pt1, space, va),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(fake_snapshot(), before);
    }

    #[test]
    fn page_table_map_syscall_uses_capabilities() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let table = kernel.create_page_table(0x1000, 0).unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(table, ObjType::PageTable, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(space, ObjType::VSpace, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::PageTableMap as u64,
                1,
                2,
                0x4000_1000,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 1,
                error: KError::Ok
            }
        );
        assert_eq!(FAKE_MEM.lock().unwrap()[0], 0x1001);
    }

    #[test]
    fn page_table_map_checks_span_and_keeps_shared_entry_zero() {
        let _guard = WALK_LOCK.lock().unwrap();

        // A level-1 table spans 1 GiB.  The user range is only one page, so
        // the whole-span preflight must fail before any write.
        reset_fake();
        set_map_range(0x4000_0000, 0x4000_2000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x2000, 4096)
            .unwrap();
        let table = kernel.create_page_table(0x1000, 0).unwrap();
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_page_table(table, space, 0x4000_1000),
            Err(KError::InvalidArg)
        );
        assert_eq!(fake_snapshot(), before);

        // A valid root entry (the shared kernel PML4/L0 entry on x86/AArch64)
        // must never be replaced.  The new table is installed one level below.
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let table = kernel.create_page_table(0x2000, 0).unwrap();
        FAKE_MEM.lock().unwrap()[0] = 0x5001; // root entry 0 -> shared table
        assert_eq!(kernel.map_page_table(table, space, 0x4000_1000), Ok(2));
        assert_eq!(FAKE_MEM.lock().unwrap()[0], 0x5001);
        assert_eq!(FAKE_MEM.lock().unwrap()[2560 + 1], 0x2001);
    }

    #[test]
    fn revoke_untyped_unmaps_frames_before_detaching_tables() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x4000, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let creator = Some(SlotAddr { tcb: root, slot: 1 });
        let pt1 = kernel
            .retype_as(untyped, ObjType::PageTable, creator)
            .unwrap();
        let pt2 = kernel
            .retype_as(untyped, ObjType::PageTable, creator)
            .unwrap();
        let pt3 = kernel
            .retype_as(untyped, ObjType::PageTable, creator)
            .unwrap();
        let frame = kernel.retype_as(untyped, ObjType::Frame, creator).unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let va = 0x4000_1000;
        kernel.map_page_table(pt1, space, va).unwrap();
        kernel.map_page_table(pt2, space, va).unwrap();
        kernel.map_page_table(pt3, space, va).unwrap();
        kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4002);

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.page_table(pt1).is_none());
        assert!(kernel.page_table(pt2).is_none());
        assert!(kernel.page_table(pt3).is_none());
        assert!(kernel.frame(frame).is_none());
        assert_eq!(FAKE_MEM.lock().unwrap()[0], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[512 + 1], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0);
    }

    #[test]
    fn walk_rejects_va_outside_user_range_without_writing() {
        let _guard = WALK_LOCK.lock().unwrap();
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&FAKE_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0, 0x10000, 4096)
            .unwrap();
        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        let before = fake_snapshot();
        for va in [0, 0x8000, 0x3000] {
            assert_eq!(
                kernel.map_frame(space, frame, va, Rights::READ | Rights::WRITE),
                Err(KError::InvalidArg)
            );
            assert_eq!(fake_snapshot(), before);
        }
    }

    #[test]
    fn walk_missing_table_does_not_write() {
        let _guard = WALK_LOCK.lock().unwrap();
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&FAKE_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0, 0x10000, 4096)
            .unwrap();
        reset_fake();
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::MissingPageTable)
        );
        assert_eq!(fake_snapshot(), before);
    }

    #[test]
    fn walk_stops_at_high_level_leaf_and_keeps_valid_leaf() {
        let _guard = WALK_LOCK.lock().unwrap();
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&FAKE_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0, 0x10000, 4096)
            .unwrap();

        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        FAKE_MEM.lock().unwrap()[512] = 0x8000 | 0b10;
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(fake_snapshot(), before);

        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        FAKE_MEM.lock().unwrap()[1536 + 1] = 0x9000 | 0b10;
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(fake_snapshot(), before);
    }

    #[test]
    fn generic_walk_safety_rules() {
        let _guard = WALK_LOCK.lock().unwrap();
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&FAKE_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0, 0x10000, 4096)
            .unwrap();

        // Missing intermediate table -> MissingPageTable and no writes.
        reset_fake();
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::MissingPageTable)
        );
        assert_eq!(fake_snapshot(), before);

        // VA outside user_va_range -> InvalidArg and no writes.
        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x8000, Rights::READ | Rights::WRITE),
            Err(KError::InvalidArg)
        );
        assert_eq!(fake_snapshot(), before);

        // Higher-level leaf is not interpreted as a table.
        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        FAKE_MEM.lock().unwrap()[512] = 0x8000 | 0b10; // leaf at level 1
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(fake_snapshot(), before);

        // Existing valid leaf is not overwritten.
        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        FAKE_MEM.lock().unwrap()[1536 + 1] = 0x9000 | 0b10;
        let before = fake_snapshot();
        assert_eq!(
            kernel.map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(fake_snapshot(), before);

        // Valid map -> unmap -> map again, with flushes.
        reset_fake();
        fake_chain(F_ROOT, F_T1, F_T2, F_T3);
        let mapping = kernel
            .map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(mapping.va, 0x1000);
        assert!(FAKE_FLUSHES.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        kernel.unmap_frame(space, 0x1000).unwrap();
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0);
        kernel
            .map_frame(space, frame, 0x1000, Rights::READ | Rights::WRITE)
            .unwrap();

        // VSpace B does not observe A's mapping.
        let space_b = kernel
            .create_vspace_with_page_size(F_ROOT_B, 0, 0x10000, 4096)
            .unwrap();
        assert!(kernel.mapping_at(space_b, 0x1000).is_none());
    }

    #[test]
    fn revoke_copy_does_not_destroy_objects_of_another_cap() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        let root_frame = kernel
            .retype_as(
                untyped,
                ObjType::Frame,
                Some(SlotAddr { tcb: root, slot: 1 }),
            )
            .unwrap();
        let client_frame = kernel
            .retype_as(
                untyped,
                ObjType::Frame,
                Some(SlotAddr {
                    tcb: client,
                    slot: 1,
                }),
            )
            .unwrap();

        // Client revokes its copy: only its own frame dies, root's frame and
        // the shared Untyped bump/accounting survive.
        assert_eq!(kernel.revoke_cap(client, 1), Ok(0));
        assert!(kernel.get(client_frame).is_none());
        assert!(kernel.get(root_frame).is_some());
        assert_eq!(kernel.untyped(untyped).unwrap().objects, 1);
        assert!(kernel.untyped(untyped).unwrap().bump > 0);
        assert!(kernel.lookup_cap(root, 1, None, Rights::NONE).is_ok());

        // Root revokes the original cap: its frame is collected and only now
        // does the empty Untyped reset its bump.
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(root_frame).is_none());
        assert_eq!(kernel.untyped(untyped).unwrap().objects, 0);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 0);
    }

    #[test]
    fn root_revoke_collects_objects_created_via_child_cap() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        let root_frame = kernel
            .retype_as(
                untyped,
                ObjType::Frame,
                Some(SlotAddr { tcb: root, slot: 1 }),
            )
            .unwrap();
        let client_frame = kernel
            .retype_as(
                untyped,
                ObjType::Frame,
                Some(SlotAddr {
                    tcb: client,
                    slot: 1,
                }),
            )
            .unwrap();

        // Revoke the ancestor first: the child cap is still present, and its
        // object must be collected together with the root's own object.
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(root_frame).is_none());
        assert!(kernel.get(client_frame).is_none());
        assert!(kernel.lookup_cap(client, 1, None, Rights::NONE).is_err());
        assert_eq!(kernel.untyped(untyped).unwrap().objects, 0);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 0);
    }

    #[test]
    fn delete_untyped_cap_is_refused_while_objects_or_children_exist() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let frame = kernel
            .retype_as(
                untyped,
                ObjType::Frame,
                Some(SlotAddr { tcb: root, slot: 1 }),
            )
            .unwrap();
        assert_eq!(kernel.delete_cap(root, 1), Err(KError::IllegalOperation));
        assert!(kernel.get(frame).is_some());
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        assert_eq!(kernel.delete_cap(root, 1), Err(KError::IllegalOperation));

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(frame).is_none());
        // After revoke there are neither objects nor child caps.
        assert!(kernel.delete_cap(root, 1).is_ok());
    }

    #[test]
    fn reparented_client_cap_cannot_revoke_surviving_object() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let server = kernel.create_tcb("server", 1).unwrap();
        let data_untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        let meta_untyped = kernel
            .create_untyped_with_limit(0x200_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(data_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(meta_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        let client = kernel
            .retype_as(
                meta_untyped,
                ObjType::Tcb,
                Some(SlotAddr { tcb: root, slot: 2 }),
            )
            .unwrap();
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        // Server gets a child of the client's cap.
        kernel
            .copy_cap(client, 1, server, 1, Rights::ALL, 0)
            .unwrap();
        let frame = kernel
            .retype_as(
                data_untyped,
                ObjType::Frame,
                Some(SlotAddr {
                    tcb: client,
                    slot: 1,
                }),
            )
            .unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let mapping = kernel
            .map_frame(space, frame, 0x4000_1000, Rights::READ | Rights::WRITE)
            .unwrap();
        assert!(kernel.mapping_at(space, mapping.va).is_some());

        // Destroy the client TCB; C's cap disappears.  The frame creator and
        // server's child cap must be reparented to the root cap.
        kernel.revoke_cap(root, 2).unwrap();
        assert!(kernel.get(client).is_none());
        assert!(kernel.get(frame).is_some());
        // Server's copy of the data Untyped must not collect the object.
        kernel.revoke_cap(server, 1).unwrap();
        assert!(kernel.get(frame).is_some());
        assert!(kernel.mapping_at(space, mapping.va).is_some());

        // The root cap is the surviving ancestor and collects it.
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(frame).is_none());
        assert!(kernel.mapping_at(space, mapping.va).is_none());
        assert!(kernel.lookup_cap(server, 1, None, Rights::NONE).is_err());
    }

    #[test]
    fn cap_ids_are_not_reused_after_delete() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let first = kernel.create_endpoint().unwrap();
        let second = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(first, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        let first_id = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().id;
        assert!(kernel.delete_cap(root, 1).is_ok());
        kernel
            .install_cap(root, 1, Cap::new(second, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        let second_id = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().id;
        assert_ne!(first_id, second_id);
        assert!(second_id > first_id);
    }

    #[test]
    fn revoke_owner_collects_reparented_creator_objects() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let data_untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        let meta_untyped = kernel
            .create_untyped_with_limit(0x200_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(data_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(meta_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        let client = kernel
            .retype_as(
                meta_untyped,
                ObjType::Tcb,
                Some(SlotAddr { tcb: root, slot: 2 }),
            )
            .unwrap();
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        let client_frame = kernel
            .retype_as(
                data_untyped,
                ObjType::Frame,
                Some(SlotAddr {
                    tcb: client,
                    slot: 1,
                }),
            )
            .unwrap();

        // Destroying the client TCB also removes its cspace (and therefore
        // the only cap that created the frame from data_untyped).
        kernel.revoke_cap(root, 2).unwrap();
        assert!(kernel.get(client).is_none());
        assert!(kernel.get(client_frame).is_some());

        // Revoking the owner Untyped must still collect the orphaned frame.
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(client_frame).is_none());
        assert_eq!(kernel.untyped(data_untyped).unwrap().objects, 0);
        assert_eq!(kernel.untyped(data_untyped).unwrap().bump, 0);
    }

    #[test]
    fn revoking_page_table_clears_reverse_map_in_another_untyped() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let space = kernel
            .create_vspace_with_page_size(0, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();
        let pt_untyped = kernel
            .create_untyped_with_limit(0x1000, 0x3000, 4096, 4)
            .unwrap();
        let frame_untyped = kernel
            .create_untyped_with_limit(0x4000, 0x1000, 4096, 2)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(pt_untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(frame_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        let pt_creator = Some(SlotAddr { tcb: root, slot: 1 });
        let frame_creator = Some(SlotAddr { tcb: root, slot: 2 });
        let pt1 = kernel
            .retype_as(pt_untyped, ObjType::PageTable, pt_creator)
            .unwrap();
        let pt2 = kernel
            .retype_as(pt_untyped, ObjType::PageTable, pt_creator)
            .unwrap();
        let pt3 = kernel
            .retype_as(pt_untyped, ObjType::PageTable, pt_creator)
            .unwrap();
        let frame = kernel
            .retype_as(frame_untyped, ObjType::Frame, frame_creator)
            .unwrap();
        let va = 0x4000_1000;
        kernel.map_page_table(pt1, space, va).unwrap();
        kernel.map_page_table(pt2, space, va).unwrap();
        kernel.map_page_table(pt3, space, va).unwrap();
        kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert!(kernel.validate_user_range(root, va, 4, false).is_ok());
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4002);

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.frame(frame).is_some());
        assert!(kernel.mapping_at(space, va).is_none());
        assert_eq!(
            kernel.validate_user_range(root, va, 4, false),
            Err(KError::InvalidArg)
        );
        assert_eq!(FAKE_MEM.lock().unwrap()[0], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[512 + 1], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0);
    }

    #[test]
    fn revoking_vspace_detaches_tables_from_another_untyped() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let vs_untyped = kernel
            .create_untyped_with_limit(0x8000, 0x2000, 4096, 2)
            .unwrap();
        let pt_untyped = kernel
            .create_untyped_with_limit(0x1000, 0x4000, 4096, 5)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(vs_untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(pt_untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let space_creator = Some(SlotAddr { tcb: root, slot: 1 });
        let space = kernel
            .retype_as(vs_untyped, ObjType::VSpace, space_creator)
            .unwrap();
        let pt_creator = Some(SlotAddr { tcb: root, slot: 2 });
        let pt1 = kernel
            .retype_as(pt_untyped, ObjType::PageTable, pt_creator)
            .unwrap();
        let pt2 = kernel
            .retype_as(pt_untyped, ObjType::PageTable, pt_creator)
            .unwrap();
        let frame = kernel
            .retype_as(pt_untyped, ObjType::Frame, pt_creator)
            .unwrap();
        let va = 0x10_0000_1000;
        assert_eq!(kernel.map_page_table(pt1, space, va), Ok(2));
        assert_eq!(kernel.map_page_table(pt2, space, va), Ok(3));
        kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();

        // The loaded VSpace is protected just like the current TCB in revoke.
        kernel.current = Some(root);
        assert_eq!(kernel.revoke_cap(root, 1), Err(KError::IllegalOperation));
        kernel.current = None;

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.vspace(space).is_none());
        assert!(kernel.tcb(root).unwrap().vspace.is_none());
        assert!(kernel.page_table(pt1).unwrap().mapped.is_none());
        assert!(kernel.page_table(pt2).unwrap().mapped.is_none());
        assert_eq!(FAKE_MEM.lock().unwrap()[512], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0);

        // The surviving table object is zeroed and can be attached again to a
        // fresh root (the old VSpace root page is not ours to modify).
        let space2 = kernel
            .create_vspace_with_page_size(0x6000, 0x10_0000_0000, 0x10_0000_0000, 4096)
            .unwrap();
        assert_eq!(kernel.map_page_table(pt1, space2, va), Ok(1));
        assert_eq!(FAKE_MEM.lock().unwrap()[3072], 0x1001);
    }

    #[test]
    fn retype_vspace_allocates_private_root_and_aux() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x4000, 4096, 8)
            .unwrap();
        let space_a = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let space_b = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let a = kernel.vspace(space_a).unwrap();
        let b = kernel.vspace(space_b).unwrap();
        assert_eq!(a.root, 0x1000);
        assert_eq!(a.aux_pa, Some(0x2000));
        assert_eq!(b.root, 0x3000);
        assert_eq!(b.aux_pa, Some(0x4000));
        assert_eq!(a.base, 0x10_0000_0000);
        assert_eq!(a.end, 0x20_0000_0000);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 0x4000);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0x9001);
        assert_eq!(FAKE_MEM.lock().unwrap()[2048], 0x9001);

        // The two roots share only the kernel slot; user PageTableMap goes to
        // A's private aux table and is invisible from B.
        let pt = kernel.create_page_table(0x5000, 0).unwrap();
        let va = 0x10_0000_1000;
        assert_eq!(kernel.map_page_table(pt, space_a, va), Ok(2));
        assert_eq!(FAKE_MEM.lock().unwrap()[1024 + 64], 0x5001);
        assert_eq!(FAKE_MEM.lock().unwrap()[2048 + 64], 0);
        // Kernel slots in the private root/aux table stay untouched.
        assert_eq!(FAKE_MEM.lock().unwrap()[512], 0x2001);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0x9001);
        assert_eq!(FAKE_MEM.lock().unwrap()[2048], 0x9001);
        assert!(kernel.mapping_at(space_b, va).is_none());

        let one_page = kernel
            .create_untyped_with_limit(0x8000, 0x1000, 4096, 2)
            .unwrap();
        assert_eq!(
            kernel.retype(one_page, ObjType::VSpace),
            Err(KError::OutOfMemory)
        );
        assert_eq!(kernel.untyped(one_page).unwrap().bump, 0);
        assert!(kernel
            .create_untyped_with_limit(0x1_0000, 0x1000, 4096, 2)
            .is_none());
    }

    #[test]
    fn untyped_object_limit_is_enforced() {
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x300_0000, 8192, 4096, 2)
            .unwrap();
        assert!(kernel.retype(untyped, ObjType::Endpoint).is_ok());
        assert!(kernel.retype(untyped, ObjType::Notification).is_ok());
        assert_eq!(
            kernel.retype(untyped, ObjType::Tcb),
            Err(KError::OutOfMemory)
        );
    }

    #[test]
    fn resume_rejects_unconfigured_tcb() {
        let mut kernel = Kernel::new();
        let child = kernel.create_tcb("child", 1).unwrap();
        assert_eq!(kernel.resume_tcb(child), Err(KError::IllegalOperation));
    }

    #[test]
    fn revoking_untyped_resets_accounting_and_children() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel.create_untyped(0x200_0000, 4096).unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(untyped, ObjType::Untyped, Rights::WRITE | Rights::GRANT),
            )
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Endpoint as u64,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.revoke_cap(root, 1), Ok(1));
        assert!(kernel.lookup_cap(root, 2, None, Rights::NONE).is_err());
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 0);
    }

    #[test]
    fn tcb_configuration_validates_layout_and_resumes() {
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(
                0x1000,
                0x4000_a000,
                4096,
                0x4000_0000,
                4096,
                0x4000_1000,
                0x8000,
            )
            .unwrap();
        let child = kernel.create_tcb("child", 1).unwrap();
        assert_eq!(
            kernel.configure_tcb(child, 0x4000_0000, 0x4000_5000, space, 5),
            Ok(())
        );
        assert_eq!(kernel.resume_tcb(child), Ok(()));
        assert!(kernel.tcb(child).is_some_and(|tcb| tcb.queued));
        assert_eq!(kernel.resume_tcb(child), Ok(()));
        assert_eq!(
            kernel.configure_tcb(child, 0x9000_0000, 0x4000_5000, space, 5),
            Err(KError::InvalidArg)
        );
        assert_eq!(
            kernel.configure_tcb(child, 0x4000_0000, 0x9000_0000, space, 5),
            Err(KError::InvalidArg)
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
        assert!(kernel.endpoint(endpoint).is_some());
        assert!(kernel.lookup_cap(root, 1, None, Rights::NONE).is_ok());
        assert!(kernel.lookup_cap(root, 2, None, Rights::NONE).is_err());
        assert!(kernel.lookup_cap(root, 3, None, Rights::NONE).is_err());
    }
}
