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
    /// Copy a user buffer into a Frame at a byte offset.
    FrameWrite {
        frame: ObjRef,
        offset: u64,
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
    /// Copy bytes into a physical frame through the platform identity map.
    fn copy_to_frame(&self, _pa: u64, _bytes: &[u8]) {}
    /// Copy bytes at a byte offset inside a physical frame.
    fn copy_to_frame_at(&self, pa: u64, offset: usize, bytes: &[u8]) {
        self.copy_to_frame(pa + offset as u64, bytes);
    }
    /// True when a leaf PTE has the architecture user bit.
    fn leaf_is_user(&self, _pte: u64) -> bool {
        false
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
    /// Make recently written instruction bytes visible to the local
    /// instruction fetch path before an executable mapping becomes live.
    fn sync_icache(&self, _pa: u64, _size: usize) {}
    /// Copy a range between two physical frames (IPC buffer transfer).
    fn copy_frame_to_frame(&self, _src: u64, _dst: u64, _size: usize) {}
    fn zero_frame(&self, _pa: u64, _size: usize) {}
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KernelStats {
    pub ipc_sent: u64,
    pub ipc_received: u64,
    pub notifications: u64,
    pub context_switches: u64,
    pub root_switches: u64,
    pub ticks: u64,
}

/// Optional writable data segment for the reusable task-space builder.
#[derive(Clone, Copy, Debug)]
pub struct TaskData {
    pub va: u64,
    pub pages: usize,
}

/// User layout used by the reusable task-space builder.
///
/// `data: None` leaves the data VA unmapped.  The last-level page table is
/// still installed by the code/stack segments when they share the same 2 MiB
/// region, so a later `FrameMap` at `DATA_VA` only needs that table.
#[derive(Clone, Copy, Debug)]
pub struct TaskLayout {
    pub code_va: u64,
    pub stack_va: u64,
    pub stack_pages: usize,
    pub data: Option<TaskData>,
}

/// Stack placement for an ELF-loaded task.
#[derive(Clone, Copy, Debug)]
pub struct StackSpec {
    /// First mapped stack page.
    pub base: u64,
    pub pages: usize,
    /// Leave the page immediately below `base` unmapped.
    pub guard: bool,
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
                root_switches: 0,
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
            root_switches: 0,
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

    /// Root page-table PA currently configured for a TCB.
    pub fn current_root(&self, tcb: ObjRef) -> Option<u64> {
        let space = self.tcb(tcb)?.vspace?;
        Some(self.vspace(space)?.root)
    }

    pub fn note_root_switch(&mut self) {
        self.stats.root_switches = self.stats.root_switches.saturating_add(1);
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
            reserved: 0,
        }))
    }

    pub fn create_frame(&mut self, pa: usize, size: usize) -> Option<ObjRef> {
        self.alloc(Kind::Frame(Frame {
            pa,
            size,
            mapped_w: 0,
            mapped_x: 0,
        }))
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

    pub fn create_user_vspace(&mut self, root: u64, base: u64, page_size: u64) -> Option<ObjRef> {
        self.create_vspace_with_page_size(root, base, page_size, page_size)
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
        if self.vspace(vspace).is_none() {
            return Err(KError::WrongType);
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

    fn user_leaf_ok(ops: &dyn PageTableOps, root: u64, va: u64, need_write: bool) -> bool {
        let levels = ops.levels();
        if levels == 0 {
            return false;
        }
        let mut table = root;
        for level in 0..levels {
            let idx = ops.index(va, level);
            let pte = ops.read_pte(table, idx);
            match ops.decode_at(pte, level) {
                Pte::Table(next) if level + 1 < levels => table = next,
                Pte::Leaf(_, rights) => {
                    // User mappings are 4 KiB leaves only.  A block/superpage
                    // inside the user range is not acceptable.
                    return level + 1 == levels
                        && ops.leaf_is_user(pte)
                        && rights.contains(Rights::READ)
                        && (!need_write || rights.contains(Rights::WRITE));
                }
                _ => return false,
            }
        }
        false
    }

    /// Validate every page touched by a user range against the real page
    /// tables.  The reverse map is no longer part of pointer validation.
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
        let ops = self.page_table_ops.ok_or(KError::MissingPageTable)?;
        if len == 0 || space.page_size == 0 || !space.page_size.is_power_of_two() {
            return Err(KError::InvalidArg);
        }
        let end = va.checked_add(len as u64).ok_or(KError::InvalidArg)?;
        let mut cursor = va & !(space.page_size - 1);
        while cursor < end {
            if !Self::user_leaf_ok(ops, space.root, cursor, need_write) {
                return Err(KError::InvalidArg);
            }
            cursor = cursor
                .checked_add(space.page_size)
                .ok_or(KError::InvalidArg)?;
        }
        Ok(())
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

    /// Find an existing Frame object by physical address.  Used to share the
    /// root code page with the server VSpace without changing ownership.
    pub fn frame_for_pa(&self, pa: u64) -> Option<ObjRef> {
        for (index, object) in self.objs.iter().enumerate() {
            if let Kind::Frame(frame) = object.kind {
                if frame.pa as u64 == pa {
                    return Some(ObjRef {
                        index: index as u8,
                        gen: object.gen,
                    });
                }
            }
        }
        None
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

    /// `(size, bump, objects, max_objects)` for logging and boot checks.
    pub fn untyped_usage(&self, r: ObjRef) -> Option<(usize, usize, usize, usize)> {
        let region = self.untyped(r)?;
        Some((region.size, region.bump, region.objects, region.max_objects))
    }

    /// Look up usage for an Untyped by its physical base.  Runtime uses this
    /// to log the data arena at test-exit after root has allocated from it.
    pub fn untyped_usage_by_base(&self, base: usize) -> Option<(usize, usize, usize, usize)> {
        self.objs.iter().find_map(|object| match object.kind {
            Kind::Untyped(ref region) if region.base == base => {
                Some((region.size, region.bump, region.objects, region.max_objects))
            }
            _ => None,
        })
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
        let (base, size, bump, page, max_objects, objects, reserved) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (
                region.base,
                region.size,
                region.bump,
                region.page_size,
                region.max_objects,
                region.objects,
                region.reserved,
            )
        };
        if page == 0 || !page.is_power_of_two() {
            return Err(KError::Alignment);
        }
        if objects
            .checked_add(reserved)
            .and_then(|used| used.checked_add(1))
            .is_none_or(|used| used > max_objects)
        {
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
                Kind::Frame(Frame {
                    pa,
                    size: page,
                    mapped_w: 0,
                    mapped_x: 0,
                })
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

    /// Carve a child Untyped region from the parent's bump and reserve its
    /// object quota in the parent.  The child itself counts as one object of
    /// the parent; its `max_objects` is charged to `parent.reserved`.
    pub fn retype_untyped_child_as(
        &mut self,
        untyped: ObjRef,
        size: usize,
        child_max_objects: usize,
        creator: Option<SlotAddr>,
    ) -> KResult<ObjRef> {
        let creator = match creator {
            Some(addr) => {
                let cap = self.cap_at(addr).ok_or(KError::InvalidCap)?;
                if cap.obj != untyped || cap.ty != ObjType::Untyped {
                    return Err(KError::InvalidCap);
                }
                Some(CapRef { addr, id: cap.id })
            }
            None => None,
        };
        let (base, parent_size, bump, page, max_objects, objects, reserved) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (
                region.base,
                region.size,
                region.bump,
                region.page_size,
                region.max_objects,
                region.objects,
                region.reserved,
            )
        };
        if page == 0 || !page.is_power_of_two() {
            return Err(KError::Alignment);
        }
        if size == 0 || size % page != 0 {
            return Err(KError::InvalidArg);
        }
        let required = objects
            .checked_add(reserved)
            .and_then(|used| used.checked_add(child_max_objects))
            .and_then(|used| used.checked_add(1))
            .ok_or(KError::OutOfMemory)?;
        if required > max_objects {
            return Err(KError::OutOfMemory);
        }
        let parent_end = base.checked_add(parent_size).ok_or(KError::OutOfMemory)?;
        let unaligned = base.checked_add(bump).ok_or(KError::OutOfMemory)?;
        let child_base = (unaligned + page - 1) & !(page - 1);
        let child_end = child_base.checked_add(size).ok_or(KError::OutOfMemory)?;
        if child_end > parent_end {
            return Err(KError::OutOfMemory);
        }
        let consumed = child_end - base;
        let kind = Kind::Untyped(Untyped {
            base: child_base,
            size,
            bump: 0,
            page_size: page,
            max_objects: child_max_objects,
            objects: 0,
            reserved: 0,
        });
        let object = self
            .alloc_owned(kind, Some(untyped), creator)
            .ok_or(KError::OutOfMemory)?;
        let region = self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)?;
        region.bump = consumed;
        region.objects = objects + 1;
        region.reserved = reserved + child_max_objects;
        Ok(object)
    }

    fn missing_table_level(&self, vspace: ObjRef, va: u64) -> Option<u8> {
        let space = self.vspace(vspace)?;
        let ops = self.page_table_ops?;
        let levels = ops.levels();
        if levels < 2 {
            return None;
        }
        let mut table = space.root;
        for level in 0..levels - 1 {
            let idx = ops.index(va, level);
            match ops.decode_at(ops.read_pte(table, idx), level) {
                Pte::Invalid => return Some((level + 1) as u8),
                Pte::Table(next) => table = next,
                Pte::Leaf(_, _) => return None,
            }
        }
        None
    }

    fn rollback_task_space(
        &mut self,
        created: &[ObjRef],
        untyped: ObjRef,
        bump: usize,
        objects: usize,
    ) {
        for object in created.iter().rev() {
            if object.is_valid() {
                let _ = self.destroy_object(*object);
            }
        }
        if let Some(region) = self.untyped_mut(untyped) {
            region.bump = bump;
            region.objects = objects;
        }
    }

    fn ensure_task_tables(
        &mut self,
        untyped: ObjRef,
        vspace: ObjRef,
        va: u64,
        created: &mut [ObjRef; MAX_OBJECTS],
        count: &mut usize,
    ) -> KResult<()> {
        loop {
            let Some(level) = self.missing_table_level(vspace, va) else {
                return Ok(());
            };
            if *count >= created.len() {
                return Err(KError::OutOfMemory);
            }
            let table = self.retype_as(untyped, ObjType::PageTable, None)?;
            created[*count] = table;
            *count += 1;
            let attached = self.map_page_table(table, vspace, va)?;
            if attached != level {
                return Err(KError::IllegalOperation);
            }
        }
    }

    fn map_task_segment(
        &mut self,
        untyped: ObjRef,
        vspace: ObjRef,
        va: u64,
        pages: usize,
        rights: Rights,
        copy: Option<&[u8]>,
        created: &mut [ObjRef; MAX_OBJECTS],
        count: &mut usize,
    ) -> KResult<()> {
        let page = self.untyped(untyped).ok_or(KError::WrongType)?.page_size;
        for page_index in 0..pages {
            if *count >= created.len() {
                return Err(KError::OutOfMemory);
            }
            let frame = self.retype_as(untyped, ObjType::Frame, None)?;
            created[*count] = frame;
            *count += 1;
            if let Some(bytes) = copy {
                let start = page_index * page;
                if start < bytes.len() {
                    let end = (start + page).min(bytes.len());
                    let pa = self.frame(frame).ok_or(KError::WrongType)?.pa as u64;
                    self.page_table_ops
                        .ok_or(KError::MissingPageTable)?
                        .copy_to_frame(pa, &bytes[start..end]);
                }
            }
            let target = va
                .checked_add((page_index * page) as u64)
                .ok_or(KError::InvalidArg)?;
            self.map_frame(vspace, frame, target, rights)?;
        }
        Ok(())
    }

    /// Build a complete task VSpace from an Untyped arena.  This is the
    /// reusable host-testable path that the root task will later use through
    /// syscalls.  On error the created objects are destroyed and the arena
    /// snapshot is restored.
    pub fn build_task_space(
        &mut self,
        untyped: ObjRef,
        image: &[u8],
        layout: TaskLayout,
    ) -> KResult<ObjRef> {
        self.build_task_space_inner(untyped, Some(image), None, layout)
    }

    /// Build a VSpace whose one-page code segment is an already existing
    /// Frame.  This is the 4.5 server path: the server maps the same physical
    /// code page as the root task without retyping or copying it again.
    pub fn build_task_space_shared_code(
        &mut self,
        untyped: ObjRef,
        code_frame: ObjRef,
        layout: TaskLayout,
    ) -> KResult<ObjRef> {
        self.build_task_space_inner(untyped, None, Some(code_frame), layout)
    }

    fn build_task_space_inner(
        &mut self,
        untyped: ObjRef,
        image: Option<&[u8]>,
        shared_code: Option<ObjRef>,
        layout: TaskLayout,
    ) -> KResult<ObjRef> {
        let (page, bump, objects) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (region.page_size, region.bump, region.objects)
        };
        if page == 0 || !page.is_power_of_two() || self.page_table_ops.is_none() {
            return Err(KError::MissingPageTable);
        }
        let code_pages = match (image, shared_code) {
            (Some(image), None) => (image.len() + page - 1) / page,
            (None, Some(frame)) => {
                let frame = self.frame(frame).ok_or(KError::WrongType)?;
                if frame.size < page {
                    return Err(KError::Alignment);
                }
                1
            }
            _ => return Err(KError::InvalidArg),
        };
        let mut created = [ObjRef::invalid(); MAX_OBJECTS];
        let mut created_count = 0usize;
        let vspace = self.retype_as(untyped, ObjType::VSpace, None)?;
        if created_count < created.len() {
            created[created_count] = vspace;
            created_count += 1;
        } else {
            let _ = self.destroy_object(vspace);
            return Err(KError::OutOfMemory);
        }

        let step = |kernel: &mut Kernel,
                    created: &mut [ObjRef; MAX_OBJECTS],
                    count: &mut usize|
         -> KResult<()> {
            kernel.ensure_task_tables(untyped, vspace, layout.code_va, created, count)?;
            if let Some(code_frame) = shared_code {
                kernel.map_frame(
                    vspace,
                    code_frame,
                    layout.code_va,
                    Rights::READ | Rights::EXEC,
                )?;
            } else if let Some(image) = image {
                kernel.map_task_segment(
                    untyped,
                    vspace,
                    layout.code_va,
                    code_pages,
                    Rights::READ | Rights::EXEC,
                    Some(image),
                    created,
                    count,
                )?;
            }
            kernel.ensure_task_tables(untyped, vspace, layout.stack_va, created, count)?;
            kernel.map_task_segment(
                untyped,
                vspace,
                layout.stack_va,
                layout.stack_pages,
                Rights::READ | Rights::WRITE,
                None,
                created,
                count,
            )?;
            if let Some(data) = layout.data {
                if data.pages != 0 {
                    kernel.ensure_task_tables(untyped, vspace, data.va, created, count)?;
                    kernel.map_task_segment(
                        untyped,
                        vspace,
                        data.va,
                        data.pages,
                        Rights::READ | Rights::WRITE,
                        None,
                        created,
                        count,
                    )?;
                }
            }
            Ok(())
        };
        if let Err(error) = step(self, &mut created, &mut created_count) {
            self.rollback_task_space(&created[..created_count], untyped, bump, objects);
            return Err(error);
        }

        Ok(vspace)
    }

    /// Load an ELF image into a VSpace.  When `vspace` is `None` a new VSpace
    /// is retyped; when it is `Some` the segments are added to that existing
    /// space.  The stack is mapped separately, with an optional unmapped guard
    /// page below it.
    pub fn build_task_space_elf(
        &mut self,
        untyped: ObjRef,
        vspace: Option<ObjRef>,
        image: &vifast_elf::ElfImage,
        bytes: &[u8],
        stack: StackSpec,
    ) -> KResult<ObjRef> {
        let (page, bump, objects) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (region.page_size, region.bump, region.objects)
        };
        if page == 0 || !page.is_power_of_two() || self.page_table_ops.is_none() {
            return Err(KError::MissingPageTable);
        }
        if stack.pages != 0 && stack.base % page as u64 != 0 {
            return Err(KError::Alignment);
        }
        let mut created = [ObjRef::invalid(); MAX_OBJECTS];
        let mut created_count = 0usize;
        let space = match vspace {
            Some(space) => {
                if self.vspace(space).is_none() {
                    return Err(KError::WrongType);
                }
                space
            }
            None => {
                let space = self.retype_as(untyped, ObjType::VSpace, None)?;
                if created_count < created.len() {
                    created[created_count] = space;
                    created_count += 1;
                } else {
                    let _ = self.destroy_object(space);
                    return Err(KError::OutOfMemory);
                }
                space
            }
        };

        let step = |kernel: &mut Kernel,
                    created: &mut [ObjRef; MAX_OBJECTS],
                    count: &mut usize|
         -> KResult<()> {
            for segment in image.segments() {
                let file_end = segment
                    .file_offset
                    .checked_add(segment.file_size)
                    .ok_or(KError::InvalidArg)?;
                if file_end > bytes.len() {
                    return Err(KError::InvalidArg);
                }
                if segment.page_count() == 0 {
                    continue;
                }
                let start = segment.vaddr & !(page as u64 - 1);
                kernel.ensure_task_tables(untyped, space, start, created, count)?;
                let rights = if segment.writable() {
                    Rights::READ | Rights::WRITE
                } else if segment.executable() {
                    Rights::READ | Rights::EXEC
                } else {
                    Rights::READ
                };
                let mut copied = 0usize;
                for page_index in 0..segment.page_count() {
                    if *count >= created.len() {
                        return Err(KError::OutOfMemory);
                    }
                    let frame = kernel.retype_as(untyped, ObjType::Frame, None)?;
                    created[*count] = frame;
                    *count += 1;
                    let target = start
                        .checked_add((page_index * page) as u64)
                        .ok_or(KError::InvalidArg)?;
                    kernel.map_frame(space, frame, target, rights)?;

                    let frame_offset = if page_index == 0 {
                        (segment.vaddr - target) as usize
                    } else {
                        0
                    };
                    let remaining = segment.file_size.saturating_sub(copied);
                    let copy_len = remaining.min(page - frame_offset);
                    if copy_len != 0 {
                        let source = segment
                            .file_offset
                            .checked_add(copied)
                            .ok_or(KError::InvalidArg)?;
                        let slice = bytes
                            .get(source..source + copy_len)
                            .ok_or(KError::InvalidArg)?;
                        let frame_pa = kernel.frame(frame).ok_or(KError::WrongType)?.pa as u64;
                        kernel
                            .page_table_ops
                            .ok_or(KError::MissingPageTable)?
                            .copy_to_frame_at(frame_pa, frame_offset, slice);
                        copied += copy_len;
                    }
                }
            }

            if stack.pages != 0 {
                kernel.ensure_task_tables(untyped, space, stack.base, created, count)?;
                for page_index in 0..stack.pages {
                    if *count >= created.len() {
                        return Err(KError::OutOfMemory);
                    }
                    let frame = kernel.retype_as(untyped, ObjType::Frame, None)?;
                    created[*count] = frame;
                    *count += 1;
                    let target = stack
                        .base
                        .checked_add((page_index * page) as u64)
                        .ok_or(KError::InvalidArg)?;
                    kernel.map_frame(space, frame, target, Rights::READ | Rights::WRITE)?;
                }
            }
            let _ = stack.guard;
            Ok(())
        };

        if let Err(error) = step(self, &mut created, &mut created_count) {
            self.rollback_task_space(&created[..created_count], untyped, bump, objects);
            return Err(error);
        }
        Ok(space)
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

    fn set_frame_mapping_counts(&mut self, frame_obj: ObjRef, mapped_w: u16, mapped_x: u16) {
        if let Some(object) = self.get_mut(frame_obj) {
            if let Kind::Frame(ref mut frame) = object.kind {
                frame.mapped_w = mapped_w;
                frame.mapped_x = mapped_x;
            }
        }
    }

    fn note_frame_unmapping(&mut self, mapping: Mapping) {
        if let Some(object) = self.get_mut(mapping.frame) {
            if let Kind::Frame(ref mut frame) = object.kind {
                if mapping.rights.contains(Rights::WRITE) {
                    frame.mapped_w = frame.mapped_w.saturating_sub(1);
                }
                if mapping.rights.contains(Rights::EXEC) {
                    frame.mapped_x = frame.mapped_x.saturating_sub(1);
                }
            }
        }
        for object in self.objs.iter_mut() {
            if let Kind::Tcb(ref mut table) = object.kind {
                if table
                    .ipc_buffer
                    .is_some_and(|buffer| buffer.frame == mapping.frame && buffer.va == mapping.va)
                {
                    table.ipc_buffer = None;
                }
            }
        }
    }

    /// Validate and install a user mapping through the generic page-table
    /// walk.  No platform-specific map function is used.  Intermediate page
    /// tables must already exist; otherwise `MissingPageTable` is returned.
    ///
    /// Rights must contain READ.  W^X is checked both on this leaf and against
    /// the Frame's live `mapped_w`/`mapped_x`, so aliases cannot combine a
    /// writable mapping and an executable mapping of the same physical frame.
    pub fn map_frame(
        &mut self,
        vspace: ObjRef,
        frame_obj: ObjRef,
        hint: u64,
        rights: Rights,
    ) -> KResult<Mapping> {
        let allowed = Rights::READ | Rights::WRITE | Rights::EXEC;
        if rights.is_empty() || !allowed.contains(rights) {
            return Err(KError::InvalidArg);
        }
        if rights.contains(Rights::WRITE) && rights.contains(Rights::EXEC) {
            return Err(KError::NoRights); // W^X within one mapping
        }
        if !rights.contains(Rights::READ) {
            // All three ports expose a readable page; W-only is not portable.
            return Err(KError::InvalidArg);
        }
        let frame = *self.frame(frame_obj).ok_or(KError::WrongType)?;
        let wants_w = rights.contains(Rights::WRITE);
        let wants_x = rights.contains(Rights::EXEC);
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
        if (wants_w && frame.mapped_x != 0) || (wants_x && frame.mapped_w != 0) {
            return Err(KError::NoRights); // W^X over every mapping of this Frame
        }
        let next_w = if wants_w {
            frame.mapped_w.checked_add(1).ok_or(KError::OutOfMemory)?
        } else {
            frame.mapped_w
        };
        let next_x = if wants_x {
            frame.mapped_x.checked_add(1).ok_or(KError::OutOfMemory)?
        } else {
            frame.mapped_x
        };
        if self.page_table_ops.is_some() {
            self.map_leaf(root, va, frame.pa as u64, rights)?;
        }
        let mapping = Mapping {
            va,
            pa: frame.pa as u64,
            len: page_size,
            rights,
            frame: frame_obj,
        };
        let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
        if let Err(error) = space.insert(mapping) {
            if self.page_table_ops.is_some() {
                let _ = self.unmap_page_table(root, va);
            }
            return Err(error);
        }
        self.set_frame_mapping_counts(frame_obj, next_w, next_x);
        if rights.contains(Rights::EXEC) {
            if let Some(ops) = self.page_table_ops {
                ops.sync_icache(frame.pa as u64, page_size as usize);
            }
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
        let mapping = {
            let space = self.vspace_mut(vspace).ok_or(KError::WrongType)?;
            space.mappings[index].take().ok_or(KError::NotMapped)?
        };
        self.note_frame_unmapping(mapping);
        Ok(mapping)
    }

    /// True if `va` is currently mapped in the shadow table.
    pub fn mapping_at(&self, vspace: ObjRef, va: u64) -> Option<Mapping> {
        self.vspace(vspace)?.find(va)
    }

    fn untyped_is_ancestor(&self, ancestor: ObjRef, mut descendant: ObjRef) -> bool {
        for _ in 0..MAX_OBJECTS {
            let Some(object) = self.get(descendant) else {
                return false;
            };
            if !matches!(object.kind, Kind::Untyped(_)) {
                return false;
            }
            let Some(parent) = object.owner else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            descendant = parent;
        }
        false
    }

    /// Check the core object/capability/scheduler/VSpace invariants.  This is
    /// intentionally deterministic and allocation-free so it can run in the
    /// self-test and in a future watchdog.
    pub fn check_invariants(&self) -> KResult<()> {
        if self.runq_len > MAX_TCBS {
            return Err(KError::IllegalOperation);
        }
        let mut frame_w = [0u16; MAX_OBJECTS];
        let mut frame_x = [0u16; MAX_OBJECTS];
        let mut physical = [(0u64, 0u64, None); MAX_OBJECTS * 2];
        let mut physical_count = 0usize;
        let mut untypeds = [(0u64, 0u64, ObjRef::invalid()); MAX_OBJECTS];
        let mut untyped_count = 0usize;
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
                    if frame.mapped_w != 0 && frame.mapped_x != 0 {
                        return Err(KError::NoRights);
                    }
                    if physical_count >= physical.len() {
                        return Err(KError::IllegalOperation);
                    }
                    physical[physical_count] = (
                        frame.pa as u64,
                        (frame.pa as u64)
                            .checked_add(frame.size as u64)
                            .ok_or(KError::InvalidArg)?,
                        object.owner,
                    );
                    physical_count += 1;
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
                            || !mapping.rights.contains(Rights::READ)
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
                        let frame_index = mapping.frame.index as usize;
                        if frame_index >= MAX_OBJECTS {
                            return Err(KError::ObjectDestroyed);
                        }
                        let frame_object = &self.objs[frame_index];
                        if frame_object.is_free() || frame_object.gen != mapping.frame.gen {
                            return Err(KError::ObjectDestroyed);
                        }
                        let Kind::Frame(ref frame) = frame_object.kind else {
                            return Err(KError::WrongType);
                        };
                        if frame.pa as u64 != mapping.pa || frame.size < mapping.len as usize {
                            return Err(KError::WrongType);
                        }
                        if mapping.rights.contains(Rights::WRITE) {
                            frame_w[frame_index] = frame_w[frame_index]
                                .checked_add(1)
                                .ok_or(KError::IllegalOperation)?;
                        }
                        if mapping.rights.contains(Rights::EXEC) {
                            frame_x[frame_index] = frame_x[frame_index]
                                .checked_add(1)
                                .ok_or(KError::IllegalOperation)?;
                        }
                        for other in space.mappings.iter().skip(i + 1).flatten() {
                            if mapping.va < other.va.saturating_add(other.len)
                                && other.va < mapping.va.saturating_add(mapping.len)
                            {
                                return Err(KError::AlreadyMapped);
                            }
                        }
                    }
                    if page == 0 || !page.is_power_of_two() || space.root % page != 0 {
                        return Err(KError::Alignment);
                    }
                    if physical_count >= physical.len() {
                        return Err(KError::IllegalOperation);
                    }
                    physical[physical_count] = (
                        space.root,
                        space.root.checked_add(page).ok_or(KError::InvalidArg)?,
                        object.owner,
                    );
                    physical_count += 1;
                    if let Some(aux) = space.aux_pa {
                        if aux % page != 0 || physical_count >= physical.len() {
                            return Err(KError::Alignment);
                        }
                        physical[physical_count] = (
                            aux,
                            aux.checked_add(page).ok_or(KError::InvalidArg)?,
                            object.owner,
                        );
                        physical_count += 1;
                    }
                }
                Kind::Untyped(ref untyped) => {
                    if untyped.bump > untyped.size
                        || untyped
                            .objects
                            .checked_add(untyped.reserved)
                            .is_none_or(|used| used > untyped.max_objects)
                    {
                        return Err(KError::IllegalOperation);
                    }
                    if untyped_count >= untypeds.len() {
                        return Err(KError::IllegalOperation);
                    }
                    let end = untyped
                        .base
                        .checked_add(untyped.size)
                        .ok_or(KError::InvalidArg)?;
                    untypeds[untyped_count] = (untyped.base as u64, end as u64, reference);
                    untyped_count += 1;
                }
                Kind::PageTable(ref table) => {
                    if table.root % DEFAULT_PAGE_SIZE != 0 {
                        return Err(KError::Alignment);
                    }
                    if physical_count >= physical.len() {
                        return Err(KError::IllegalOperation);
                    }
                    physical[physical_count] = (
                        table.root,
                        table
                            .root
                            .checked_add(DEFAULT_PAGE_SIZE as u64)
                            .ok_or(KError::InvalidArg)?,
                        object.owner,
                    );
                    physical_count += 1;
                }
                Kind::Free => {}
            }
        }
        // A physical page is owned by at most one live object.  This is the
        // precondition that makes per-Frame W^X meaningful even when a Frame
        // is mapped through several VSpaces.
        for i in 0..physical_count {
            let (start, end, owner) = physical[i];
            if start >= end {
                return Err(KError::Alignment);
            }
            if let Some(owner) = owner {
                let region = self.untyped(owner).ok_or(KError::ObjectDestroyed)?;
                let region_start = region.base as u64;
                let region_end = region_start
                    .checked_add(region.size as u64)
                    .ok_or(KError::InvalidArg)?;
                let allocated_end = region_start
                    .checked_add(region.bump as u64)
                    .ok_or(KError::InvalidArg)?;
                if start < region_start || end > region_end || end > allocated_end {
                    return Err(KError::IllegalOperation);
                }
                for (child_start, child_end, child) in untypeds[..untyped_count].iter() {
                    if start < *child_end
                        && *child_start < end
                        && self.untyped_is_ancestor(owner, *child)
                    {
                        return Err(KError::IllegalOperation);
                    }
                }
            }
            for other in physical[..i].iter() {
                if start < other.1 && other.0 < end {
                    return Err(KError::IllegalOperation);
                }
            }
        }
        // Untyped ranges may nest only along the owner chain (a future child
        // Untyped inside its parent).  Sibling or unrelated leaf ranges may not
        // overlap, otherwise two independent objects could alias one PA.
        for i in 0..untyped_count {
            let (start, end, obj) = untypeds[i];
            if start >= end {
                return Err(KError::Alignment);
            }
            if let Some(parent) = self.get(obj).and_then(|object| object.owner) {
                let region = self.untyped(parent).ok_or(KError::ObjectDestroyed)?;
                let parent_start = region.base as u64;
                let parent_end = parent_start
                    .checked_add(region.size as u64)
                    .ok_or(KError::InvalidArg)?;
                let parent_allocated = parent_start
                    .checked_add(region.bump as u64)
                    .ok_or(KError::InvalidArg)?;
                if start < parent_start || end > parent_end || end > parent_allocated {
                    return Err(KError::IllegalOperation);
                }
            }
            for other in untypeds[..i].iter() {
                if start < other.1 && other.0 < end {
                    let contains_other = start <= other.0 && end >= other.1;
                    let contained_by_other = other.0 <= start && other.1 >= end;
                    if !(contains_other && self.untyped_is_ancestor(obj, other.2))
                        && !(contained_by_other && self.untyped_is_ancestor(other.2, obj))
                    {
                        return Err(KError::IllegalOperation);
                    }
                }
            }
        }
        // The object/reserved counters are derived from the live ownership
        // tree.  This also makes the parent-child quota split explicit.
        for i in 0..untyped_count {
            let (_, _, untyped) = untypeds[i];
            let region = self.untyped(untyped).ok_or(KError::ObjectDestroyed)?;
            let mut objects = 0usize;
            let mut reserved = 0usize;
            for candidate in self.objs.iter() {
                if candidate.is_free() || candidate.owner != Some(untyped) {
                    continue;
                }
                objects = objects.saturating_add(1);
                if let Kind::Untyped(ref child) = candidate.kind {
                    reserved = reserved.saturating_add(child.max_objects);
                }
            }
            if region.objects != objects || region.reserved != reserved {
                return Err(KError::IllegalOperation);
            }
        }
        // Counters are derived from the reverse map and checked after every
        // VSpace has contributed its leaves, independent of object order.
        for i in 0..MAX_OBJECTS {
            if let Kind::Frame(ref frame) = self.objs[i].kind {
                if frame_w[i] != frame.mapped_w || frame_x[i] != frame.mapped_x {
                    return Err(KError::IllegalOperation);
                }
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
        if cap.root && cap.ty != ObjType::Untyped {
            return Err(KError::InvalidArg);
        }
        cap.id = self.next_cap_id;
        cap.root = cap.parent.is_none() && cap.ty == ObjType::Untyped;
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
            let mut removed = [None; MAX_MAPPINGS];
            let mut removed_count = 0;
            {
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
                            removed[removed_count] = Some(mapping);
                            removed_count += 1;
                        }
                    }
                }
            }
            for mapping in removed.iter().take(removed_count).flatten() {
                self.note_frame_unmapping(*mapping);
            }
        }
    }

    fn unmap_space(&mut self, obj: ObjRef) {
        let ops = self.page_table_ops;
        let mut removed = [None; MAX_MAPPINGS];
        let mut removed_count = 0;
        {
            let Some(space) = self.vspace_mut(obj) else {
                return;
            };
            for slot in space.mappings.iter_mut() {
                if let Some(mapping) = *slot {
                    if let Some(ops) = ops {
                        let _ = Self::unmap_with_ops(ops, space.root, mapping.va);
                    }
                    *slot = None;
                    removed[removed_count] = Some(mapping);
                    removed_count += 1;
                }
            }
        }
        for mapping in removed.iter().take(removed_count).flatten() {
            self.note_frame_unmapping(*mapping);
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
                if let Some(fault) = table.fault_ep {
                    if fault.addr.tcb == r
                        && table
                            .cspace
                            .get(fault.addr.slot as usize)
                            .is_none_or(Option::is_none)
                    {
                        table.fault_ep = None;
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
        let mut removed = [None; MAX_MAPPINGS];
        let mut removed_count = 0;
        {
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
                        removed[removed_count] = Some(mapping);
                        removed_count += 1;
                    }
                }
            }
        }
        for mapping in removed.iter().take(removed_count).flatten() {
            self.note_frame_unmapping(*mapping);
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
            Some(Kind::Untyped(_)) => 4,
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
        if self.root == Some(obj) {
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

    /// Collect the objects owned by one Untyped subtree.  Direct objects obey
    /// the exact creator-cap rule; once a child Untyped itself is authorized
    /// for destruction, everything inside that child is collected too.
    fn collect_untyped_subtree(
        &self,
        untyped: ObjRef,
        target: CapRef,
        target_root: bool,
        force: bool,
        objects: &mut [ObjRef; MAX_OBJECTS],
        count: &mut usize,
    ) {
        for i in 0..MAX_OBJECTS {
            let object = &self.objs[i];
            if object.is_free() || object.owner != Some(untyped) {
                continue;
            }
            let created_here = force
                || match object.creator {
                    Some(creator) => self.cap_ref_is_descendant(creator, target),
                    None => target_root,
                };
            if !created_here {
                continue;
            }
            let reference = ObjRef {
                index: i as u8,
                gen: object.gen,
            };
            let child = matches!(object.kind, Kind::Untyped(_));
            if child {
                // Post-order: grandchildren and child objects are listed before
                // the child itself, matching the destruction order.
                self.collect_untyped_subtree(reference, target, target_root, true, objects, count);
            }
            if *count < objects.len() && !objects[..*count].contains(&reference) {
                objects[*count] = reference;
                *count += 1;
            }
        }
    }

    fn recount_untyped_accounting(&mut self) {
        let mut object_counts = [0usize; MAX_OBJECTS];
        let mut reserved_counts = [0usize; MAX_OBJECTS];
        for object in self.objs.iter() {
            if object.is_free() {
                continue;
            }
            let Some(owner) = object.owner else {
                continue;
            };
            let index = owner.index as usize;
            if index >= MAX_OBJECTS || self.objs[index].gen != owner.gen {
                continue;
            }
            object_counts[index] = object_counts[index].saturating_add(1);
            if let Kind::Untyped(ref child) = object.kind {
                reserved_counts[index] = reserved_counts[index].saturating_add(child.max_objects);
            }
        }
        for (index, object) in self.objs.iter_mut().enumerate() {
            if let Kind::Untyped(ref mut region) = object.kind {
                region.objects = object_counts[index];
                region.reserved = reserved_counts[index];
            }
        }
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
            self.collect_untyped_subtree(
                target_cap.obj,
                target_ref,
                target_cap.root,
                false,
                &mut objects,
                &mut object_count,
            );
            if objects[..object_count].contains(&self.current.unwrap_or(ObjRef::invalid())) {
                return Err(KError::IllegalOperation);
            }
            // Destroy in dependency order: Frame leaves first, then page
            // tables deepest-first, then VSpace roots, and finally child
            // Untyped containers.
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
            self.recount_untyped_accounting();
            if let Some(region) = self.untyped_mut(target_cap.obj) {
                if region.objects == 0 {
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

    /// Copy bytes into an existing Frame at a byte offset.  This is used by
    /// loaders that hold a Frame capability but have no mapping of their own.
    pub fn copy_frame_bytes(
        &mut self,
        frame_ref: ObjRef,
        offset: usize,
        bytes: &[u8],
    ) -> KResult<()> {
        let frame = *self.frame(frame_ref).ok_or(KError::WrongType)?;
        let end = offset.checked_add(bytes.len()).ok_or(KError::InvalidArg)?;
        if end > frame.size {
            return Err(KError::InvalidArg);
        }
        let ops = self.page_table_ops.ok_or(KError::MissingPageTable)?;
        ops.copy_to_frame(frame.pa as u64 + offset as u64, bytes);
        Ok(())
    }

    pub fn set_ipc_buffer(&mut self, tcb: ObjRef, frame: ObjRef, va: u64) -> KResult<()> {
        let vspace = self
            .tcb(tcb)
            .and_then(|table| table.vspace)
            .ok_or(KError::InvalidArg)?;
        let mapping = self.mapping_at(vspace, va).ok_or(KError::NotMapped)?;
        if mapping.frame != frame
            || mapping.va != va
            || !mapping.rights.contains(Rights::WRITE)
            || mapping.len < DEFAULT_PAGE_SIZE
        {
            return Err(KError::InvalidArg);
        }
        let frame_object = self.frame(frame).ok_or(KError::WrongType)?;
        if frame_object.size < mapping.len as usize {
            return Err(KError::InvalidArg);
        }
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        table.ipc_buffer = Some(IpcBuffer { frame, va });
        Ok(())
    }

    fn copy_ipc_buffers(&self, sender: ObjRef, receiver: ObjRef) {
        let Some(sender_buffer) = self.tcb(sender).and_then(|table| table.ipc_buffer) else {
            return;
        };
        let Some(receiver_buffer) = self.tcb(receiver).and_then(|table| table.ipc_buffer) else {
            return;
        };
        let Some(ops) = self.page_table_ops else {
            return;
        };
        let Some(sender_frame) = self.frame(sender_buffer.frame) else {
            return;
        };
        let Some(receiver_frame) = self.frame(receiver_buffer.frame) else {
            return;
        };
        let size = IPC_BUFFER_WORDS * core::mem::size_of::<u64>();
        if sender_frame.size < size || receiver_frame.size < size {
            return;
        }
        ops.copy_frame_to_frame(sender_frame.pa as u64, receiver_frame.pa as u64, size);
    }

    pub fn set_fault_endpoint(&mut self, tcb: ObjRef, endpoint: Option<CapRef>) -> KResult<()> {
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        table.fault_ep = endpoint;
        Ok(())
    }

    pub fn fault_endpoint(&self, tcb: ObjRef) -> Option<ObjRef> {
        let fault = self.tcb(tcb)?.fault_ep?;
        let cap = self.cap_at(fault.addr)?;
        if cap.id != fault.id || cap.ty != ObjType::Endpoint {
            return None;
        }
        self.endpoint(cap.obj)?;
        Some(cap.obj)
    }

    /// Deliver a fault message to the TCB's configured endpoint and leave the
    /// faulting thread in `BlockedFault`.  If the receiver is not waiting the
    /// message is queued without making the faulting thread runnable again.
    pub fn fault_to_endpoint(
        &mut self,
        tcb: ObjRef,
        endpoint: ObjRef,
        msg: Message,
    ) -> KResult<()> {
        let delivered = self.send(tcb, endpoint, 0, msg, false)?;
        let mut index = 0;
        while index < self.runq_len {
            if self.runq[index] == Some(tcb) {
                let mut shift = index;
                while shift + 1 < self.runq_len {
                    self.runq[shift] = self.runq[shift + 1];
                    shift += 1;
                }
                self.runq_len -= 1;
                self.runq[self.runq_len] = None;
            } else {
                index += 1;
            }
        }
        let table = self.tcb_mut(tcb).ok_or(KError::ObjectDestroyed)?;
        table.state = ThreadState::BlockedFault;
        table.queued = false;
        let _ = delivered;
        if self.current == Some(tcb) {
            self.current = None;
        }
        Ok(())
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
            n if n == SyscallId::ReplyRecv as u64 => {
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
                let msg = match Message::new(a2, &[a3, a4, a5]) {
                    Ok(msg) => msg,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.reply_recv(tcb, endpoint, recv_slot, msg) {
                    Ok(Some(message)) => message_action(KError::Ok, 0, &message),
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
                    n if n == ObjType::Untyped as u64 => ObjType::Untyped,
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
                let creator = Some(SlotAddr {
                    tcb,
                    slot: untyped_slot,
                });
                let object = match ty {
                    ObjType::Untyped => {
                        self.retype_untyped_child_as(untyped.obj, a3 as usize, a4 as usize, creator)
                    }
                    _ => self.retype_as(untyped.obj, ty, creator),
                };
                let object = match object {
                    Ok(object) => object,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let rights = if ty == ObjType::Frame {
                    Rights::READ | Rights::WRITE | Rights::EXEC | Rights::GRANT
                } else {
                    Rights::READ | Rights::WRITE | Rights::GRANT
                };
                let mut cap = Cap::new(object, ty, rights);
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
                let fault_slot = slot_arg(a5).unwrap_or(0);
                let fault = if fault_slot != 0 {
                    let Some(fault_cap) = self.cap_at(SlotAddr {
                        tcb: cap.obj,
                        slot: fault_slot,
                    }) else {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidCap,
                        };
                    };
                    if fault_cap.ty != ObjType::Endpoint {
                        return UserAction::Return {
                            value: 0,
                            error: KError::WrongType,
                        };
                    }
                    Some(CapRef {
                        addr: SlotAddr {
                            tcb: cap.obj,
                            slot: fault_slot,
                        },
                        id: fault_cap.id,
                    })
                } else {
                    None
                };
                match self.configure_tcb(cap.obj, a1, a2, vspace.obj, a4 as u8) {
                    Ok(()) => {
                        if let Err(error) = self.set_fault_endpoint(cap.obj, fault) {
                            return UserAction::Return { value: 0, error };
                        }
                        UserAction::ConfigureTcb {
                            tcb: cap.obj,
                            entry: a1,
                            stack: a2,
                        }
                    }
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
            n if n == SyscallId::TcbSetIpcBuffer as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Tcb),
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
                match self.set_ipc_buffer(target.obj, frame.obj, a2) {
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
            n if n == SyscallId::FrameWrite as u64 => {
                let frame = match self.lookup_cap(
                    tcb,
                    slot_arg(a0).unwrap_or(0),
                    Some(ObjType::Frame),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                UserAction::FrameWrite {
                    frame: frame.obj,
                    offset: a1,
                    ptr: a2,
                    len: a3,
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
                // A VSpace capability alone must not allow a task to tear
                // down mappings of frames it does not own.  The caller must
                // also present a Frame cap for the exact PA being unmapped.
                let frame = match self.lookup_cap(
                    tcb,
                    slot_arg(a2).unwrap_or(0),
                    Some(ObjType::Frame),
                    Rights::NONE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let Some(frame_object) = self.frame(frame.obj) else {
                    return UserAction::Return {
                        value: 0,
                        error: KError::ObjectDestroyed,
                    };
                };
                let Some(mapping) = self.mapping_at(space.obj, a1) else {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NotMapped,
                    };
                };
                if mapping.pa != frame_object.pa as u64 {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NoRights,
                    };
                }
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
    fn bootinfo_layout_is_stable() {
        let info = BootInfo {
            magic: BOOTINFO_MAGIC,
            version: BOOTINFO_VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            root_vspace: 1,
            meta_untyped: 2,
            data_untyped: 3,
            server_vspace: 12,
            user_code_first: 20,
            user_code_count: 1,
            free_slot_start: 21,
            free_slot_end: 64,
            server_elf_va: 0x1000_0000,
            server_elf_len: 4096,
            client_elf_va: 0x1000_2000,
            client_elf_len: 4096,
        };
        let base = &info as *const BootInfo as usize;
        assert_eq!(core::mem::size_of::<BootInfo>(), 64);
        for (offset, field) in [
            (0usize, &info.magic as *const u64 as usize),
            (8, &info.version as *const u32 as usize),
            (12, &info.size as *const u32 as usize),
            (16, &info.root_vspace as *const u16 as usize),
            (18, &info.meta_untyped as *const u16 as usize),
            (20, &info.data_untyped as *const u16 as usize),
            (22, &info.server_vspace as *const u16 as usize),
            (24, &info.user_code_first as *const u16 as usize),
            (26, &info.user_code_count as *const u16 as usize),
            (28, &info.free_slot_start as *const u16 as usize),
            (30, &info.free_slot_end as *const u16 as usize),
            (32, &info.server_elf_va as *const u64 as usize),
            (40, &info.server_elf_len as *const u64 as usize),
            (48, &info.client_elf_va as *const u64 as usize),
            (56, &info.client_elf_len as *const u64 as usize),
        ] {
            assert_eq!(field - base, offset);
        }
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
    fn frame_wx_is_enforced_across_aliases_and_vspaces() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let va = 0x10_0000_0000;
        let space_a = kernel
            .create_vspace_with_page_size(0x1000, va, 0x10_0000, 4096)
            .unwrap();
        let space_b = kernel
            .create_vspace_with_page_size(0x2000, va, 0x10_0000, 4096)
            .unwrap();
        for (space, tables) in [
            (space_a, [0x3000, 0x4000, 0x5000]),
            (space_b, [0x6000, 0x7000, 0x8000]),
        ] {
            for table in tables {
                let pt = kernel.create_page_table(table, 0).unwrap();
                assert!(kernel.map_page_table(pt, space, va).is_ok());
            }
        }

        // READ is mandatory even on architectures whose PTEs encode a
        // writable page without a separate readable bit.
        let before = table_snapshot(&MAP_OPS, 0x1000, 0);
        for rights in [Rights::NONE, Rights::WRITE, Rights::EXEC] {
            assert_eq!(
                kernel.map_frame(space_a, frame, va, rights),
                Err(KError::InvalidArg)
            );
            assert_eq!(table_snapshot(&MAP_OPS, 0x1000, 0), before);
        }

        let mapping = kernel
            .map_frame(space_a, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 1);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 0);
        assert!(kernel.check_invariants().is_ok());

        let before_a = table_snapshot(&MAP_OPS, 0x1000, 0);
        let before_b = table_snapshot(&MAP_OPS, 0x2000, 0);
        // A different VA in the same VSpace and the same VA in another
        // VSpace must both fail before any PTE is written.
        assert_eq!(
            kernel.map_frame(space_a, frame, va + 0x1000, Rights::READ | Rights::EXEC),
            Err(KError::NoRights)
        );
        assert_eq!(
            kernel.map_frame(space_b, frame, va, Rights::READ | Rights::EXEC),
            Err(KError::NoRights)
        );
        assert_eq!(table_snapshot(&MAP_OPS, 0x1000, 0), before_a);
        assert_eq!(table_snapshot(&MAP_OPS, 0x2000, 0), before_b);
        assert!(kernel.mapping_at(space_a, va + 0x1000).is_none());
        assert!(kernel.mapping_at(space_b, va).is_none());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 1);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 0);

        // Removing the writable leaf releases the counter; the same Frame can
        // then be mapped executable.
        assert_eq!(kernel.unmap_frame(space_a, mapping.va), Ok(mapping));
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 0);
        assert!(kernel
            .map_frame(space_a, frame, va, Rights::READ | Rights::EXEC)
            .is_ok());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 1);
        assert_eq!(
            kernel.map_frame(space_a, frame, va + 0x1000, Rights::READ | Rights::WRITE),
            Err(KError::NoRights)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn physical_page_ranges_are_unique() {
        let mut alias = Kernel::new();
        assert!(alias.create_frame(0x1000, 4096).is_some());
        assert!(alias.create_frame(0x1000, 4096).is_some());
        assert_eq!(alias.check_invariants(), Err(KError::IllegalOperation));

        let mut mixed = Kernel::new();
        assert!(mixed.create_page_table(0x1000, 0).is_some());
        assert!(mixed
            .create_vspace(0x1000, 0x10_0000_0000, 0x1000)
            .is_some());
        assert_eq!(mixed.check_invariants(), Err(KError::IllegalOperation));

        let mut aux_alias = Kernel::new();
        let space = aux_alias
            .create_vspace(0x1000, 0x10_0000_0000, 0x1000)
            .unwrap();
        if let Kind::VSpace(ref mut space) = aux_alias.objs[space.index as usize].kind {
            space.aux_pa = Some(0x2000);
        }
        assert!(aux_alias.create_frame(0x2000, 4096).is_some());
        assert_eq!(aux_alias.check_invariants(), Err(KError::IllegalOperation));

        let mut overlapping_untyped = Kernel::new();
        assert!(overlapping_untyped
            .create_untyped_with_limit(0x1000, 0x2000, 4096, 4)
            .is_some());
        assert!(overlapping_untyped
            .create_untyped_with_limit(0x2000, 0x2000, 4096, 4)
            .is_some());
        assert_eq!(
            overlapping_untyped.check_invariants(),
            Err(KError::IllegalOperation)
        );

        let mut clean = Kernel::new();
        assert!(clean.create_frame(0x1000, 4096).is_some());
        assert!(clean.create_page_table(0x2000, 0).is_some());
        assert!(clean
            .create_vspace(0x3000, 0x10_0000_0000, 0x1000)
            .is_some());
        assert!(clean.check_invariants().is_ok());
    }

    #[test]
    fn frame_map_rejects_counter_overflow() {
        let mut kernel = Kernel::new();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let vspace = kernel
            .create_vspace(0x1000, 0x10_0000_0000, 0x1000)
            .unwrap();
        if let Kind::Frame(ref mut frame) = kernel.objs[frame.index as usize].kind {
            frame.mapped_w = u16::MAX;
        }
        assert_eq!(
            kernel.map_frame(vspace, frame, 0x10_0000_0000, Rights::READ | Rights::WRITE),
            Err(KError::OutOfMemory)
        );
        assert!(kernel.mapping_at(vspace, 0x10_0000_0000).is_none());
    }

    #[test]
    fn icache_syncs_only_for_exec_mappings() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let va = 0x10_0000_0000;
        let space = kernel
            .create_vspace_with_page_size(0x1000, va, 0x2000, 4096)
            .unwrap();
        for table in [0x3000, 0x4000, 0x5000] {
            let pt = kernel.create_page_table(table, 0).unwrap();
            assert!(kernel.map_page_table(pt, space, va).is_ok());
        }

        let mapping = kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(
            FAKE_ICACHE_SYNCS.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        kernel.unmap_frame(space, mapping.va).unwrap();
        assert!(kernel
            .map_frame(space, frame, va, Rights::READ | Rights::EXEC)
            .is_ok());
        assert_eq!(
            FAKE_ICACHE_SYNCS.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(kernel
            .map_frame(space, frame, va + 0x1000, Rights::READ | Rights::EXEC)
            .is_ok());
        assert_eq!(
            FAKE_ICACHE_SYNCS.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    }

    #[test]
    fn revoking_vspace_releases_frame_wx_counters() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space_untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        let frame_untyped = kernel
            .create_untyped_with_limit(0x200_0000, 0x1000, 4096, 2)
            .unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(space_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(frame_untyped, ObjType::Untyped, Rights::ALL),
            )
            .unwrap();
        let space = kernel
            .retype_as(
                space_untyped,
                ObjType::VSpace,
                Some(SlotAddr { tcb: root, slot: 1 }),
            )
            .unwrap();
        let frame = kernel
            .retype_as(
                frame_untyped,
                ObjType::Frame,
                Some(SlotAddr { tcb: root, slot: 2 }),
            )
            .unwrap();
        let va = 0x10_0000_a000;
        kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 1);
        assert!(kernel.check_invariants().is_ok());

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.vspace(space).is_none());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 0);
        assert!(kernel.check_invariants().is_ok());

        let replacement = kernel.create_vspace(0x3000, 0x5000_a000, 0x1000).unwrap();
        assert!(kernel
            .map_frame(replacement, frame, 0x5000_a000, Rights::READ | Rights::EXEC)
            .is_ok());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 1);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn frame_unmap_requires_matching_frame_cap() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xf000, 4096, 16)
            .unwrap();
        let data_va = 0x10_0000_a000;
        let space = kernel
            .build_task_space(
                untyped,
                b"UNMAP",
                TaskLayout {
                    code_va: 0x10_0000_0000,
                    stack_va: 0x10_0000_1000,
                    stack_pages: 2,
                    data: None,
                },
            )
            .unwrap();
        let frame = kernel.retype(untyped, ObjType::Frame).unwrap();
        kernel
            .map_frame(space, frame, data_va, Rights::READ | Rights::WRITE)
            .unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(space, ObjType::VSpace, Rights::READ | Rights::WRITE),
            )
            .unwrap();

        // VSpace WRITE alone is insufficient: no Frame cap is presented.
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameUnmap as u64,
                1,
                0x10_0000_0000,
                0,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap,
            }
        );
        assert!(kernel.mapping_at(space, 0x10_0000_0000).is_some());

        kernel
            .install_cap(
                root,
                2,
                Cap::new(frame, ObjType::Frame, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        // The Frame cap must match the PA of the mapping being removed.
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameUnmap as u64,
                1,
                0x10_0000_0000,
                2,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights,
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::FrameUnmap as u64, 1, data_va, 2, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok,
            }
        );
        assert!(kernel.mapping_at(space, data_va).is_none());
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
    fn frame_map_rights_are_bounded_by_frame_cap_rights() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let vspace = kernel
            .create_vspace_with_page_size(0x1000, 0x10_0000_0000, 0x1000, 4096)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(vspace, ObjType::VSpace, Rights::WRITE))
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(frame, ObjType::Frame, Rights::READ | Rights::EXEC),
            )
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                1,
                2,
                0x10_0000_0000,
                (Rights::READ | Rights::WRITE).0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights,
            }
        );
        assert!(kernel.mapping_at(vspace, 0x10_0000_0000).is_none());

        kernel
            .install_cap(
                root,
                4,
                Cap::new(
                    frame,
                    ObjType::Frame,
                    Rights::READ | Rights::WRITE | Rights::EXEC,
                ),
            )
            .unwrap();
        for rights in [Rights::NONE, Rights::WRITE, Rights::EXEC] {
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::FrameMap as u64,
                    1,
                    4,
                    0x10_0000_0000,
                    rights.0 as u64,
                    0,
                    0
                ),
                UserAction::Return {
                    value: 0,
                    error: KError::InvalidArg,
                }
            );
        }
        assert!(kernel.mapping_at(vspace, 0x10_0000_0000).is_none());
    }

    #[test]
    fn frame_map_cannot_alias_wx_through_syscall() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let vspace = kernel
            .create_vspace(0x1000, 0x10_0000_0000, 0x2000)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(vspace, ObjType::VSpace, Rights::WRITE))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        let va = 0x10_0000_0000;
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                1,
                2,
                va,
                (Rights::READ | Rights::WRITE).0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: va,
                error: KError::Ok,
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                1,
                2,
                va + 0x1000,
                (Rights::READ | Rights::EXEC).0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights,
            }
        );
        assert!(kernel.mapping_at(vspace, va + 0x1000).is_none());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 1);

        assert_eq!(
            kernel.user_syscall(root, SyscallId::FrameUnmap as u64, 1, va, 2, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok,
            }
        );
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                1,
                2,
                va,
                (Rights::READ | Rights::EXEC).0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: va,
                error: KError::Ok,
            }
        );
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 1);
        assert!(kernel.check_invariants().is_ok());
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
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x8000, 4096, 16)
            .unwrap();
        let space = kernel
            .build_task_space(
                untyped,
                b"HELLO",
                TaskLayout {
                    code_va: 0x10_0000_0000,
                    stack_va: 0x10_0000_1000,
                    stack_pages: 2,
                    data: Some(TaskData {
                        va: 0x10_0000_a000,
                        pages: 1,
                    }),
                },
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
            kernel.user_syscall(root, SyscallId::Read as u64, 0, 0x10_0000_1000, 8, 0, 0, 0),
            UserAction::Read {
                fd: 0,
                ptr: 0x10_0000_1000,
                len: 8
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Read as u64, 0, 0x10_0000_0000, 8, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::Write as u64, 0, 0x10_0000_0000, 8, 0, 0, 0),
            UserAction::Write {
                fd: 0,
                ptr: 0x10_0000_0000,
                len: 8
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::DebugWrite as u64,
                0x10_0000_0000,
                8,
                0,
                0,
                0,
                0
            ),
            UserAction::Debug {
                ptr: 0x10_0000_0000,
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
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
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
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x8000, 4096, 16)
            .unwrap();
        let space = kernel
            .build_task_space(
                untyped,
                b"X",
                TaskLayout {
                    code_va: 0x10_0000_0000,
                    stack_va: 0x10_0000_1000,
                    stack_pages: 2,
                    data: Some(TaskData {
                        va: 0x10_0000_a000,
                        pages: 1,
                    }),
                },
            )
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();
        assert!(kernel
            .validate_user_range(root, 0x10_0000_0000, 1, false)
            .is_ok());
        assert_eq!(
            kernel.validate_user_range(root, 0x10_0000_0000, 1, true),
            Err(KError::InvalidArg)
        );
        assert!(kernel
            .validate_user_range(root, 0x10_0000_1000, 1, true)
            .is_ok());
        assert_eq!(
            kernel.validate_user_range(root, 0x10_0000_afff, 2, true),
            Err(KError::InvalidArg)
        );
        assert_eq!(
            kernel.validate_user_range(root, u64::MAX - 1, 4, false),
            Err(KError::InvalidArg)
        );
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
    static FAKE_ICACHE_SYNCS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
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
        fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
            let mut pte = (pa & !0xfff) | 0b10;
            if rights.contains(Rights::WRITE) {
                pte |= 0b100;
            }
            if rights.contains(Rights::EXEC) {
                pte |= 0b1000;
            }
            pte
        }
        fn encode_table(&self, pa: u64) -> u64 {
            (pa & !0xfff) | 0b01
        }
        fn decode(&self, pte: u64) -> Pte {
            if pte == 0 {
                Pte::Invalid
            } else if pte & 0b10 != 0 {
                let mut rights = Rights::READ;
                if pte & 0b100 != 0 {
                    rights = rights | Rights::WRITE;
                }
                if pte & 0b1000 != 0 {
                    rights = rights | Rights::EXEC;
                }
                Pte::Leaf(pte & !0xfff, rights)
            } else {
                Pte::Table(pte & !0xfff)
            }
        }
        fn leaf_is_user(&self, pte: u64) -> bool {
            pte & 0b10 != 0
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
        FAKE_ICACHE_SYNCS.store(0, std::sync::atomic::Ordering::SeqCst);
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
        fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
            let mut pte = (pa & !0xfff) | 0b10;
            if rights.contains(Rights::WRITE) {
                pte |= 0b100;
            }
            if rights.contains(Rights::EXEC) {
                pte |= 0b1000;
            }
            pte
        }
        fn encode_table(&self, pa: u64) -> u64 {
            (pa & !0xfff) | 0b01
        }
        fn decode(&self, pte: u64) -> Pte {
            if pte == 0 {
                Pte::Invalid
            } else if pte & 0b10 != 0 {
                let mut rights = Rights::READ;
                if pte & 0b100 != 0 {
                    rights = rights | Rights::WRITE;
                }
                if pte & 0b1000 != 0 {
                    rights = rights | Rights::EXEC;
                }
                Pte::Leaf(pte & !0xfff, rights)
            } else {
                Pte::Table(pte & !0xfff)
            }
        }
        fn decode_at(&self, pte: u64, _level: usize) -> Pte {
            self.decode(pte)
        }
        fn leaf_is_user(&self, pte: u64) -> bool {
            pte & 0b10 != 0
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
        fn sync_icache(&self, _pa: u64, _size: usize) {
            FAKE_ICACHE_SYNCS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn copy_frame_to_frame(&self, src: u64, dst: u64, size: usize) {
            let bytes: Vec<u8> = {
                let mem = FAKE_MEM.lock().unwrap();
                (0..size)
                    .map(|index| {
                        let byte_index = src as usize + index;
                        let word = mem[byte_index / 8];
                        ((word >> ((byte_index % 8) * 8)) & 0xff) as u8
                    })
                    .collect()
            };
            let mut mem = FAKE_MEM.lock().unwrap();
            for (index, byte) in bytes.iter().enumerate() {
                let byte_index = dst as usize + index;
                let word_index = byte_index / 8;
                let shift = (byte_index % 8) * 8;
                if let Some(word) = mem.get_mut(word_index) {
                    *word = (*word & !(0xffu64 << shift)) | ((*byte as u64) << shift);
                }
            }
        }
        fn copy_to_frame(&self, pa: u64, bytes: &[u8]) {
            let mut mem = FAKE_MEM.lock().unwrap();
            for (index, byte) in bytes.iter().enumerate() {
                let byte_index = pa as usize + index;
                let word = byte_index / 8;
                let shift = (byte_index % 8) * 8;
                if let Some(slot) = mem.get_mut(word) {
                    *slot = (*slot & !(0xffu64 << shift)) | ((*byte as u64) << shift);
                }
            }
        }

        fn zero_frame(&self, pa: u64, size: usize) {
            let base = ((pa >> 12) as usize) * 512;
            let words = (size + 7) / 8;
            if let Some(slice) = FAKE_MEM.lock().unwrap().get_mut(base..base + words) {
                slice.fill(0);
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum PteFormat {
        Riscv,
        X86,
        AArch,
    }

    struct PteFakeOps(PteFormat);

    static PTE_FAKE_MEM: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
    static PTE_RISCV: PteFakeOps = PteFakeOps(PteFormat::Riscv);
    static PTE_X86: PteFakeOps = PteFakeOps(PteFormat::X86);
    static PTE_AARCH: PteFakeOps = PteFakeOps(PteFormat::AArch);

    impl PageTableOps for PteFakeOps {
        fn levels(&self) -> usize {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::LEVELS,
                PteFormat::X86 => vifast_pte::x86_64::LEVELS,
                PteFormat::AArch => vifast_pte::aarch64::LEVELS,
            }
        }
        fn index(&self, va: u64, level: usize) -> usize {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::index(va, level),
                PteFormat::X86 => vifast_pte::x86_64::index(va, level),
                PteFormat::AArch => vifast_pte::aarch64::index(va, level),
            }
        }
        fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
            let base = ((table_pa >> 12) as usize) * 512;
            PTE_FAKE_MEM
                .lock()
                .unwrap()
                .get(base + idx)
                .copied()
                .unwrap_or(0)
        }
        fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
            let base = ((table_pa >> 12) as usize) * 512;
            if let Some(slot) = PTE_FAKE_MEM.lock().unwrap().get_mut(base + idx) {
                *slot = pte;
            }
        }
        fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::encode_leaf(pa, rights),
                PteFormat::X86 => vifast_pte::x86_64::encode_leaf(pa, rights),
                PteFormat::AArch => vifast_pte::aarch64::encode_leaf(pa, rights),
            }
        }
        fn encode_table(&self, pa: u64) -> u64 {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::encode_table(pa),
                PteFormat::X86 => vifast_pte::x86_64::encode_table(pa),
                PteFormat::AArch => vifast_pte::aarch64::encode_table(pa),
            }
        }
        fn decode(&self, pte: u64) -> Pte {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::decode(pte),
                PteFormat::X86 => vifast_pte::x86_64::decode(pte),
                PteFormat::AArch => vifast_pte::aarch64::decode(pte),
            }
        }
        fn decode_at(&self, pte: u64, level: usize) -> Pte {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::decode_at(pte, level),
                PteFormat::X86 => vifast_pte::x86_64::decode_at(pte, level),
                PteFormat::AArch => vifast_pte::aarch64::decode_at(pte, level),
            }
        }
        fn leaf_is_user(&self, pte: u64) -> bool {
            match self.0 {
                PteFormat::Riscv => vifast_pte::riscv::is_user(pte),
                PteFormat::X86 => vifast_pte::x86_64::is_user(pte),
                PteFormat::AArch => vifast_pte::aarch64::is_user(pte),
            }
        }
        fn user_va_range(&self) -> (u64, u64) {
            (vifast_pte::USER_VA_START, vifast_pte::USER_VA_END)
        }
        fn vspace_pages(&self) -> usize {
            match self.0 {
                PteFormat::Riscv => 1,
                PteFormat::X86 | PteFormat::AArch => 2,
            }
        }
        fn copy_to_frame(&self, pa: u64, bytes: &[u8]) {
            let mut mem = PTE_FAKE_MEM.lock().unwrap();
            for (index, byte) in bytes.iter().enumerate() {
                let byte_index = pa as usize + index;
                let word = byte_index / 8;
                let shift = (byte_index % 8) * 8;
                if let Some(slot) = mem.get_mut(word) {
                    *slot = (*slot & !(0xffu64 << shift)) | ((*byte as u64) << shift);
                }
            }
        }
        fn zero_frame(&self, pa: u64, size: usize) {
            let base = ((pa >> 12) as usize) * 512;
            let words = (size + 7) / 8;
            if let Some(slice) = PTE_FAKE_MEM.lock().unwrap().get_mut(base..base + words) {
                slice.fill(0);
            }
        }
    }

    fn reset_pte_fake() {
        let mut mem = PTE_FAKE_MEM.lock().unwrap();
        mem.clear();
        mem.resize(8192, 0);
    }

    fn pte_ops(format: PteFormat) -> &'static dyn PageTableOps {
        match format {
            PteFormat::Riscv => &PTE_RISCV,
            PteFormat::X86 => &PTE_X86,
            PteFormat::AArch => &PTE_AARCH,
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
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4006);

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
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4006);

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
        assert!(
            !kernel
                .cap_at(SlotAddr {
                    tcb: server,
                    slot: 1
                })
                .unwrap()
                .root
        );
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
    fn multiple_dying_tcbs_reparent_through_dead_intermediate_cap() {
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
        let tcb1 = kernel
            .retype_as(
                meta_untyped,
                ObjType::Tcb,
                Some(SlotAddr { tcb: root, slot: 2 }),
            )
            .unwrap();
        let tcb2 = kernel
            .retype_as(
                meta_untyped,
                ObjType::Tcb,
                Some(SlotAddr { tcb: root, slot: 2 }),
            )
            .unwrap();
        kernel.copy_cap(root, 1, tcb1, 1, Rights::ALL, 0).unwrap();
        kernel.copy_cap(tcb1, 1, tcb2, 1, Rights::ALL, 0).unwrap();
        let frame = kernel
            .retype_as(
                data_untyped,
                ObjType::Frame,
                Some(SlotAddr { tcb: tcb2, slot: 1 }),
            )
            .unwrap();

        // One revocation kills both TCBs.  The intermediate cap is dying too,
        // so the frame creator must land on the root cap, not on tcb1's cap.
        kernel.revoke_cap(root, 2).unwrap();
        assert!(kernel.get(tcb1).is_none());
        assert!(kernel.get(tcb2).is_none());
        assert!(kernel.get(frame).is_some());
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(frame).is_none());
        assert_eq!(kernel.untyped(data_untyped).unwrap().objects, 0);
        assert_eq!(kernel.untyped(data_untyped).unwrap().bump, 0);
    }

    #[test]
    fn boot_creator_none_objects_are_not_collected_by_data_cap() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let boot = kernel
            .create_untyped_with_limit(0x1000, 0x1000, 4096, 2)
            .unwrap();
        let data = kernel
            .create_untyped_with_limit(0x2000, 0x1000, 4096, 2)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(data, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let boot_frame = kernel.retype(boot, ObjType::Frame).unwrap();
        let data_frame = kernel
            .retype_as(data, ObjType::Frame, Some(SlotAddr { tcb: root, slot: 1 }))
            .unwrap();
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(boot_frame).is_some());
        assert!(kernel.get(data_frame).is_none());
    }

    #[test]
    fn root_flag_does_not_propagate_to_copies_or_ipc() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().root);
        kernel.copy_cap(root, 1, client, 1, Rights::ALL, 0).unwrap();
        assert!(
            !kernel
                .cap_at(SlotAddr {
                    tcb: client,
                    slot: 1
                })
                .unwrap()
                .root
        );
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        assert_eq!(kernel.recv(client, endpoint, 3), Ok(None));
        let msg = Message::new(0x51, &[1]).unwrap().with_cap(1);
        assert_eq!(kernel.send(root, endpoint, 0, msg, false), Ok(true));
        assert!(
            !kernel
                .cap_at(SlotAddr {
                    tcb: client,
                    slot: 3
                })
                .unwrap()
                .root
        );
    }

    #[test]
    fn root_tcb_cannot_be_destroyed_by_untyped_revoke() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x100_0000, 0x1000, 4096, 2)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        // Test the otherwise-unreachable ownership path explicitly.
        kernel.objs[root.index as usize].owner = Some(untyped);
        assert_eq!(kernel.revoke_cap(root, 1), Err(KError::IllegalOperation));
        assert!(kernel.get(root).is_some());
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
        assert_eq!(FAKE_MEM.lock().unwrap()[1536 + 1], 0x4006);

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.frame(frame).is_some());
        assert!(kernel.mapping_at(space, va).is_none());
        assert_eq!(kernel.frame(frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(frame).unwrap().mapped_x, 0);
        assert!(kernel.check_invariants().is_ok());
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
    fn build_task_space_maps_code_stack_and_rolls_back() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x8000, 4096, 16)
            .unwrap();
        let data_va = 0x10_0000_a000;
        let layout = TaskLayout {
            code_va: 0x10_0000_0000,
            stack_va: 0x10_0000_1000,
            stack_pages: 2,
            data: Some(TaskData {
                va: data_va,
                pages: 1,
            }),
        };
        let image = b"BUILD-OK";
        let vspace = kernel.build_task_space(untyped, image, layout).unwrap();
        assert_eq!(
            kernel.mapping_at(vspace, layout.code_va).unwrap().rights,
            Rights::READ | Rights::EXEC
        );
        assert_eq!(
            kernel.mapping_at(vspace, layout.stack_va).unwrap().rights,
            Rights::READ | Rights::WRITE
        );
        assert_eq!(
            kernel
                .mapping_at(vspace, layout.stack_va + 4096)
                .unwrap()
                .rights,
            Rights::READ | Rights::WRITE
        );
        assert_eq!(
            kernel.mapping_at(vspace, data_va).unwrap().rights,
            Rights::READ | Rights::WRITE
        );
        // Kernel slots in the private root/aux table remain intact.
        assert_eq!(FAKE_MEM.lock().unwrap()[512], 0x2001);
        assert_eq!(FAKE_MEM.lock().unwrap()[1024], 0x9001);
        let code_pa = kernel.mapping_at(vspace, layout.code_va).unwrap().pa;
        let code_base = ((code_pa >> 12) as usize) * 512;
        assert_eq!(FAKE_MEM.lock().unwrap()[code_base] & 0xff, b'B' as u64);

        // An arena with exactly one page too few must roll back completely.
        let small = kernel
            .create_untyped_with_limit(0x9000, 0x7000, 4096, 16)
            .unwrap();
        let before_bump = kernel.untyped(small).unwrap().bump;
        let before_objects = kernel.untyped(small).unwrap().objects;
        assert_eq!(
            kernel.build_task_space(small, image, layout),
            Err(KError::OutOfMemory)
        );
        assert_eq!(kernel.untyped(small).unwrap().bump, before_bump);
        assert_eq!(kernel.untyped(small).unwrap().objects, before_objects);
    }

    fn table_snapshot(
        ops: &dyn PageTableOps,
        table_pa: u64,
        level: usize,
    ) -> std::vec::Vec<(u64, usize, u64)> {
        let levels = ops.levels();
        let mut snapshot = std::vec::Vec::new();
        for index in 0..512 {
            let pte = ops.read_pte(table_pa, index);
            snapshot.push((table_pa, index, pte));
            if let Pte::Table(next) = ops.decode_at(pte, level) {
                if level + 1 < levels {
                    snapshot.extend(table_snapshot(ops, next, level + 1));
                }
            }
        }
        snapshot
    }

    fn count_user_leaves(ops: &dyn PageTableOps, table_pa: u64, level: usize) -> usize {
        let levels = ops.levels();
        let mut count = 0;
        for index in 0..512 {
            let pte = ops.read_pte(table_pa, index);
            match ops.decode_at(pte, level) {
                Pte::Table(next) if level + 1 < levels => {
                    count += count_user_leaves(ops, next, level + 1);
                }
                Pte::Leaf(_, _) if level + 1 == levels && ops.leaf_is_user(pte) => {
                    count += 1;
                }
                _ => {}
            }
        }
        count
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn test_elf(machine: u16) -> std::vec::Vec<u8> {
        const PH: usize = 64;
        const PHDR: usize = 56;
        const R: u32 = 4;
        const X: u32 = 1;
        const W: u32 = 2;
        let base = vifast_abi::user_layout::ROOT_ELF_BASE as u64;
        let mut bytes = std::vec![0u8; 0x10000];
        bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        put_u16(&mut bytes, 16, 2);
        put_u16(&mut bytes, 18, machine);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 24, base + 0x1010);
        put_u64(&mut bytes, 32, PH as u64);
        put_u16(&mut bytes, 52, PH as u16);
        put_u16(&mut bytes, 54, PHDR as u16);
        put_u16(&mut bytes, 56, 4);
        let segments = [
            (base, R, 0x1000usize, 0x100usize, 0x100usize),
            (base + 0x1000, R | X, 0x2000, 0x100, 0x100),
            (base + 0x2000, R | W, 0x3000, 0x100, 0x2800),
            (base + 0x5100, R, 0x8100, 0x100, 0x100),
        ];
        for (index, (vaddr, flags, offset, filesz, memsz)) in segments.iter().copied().enumerate() {
            let p = PH + index * PHDR;
            put_u32(&mut bytes, p, 1);
            put_u32(&mut bytes, p + 4, flags);
            put_u64(&mut bytes, p + 8, offset as u64);
            put_u64(&mut bytes, p + 16, vaddr);
            put_u64(&mut bytes, p + 32, filesz as u64);
            put_u64(&mut bytes, p + 40, memsz as u64);
            put_u64(&mut bytes, p + 48, 4096);
            bytes[offset..offset + filesz].fill(0x40 + index as u8);
        }
        bytes
    }

    fn fake_byte(pa: u64, offset: usize) -> u8 {
        let mem = PTE_FAKE_MEM.lock().unwrap();
        let byte = pa as usize + offset;
        let word = mem[byte / 8];
        (word >> ((byte % 8) * 8)) as u8
    }

    fn test_format_machine(format: PteFormat) -> u16 {
        match format {
            PteFormat::Riscv => vifast_pte::riscv::ELF_MACHINE,
            PteFormat::X86 => vifast_pte::x86_64::ELF_MACHINE,
            PteFormat::AArch => vifast_pte::aarch64::ELF_MACHINE,
        }
    }

    fn elf_case(format: PteFormat) {
        reset_pte_fake();
        let ops = pte_ops(format);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(ops);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xf000, 4096, 32)
            .unwrap();
        let bytes = test_elf(test_format_machine(format));
        let image = vifast_elf::parse(
            &bytes,
            test_format_machine(format),
            vifast_pte::USER_VA_START,
            vifast_pte::USER_VA_END,
        )
        .unwrap();
        let base = vifast_abi::user_layout::ROOT_ELF_BASE as u64;
        let stack = StackSpec {
            base: vifast_abi::user_layout::ROOT_ELF_STACK_BASE as u64,
            pages: 4,
            guard: true,
        };
        let vspace = kernel
            .build_task_space_elf(untyped, None, &image, &bytes, stack)
            .unwrap();
        let root = kernel.root().unwrap();
        kernel.set_tcb_vspace(root, vspace).unwrap();
        assert_eq!(
            kernel.mapping_at(vspace, base).unwrap().rights,
            Rights::READ
        );
        assert_eq!(
            kernel.mapping_at(vspace, base + 0x1000).unwrap().rights,
            Rights::READ | Rights::EXEC
        );
        assert_eq!(
            kernel.mapping_at(vspace, base + 0x2000).unwrap().rights,
            Rights::READ | Rights::WRITE
        );

        // A segment starting in the middle of a page copies at its exact
        // virtual offset; the bytes before it remain zero.
        let mid = kernel.mapping_at(vspace, base + 0x5000).unwrap();
        assert_eq!(fake_byte(mid.pa, 0), 0);
        assert_eq!(fake_byte(mid.pa, 0x100), 0x40 + 3);

        // RW filesz is 0x100 but memsz spans three pages; the rest is BSS.
        let data = kernel.mapping_at(vspace, base + 0x2000).unwrap();
        assert_eq!(fake_byte(data.pa, 0xff), 0x40 + 2);
        assert_eq!(fake_byte(data.pa, 0x100), 0);
        let bss = kernel.mapping_at(vspace, base + 0x4000).unwrap();
        assert_eq!(fake_byte(bss.pa, 0), 0);
        assert_eq!(fake_byte(bss.pa, 0xfff), 0);

        // Guard is not mapped and validation rejects it.
        let guard = vifast_abi::user_layout::ROOT_ELF_STACK_BASE as u64 - 4096;
        assert!(kernel.mapping_at(vspace, guard).is_none());
        assert_eq!(
            kernel.validate_user_range(root, guard, 1, false),
            Err(KError::InvalidArg)
        );
        let root_pa = kernel.vspace(vspace).unwrap().root;
        assert_eq!(count_user_leaves(ops, root_pa, 0), 10);
    }

    #[test]
    fn elf_builder_riscv_maps_segments_and_guard() {
        let _guard = WALK_LOCK.lock().unwrap();
        elf_case(PteFormat::Riscv);
    }

    #[test]
    fn elf_builder_x86_64_maps_segments_and_guard() {
        let _guard = WALK_LOCK.lock().unwrap();
        elf_case(PteFormat::X86);
    }

    #[test]
    fn elf_builder_aarch64_maps_segments_and_guard() {
        let _guard = WALK_LOCK.lock().unwrap();
        elf_case(PteFormat::AArch);
    }

    #[test]
    fn elf_builder_extends_existing_vspace() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_pte_fake();
        let ops = pte_ops(PteFormat::X86);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(ops);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xf000, 4096, 32)
            .unwrap();
        let code_va = vifast_abi::user_layout::CODE_VA as u64;
        let existing = kernel
            .build_task_space(
                untyped,
                b"OLD",
                TaskLayout {
                    code_va,
                    stack_va: vifast_abi::user_layout::STACK_BASE as u64,
                    stack_pages: 0,
                    data: None,
                },
            )
            .unwrap();
        let bytes = test_elf(vifast_pte::x86_64::ELF_MACHINE);
        let image = vifast_elf::parse(
            &bytes,
            vifast_pte::x86_64::ELF_MACHINE,
            vifast_abi::user_layout::ROOT_ELF_BASE as u64,
            vifast_abi::user_layout::ROOT_ELF_STACK_GUARD as u64,
        )
        .unwrap();
        let extended = kernel
            .build_task_space_elf(
                untyped,
                Some(existing),
                &image,
                &bytes,
                StackSpec {
                    base: vifast_abi::user_layout::ROOT_ELF_STACK_BASE as u64,
                    pages: 1,
                    guard: true,
                },
            )
            .unwrap();
        assert_eq!(extended, existing);
        assert!(kernel.mapping_at(existing, code_va).is_some());
        assert!(kernel
            .mapping_at(existing, vifast_abi::user_layout::ROOT_ELF_BASE as u64)
            .is_some());
        let root_pa = kernel.vspace(existing).unwrap().root;
        assert_eq!(count_user_leaves(ops, root_pa, 0), 8);
    }

    #[test]
    fn elf_builder_extends_existing_vspace_and_rolls_back() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_pte_fake();
        let ops = pte_ops(PteFormat::X86);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(ops);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xf000, 4096, 16)
            .unwrap();
        let code_va = vifast_abi::user_layout::CODE_VA as u64;
        let existing = kernel
            .build_task_space(
                untyped,
                b"OLD",
                TaskLayout {
                    code_va,
                    stack_va: vifast_abi::user_layout::STACK_BASE as u64,
                    stack_pages: 0,
                    data: None,
                },
            )
            .unwrap();
        let bytes = test_elf(vifast_pte::x86_64::ELF_MACHINE);
        let image = vifast_elf::parse(
            &bytes,
            vifast_pte::x86_64::ELF_MACHINE,
            vifast_abi::user_layout::ROOT_ELF_BASE as u64,
            vifast_abi::user_layout::ROOT_ELF_STACK_GUARD as u64,
        )
        .unwrap();
        let before_bump = kernel.untyped(untyped).unwrap().bump;
        let before_objects = kernel.untyped(untyped).unwrap().objects;
        let root_pa = kernel.vspace(existing).unwrap().root;
        let before_tables = table_snapshot(ops, root_pa, 0);
        let error = kernel
            .build_task_space_elf(
                untyped,
                Some(existing),
                &image,
                &bytes,
                StackSpec {
                    base: vifast_abi::user_layout::ROOT_ELF_STACK_BASE as u64,
                    pages: 4,
                    guard: true,
                },
            )
            .unwrap_err();
        assert!(matches!(error, KError::OutOfMemory | KError::InvalidArg));
        assert_eq!(kernel.untyped(untyped).unwrap().bump, before_bump);
        assert_eq!(kernel.untyped(untyped).unwrap().objects, before_objects);
        assert!(kernel.mapping_at(existing, code_va).is_some());
        assert_eq!(count_user_leaves(ops, root_pa, 0), 1);
        let after_tables = table_snapshot(ops, root_pa, 0);
        assert_eq!(before_tables.len(), after_tables.len());
        for (index, (before, after)) in before_tables.iter().zip(after_tables.iter()).enumerate() {
            if before != after {
                panic!("page-table snapshot differs at {index}: {before:?} != {after:?}");
            }
        }
    }

    #[test]
    fn shared_code_builder_maps_the_same_frame_in_two_vspaces() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xa000, 4096, 16)
            .unwrap();
        let root_layout = TaskLayout {
            code_va: 0x10_0000_0000,
            stack_va: 0x10_0000_1000,
            stack_pages: 0,
            data: None,
        };
        let root_space = kernel
            .build_task_space(untyped, b"SHARED", root_layout)
            .unwrap();
        let code = kernel.mapping_at(root_space, root_layout.code_va).unwrap();
        let code_frame = kernel.frame_for_pa(code.pa).unwrap();
        let server_layout = TaskLayout {
            code_va: 0x10_0000_0000,
            stack_va: 0x10_0000_2000,
            stack_pages: 1,
            data: None,
        };
        let server_space = kernel
            .build_task_space_shared_code(untyped, code_frame, server_layout)
            .unwrap();
        assert_eq!(
            kernel.mapping_at(server_space, server_layout.code_va),
            Some(code)
        );
        assert_eq!(kernel.frame(code_frame).unwrap().mapped_w, 0);
        assert_eq!(kernel.frame(code_frame).unwrap().mapped_x, 2);
        assert_eq!(
            kernel
                .mapping_at(server_space, server_layout.stack_va)
                .unwrap()
                .rights,
            Rights::READ | Rights::WRITE
        );
        assert!(kernel.mapping_at(server_space, 0x10_0000_3000).is_none());
        // The server VSpace must contain exactly two user leaves: shared code
        // and its private stack.  An accidental extra root-stack mapping here
        // would make the isolation test pass for the wrong reason later.
        let root_pa = kernel.vspace(root_space).unwrap().root;
        let server_pa = kernel.vspace(server_space).unwrap().root;
        assert_eq!(count_user_leaves(&MAP_OPS, root_pa, 0), 1);
        assert_eq!(count_user_leaves(&MAP_OPS, server_pa, 0), 2);
    }

    #[test]
    fn shared_frame_is_unmapped_from_two_vspaces() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x4000_0000, 0x8000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0x1000, 4096, 2)
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
        let space_a = kernel
            .create_vspace_with_page_size(0x2000, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let space_b = kernel
            .create_vspace_with_page_size(0x3000, 0x4000_0000, 0x4000_0000, 4096)
            .unwrap();
        let va = 0x4000_1000;
        for (space, tables) in [
            (space_a, [0x4000, 0x6000, 0x8000]),
            (space_b, [0x5000, 0x7000, 0x9000]),
        ] {
            for table in tables {
                let pt = kernel.create_page_table(table, 0).unwrap();
                kernel.map_page_table(pt, space, va).unwrap();
            }
            kernel
                .map_frame(space, frame, va, Rights::READ | Rights::EXEC)
                .unwrap();
        }
        assert_eq!(FAKE_MEM.lock().unwrap()[4096 + 1], 0x100a);
        assert_eq!(FAKE_MEM.lock().unwrap()[4608 + 1], 0x100a);

        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(frame).is_none());
        assert!(kernel.mapping_at(space_a, va).is_none());
        assert!(kernel.mapping_at(space_b, va).is_none());
        assert_eq!(FAKE_MEM.lock().unwrap()[4096 + 1], 0);
        assert_eq!(FAKE_MEM.lock().unwrap()[4608 + 1], 0);
    }

    fn pte_fake_case(format: PteFormat) {
        reset_pte_fake();
        let ops = pte_ops(format);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(ops);
        let untyped = kernel
            .create_untyped_with_limit(0x1000, 0xa000, 4096, 16)
            .unwrap();
        let data_va = 0x10_0000_a000;
        let layout = TaskLayout {
            code_va: 0x10_0000_0000,
            stack_va: 0x10_0000_1000,
            stack_pages: 2,
            data: None,
        };
        let vspace = kernel.build_task_space(untyped, b"PTE", layout).unwrap();
        let root = kernel.root().unwrap();
        kernel.set_tcb_vspace(root, vspace).unwrap();
        assert!(kernel
            .validate_user_range(root, layout.code_va, 1, false)
            .is_ok());
        assert_eq!(
            kernel.validate_user_range(root, layout.code_va, 1, true),
            Err(KError::InvalidArg)
        );
        assert!(kernel
            .validate_user_range(root, layout.stack_va, 1, true)
            .is_ok());
        assert!(kernel.mapping_at(vspace, data_va).is_none());
        assert_eq!(
            kernel.validate_user_range(root, data_va, 1, true),
            Err(KError::InvalidArg)
        );

        // The code/stack mappings already installed the final-level table for
        // +0xa000.  The client can therefore FrameMap there without issuing
        // PageTableMap itself.
        let frame = kernel.retype(untyped, ObjType::Frame).unwrap();
        assert!(kernel
            .map_frame(vspace, frame, data_va, Rights::READ | Rights::WRITE)
            .is_ok());
        assert!(kernel.validate_user_range(root, data_va, 1, true).is_ok());
        assert_eq!(
            kernel.validate_user_range(root, 0x8000_0000, 1, false),
            Err(KError::InvalidArg)
        );
        kernel.unmap_frame(vspace, layout.code_va).unwrap();
        assert_eq!(
            kernel.validate_user_range(root, layout.code_va, 1, false),
            Err(KError::InvalidArg)
        );

        // A user leaf above the final level is never accepted, even if it has
        // the user bit and R/W rights.
        reset_pte_fake();
        let va = vifast_pte::USER_VA_START;
        let (idx, pte) = match format {
            PteFormat::Riscv => (
                vifast_pte::riscv::index(va, 0),
                ops.encode_leaf(0x9000, Rights::READ | Rights::WRITE),
            ),
            PteFormat::X86 => (
                vifast_pte::x86_64::index(va, 0),
                ops.encode_leaf(0x9000, Rights::READ | Rights::WRITE) | vifast_pte::x86_64::PS,
            ),
            PteFormat::AArch => (
                vifast_pte::aarch64::index(va, 0),
                ops.encode_leaf(0x9000, Rights::READ | Rights::WRITE) & !0b10,
            ),
        };
        ops.write_pte(0, idx, pte);
        assert!(!Kernel::user_leaf_ok(ops, 0, va, false));
    }

    #[test]
    fn pte_fake_riscv_builds_and_validates() {
        let _guard = WALK_LOCK.lock().unwrap();
        pte_fake_case(PteFormat::Riscv);
    }

    #[test]
    fn pte_fake_x86_64_builds_and_validates() {
        let _guard = WALK_LOCK.lock().unwrap();
        pte_fake_case(PteFormat::X86);
    }

    #[test]
    fn pte_fake_aarch64_builds_and_validates() {
        let _guard = WALK_LOCK.lock().unwrap();
        pte_fake_case(PteFormat::AArch);
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
    fn untyped_child_range_and_quota_are_checked() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x4000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let before = *kernel.untyped(parent).unwrap();

        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                2,
                0x5000,
                1,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::OutOfMemory,
            }
        );
        assert_eq!(*kernel.untyped(parent).unwrap(), before);
        assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).is_none());
        assert!(kernel.check_invariants().is_ok());

        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                2,
                0x1000,
                4,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::OutOfMemory,
            }
        );
        assert_eq!(*kernel.untyped(parent).unwrap(), before);
        assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn untyped_sibling_children_do_not_share_pa() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x8000, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        for (slot, size) in [(2, 0x2000usize), (3, 0x2000usize)] {
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    1,
                    ObjType::Untyped as u64,
                    slot,
                    size as u64,
                    1,
                    0
                ),
                UserAction::Return {
                    value: slot as u64,
                    error: KError::Ok,
                }
            );
            assert!(kernel.check_invariants().is_ok());
        }
        let child_a = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        let child_b = kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).unwrap().obj;
        let a = *kernel.untyped(child_a).unwrap();
        let b = *kernel.untyped(child_b).unwrap();
        assert!(a.base + a.size <= b.base || b.base + b.size <= a.base);

        for (src, dest) in [(2, 4), (3, 5)] {
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    src,
                    ObjType::Frame as u64,
                    dest,
                    0,
                    0,
                    0
                ),
                UserAction::Return {
                    value: dest as u64,
                    error: KError::Ok,
                }
            );
            assert!(kernel.check_invariants().is_ok());
        }
        let frame_a = kernel.cap_at(SlotAddr { tcb: root, slot: 4 }).unwrap().obj;
        let frame_b = kernel.cap_at(SlotAddr { tcb: root, slot: 5 }).unwrap().obj;
        let pa_a = kernel.frame(frame_a).unwrap().pa as u64;
        let pa_b = kernel.frame(frame_b).unwrap().pa as u64;
        assert_ne!(pa_a, pa_b);
        assert!(pa_a >= a.base as u64 && pa_a < (a.base + a.size) as u64);
        assert!(pa_b >= b.base as u64 && pa_b < (b.base + b.size) as u64);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn untyped_child_frame_stays_in_child_range() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x4000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                2,
                0x2000,
                2,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok,
            }
        );
        let child = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        let region = *kernel.untyped(child).unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                2,
                ObjType::Frame as u64,
                3,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 3,
                error: KError::Ok,
            }
        );
        let frame = kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).unwrap().obj;
        let pa = kernel.frame(frame).unwrap().pa as u64;
        assert!(pa >= region.base as u64 && pa < (region.base + region.size) as u64);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn revoking_untyped_parent_destroys_child_mapping_and_reuses_zeroed_pa() {
        let _guard = WALK_LOCK.lock().unwrap();
        for format in [PteFormat::Riscv, PteFormat::X86, PteFormat::AArch] {
            reset_pte_fake();
            let ops = pte_ops(format);
            let mut kernel = Kernel::new();
            kernel.set_page_table_ops(ops);
            let root = kernel.root().unwrap();
            let parent = kernel
                .create_untyped_with_limit(0x2000, 0x4000, 4096, 4)
                .unwrap();
            kernel
                .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
                .unwrap();
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    1,
                    ObjType::Untyped as u64,
                    2,
                    0x2000,
                    1,
                    0
                ),
                UserAction::Return {
                    value: 2,
                    error: KError::Ok,
                }
            );
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    2,
                    ObjType::Frame as u64,
                    3,
                    0,
                    0,
                    0
                ),
                UserAction::Return {
                    value: 3,
                    error: KError::Ok,
                }
            );
            let child = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
            let frame = kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).unwrap().obj;
            let old_pa = kernel.frame(frame).unwrap().pa as u64;

            let va = 0x10_0000_0000;
            let space = kernel
                .create_vspace_with_page_size(0xa000, va, 0x2000, 4096)
                .unwrap();
            for index in 0..ops.levels().saturating_sub(1) {
                let table = 0x6000u64 + index as u64 * 0x1000;
                let pt = kernel.create_page_table(table, 0).unwrap();
                assert!(kernel.map_page_table(pt, space, va).is_ok());
            }
            let mapping = kernel
                .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
                .unwrap();
            assert_eq!(kernel.frame(frame).unwrap().mapped_w, 1);
            let word = ((old_pa >> 12) as usize) * 512;
            PTE_FAKE_MEM.lock().unwrap()[word] = 0xDEAD_BEEF;
            assert!(kernel.check_invariants().is_ok());

            kernel.revoke_cap(root, 1).unwrap();
            assert!(kernel.untyped(child).is_none());
            assert!(kernel.frame(frame).is_none());
            assert!(kernel.mapping_at(space, mapping.va).is_none());
            assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).is_none());
            assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).is_none());
            let region = *kernel.untyped(parent).unwrap();
            assert_eq!(region.objects, 0);
            assert_eq!(region.reserved, 0);
            assert_eq!(region.bump, 0);
            assert!(kernel.check_invariants().is_ok());

            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    1,
                    ObjType::Frame as u64,
                    4,
                    0,
                    0,
                    0
                ),
                UserAction::Return {
                    value: 4,
                    error: KError::Ok,
                }
            );
            let replacement = kernel.cap_at(SlotAddr { tcb: root, slot: 4 }).unwrap().obj;
            assert_eq!(kernel.frame(replacement).unwrap().pa as u64, old_pa);
            assert_eq!(PTE_FAKE_MEM.lock().unwrap()[word], 0);
            assert!(kernel.check_invariants().is_ok());
        }
    }

    #[test]
    fn revoking_child_untyped_leaves_parent_objects_alive() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x4000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
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
                error: KError::Ok,
            }
        );
        let frame = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                3,
                0x1000,
                1,
                0
            ),
            UserAction::Return {
                value: 3,
                error: KError::Ok,
            }
        );
        let child = kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).unwrap().obj;
        let bump_before = kernel.untyped(parent).unwrap().bump;
        kernel.revoke_cap(root, 3).unwrap();
        assert!(kernel.get(child).is_some());
        assert_eq!(kernel.untyped(child).unwrap().bump, 0);
        assert!(kernel.get(frame).is_some());
        let region = *kernel.untyped(parent).unwrap();
        assert_eq!(region.objects, 2);
        assert_eq!(region.reserved, 1);
        assert_eq!(region.bump, bump_before);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn untyped_child_caps_and_copies_are_not_root() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x2000, 4096, 4)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                2,
                0x1000,
                1,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok,
            }
        );
        let child_cap = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap();
        assert!(!child_cap.root);
        assert!(child_cap.parent.is_some());
        assert_eq!(kernel.delete_cap(root, 1), Err(KError::IllegalOperation));
        kernel.copy_cap(root, 2, root, 4, Rights::ALL, 0).unwrap();
        assert!(!kernel.cap_at(SlotAddr { tcb: root, slot: 4 }).unwrap().root);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn revoking_nested_untyped_collects_all_descendants() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x8000, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        for (src, slot, size, max) in [(1, 2, 0x4000usize, 4usize), (2, 3, 0x2000, 2)] {
            assert_eq!(
                kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    src,
                    ObjType::Untyped as u64,
                    slot,
                    size as u64,
                    max as u64,
                    0
                ),
                UserAction::Return {
                    value: slot as u64,
                    error: KError::Ok,
                }
            );
            assert!(kernel.check_invariants().is_ok());
        }
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                3,
                ObjType::Frame as u64,
                4,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 4,
                error: KError::Ok,
            }
        );
        let child = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap().obj;
        let grandchild = kernel.cap_at(SlotAddr { tcb: root, slot: 3 }).unwrap().obj;
        let frame = kernel.cap_at(SlotAddr { tcb: root, slot: 4 }).unwrap().obj;
        assert!(kernel.check_invariants().is_ok());
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(child).is_none());
        assert!(kernel.get(grandchild).is_none());
        assert!(kernel.get(frame).is_none());
        assert_eq!(kernel.untyped(parent).unwrap().reserved, 0);
        assert_eq!(kernel.untyped(parent).unwrap().objects, 0);
        assert_eq!(kernel.untyped(parent).unwrap().bump, 0);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn child_cap_reparents_when_holder_tcb_dies() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x100_0000, 0x4000, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Untyped as u64,
                2,
                0x2000,
                2,
                0
            ),
            UserAction::Return {
                value: 2,
                error: KError::Ok,
            }
        );
        let child_cap = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap();
        let child = child_cap.obj;
        let client = kernel.create_tcb("client", 1).unwrap();
        kernel.copy_cap(root, 2, client, 1, Rights::ALL, 0).unwrap();
        let frame = kernel
            .retype_as(
                child,
                ObjType::Frame,
                Some(SlotAddr {
                    tcb: client,
                    slot: 1,
                }),
            )
            .unwrap();
        assert!(kernel.check_invariants().is_ok());
        kernel.destroy_object(client).unwrap();
        assert_eq!(
            kernel.get(frame).unwrap().creator,
            Some(CapRef {
                addr: SlotAddr { tcb: root, slot: 2 },
                id: child_cap.id,
            })
        );
        assert!(kernel.check_invariants().is_ok());
        kernel.revoke_cap(root, 1).unwrap();
        assert!(kernel.get(child).is_none());
        assert!(kernel.get(frame).is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn reply_recv_without_caller_only_receives() {
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 1).unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let reply = Message::new(0x55, &[42]).unwrap();
        assert_eq!(kernel.reply_recv(server, endpoint, 4, reply), Ok(None));
        assert_eq!(kernel.tcb(server).unwrap().state, ThreadState::BlockedRecv);

        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, endpoint, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        assert!(kernel.tcb(server).unwrap().caller.is_some());
        assert_eq!(kernel.pick(), Some(server));

        let reply = Message::new(0x55, &[42]).unwrap();
        assert_eq!(kernel.reply_recv(server, endpoint, 4, reply), Ok(None));
        assert_eq!(kernel.tcb(server).unwrap().state, ThreadState::BlockedRecv);
        assert_eq!(kernel.take_resume(client).unwrap().label, 0x55);
        assert_eq!(kernel.pick(), Some(client));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn reply_recv_replies_before_taking_next_caller() {
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 1).unwrap();
        let first = kernel.create_tcb("first", 1).unwrap();
        let second = kernel.create_tcb("second", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(
            kernel.reply_recv(server, endpoint, 4, Message::empty(0)),
            Ok(None)
        );
        let call_a = Message::new(0xA, &[1]).unwrap();
        assert_eq!(kernel.send(first, endpoint, 0, call_a, true), Ok(true));
        assert_eq!(kernel.take_resume(server).unwrap().label, 0xA);
        assert!(kernel.tcb(server).unwrap().caller.is_some());
        assert_eq!(kernel.pick(), Some(server));

        let call_b = Message::new(0xB, &[2]).unwrap();
        assert_eq!(kernel.send(second, endpoint, 0, call_b, true), Ok(false));
        assert_eq!(kernel.tcb(second).unwrap().state, ThreadState::BlockedSend);
        let reply = Message::new(0x55, &[42]).unwrap();
        let next = kernel
            .reply_recv(server, endpoint, 4, reply)
            .unwrap()
            .unwrap();
        assert_eq!(next.label, 0xB);
        assert_eq!(kernel.take_resume(first).unwrap().label, 0x55);
        assert_eq!(kernel.tcb(server).unwrap().caller.unwrap().0, second);
        assert_eq!(kernel.tcb(second).unwrap().state, ThreadState::BlockedReply);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn reply_recv_rejects_stale_caller_epoch() {
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 1).unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(
            kernel.reply_recv(server, endpoint, 4, Message::empty(0)),
            Ok(None)
        );
        let call = Message::new(0xAB, &[1]).unwrap();
        assert_eq!(kernel.send(client, endpoint, 0, call, true), Ok(true));
        assert_eq!(kernel.take_resume(server).unwrap().label, 0xAB);
        let (server_caller, _) = kernel.tcb(server).unwrap().caller.unwrap();
        let stale_epoch = kernel.tcb(server).unwrap().reply_epoch;
        kernel.tcb_mut(server).unwrap().caller = Some((server_caller, stale_epoch.wrapping_add(1)));
        assert_eq!(
            kernel.reply_recv(server, endpoint, 4, Message::empty(0)),
            Err(KError::NoCaller)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn fault_endpoint_delivers_to_waiting_handler() {
        let mut kernel = Kernel::new();
        let faulting = kernel.create_tcb("faulting", 1).unwrap();
        let handler = kernel.create_tcb("handler", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(
                faulting,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        let cap = kernel
            .cap_at(SlotAddr {
                tcb: faulting,
                slot: 1,
            })
            .unwrap();
        kernel
            .set_fault_endpoint(
                faulting,
                Some(CapRef {
                    addr: SlotAddr {
                        tcb: faulting,
                        slot: 1,
                    },
                    id: cap.id,
                }),
            )
            .unwrap();
        assert_eq!(kernel.fault_endpoint(faulting), Some(endpoint));
        assert_eq!(kernel.recv(handler, endpoint, 4), Ok(None));
        let message = Message::new(LABEL_FAULT, &[0x1234, 7, 0, 0]).unwrap();
        kernel
            .fault_to_endpoint(faulting, endpoint, message)
            .unwrap();
        assert_eq!(
            kernel.tcb(faulting).unwrap().state,
            ThreadState::BlockedFault
        );
        let received = kernel.take_resume(handler).unwrap();
        assert_eq!(received.label, LABEL_FAULT);
        assert_eq!(received.words()[0], 0x1234);
        let check = kernel.check_invariants();
        assert!(check.is_ok(), "{check:?}");
    }

    #[test]
    fn fault_endpoint_queues_when_handler_is_not_waiting() {
        let mut kernel = Kernel::new();
        let faulting = kernel.create_tcb("faulting", 1).unwrap();
        let handler = kernel.create_tcb("handler", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let message = Message::new(LABEL_FAULT, &[0x55, 1, 0, 0]).unwrap();
        kernel
            .fault_to_endpoint(faulting, endpoint, message)
            .unwrap();
        assert_eq!(
            kernel.tcb(faulting).unwrap().state,
            ThreadState::BlockedFault
        );
        let received = kernel.recv(handler, endpoint, 4).unwrap().unwrap();
        assert_eq!(received.label, LABEL_FAULT);
        assert_eq!(received.words()[0], 0x55);
        assert_eq!(
            kernel.tcb(faulting).unwrap().state,
            ThreadState::BlockedFault
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn revoking_fault_endpoint_cap_clears_link() {
        let mut kernel = Kernel::new();
        let faulting = kernel.create_tcb("faulting", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(
                faulting,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        let cap = kernel
            .cap_at(SlotAddr {
                tcb: faulting,
                slot: 1,
            })
            .unwrap();
        kernel
            .set_fault_endpoint(
                faulting,
                Some(CapRef {
                    addr: SlotAddr {
                        tcb: faulting,
                        slot: 1,
                    },
                    id: cap.id,
                }),
            )
            .unwrap();
        let message = Message::new(LABEL_FAULT, &[0x99, 1, 0, 0]).unwrap();
        kernel
            .fault_to_endpoint(faulting, endpoint, message)
            .unwrap();
        assert!(kernel.delete_cap(faulting, 1).is_ok());
        assert_eq!(kernel.fault_endpoint(faulting), None);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn frame_write_copies_bytes_and_rejects_overflow() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_pte_fake();
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(pte_ops(PteFormat::X86));
        let frame = kernel.create_frame(0x2000, 4096).unwrap();
        assert_eq!(kernel.copy_frame_bytes(frame, 0, b"abc"), Ok(()));
        assert_eq!(fake_byte(0x2000, 0), b'a');
        assert_eq!(fake_byte(0x2000, 2), b'c');
        assert_eq!(
            kernel.copy_frame_bytes(frame, 4095, &[1, 2]),
            Err(KError::InvalidArg)
        );
    }

    #[test]
    fn ipc_buffer_requires_rw_mapping_and_copies_words() {
        let _guard = WALK_LOCK.lock().unwrap();
        reset_fake();
        set_map_range(0x10_0000_0000, 0x20_0000_0000);
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&MAP_OPS);
        let sender = kernel.create_tcb("sender", 1).unwrap();
        let receiver = kernel.create_tcb("receiver", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let va = 0x10_0000_0000;
        let sender_space = kernel
            .create_vspace_with_page_size(0x1000, va, 0x1000, 4096)
            .unwrap();
        let receiver_space = kernel
            .create_vspace_with_page_size(0x2000, va, 0x1000, 4096)
            .unwrap();
        let sender_frame = kernel.create_frame(0x9000, 4096).unwrap();
        let receiver_frame = kernel.create_frame(0xa000, 4096).unwrap();
        kernel.set_tcb_vspace(sender, sender_space).unwrap();
        kernel.set_tcb_vspace(receiver, receiver_space).unwrap();
        for (space, tables) in [
            (sender_space, [0x3000, 0x4000, 0x5000]),
            (receiver_space, [0x6000, 0x7000, 0x8000]),
        ] {
            for table in tables {
                let pt = kernel.create_page_table(table, 0).unwrap();
                assert!(kernel.map_page_table(pt, space, va).is_ok());
            }
        }
        assert_eq!(
            kernel.set_ipc_buffer(sender, sender_frame, va),
            Err(KError::NotMapped)
        );
        kernel
            .map_frame(sender_space, sender_frame, va, Rights::READ)
            .unwrap();
        assert_eq!(
            kernel.set_ipc_buffer(sender, sender_frame, va),
            Err(KError::InvalidArg)
        );
        kernel.unmap_frame(sender_space, va).unwrap();
        kernel
            .map_frame(sender_space, sender_frame, va, Rights::READ | Rights::WRITE)
            .unwrap();
        kernel
            .map_frame(
                receiver_space,
                receiver_frame,
                va,
                Rights::READ | Rights::WRITE,
            )
            .unwrap();
        assert!(kernel.set_ipc_buffer(sender, sender_frame, va).is_ok());
        assert!(kernel.set_ipc_buffer(receiver, receiver_frame, va).is_ok());
        let source_words = [0x1122_3344_5566_7788u64, 0x99aa_bbcc_ddee_ff00];
        let mut source_bytes = [0u8; 16];
        for (index, word) in source_words.iter().enumerate() {
            source_bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        MAP_OPS.copy_to_frame(0x9000, &source_bytes);
        assert_eq!(kernel.recv(receiver, endpoint, 4), Ok(None));
        assert!(kernel
            .send(
                sender,
                endpoint,
                0,
                Message::new(0x77, &[1]).unwrap(),
                false
            )
            .unwrap());
        assert_eq!(kernel.take_resume(receiver).unwrap().label, 0x77);
        let (d0, d1) = {
            let mem = FAKE_MEM.lock().unwrap();
            let base = ((0xa000 >> 12) as usize) * 512;
            (mem[base], mem[base + 1])
        };
        assert_eq!(d0, source_words[0]);
        assert_eq!(d1, source_words[1]);
        assert!(kernel.check_invariants().is_ok());

        kernel.unmap_frame(sender_space, va).unwrap();
        assert!(kernel.tcb(sender).unwrap().ipc_buffer.is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn syscall_fuzz_three_pte_formats_keeps_invariants() {
        let _guard = WALK_LOCK.lock().unwrap();
        let mut seed = 0x5eed_1234_abcd_9876u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            seed
        };
        for format in [PteFormat::Riscv, PteFormat::X86, PteFormat::AArch] {
            reset_pte_fake();
            let ops = pte_ops(format);
            let mut kernel = Kernel::new();
            kernel.set_page_table_ops(ops);
            let root = kernel.root().unwrap();
            let untyped = kernel
                .create_untyped_with_limit(0x4000, 0xb000, 4096, 64)
                .unwrap();
            let space = kernel
                .create_vspace_with_page_size(0x1000, 0x10_0000_0000, 0x1000, 4096)
                .unwrap();
            let frame = kernel.create_frame(0x2000, 4096).unwrap();
            let table = kernel.create_page_table(0x3000, 0).unwrap();
            for (slot, object, ty, rights) in [
                (1, untyped, ObjType::Untyped, Rights::ALL),
                (4, space, ObjType::VSpace, Rights::ALL),
                (5, frame, ObjType::Frame, Rights::ALL),
                (6, table, ObjType::PageTable, Rights::ALL),
            ] {
                kernel
                    .install_cap(root, slot, Cap::new(object, ty, rights))
                    .unwrap();
            }
            let types = [
                ObjType::Frame,
                ObjType::PageTable,
                ObjType::VSpace,
                ObjType::Tcb,
                ObjType::Endpoint,
                ObjType::Notification,
                ObjType::Untyped,
            ];
            let mut successes = 0usize;
            for step in 0..10_000usize {
                let choice = next() % 7;
                let slot = 7 + (next() as usize % 25);
                let other = 7 + (next() as usize % 25);
                let va = 0x10_0000_0000u64 + (next() & 0x3f) * 0x1000;
                let rights = Rights((next() as u8) & Rights::ALL.0);
                let (number, a0, a1, a2, a3, a4, a5) = match choice {
                    0 => (
                        SyscallId::Retype as u64,
                        1,
                        types[(next() as usize) % types.len()] as u64,
                        slot as u64,
                        (next() & 0x1fff) as u64,
                        (next() & 0x3f) as u64,
                        0,
                    ),
                    1 => (
                        SyscallId::CapCopy as u64,
                        1 + (next() % 31) as u64,
                        other as u64,
                        slot as u64,
                        rights.0 as u64,
                        0,
                        0,
                    ),
                    2 => (SyscallId::CapDelete as u64, slot as u64, 0, 0, 0, 0, 0),
                    3 => (SyscallId::CapRevoke as u64, slot as u64, 0, 0, 0, 0, 0),
                    4 => (
                        SyscallId::FrameMap as u64,
                        4,
                        if next() & 1 == 0 { 5 } else { other as u64 },
                        va,
                        rights.0 as u64,
                        0,
                        0,
                    ),
                    5 => (SyscallId::FrameUnmap as u64, 4, va, 0, 0, 0, 0),
                    _ => (
                        SyscallId::PageTableMap as u64,
                        if next() & 1 == 0 { 6 } else { other as u64 },
                        4,
                        va,
                        0,
                        0,
                        0,
                    ),
                };
                if let UserAction::Return {
                    error: KError::Ok, ..
                } = kernel.user_syscall(root, number, a0, a1, a2, a3, a4, a5)
                {
                    successes += 1;
                }
                assert_eq!(
                    kernel.check_invariants(),
                    Ok(()),
                    "format {:?} failed at step {}",
                    format,
                    step
                );
            }
            assert!(
                successes > 100,
                "format {:?} only produced {} successful syscalls",
                format,
                successes
            );
        }
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
    fn tcb_configuration_sets_context_and_resumes() {
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let child = kernel.create_tcb("child", 1).unwrap();
        assert_eq!(
            kernel.configure_tcb(child, 0x4000_0000, 0x4000_5000, space, 5),
            Ok(())
        );
        assert_eq!(kernel.resume_tcb(child), Ok(()));
        assert!(kernel.tcb(child).is_some_and(|tcb| tcb.queued));
        assert_eq!(kernel.resume_tcb(child), Ok(()));
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
