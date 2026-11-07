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

/// Lấy slot từ tham số syscall; slot không hợp lệ (> 255) trả `InvalidArg`
/// thay vì lặng lẽ rơi về slot 0 (vòng 84, audit: 53 chỗ `unwrap_or(0)` trước
/// đây biến một lời gọi sai thành thao tác trên self-TCB của caller).
macro_rules! slot_or_invalid {
    ($value:expr) => {
        match slot_arg($value) {
            Ok(slot) => slot,
            Err(error) => return UserAction::Return { value: 0, error },
        }
    };
}

/// Number of hardware threads whose current TCB is tracked by the scheduler.
pub const MAX_HARTS: usize = 4;

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
    /// Rebuild a TCB context at its configured entry/stack.
    RestartTcb {
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
    /// A TCB was suspended; wake its previous hart if it was running.
    TcbSuspended {
        hart: Option<usize>,
    },
    /// Replace an existing leaf mapping in place (test/remap path).
    FrameRemap {
        space: ObjRef,
        frame: ObjRef,
        va: u64,
        rights: Rights,
    },
    /// Read one 512-byte block sector into a user buffer.
    BlockRead {
        lba: u64,
        ptr: u64,
    },
    /// Write one 512-byte block sector from a user buffer.
    BlockWrite {
        lba: u64,
        ptr: u64,
    },
    /// Flush the block device's write cache.
    BlockFlush,
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
    /// Sleep until the timer deadline installed by the kernel.
    Sleep,
    /// Poll one console byte; `WouldBlock` means no input is available.
    ConsoleRead,
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
    /// Flush a VA range tagged by `asid` on the local hart and on selected
    /// remote harts.  The default preserves the old local-only behaviour for
    /// fake page-table implementations.
    fn flush_va_asid(&self, _asid: u16, _va: u64, _pages: usize, _targets: u32) {
        self.flush_va(_va);
    }
    /// Full-ASID fallback.  Default keeps old full-user behavior.
    fn flush_all_asid(&self, _asid: u16, _targets: u32) {
        self.flush_all_user();
    }
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
    /// One fixed-capacity ready queue per hart.  A TCB is enqueued only once;
    /// an idle hart may steal an eligible TCB from another queue.
    pub(crate) runq: [[Option<ObjRef>; MAX_TCBS]; MAX_HARTS],
    pub(crate) runq_len: [usize; MAX_HARTS],
    pub(crate) sched_local: [u64; MAX_HARTS],
    pub(crate) sched_steals: [u64; MAX_HARTS],
    pub(crate) current: [Option<ObjRef>; MAX_HARTS],
    pub(crate) seq: u64,
    pub(crate) ticks: [u64; MAX_HARTS],
    pub(crate) system_ticks: core::sync::atomic::AtomicU64,
    pub(crate) root: Option<ObjRef>,
    pub(crate) fs: fs::RamFs,
    pub(crate) stats: KernelStats,
    pub(crate) page_table_ops: Option<&'static dyn PageTableOps>,
    /// Root's cspace is separate so ordinary TCBs stay compact.
    pub(crate) root_cspace: [Option<Cap>; MAX_CAPS_ROOT],
    pub(crate) next_cap_id: u64,
    pub(crate) next_asid: u16,
}

// Vòng 84 (audit #3): biến "ngân sách static" thành ràng buộc lúc biên dịch
// thay vì chỉ kiểm tra ở runtime. Trên AArch64 `Kernel` nằm sát `BKL` (khe
// 8 byte), nên mọi thay đổi làm nó phình quá ngân sách phải fail ngay khi
// build chứ không phải lúc boot trên một kiến trúc.
const _: () = assert!(
    core::mem::size_of::<Kernel>() <= KERNEL_STATIC_BUDGET,
    "Kernel vượt KERNEL_STATIC_BUDGET: tăng ngân sách chỉ khi đã kiểm tra khe \
     tĩnh của AArch64 (build.sh in gap=)"
);

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel {
    pub const fn new_const() -> Kernel {
        Kernel {
            objs: [Object::free(); MAX_OBJECTS],
            runq: [[None; MAX_TCBS]; MAX_HARTS],
            runq_len: [0; MAX_HARTS],
            sched_local: [0; MAX_HARTS],
            sched_steals: [0; MAX_HARTS],
            current: [None; MAX_HARTS],
            seq: 0,
            ticks: [0; MAX_HARTS],
            system_ticks: core::sync::atomic::AtomicU64::new(0),
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
            root_cspace: [None; MAX_CAPS_ROOT],
            next_cap_id: 1,
            next_asid: 1,
        }
    }

    /// Reinitialize a statically allocated kernel in place.  This avoids
    /// constructing a large `Kernel` temporary on the boot stack.
    pub fn reset(&mut self) {
        for object in self.objs.iter_mut() {
            *object = Object::free();
        }
        self.runq = [[None; MAX_TCBS]; MAX_HARTS];
        self.runq_len = [0; MAX_HARTS];
        self.sched_local = [0; MAX_HARTS];
        self.sched_steals = [0; MAX_HARTS];
        self.current = [None; MAX_HARTS];
        self.seq = 0;
        self.ticks = [0; MAX_HARTS];
        self.system_ticks
            .store(0, core::sync::atomic::Ordering::Release);
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
        self.root_cspace = [None; MAX_CAPS_ROOT];
        self.next_cap_id = 1;
        self.next_asid = 1;
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

    pub(crate) fn cspace(&self, tcb: ObjRef) -> Option<&[Option<Cap>]> {
        if self.root == Some(tcb) {
            Some(&self.root_cspace)
        } else {
            let table = self.tcb(tcb)?;
            if let Some(cnode) = table.cnode {
                self.cnode(cnode).map(|node| node.slots.as_slice())
            } else {
                Some(table.cspace.as_slice())
            }
        }
    }

    pub(crate) fn cspace_mut(&mut self, tcb: ObjRef) -> Option<&mut [Option<Cap>]> {
        if self.root == Some(tcb) {
            Some(&mut self.root_cspace)
        } else {
            let cnode = self.tcb(tcb)?.cnode;
            if let Some(cnode) = cnode {
                self.cnode_mut(cnode).map(|node| node.slots.as_mut_slice())
            } else {
                self.tcb_mut(tcb).map(|table| table.cspace.as_mut_slice())
            }
        }
    }

    pub(crate) fn cspace_capacity(&self, tcb: ObjRef) -> usize {
        if self.root == Some(tcb) {
            MAX_CAPS_ROOT
        } else {
            self.tcb(tcb)
                .and_then(|table| table.cnode)
                .and_then(|cnode| self.cnode(cnode))
                .map(CNode::capacity)
                .unwrap_or(MAX_CAPS_TASK)
        }
    }

    /// Map a Frame into a software IOMMU domain.  DMA mappings are
    /// read/write only (never executable) and use the domain's IOVA window.
    pub fn iommu_map(
        &mut self,
        domain: ObjRef,
        frame_obj: ObjRef,
        iova: u64,
        rights: Rights,
    ) -> KResult<IommuMapping> {
        let frame = *self.frame(frame_obj).ok_or(KError::WrongType)?;
        let allowed = Rights::READ | Rights::WRITE;
        if rights.is_empty() || !allowed.contains(rights) {
            return Err(KError::InvalidArg);
        }
        if frame.pa as u64 % DEFAULT_PAGE_SIZE != 0
            || (frame.size as u64) < DEFAULT_PAGE_SIZE
            || iova % DEFAULT_PAGE_SIZE != 0
            || iova < IOMMU_IOVA_BASE
            || iova.saturating_add(DEFAULT_PAGE_SIZE) > IOMMU_IOVA_BASE.saturating_add(0x4000_0000)
        {
            return Err(KError::Alignment);
        }
        if rights.contains(Rights::WRITE) && frame.mapped_x != 0 {
            return Err(KError::NoRights);
        }
        let mapping = IommuMapping {
            iova,
            pa: frame.pa as u64,
            len: DEFAULT_PAGE_SIZE,
            rights,
            frame: frame_obj,
        };
        self.iommu_mut(domain)
            .ok_or(KError::WrongType)?
            .insert(mapping)?;
        Ok(mapping)
    }

    pub fn iommu_unmap(&mut self, domain: ObjRef, iova: u64) -> KResult<IommuMapping> {
        self.iommu_mut(domain)
            .ok_or(KError::WrongType)?
            .remove(iova)
    }

    fn unmap_iommu_frame_everywhere(&mut self, frame: ObjRef) {
        for index in 0..MAX_OBJECTS {
            let obj = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if let Some(domain) = self.iommu_mut(obj) {
                domain.remove_frame(frame);
            }
        }
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

    /// Hardware address-space tag for the TCB's current VSpace.
    pub fn current_asid(&self, tcb: ObjRef) -> Option<u16> {
        let space = self.tcb(tcb)?.vspace?;
        Some(self.vspace(space)?.asid)
    }

    pub fn note_vspace_hart_loaded(&mut self, tcb: ObjRef, hart: usize) {
        if hart >= MAX_HARTS {
            return;
        }
        let Some(space_ref) = self.tcb(tcb).and_then(|table| table.vspace) else {
            return;
        };
        if let Some(space) = self.vspace_mut(space_ref) {
            space.harts_loaded |= 1 << hart;
        }
    }

    pub fn vspace_harts_loaded(&self, vspace: ObjRef) -> u32 {
        self.vspace(vspace)
            .map(|space| space.harts_loaded)
            .unwrap_or(0)
    }

    pub fn clear_vspace_harts(&mut self, vspace: ObjRef, mask: u32) -> u32 {
        if let Some(space) = self.vspace_mut(vspace) {
            space.harts_loaded &= !mask;
            space.harts_loaded
        } else {
            0
        }
    }

    fn asid_in_use(&self, asid: u16) -> bool {
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self
                .vspace(reference)
                .is_some_and(|space| space.asid == asid)
            {
                return true;
            }
        }
        false
    }

    fn take_asid(&mut self) -> u16 {
        // Skip ASIDs still owned by a live VSpace when the counter wraps.
        // With MAX_OBJECTS < 4095 this normally returns on the first try.
        for _ in 0..0x0fff {
            let asid = self.next_asid;
            self.next_asid = if asid >= 0x0fff { 1 } else { asid + 1 };
            if !self.asid_in_use(asid) {
                return asid;
            }
        }
        // ASID 0 is reserved by all three ports; never hand it out.
        0
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

    /// Allocate one IRQ object.  Only one live object may represent a given
    /// logical IRQ at a time, so notification routing is unambiguous.
    pub fn create_irq(&mut self, number: u32) -> Option<ObjRef> {
        if self
            .objs
            .iter()
            .any(|object| matches!(object.kind, Kind::Irq(irq) if irq.number == number))
        {
            return None;
        }
        self.alloc(Kind::Irq(Irq {
            number,
            notification: None,
            pending: false,
        }))
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
        let slot = self.free_context_slot()?;
        let object = self.alloc(Kind::Tcb(Tcb::new(name, priority)))?;
        if let Some(tcb) = self.tcb_mut(object) {
            tcb.context_slot = slot;
        }
        Some(object)
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
            free: [None; MAX_UNTYPED_FREE],
            leaked: 0,
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
        let asid = self.take_asid();
        let object = self.alloc(Kind::VSpace(VSpace::with_page_size(
            root, base, size, page_size,
        )))?;
        if let Some(space) = self.vspace_mut(object) {
            space.asid = asid;
        }
        Some(object)
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

    pub fn set_tcb_affinity(&mut self, tcb: ObjRef, mask: u8) -> KResult<()> {
        if mask == 0 || mask & !((1u8 << MAX_HARTS) - 1) != 0 {
            return Err(KError::InvalidArg);
        }
        let queued = self.tcb(tcb).is_some_and(|table| table.queued);
        if queued {
            self.remove_from_runq(tcb);
        }
        self.tcb_mut(tcb).ok_or(KError::WrongType)?.affinity = mask;
        if queued {
            self.enqueue(tcb);
        }
        Ok(())
    }

    /// Configure a retyped scheduler context.  The budget is expressed in
    /// timer ticks and must fit inside one period.
    pub fn configure_sched_context(
        &mut self,
        context_ref: ObjRef,
        budget: u64,
        period: u64,
    ) -> KResult<()> {
        if budget == 0 || period == 0 || budget > period {
            return Err(KError::InvalidArg);
        }
        let now = self.fast_time();
        let context = self
            .sched_context_mut(context_ref)
            .ok_or(KError::WrongType)?;
        context.budget = budget;
        context.period = period;
        context.remaining = budget;
        context.period_start = now;
        Ok(())
    }

    /// Bind a configured scheduler context to a TCB.  A context may serve one
    /// TCB at a time; binding is idempotent for the same pair.
    pub fn bind_sched_context(&mut self, tcb: ObjRef, context_ref: ObjRef) -> KResult<()> {
        let context = *self.sched_context(context_ref).ok_or(KError::WrongType)?;
        if context.budget == 0 || context.period == 0 || context.budget > context.period {
            return Err(KError::InvalidArg);
        }
        let table = self.tcb(tcb).ok_or(KError::WrongType)?;
        if let Some(existing) = table.sched_context {
            if existing == context_ref {
                return Ok(());
            }
            return Err(KError::AlreadyMapped);
        }
        if context.bound.is_some_and(|bound| bound != tcb) {
            return Err(KError::AlreadyMapped);
        }
        let now = self.fast_time();
        {
            let context = self
                .sched_context_mut(context_ref)
                .ok_or(KError::ObjectDestroyed)?;
            context.bound = Some(tcb);
            context.remaining = context.budget;
            context.period_start = now;
        }
        let table = self.tcb_mut(tcb).ok_or(KError::ObjectDestroyed)?;
        table.sched_context = Some(context_ref);
        Ok(())
    }

    /// Bind a notification to an IRQ object.  If the source is already
    /// pending (for example the timer fired before user space configured it),
    /// signal immediately so a waiter cannot miss the edge.
    pub fn set_irq_notification(&mut self, irq_ref: ObjRef, notification: ObjRef) -> KResult<()> {
        let irq = *self.irq(irq_ref).ok_or(KError::WrongType)?;
        if self.notification(notification).is_none() {
            return Err(KError::WrongType);
        }
        if let Some(existing) = irq.notification {
            if existing == notification {
                return Ok(());
            }
            return Err(KError::AlreadyMapped);
        }
        self.irq_mut(irq_ref)
            .ok_or(KError::ObjectDestroyed)?
            .notification = Some(notification);
        if irq.pending {
            self.signal(notification, 1u64 << (irq.number & 63))?;
        }
        Ok(())
    }

    pub fn irq_ack(&mut self, irq_ref: ObjRef) -> KResult<()> {
        let irq = self.irq_mut(irq_ref).ok_or(KError::WrongType)?;
        irq.pending = false;
        Ok(())
    }

    /// Raise a logical IRQ from a platform interrupt handler.  Edges are
    /// coalesced until user space acknowledges the object.  Returns true when
    /// a new pending edge was recorded.
    pub fn irq_raise(&mut self, number: u32) -> bool {
        let mut target = None;
        for (index, object) in self.objs.iter().enumerate() {
            if let Kind::Irq(irq) = object.kind {
                if irq.number == number {
                    target = Some((
                        ObjRef {
                            index: index as u8,
                            gen: object.gen,
                        },
                        irq.notification,
                        irq.pending,
                    ));
                    break;
                }
            }
        }
        let Some((irq_ref, notification, pending)) = target else {
            return false;
        };
        if pending {
            return false;
        }
        if let Some(irq) = self.irq_mut(irq_ref) {
            irq.pending = true;
        }
        if let Some(notification) = notification {
            let _ = self.signal(notification, 1u64 << (number & 63));
        }
        true
    }

    /// Replenish expired budget blocks.  These TCBs were preempted asynchronously
    /// by the timer, so no syscall result is installed in `resume`.
    fn wake_budgeters(&mut self, now: u64) -> usize {
        let mut woke = 0usize;
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            let expired = match self.objs[index].kind {
                Kind::Tcb(tcb) => {
                    tcb.state == ThreadState::BlockedBudget
                        && tcb.wake_at.is_some_and(|deadline| deadline <= now)
                }
                _ => false,
            };
            if !expired {
                continue;
            }
            let context = self.tcb(reference).and_then(|table| table.sched_context);
            if let Some(context) = context {
                if let Some(context) = self.sched_context_mut(context) {
                    context.remaining = context.budget;
                    context.period_start = now;
                }
            }
            if let Some(tcb) = self.tcb_mut(reference) {
                tcb.wake_at = None;
                tcb.state = ThreadState::Ready;
                tcb.resume = None;
                tcb.resume_error = None;
            }
            self.enqueue(reference);
            woke += 1;
        }
        woke
    }

    /// Charge one timer tick to the current TCB.  Returns true when this tick
    /// exhausted its budget and parked it until the next period.
    fn charge_current_budget(&mut self, hart: usize, now: u64) -> bool {
        let Some(current) = self.current_on(hart) else {
            return false;
        };
        let Some(table) = self.tcb(current) else {
            return false;
        };
        if table.state != ThreadState::Running {
            return false;
        }
        let Some(context_ref) = table.sched_context else {
            return false;
        };
        let Some(context) = self.sched_context(context_ref) else {
            if let Some(table) = self.tcb_mut(current) {
                table.sched_context = None;
            }
            return false;
        };
        let (budget, period, context_start, context_remaining) = (
            context.budget,
            context.period,
            context.period_start,
            context.remaining,
        );
        if budget == 0 || period == 0 {
            return false;
        }
        let mut period_start = context_start;
        let mut remaining = context_remaining;
        if now >= period_start.saturating_add(period) {
            period_start = now;
            remaining = budget;
        }
        if remaining != 0 {
            remaining -= 1;
        }
        let exhausted = remaining == 0;
        let deadline = period_start
            .saturating_add(period)
            .max(now.saturating_add(1));
        if let Some(context) = self.sched_context_mut(context_ref) {
            context.period_start = period_start;
            context.remaining = remaining;
        }
        if exhausted {
            if let Some(table) = self.tcb_mut(current) {
                table.state = ThreadState::BlockedBudget;
                table.wake_at = Some(deadline);
                table.queued = false;
            }
        }
        exhausted
    }

    pub fn set_tcb_priority(
        &mut self,
        target: ObjRef,
        priority: u8,
        caller: ObjRef,
    ) -> KResult<()> {
        if priority > MAX_PRIORITY {
            return Err(KError::PrioTooHigh);
        }
        let caller_priority = self.tcb(caller).ok_or(KError::InvalidCap)?.priority;
        let table = self.tcb(target).ok_or(KError::WrongType)?;
        if priority > table.priority || priority > caller_priority {
            return Err(KError::PrioTooHigh);
        }
        self.tcb_mut(target).ok_or(KError::WrongType)?.priority = priority;
        Ok(())
    }

    fn remove_from_runq(&mut self, tcb: ObjRef) {
        for hart in 0..MAX_HARTS {
            let mut index = 0;
            while index < self.runq_len[hart] {
                if self.runq[hart][index] == Some(tcb) {
                    let mut shift = index;
                    while shift + 1 < self.runq_len[hart] {
                        self.runq[hart][shift] = self.runq[hart][shift + 1];
                        shift += 1;
                    }
                    self.runq_len[hart] -= 1;
                    self.runq[hart][self.runq_len[hart]] = None;
                } else {
                    index += 1;
                }
            }
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.queued = false;
        }
    }

    /// Install or clear an external flat CNode as a TCB's capability space.
    /// Only an inactive/suspended/faulted TCB may be switched so a running
    /// task never observes a cspace replacement mid-syscall.
    pub fn set_tcb_cnode(&mut self, target: ObjRef, cnode: Option<ObjRef>) -> KResult<()> {
        if self.root == Some(target) || target == self.root.unwrap_or(ObjRef::invalid()) {
            return Err(KError::IllegalOperation);
        }
        let table = self.tcb(target).ok_or(KError::WrongType)?;
        if table.queued
            || !matches!(
                table.state,
                ThreadState::Inactive | ThreadState::Suspended | ThreadState::Faulted
            )
        {
            return Err(KError::IllegalOperation);
        }
        if let Some(cnode) = cnode {
            if self.cnode(cnode).is_none() {
                return Err(KError::WrongType);
            }
        }
        self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?.cnode = cnode;
        Ok(())
    }

    /// Mint a derived cap directly into a flat CNode.  The source cap must be
    /// in the caller's current cspace; the destination CNode is named by a
    /// separate cap held by the caller.
    pub fn cnode_mint(
        &mut self,
        caller: ObjRef,
        source_slot: u8,
        cnode: ObjRef,
        dest_index: u8,
        rights: Rights,
        badge: u64,
    ) -> KResult<()> {
        let source = self.lookup_cap(caller, source_slot, None, Rights::NONE)?;
        let index = dest_index as usize;
        if index == 0 || self.cnode(cnode).is_none() {
            return Err(KError::WrongType);
        }
        if index >= MAX_CAPS_TASK {
            return Err(KError::InvalidArg);
        }
        if self
            .cnode(cnode)
            .and_then(|node| node.slots.get(index))
            .is_some_and(Option::is_some)
        {
            return Err(KError::SlotOccupied);
        }
        let mut derived = source.derive(rights, badge)?;
        derived.parent = Some(CapRef {
            addr: SlotAddr {
                tcb: caller,
                slot: source_slot,
            },
            id: source.id,
        });
        derived.id = self.next_cap_id;
        self.next_cap_id = self.next_cap_id.wrapping_add(1);
        if self.next_cap_id == 0 {
            self.next_cap_id = 1;
        }
        if self.get(derived.obj).is_none() {
            return Err(KError::ObjectDestroyed);
        }
        let node = self.cnode_mut(cnode).ok_or(KError::ObjectDestroyed)?;
        node.slots[index] = Some(derived);
        Ok(())
    }

    /// Suspend a TCB.  Running targets keep their saved `current` slot until
    /// the next timer switch; Endpoint/Notification waiters are removed from
    /// their queues and receive `WouldBlock` when resumed.  Root and the
    /// caller itself are never suspendable.
    pub fn suspend_tcb(&mut self, target: ObjRef, caller: ObjRef) -> KResult<Option<usize>> {
        if self.root == Some(target) || target == caller {
            return Err(KError::IllegalOperation);
        }
        let state = self.tcb(target).ok_or(KError::WrongType)?.state;
        match state {
            ThreadState::Inactive | ThreadState::Faulted | ThreadState::BlockedFault => {
                return Err(KError::IllegalOperation)
            }
            ThreadState::Suspended => return Err(KError::AlreadyMapped),
            ThreadState::BlockedReply => return Err(KError::IllegalOperation),
            ThreadState::Running => {
                let hart = (0..MAX_HARTS)
                    .find(|hart| self.current[*hart] == Some(target))
                    .ok_or(KError::IllegalOperation)?;
                self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?.state = ThreadState::Suspended;
                return Ok(Some(hart));
            }
            ThreadState::Ready => {
                self.remove_from_runq(target);
            }
            ThreadState::BlockedSleep => {
                self.remove_from_runq(target);
                let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
                table.wake_at = None;
                table.resume = Some(Message::empty(0));
                table.resume_error = Some(KError::WouldBlock);
            }
            ThreadState::BlockedBudget => {
                self.remove_from_runq(target);
                let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
                table.wake_at = None;
                table.resume = None;
                table.resume_error = None;
            }
            ThreadState::BlockedSend
            | ThreadState::BlockedRecv
            | ThreadState::BlockedNotification => {
                self.remove_from_runq(target);
                for i in 0..MAX_OBJECTS {
                    let r = ObjRef {
                        index: i as u8,
                        gen: self.objs[i].gen,
                    };
                    match self.objs[i].kind {
                        Kind::Endpoint(_) => {
                            if let Some(endpoint) = self.endpoint_mut(r) {
                                endpoint.send.remove(target);
                                endpoint.recv.remove(target);
                            }
                        }
                        Kind::Notification(_) => {
                            if let Some(notification) = self.notification_mut(r) {
                                for slot in notification.waiters.iter_mut() {
                                    if *slot == Some(target) {
                                        *slot = None;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
                table.pending = None;
                table.resume = Some(Message::empty(0));
                table.resume_error = Some(KError::WouldBlock);
            }
        }
        self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?.state = ThreadState::Suspended;
        Ok(None)
    }

    pub fn resume_tcb(&mut self, tcb: ObjRef) -> KResult<()> {
        let (entry, vspace, state, sched) = {
            let table = self.tcb(tcb).ok_or(KError::WrongType)?;
            (table.entry, table.vspace, table.state, table.sched_context)
        };
        if entry == 0 || vspace.is_none() {
            return Err(KError::IllegalOperation);
        }
        if state == ThreadState::Suspended {
            if let Some(sched) = sched {
                let now = self.fast_time();
                if let Some(context) = self.sched_context_mut(sched) {
                    context.remaining = context.budget;
                    context.period_start = now;
                }
            }
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.state = ThreadState::Ready;
        }
        self.enqueue(tcb);
        Ok(())
    }

    /// Reset a non-running TCB to its configured entry/stack and make it
    /// runnable again.  The caller must rebuild the architecture context.
    pub fn restart_tcb(&mut self, target: ObjRef, caller: ObjRef) -> KResult<(u64, u64)> {
        if target == caller {
            return Err(KError::IllegalOperation);
        }
        let table = self.tcb(target).ok_or(KError::WrongType)?;
        if table.state == ThreadState::Running || table.caller.is_some() {
            return Err(KError::IllegalOperation);
        }
        if table.entry == 0 || table.vspace.is_none() {
            return Err(KError::IllegalOperation);
        }
        let entry = table.entry;
        let stack = table.stack;
        let sched = table.sched_context;
        self.remove_from_runq(target);
        if let Some(sched) = sched {
            let now = self.fast_time();
            if let Some(context) = self.sched_context_mut(sched) {
                context.remaining = context.budget;
                context.period_start = now;
            }
        }
        let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
        table.state = ThreadState::Ready;
        table.pending = None;
        table.resume = None;
        table.resume_error = None;
        table.recv_slot = 0;
        table.wake_at = None;
        table.fds = [None; MAX_FDS];
        self.enqueue(target);
        Ok((entry, stack))
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

    fn free_context_slot(&self) -> Option<u8> {
        for slot in 0..MAX_TCBS {
            let used = self.objs.iter().any(|object| {
                matches!(&object.kind, Kind::Tcb(tcb) if tcb.context_slot as usize == slot)
            });
            if !used {
                return Some(slot as u8);
            }
        }
        None
    }

    /// Dense context slot for a live TCB, if assigned.
    pub fn context_slot(&self, r: ObjRef) -> Option<u8> {
        let slot = self.tcb(r)?.context_slot;
        ((slot as usize) < MAX_TCBS).then_some(slot)
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

    pub fn sched_context(&self, r: ObjRef) -> Option<&SchedContext> {
        match self.get(r)?.kind {
            Kind::SchedContext(ref context) => Some(context),
            _ => None,
        }
    }

    pub fn sched_context_mut(&mut self, r: ObjRef) -> Option<&mut SchedContext> {
        match self.get_mut(r)?.kind {
            Kind::SchedContext(ref mut context) => Some(context),
            _ => None,
        }
    }

    pub fn cnode(&self, r: ObjRef) -> Option<&CNode> {
        match self.get(r)?.kind {
            Kind::CNode(ref node) => Some(node),
            _ => None,
        }
    }

    pub fn cnode_mut(&mut self, r: ObjRef) -> Option<&mut CNode> {
        match self.get_mut(r)?.kind {
            Kind::CNode(ref mut node) => Some(node),
            _ => None,
        }
    }

    pub fn irq(&self, r: ObjRef) -> Option<&Irq> {
        match self.get(r)?.kind {
            Kind::Irq(ref irq) => Some(irq),
            _ => None,
        }
    }

    pub fn irq_mut(&mut self, r: ObjRef) -> Option<&mut Irq> {
        match self.get_mut(r)?.kind {
            Kind::Irq(ref mut irq) => Some(irq),
            _ => None,
        }
    }

    pub fn iommu(&self, r: ObjRef) -> Option<&IommuDomain> {
        match self.get(r)?.kind {
            Kind::Iommu(ref domain) => Some(domain),
            _ => None,
        }
    }

    pub fn iommu_mut(&mut self, r: ObjRef) -> Option<&mut IommuDomain> {
        match self.get_mut(r)?.kind {
            Kind::Iommu(ref mut domain) => Some(domain),
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
    /// Allocate `charge` bytes from an Untyped region.  Exact-size free
    /// ranges win, then any larger free range, then the bump watermark.
    fn untyped_take(&mut self, untyped: ObjRef, charge: usize, align: usize) -> KResult<usize> {
        let region = *self.untyped(untyped).ok_or(KError::WrongType)?;
        if charge == 0 || align == 0 || !align.is_power_of_two() {
            return Err(KError::Alignment);
        }
        let base = region.base;
        let mut chosen: Option<(usize, usize)> = None;
        for exact in [true, false] {
            for (slot, entry) in region.free.iter().enumerate() {
                let Some(free) = entry else { continue };
                let len = free.len as usize;
                if len < charge || (exact && len != charge) {
                    continue;
                }
                let offset = free.offset as usize;
                let Some(pa) = base.checked_add(offset) else {
                    continue;
                };
                if pa % align != 0 {
                    continue;
                }
                chosen = Some((slot, offset));
                break;
            }
            if chosen.is_some() {
                break;
            }
        }
        if let Some((slot, offset)) = chosen {
            let mut region = *self.untyped(untyped).ok_or(KError::WrongType)?;
            let free = region.free[slot].take().ok_or(KError::IllegalOperation)?;
            let remaining = free.len as usize - charge;
            if remaining > 0 {
                region.free[slot] = Some(FreeRegion {
                    offset: (offset + charge) as u32,
                    len: remaining as u32,
                });
            }
            *self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)? = region;
            return Ok(offset);
        }
        let unaligned = region
            .base
            .checked_add(region.bump)
            .ok_or(KError::OutOfMemory)?;
        let pa = (unaligned + align - 1) & !(align - 1);
        let end = region
            .base
            .checked_add(region.size)
            .ok_or(KError::OutOfMemory)?;
        if pa.checked_add(charge).ok_or(KError::OutOfMemory)? > end {
            return Err(KError::OutOfMemory);
        }
        let offset = pa - region.base;
        let mut region = *self.untyped(untyped).ok_or(KError::WrongType)?;
        region.bump = offset.checked_add(charge).ok_or(KError::OutOfMemory)?;
        *self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)? = region;
        Ok(offset)
    }

    /// Return one byte range to its Untyped.  Ranges touching the bump
    /// watermark shrink it; the rest are coalesced into the fixed free list.
    /// A full list keeps the object destroyed but counts the bytes as leaked.
    fn untyped_reclaim(&mut self, untyped: ObjRef, offset: usize, len: usize) {
        if len == 0 {
            return;
        }
        let Some(mut region) = self.untyped(untyped).copied() else {
            return;
        };
        let Some(end) = offset.checked_add(len) else {
            region.leaked = region.leaked.saturating_add(len as u32);
            if let Some(dest) = self.untyped_mut(untyped) {
                *dest = region;
            }
            return;
        };
        if offset >= end || end > region.size {
            region.leaked = region.leaked.saturating_add(len as u32);
            if let Some(dest) = self.untyped_mut(untyped) {
                *dest = region;
            }
            return;
        }
        for entry in region.free.iter().flatten() {
            let start = entry.offset as usize;
            let entry_end = start.saturating_add(entry.len as usize);
            if offset < entry_end && start < end {
                // Overlapping free range: treat as a leak, never corrupt.
                region.leaked = region.leaked.saturating_add(len as u32);
                if let Some(dest) = self.untyped_mut(untyped) {
                    *dest = region;
                }
                return;
            }
        }
        let mut start = offset;
        let mut stop = end;
        loop {
            let mut merged = false;
            for slot in 0..MAX_UNTYPED_FREE {
                let Some(free) = region.free[slot] else {
                    continue;
                };
                let free_start = free.offset as usize;
                let free_end = free_start.saturating_add(free.len as usize);
                if free_end == start || free_start == stop {
                    region.free[slot] = None;
                    start = start.min(free_start);
                    stop = stop.max(free_end);
                    merged = true;
                    break;
                }
            }
            if !merged {
                break;
            }
        }
        if stop == region.bump {
            region.bump = start;
            // Lowering the watermark can make a free region that was one
            // slot below it adjacent; cascade so repeated frees always
            // collapse back to the bump instead of fragmenting.
            loop {
                let mut lower: Option<(usize, usize)> = None;
                for slot in 0..MAX_UNTYPED_FREE {
                    let Some(free) = region.free[slot] else {
                        continue;
                    };
                    let free_start = free.offset as usize;
                    if free_start.saturating_add(free.len as usize) == region.bump {
                        lower = Some((slot, free_start));
                        break;
                    }
                }
                let Some((slot, free_start)) = lower else {
                    break;
                };
                region.free[slot] = None;
                region.bump = free_start;
            }
        } else if let Some(slot) = region.free.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(FreeRegion {
                offset: start as u32,
                len: (stop - start) as u32,
            });
        } else {
            region.leaked = region.leaked.saturating_add(len as u32);
        }
        if let Some(dest) = self.untyped_mut(untyped) {
            *dest = region;
        }
    }

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
        let context_slot = if ty == ObjType::Tcb {
            Some(self.free_context_slot().ok_or(KError::OutOfMemory)?)
        } else {
            None
        };
        let (base, size, page, max_objects, objects, reserved) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (
                region.base,
                region.size,
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
            ObjType::CNode => (
                Kind::CNode(CNode::default()),
                OBJ_CNODE_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::SchedContext => (
                Kind::SchedContext(SchedContext::default()),
                OBJ_SCHED_CONTEXT_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::Iommu => (
                Kind::Iommu(IommuDomain::default()),
                OBJ_IOMMU_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            ObjType::BlockDevice => (
                Kind::BlockDevice(BlockDevice::default()),
                OBJ_BLOCK_DEVICE_BYTES,
                OBJ_METADATA_ALIGN,
            ),
            _ => return Err(KError::NotImplemented),
        };
        let offset = self.untyped_take(untyped, charge, align)?;
        // Vòng 82 (audit P1-1): mọi lỗi SAU khi đã lấy byte phải trả lại
        // Untyped, nếu không mỗi lần retype thất bại lại đốt vĩnh viễn một
        // khoảng (đúng kiểu làm cạn data/meta Untyped trên x86).
        let Some(pa) = base.checked_add(offset) else {
            self.untyped_reclaim(untyped, offset, charge);
            return Err(KError::OutOfMemory);
        };
        let Some(end) = base.checked_add(size) else {
            self.untyped_reclaim(untyped, offset, charge);
            return Err(KError::OutOfMemory);
        };
        match pa.checked_add(charge) {
            Some(taken_end) if taken_end <= end => {}
            _ => {
                self.untyped_reclaim(untyped, offset, charge);
                return Err(KError::OutOfMemory);
            }
        }
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
            space.asid = self.take_asid();
            Kind::VSpace(space)
        } else if ty == ObjType::Iommu {
            Kind::Iommu(IommuDomain::default())
        } else {
            kind
        };
        let object = match self.alloc_owned(kind, Some(untyped), creator) {
            Some(object) => object,
            None => {
                self.untyped_reclaim(untyped, offset, charge);
                return Err(KError::OutOfMemory);
            }
        };
        if let Some(record) = self.get_mut(object) {
            record.alloc_offset = offset as u32;
            record.alloc_len = charge as u32;
            if let (Kind::Tcb(tcb), Some(slot)) = (&mut record.kind, context_slot) {
                tcb.context_slot = slot;
            }
        }
        let region = self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)?;
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
        let (base, parent_size, page, max_objects, objects, reserved) = {
            let region = self.untyped(untyped).ok_or(KError::WrongType)?;
            (
                region.base,
                region.size,
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
        let offset = self.untyped_take(untyped, size, page)?;
        let child_base = match base.checked_add(offset) {
            Some(value) => value,
            None => {
                self.untyped_reclaim(untyped, offset, size);
                return Err(KError::OutOfMemory);
            }
        };
        let parent_end = match base.checked_add(parent_size) {
            Some(value) => value,
            None => {
                self.untyped_reclaim(untyped, offset, size);
                return Err(KError::OutOfMemory);
            }
        };
        let child_end = match child_base.checked_add(size) {
            Some(value) => value,
            None => {
                self.untyped_reclaim(untyped, offset, size);
                return Err(KError::OutOfMemory);
            }
        };
        if child_end > parent_end {
            self.untyped_reclaim(untyped, offset, size);
            return Err(KError::OutOfMemory);
        }
        let kind = Kind::Untyped(Untyped {
            base: child_base,
            size,
            bump: 0,
            page_size: page,
            max_objects: child_max_objects,
            objects: 0,
            reserved: 0,
            free: [None; MAX_UNTYPED_FREE],
            leaked: 0,
        });
        let object = match self.alloc_owned(kind, Some(untyped), creator) {
            Some(object) => object,
            None => {
                self.untyped_reclaim(untyped, offset, size);
                return Err(KError::OutOfMemory);
            }
        };
        if let Some(record) = self.get_mut(object) {
            record.alloc_offset = offset as u32;
            record.alloc_len = size as u32;
        }
        let region = self.untyped_mut(untyped).ok_or(KError::ObjectDestroyed)?;
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
        let mut all_destroyed = true;
        for object in created.iter().rev() {
            if object.is_valid() && self.destroy_object(*object).is_err() {
                all_destroyed = false;
            }
        }
        // Destroying an object already returns its bytes to the Untyped free
        // list or bump watermark, so only the object counter is repaired here
        // and only when every rollback teardown succeeded.
        let _ = bump;
        if all_destroyed {
            if let Some(region) = self.untyped_mut(untyped) {
                region.objects = objects;
            }
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

    /// Replace an existing leaf mapping without an unmapped window.  The new
    /// PTE is written first, then one per-VA shootdown publishes it to every
    /// hart that may hold the old translation.  This is the path used by the
    /// continuous-reader TLB test, where unmap+map would fault the reader.
    pub fn remap_frame(
        &mut self,
        vspace: ObjRef,
        frame_obj: ObjRef,
        va: u64,
        rights: Rights,
    ) -> KResult<Mapping> {
        let allowed = Rights::READ | Rights::WRITE | Rights::EXEC;
        if rights.is_empty() || !allowed.contains(rights) || !rights.contains(Rights::READ) {
            return Err(KError::InvalidArg);
        }
        if rights.contains(Rights::WRITE) && rights.contains(Rights::EXEC) {
            return Err(KError::NoRights);
        }
        let old = self.mapping_at(vspace, va).ok_or(KError::NotMapped)?;
        if old.frame == frame_obj {
            return Err(KError::AlreadyMapped);
        }
        let frame = *self.frame(frame_obj).ok_or(KError::WrongType)?;
        let page_size = self
            .vspace(vspace)
            .map(|space| space.page_size)
            .ok_or(KError::WrongType)?;
        if frame.size < page_size as usize || frame.pa as u64 % page_size != 0 {
            return Err(KError::Alignment);
        }
        let wants_w = rights.contains(Rights::WRITE);
        let wants_x = rights.contains(Rights::EXEC);
        if (wants_w && frame.mapped_x != 0) || (wants_x && frame.mapped_w != 0) {
            return Err(KError::NoRights);
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
        let root = self
            .vspace(vspace)
            .map(|space| space.root)
            .ok_or(KError::WrongType)?;
        if self.page_table_ops.is_some() {
            self.replace_leaf(root, va, frame.pa as u64, rights)?;
        }
        let mapping = Mapping {
            va,
            pa: frame.pa as u64,
            len: page_size,
            rights,
            frame: frame_obj,
        };
        let index = self
            .vspace(vspace)
            .and_then(|space| space.mapping_index(va))
            .ok_or(KError::NotMapped)?;
        self.note_frame_unmapping(old);
        self.vspace_mut(vspace).ok_or(KError::WrongType)?.mappings[index] = Some(mapping);
        self.set_frame_mapping_counts(frame_obj, next_w, next_x);
        self.flush_vspace_range(vspace, va, 1);
        if rights.contains(Rights::EXEC) {
            if let Some(ops) = self.page_table_ops {
                ops.sync_icache(frame.pa as u64, page_size as usize);
            }
        }
        Ok(mapping)
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
        let (root, page_size, base, end, next_free) = {
            let space = self.vspace(vspace).ok_or(KError::WrongType)?;
            (
                space.root,
                space.page_size,
                space.base,
                space.end,
                space.find_free_va(space.page_size),
            )
        };
        if page_size == 0
            || !page_size.is_power_of_two()
            || frame.pa as u64 % page_size != 0
            || (frame.size as u64) < page_size
        {
            return Err(KError::Alignment);
        }
        let va = if hint == 0 {
            next_free.ok_or(KError::OutOfMemory)?
        } else {
            hint
        };
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
        // Publish the new leaf to every hart that may still hold an older
        // translation for this VA.  This also covers a remap performed after
        // an unmap: the previous remote invalidation raced with the PTE store
        // on x86, so the map itself must be part of the shootdown.
        self.flush_vspace_range(vspace, va, (page_size / 4096).max(1) as usize);
        if rights.contains(Rights::EXEC) {
            if let Some(ops) = self.page_table_ops {
                ops.sync_icache(frame.pa as u64, page_size as usize);
            }
        }
        Ok(mapping)
    }

    fn flush_vspace_range(&mut self, vspace: ObjRef, va: u64, pages: usize) {
        let Some(ops) = self.page_table_ops else {
            return;
        };
        let Some((asid, mask, page_size)) = self
            .vspace(vspace)
            .map(|space| (space.asid, space.harts_loaded, space.page_size))
        else {
            return;
        };
        if mask == 0 {
            return;
        }
        if pages > 16 || pages.saturating_mul(page_size as usize) > 16 * 4096 {
            ops.flush_all_asid(asid, mask);
            // A full-ASID flush on every loaded hart makes the mask empty.
            if let Some(space) = self.vspace_mut(vspace) {
                space.harts_loaded = 0;
            }
        } else {
            ops.flush_va_asid(asid, va, pages, mask);
        }
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
        self.flush_vspace_range(vspace, va, 1);
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
        let mut total_queued = 0usize;
        for hart in 0..MAX_HARTS {
            if self.runq_len[hart] > MAX_TCBS {
                return Err(KError::IllegalOperation);
            }
            for slot in self.runq_len[hart]..MAX_TCBS {
                if self.runq[hart][slot].is_some() {
                    return Err(KError::IllegalOperation);
                }
            }
            total_queued += self.runq_len[hart];
        }
        if total_queued > MAX_TCBS {
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
                    if let Some(sched) = tcb.sched_context {
                        let Some(context) = self.sched_context(sched) else {
                            return Err(KError::ObjectDestroyed);
                        };
                        if context.bound != Some(reference) {
                            return Err(KError::IllegalOperation);
                        }
                    }
                    if let Some(cnode) = tcb.cnode {
                        if self.cnode(cnode).is_none() {
                            return Err(KError::ObjectDestroyed);
                        }
                    }
                    if let Some(slots) = self.cspace(reference) {
                        for cap in slots.iter().flatten() {
                            let Some(object) = self.get(cap.obj) else {
                                return Err(KError::ObjectDestroyed);
                            };
                            if object.kind.obj_type() != Some(cap.ty) {
                                return Err(KError::WrongType);
                            }
                            if cap.badge != 0 {
                                if let Some(parent) = cap.parent {
                                    if let Some(source) = self.cap_at(parent.addr) {
                                        if source.id == parent.id
                                            && source.badge != 0
                                            && source.badge != cap.badge
                                        {
                                            return Err(KError::NoRights);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    for fd in tcb.fds.iter().flatten() {
                        if self.fs.file(fd.file as usize).is_none() {
                            return Err(KError::IllegalOperation);
                        }
                    }
                    if tcb.queued {
                        let mut found = false;
                        for hart in 0..MAX_HARTS {
                            for queued in self.runq[hart][..self.runq_len[hart]].iter().flatten() {
                                if *queued == reference {
                                    found = true;
                                }
                            }
                        }
                        if !found {
                            return Err(KError::IllegalOperation);
                        }
                    }
                }
                Kind::Endpoint(_) => {}
                Kind::Notification(ref notification) => {
                    for waiter in notification.waiters.into_iter().flatten() {
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
                    if space.harts_loaded & !((1u32 << MAX_HARTS) - 1) != 0 {
                        return Err(KError::IllegalOperation);
                    }
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
                Kind::BlockDevice(_) => {}
                Kind::Iommu(ref domain) => {
                    for (index, mapping) in domain.mappings.iter().enumerate() {
                        let Some(mapping) = mapping else { continue };
                        if mapping.len != DEFAULT_PAGE_SIZE
                            || mapping.iova % DEFAULT_PAGE_SIZE != 0
                            || mapping.pa % DEFAULT_PAGE_SIZE != 0
                            || mapping.iova < IOMMU_IOVA_BASE
                            || mapping.iova.saturating_add(mapping.len)
                                > IOMMU_IOVA_BASE.saturating_add(0x4000_0000)
                            || mapping.rights.is_empty()
                            || !(Rights::READ | Rights::WRITE).contains(mapping.rights)
                        {
                            return Err(KError::IllegalOperation);
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
                        for other in domain.mappings.iter().skip(index + 1).flatten() {
                            if mapping.iova < other.iova.saturating_add(other.len)
                                && other.iova < mapping.iova.saturating_add(mapping.len)
                            {
                                return Err(KError::AlreadyMapped);
                            }
                        }
                    }
                }
                Kind::CNode(ref node) => {
                    for cap in node.slots.iter().flatten() {
                        let Some(object) = self.get(cap.obj) else {
                            return Err(KError::ObjectDestroyed);
                        };
                        if object.kind.obj_type() != Some(cap.ty) {
                            return Err(KError::WrongType);
                        }
                    }
                }
                Kind::Irq(ref irq) => {
                    if let Some(notification) = irq.notification {
                        if self.notification(notification).is_none() {
                            return Err(KError::ObjectDestroyed);
                        }
                    }
                }
                Kind::SchedContext(ref context) => {
                    let configured = context.budget != 0 || context.period != 0;
                    if configured
                        && (context.budget == 0
                            || context.period == 0
                            || context.budget > context.period
                            || context.remaining > context.budget)
                    {
                        return Err(KError::IllegalOperation);
                    }
                    if let Some(bound) = context.bound {
                        let Some(tcb) = self.tcb(bound) else {
                            return Err(KError::ObjectDestroyed);
                        };
                        if tcb.sched_context != Some(reference) {
                            return Err(KError::IllegalOperation);
                        }
                    }
                }
                Kind::Untyped(ref untyped) => {
                    if untyped.bump > untyped.size
                        || untyped
                            .objects
                            .checked_add(untyped.reserved)
                            .is_none_or(|used| used > untyped.max_objects)
                        || untyped.leaked as usize > untyped.size
                    {
                        return Err(KError::IllegalOperation);
                    }
                    for (slot, entry) in untyped.free.iter().enumerate() {
                        let Some(free) = entry else { continue };
                        let start = free.offset as usize;
                        let Some(end) = start.checked_add(free.len as usize) else {
                            return Err(KError::IllegalOperation);
                        };
                        if free.len == 0 || end > untyped.bump {
                            return Err(KError::IllegalOperation);
                        }
                        for other in untyped.free.iter().skip(slot + 1).flatten() {
                            let other_start = other.offset as usize;
                            let other_end = other_start.saturating_add(other.len as usize);
                            if start < other_end && other_start < end {
                                return Err(KError::IllegalOperation);
                            }
                        }
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
        for hart in 0..MAX_HARTS {
            if let Some(current) = self.current[hart] {
                let Some(tcb) = self.tcb(current) else {
                    return Err(KError::ObjectDestroyed);
                };
                if !matches!(tcb.state, ThreadState::Running | ThreadState::Suspended) || tcb.queued
                {
                    return Err(KError::IllegalOperation);
                }
                for other in (hart + 1)..MAX_HARTS {
                    if self.current[other] == Some(current) {
                        return Err(KError::IllegalOperation);
                    }
                }
            }
        }
        for hart in 0..MAX_HARTS {
            for index in 0..self.runq_len[hart] {
                let Some(queued) = self.runq[hart][index] else {
                    return Err(KError::IllegalOperation);
                };
                let Some(tcb) = self.tcb(queued) else {
                    return Err(KError::ObjectDestroyed);
                };
                if tcb.state != ThreadState::Ready
                    || !tcb.queued
                    || tcb.affinity & (1u8 << hart) == 0
                {
                    return Err(KError::IllegalOperation);
                }
                for other_hart in 0..MAX_HARTS {
                    for other_index in 0..self.runq_len[other_hart] {
                        if self.runq[other_hart][other_index] == Some(queued)
                            && (other_hart != hart || other_index != index)
                        {
                            return Err(KError::IllegalOperation);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn install_cap(&mut self, tcb: ObjRef, slot: u8, mut cap: Cap) -> KResult<()> {
        if slot == 0 || slot as usize >= self.cspace_capacity(tcb) {
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
        let slots = self.cspace_mut(tcb).ok_or(KError::InvalidCap)?;
        if slots[slot as usize].is_some() {
            return Err(KError::SlotOccupied);
        }
        slots[slot as usize] = Some(cap);
        Ok(())
    }

    pub fn lookup_cap(
        &self,
        tcb: ObjRef,
        slot: u8,
        want: Option<ObjType>,
        need: Rights,
    ) -> KResult<Cap> {
        let cap = self
            .cspace(tcb)
            .ok_or(KError::InvalidCap)?
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
        self.cspace(addr.tcb)?
            .get(addr.slot as usize)
            .and_then(|cap| *cap)
    }

    pub fn delete_cap(&mut self, tcb: ObjRef, slot: u8) -> KResult<Cap> {
        let addr = SlotAddr { tcb, slot };
        let cap = self.cap_at(addr).ok_or(KError::InvalidCap)?;
        // Rule: a caller may not delete its own TCB cap or its current VSpace
        // cap.  The kernel holds strong references for scheduling, but keeping
        // these visible prevents a task from boxing itself out of TCB/VSpace
        // operations while it is running.
        let own_vspace = self.tcb(tcb).and_then(|table| table.vspace);
        if cap.obj == tcb || own_vspace == Some(cap.obj) {
            return Err(KError::IllegalOperation);
        }
        // A deletion that would orphan a live object or a child capability is
        // refused for every cap type, not just Untyped.  The object itself is
        // not destroyed: deleting a cap only removes the slot.
        let target = CapRef { addr, id: cap.id };
        let has_live_objects = self.objs.iter().any(|object| {
            object.owner == Some(cap.obj)
                && object
                    .creator
                    .is_some_and(|creator| self.cap_ref_is_descendant(creator, target))
        });
        let has_child_caps = (0..MAX_OBJECTS).any(|i| {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            self.cspace(r).is_some_and(|slots| {
                slots
                    .iter()
                    .flatten()
                    .any(|child| child.parent.is_some() && self.is_descendant(child, target))
            })
        });
        let has_cnode_child = self.objs.iter().any(|object| match object.kind {
            Kind::CNode(ref node) => node
                .slots
                .iter()
                .flatten()
                .any(|child| child.parent.is_some() && self.is_descendant(child, target)),
            _ => false,
        });
        if has_live_objects || has_child_caps || has_cnode_child {
            return Err(KError::IllegalOperation);
        }
        self.cspace_mut(tcb)
            .and_then(|slots| slots.get_mut(slot as usize)?.take())
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
        let Some(state) = self.tcb(tcb).map(|table| table.state) else {
            return;
        };
        // Vòng 98 (soi chay K1): không được đặt Ready + enqueue một task đang
        // Running trên hart khác — nó sẽ vừa Running vừa queued và có thể bị
        // chọn chạy lần hai (vỡ loại trừ của BKL). Task đang chạy chỉ nhận
        // `resume_error` và thấy lỗi ở lần trả syscall kế tiếp.
        let was_blocked = matches!(
            state,
            ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
                | ThreadState::BlockedSleep
                | ThreadState::BlockedFault
        );
        if let Some(table) = self.tcb_mut(tcb) {
            table.pending = None;
            table.resume = Some(Message::empty(0));
            table.resume_error = Some(error);
            if was_blocked {
                table.state = ThreadState::Ready;
            }
        }
        if was_blocked {
            self.enqueue(tcb);
        }
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
        if self.tcb(obj).is_some_and(|table| table.cnode.is_some()) {
            // A CNode is an independent object; its slots are not owned by
            // the TCB that happens to use it as its current cspace.
            return;
        }
        let Some(slots) = self.cspace(obj) else {
            return;
        };
        let mut dying = [CapRef::default(); MAX_CAPS_ROOT];
        let mut survivors = [None; MAX_CAPS_ROOT];
        let mut count = 0usize;
        for (slot, cap) in slots.iter().enumerate() {
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
            let Some(slots) = self.cspace_mut(r) else {
                continue;
            };
            for cap in slots.iter_mut().flatten() {
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
        if let Some(sched) = self.tcb(obj).and_then(|table| table.sched_context) {
            if let Some(context) = self.sched_context_mut(sched) {
                if context.bound == Some(obj) {
                    context.bound = None;
                }
            }
        }
        self.remove_from_runq(obj);
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
                // Vòng 88: gỡ TCB khỏi mọi slot waiter khi nó bị destroy.
                for slot in notification.waiters.iter_mut() {
                    if *slot == Some(obj) {
                        *slot = None;
                    }
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

    fn destroy_cnode(&mut self, obj: ObjRef) -> KResult<()> {
        for index in 0..MAX_OBJECTS {
            let tcb = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self.tcb(tcb).is_some_and(|table| table.cnode == Some(obj)) {
                return Err(KError::IllegalOperation);
            }
        }
        if let Some(node) = self.cnode_mut(obj) {
            node.slots = [None; MAX_CAPS_TASK];
        }
        Ok(())
    }

    fn destroy_sched_context(&mut self, obj: ObjRef) {
        let mut wake = [ObjRef::invalid(); MAX_TCBS];
        let mut count = 0usize;
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            let Some(table) = self.tcb_mut(reference) else {
                continue;
            };
            if table.sched_context == Some(obj) {
                table.sched_context = None;
                if table.state == ThreadState::BlockedBudget {
                    table.state = ThreadState::Ready;
                    table.wake_at = None;
                    table.resume = None;
                    table.resume_error = None;
                    if count < wake.len() {
                        wake[count] = reference;
                        count += 1;
                    }
                }
            }
        }
        if let Some(context) = self.sched_context_mut(obj) {
            context.bound = None;
        }
        for reference in wake.iter().take(count) {
            self.enqueue(*reference);
        }
    }

    fn destroy_notification(&mut self, obj: ObjRef) {
        let waiters = if let Some(notification) = self.notification_mut(obj) {
            notification.word = 0;
            let pending = notification.waiters;
            notification.waiters = [None; NOTIFICATION_WAITERS];
            pending
        } else {
            [None; NOTIFICATION_WAITERS]
        };
        for waiter in waiters.into_iter().flatten() {
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

    fn replace_leaf(&self, root: u64, va: u64, pa: u64, rights: Rights) -> KResult<()> {
        let ops = self.page_table_ops.ok_or(KError::MissingPageTable)?;
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
                Pte::Leaf(_, _) if last => {
                    ops.write_pte(table, idx, ops.encode_leaf(pa, rights));
                    ops.flush_va(va);
                    return Ok(());
                }
                Pte::Table(next) if !last => table = next,
                _ => return Err(KError::AlreadyMapped),
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
            if let Some(slots) = self.cspace_mut(r) {
                for slot in slots.iter_mut() {
                    if slot.is_some_and(|cap| cap.obj == obj) {
                        *slot = None;
                    }
                }
                let fault = self.tcb(r).and_then(|table| table.fault_ep);
                if let Some(fault) = fault {
                    if fault.addr.tcb == r
                        && self
                            .cspace(r)
                            .and_then(|slots| slots.get(fault.addr.slot as usize))
                            .is_none_or(Option::is_none)
                    {
                        if let Some(table) = self.tcb_mut(r) {
                            table.fault_ep = None;
                        }
                    }
                }
            }
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(node) = self.cnode_mut(r) {
                for slot in node.slots.iter_mut() {
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
        // Invalidate the ASID on every hart that loaded this VSpace before
        // its page-table pages can be recycled.  Without this a stale TLB
        // entry could alias a future VSpace once ASIDs wrap.
        if let Some(space) = self.vspace(obj) {
            let (asid, mask) = (space.asid, space.harts_loaded);
            if mask != 0 {
                if let Some(ops) = self.page_table_ops {
                    ops.flush_all_asid(asid, mask);
                }
            }
            if let Some(space) = self.vspace_mut(obj) {
                space.harts_loaded = 0;
            }
        }
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

    /// True when `reference` still names a live cap for `object`.
    fn cap_ref_matches(&self, reference: CapRef, object: ObjRef) -> bool {
        self.cap_at(reference.addr)
            .is_some_and(|cap| cap.id == reference.id && cap.obj == object)
    }

    /// Count capabilities that still point at `object`, ignoring the cap the
    /// caller is about to destroy.  Every capability lives in exactly one
    /// physical table: the root cspace, a TCB's embedded cspace, or a CNode.
    fn other_caps_to(&self, object: ObjRef, keep: u64) -> usize {
        let mut count = 0usize;
        for cap in self.root_cspace.iter().flatten() {
            if cap.obj == object && cap.id != keep {
                count += 1;
            }
        }
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self.root == Some(reference) {
                continue;
            }
            if let Some(table) = self.tcb(reference) {
                if table.cnode.is_none() {
                    for cap in table.cspace.iter().flatten() {
                        if cap.obj == object && cap.id != keep {
                            count += 1;
                        }
                    }
                }
            }
            if let Some(node) = self.cnode(reference) {
                for cap in node.slots.iter().flatten() {
                    if cap.obj == object && cap.id != keep {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    fn frame_still_mapped(&self, object: ObjRef, frame: Frame) -> bool {
        let start = frame.pa as u64;
        let end = start.saturating_add(frame.size as u64);
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if let Some(space) = self.vspace(reference) {
                if space.mappings.iter().flatten().any(|mapping| {
                    mapping.frame == object || (mapping.pa >= start && mapping.pa < end)
                }) {
                    return true;
                }
            }
            if let Some(domain) = self.iommu(reference) {
                if domain
                    .mappings
                    .iter()
                    .flatten()
                    .any(|mapping| mapping.frame == object)
                {
                    return true;
                }
            }
            if let Some(table) = self.tcb(reference) {
                if table
                    .ipc_buffer
                    .is_some_and(|buffer| buffer.frame == object)
                {
                    return true;
                }
            }
        }
        false
    }

    fn endpoint_in_use(&self, object: ObjRef) -> bool {
        let Some(endpoint) = self.endpoint(object) else {
            return true;
        };
        if !endpoint.send.is_empty() || !endpoint.recv.is_empty() {
            return true;
        }
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if let Some(table) = self.tcb(reference) {
                if table
                    .fault_ep
                    .is_some_and(|fault| self.cap_ref_matches(fault, object))
                {
                    return true;
                }
            }
        }
        false
    }

    fn notification_in_use(&self, object: ObjRef) -> bool {
        if self
            .notification(object)
            .is_some_and(|notification| notification.waiters.iter().any(Option::is_some))
        {
            return true;
        }
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self
                .tcb(reference)
                .is_some_and(|table| table.bound_ntfn == Some(object))
            {
                return true;
            }
            if self
                .irq(reference)
                .is_some_and(|irq| irq.notification == Some(object))
            {
                return true;
            }
        }
        false
    }

    fn page_table_has_entries(&self, root: u64) -> bool {
        let Some(ops) = self.page_table_ops else {
            return false;
        };
        (0..512).any(|index| ops.read_pte(root, index) != 0)
    }

    fn vspace_in_use(&self, object: ObjRef) -> bool {
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self
                .tcb(reference)
                .is_some_and(|table| table.vspace == Some(object))
            {
                return true;
            }
            if reference == object
                && self
                    .vspace(object)
                    .is_some_and(|space| space.mappings.iter().any(Option::is_some))
            {
                return true;
            }
            if let Some(table) = self.page_table(reference) {
                if table.mapped.is_some_and(|mapping| mapping.vspace == object) {
                    return true;
                }
            }
        }
        false
    }

    fn cnode_in_use(&self, object: ObjRef) -> bool {
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self
                .tcb(reference)
                .is_some_and(|table| table.cnode == Some(object))
            {
                return true;
            }
        }
        false
    }

    fn object_in_use(&self, object: ObjRef, ty: ObjType) -> bool {
        match ty {
            ObjType::Frame => self
                .frame(object)
                .is_some_and(|frame| self.frame_still_mapped(object, *frame)),
            ObjType::Endpoint => self.endpoint_in_use(object),
            ObjType::Notification => self.notification_in_use(object),
            ObjType::PageTable => self.page_table(object).is_some_and(|table| {
                table.mapped.is_some() || self.page_table_has_entries(table.root)
            }),
            ObjType::VSpace => self.vspace_in_use(object),
            ObjType::CNode => self.cnode_in_use(object),
            ObjType::SchedContext => false,
            _ => true,
        }
    }

    /// Destroy one retyped object through its root capability.  The caller
    /// must hold the original retype cap (not a copy/mint), every derived cap
    /// must already be revoked, and the object must not be in use.  The
    /// object slot is released by `destroy_object`; memory reclamation is
    /// handled by the Untyped that owns it.
    pub fn destroy_cap_object(&mut self, tcb: ObjRef, slot: u8) -> KResult<()> {
        let cap = self.lookup_cap(tcb, slot, None, Rights::WRITE)?;
        let creator = self.get(cap.obj).ok_or(KError::ObjectDestroyed)?.creator;
        match cap.ty {
            ObjType::Frame
            | ObjType::Endpoint
            | ObjType::Notification
            | ObjType::PageTable
            | ObjType::VSpace
            | ObjType::CNode
            | ObjType::SchedContext => {}
            _ => return Err(KError::InvalidArg),
        }
        let Some(creator) = creator else {
            return Err(KError::NoRights);
        };
        if cap.root || cap.parent != Some(creator) {
            return Err(KError::NoRights);
        }
        if self.other_caps_to(cap.obj, cap.id) != 0 {
            return Err(KError::HasDependents);
        }
        if self.object_in_use(cap.obj, cap.ty) {
            return Err(KError::Busy);
        }
        self.destroy_object(cap.obj)
    }

    fn destroy_object(&mut self, obj: ObjRef) -> KResult<()> {
        let Some(kind) = self.get(obj).map(|object| object.kind) else {
            return Ok(());
        };
        if self.current.contains(&Some(obj)) {
            return Err(KError::IllegalOperation);
        }
        if self.root == Some(obj) {
            return Err(KError::IllegalOperation);
        }
        if matches!(kind, Kind::VSpace(_))
            && self
                .current
                .iter()
                .flatten()
                .any(|current| self.tcb(*current).and_then(|tcb| tcb.vspace) == Some(obj))
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
            Kind::CNode(_) => self.destroy_cnode(obj)?,
            Kind::SchedContext(_) => self.destroy_sched_context(obj),
            Kind::Frame(frame) => {
                self.unmap_frame_everywhere(frame);
                self.unmap_iommu_frame_everywhere(obj);
            }
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
                    // Vòng 83 (audit #2 P1-6): TCB vừa bị Faulted phải được
                    // gỡ khỏi runqueue; nếu không nó vẫn được pick_on chọn và
                    // chạy với vspace = None.
                    if self.tcb(r).map(|tcb| tcb.state) == Some(ThreadState::Faulted) {
                        self.remove_from_runq(r);
                    }
                }
            }
            Kind::Irq(_)
            | Kind::Iommu(_)
            | Kind::BlockDevice(_)
            | Kind::Untyped(_)
            | Kind::Free => {}
        }
        self.remove_caps_to(obj);
        let (owner, reserved, alloc_offset, alloc_len) = match self.get(obj) {
            Some(object) => {
                let reserved = match object.kind {
                    Kind::Untyped(child) => child.max_objects,
                    _ => 0,
                };
                (
                    object.owner,
                    reserved,
                    object.alloc_offset as usize,
                    object.alloc_len as usize,
                )
            }
            None => (None, 0, 0, 0),
        };
        if let Some(owner) = owner {
            if let Some(region) = self.untyped_mut(owner) {
                region.objects = region.objects.saturating_sub(1);
                region.reserved = region.reserved.saturating_sub(reserved);
            }
            if alloc_len != 0 {
                self.untyped_reclaim(owner, alloc_offset, alloc_len);
            }
        }
        if let Some(object) = self.get_mut(obj) {
            if let Kind::Tcb(tcb) = &mut object.kind {
                tcb.context_slot = 0xFF;
            }
            object.kind = Kind::Free;
            object.owner = None;
            object.creator = None;
            object.alloc_offset = 0;
            object.alloc_len = 0;
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
            if self
                .current
                .iter()
                .flatten()
                .any(|current| objects[..object_count].contains(current))
            {
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

        const BITS: usize = (MAX_OBJECTS * MAX_CAPS_ROOT + 63) / 64;
        let mut victim_bits = [0u64; BITS];
        let mut removed = 0usize;
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let mark = |s: usize, cap: Cap, victim_bits: &mut [u64; BITS], removed: &mut usize| {
                let remove = self.is_descendant(&cap, target_ref)
                    || (target_cap.ty == ObjType::Untyped
                        && objects[..object_count].contains(&cap.obj));
                if remove {
                    let bit = i * MAX_CAPS_ROOT + s;
                    victim_bits[bit / 64] |= 1u64 << (bit % 64);
                    *removed += 1;
                }
            };
            if let Some(slots) = self.cspace(r) {
                for s in 1..slots.len() {
                    if let Some(cap) = slots[s] {
                        mark(s, cap, &mut victim_bits, &mut removed);
                    }
                }
            }
            if let Some(node) = self.cnode(r) {
                for s in 1..node.slots.len() {
                    if let Some(cap) = node.slots[s] {
                        mark(s, cap, &mut victim_bits, &mut removed);
                    }
                }
            }
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            let capacity = self.cspace_capacity(r);
            for s in 1..capacity {
                let bit = i * MAX_CAPS_ROOT + s;
                if victim_bits[bit / 64] & (1u64 << (bit % 64)) != 0 {
                    if let Some(slots) = self.cspace_mut(r) {
                        slots[s] = None;
                    }
                }
            }
            if let Some(node) = self.cnode_mut(r) {
                for s in 1..node.slots.len() {
                    let bit = i * MAX_CAPS_ROOT + s;
                    if victim_bits[bit / 64] & (1u64 << (bit % 64)) != 0 {
                        node.slots[s] = None;
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
        let slots = self.cspace_mut(tcb).ok_or(KError::InvalidCap)?;
        let cap = slots
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
        let Some(table) = self.tcb(tcb) else {
            return;
        };
        if table.queued || !matches!(table.state, ThreadState::Inactive | ThreadState::Ready) {
            return;
        }
        let affinity = table.affinity;
        // Vòng 98 (soi chay K3): chọn hart **ít việc nhất** trong các hart hợp
        // affinity. Trước đây luôn lấy hart chỉ số nhỏ nhất nên hart 0 đầy
        // (18) trước khi tràn sang hart 1, làm mất cân bằng tải.
        let mut target: Option<usize> = None;
        let mut best_len = usize::MAX;
        for hart in 0..MAX_HARTS {
            if affinity & (1u8 << hart) == 0 {
                continue;
            }
            let len = self.runq_len[hart];
            if len < MAX_TCBS && (target.is_none() || len < best_len) {
                target = Some(hart);
                best_len = len;
            }
        }
        // Audit #4 P1-6 được xét ở vòng 89: "enqueue bỏ rơi TCB Ready" chỉ xảy
        // ra khi mọi hàng ứng viên đầy, mà mỗi hàng chứa tới MAX_TCBS = 18 =
        // tổng số TCB, nên với các hằng số hiện tại nó không thể xảy ra. Giữ
        // nguyên `return` (không thêm đường chưa kiểm thử); nếu sau này tăng
        // MAX_TCBS hoặc giảm dung lượng hàng, phải xem lại.
        let (hart, _fell_back) = match target {
            Some(hart) => (hart, false),
            None => return,
        };
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
        self.runq[hart][self.runq_len[hart]] = Some(tcb);
        self.runq_len[hart] += 1;
    }

    pub fn has_ready_on(&self, hart: usize) -> bool {
        if hart >= MAX_HARTS {
            return false;
        }
        for owner in 0..MAX_HARTS {
            for queued in self.runq[owner][..self.runq_len[owner]].iter().flatten() {
                if self
                    .tcb(*queued)
                    .is_some_and(|tcb| tcb.affinity & (1u8 << hart) != 0)
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn runq_depth(&self, hart: usize) -> usize {
        self.runq_len.get(hart).copied().unwrap_or(0)
    }

    /// Local picks and steals performed by a hart since reset.
    pub fn scheduler_stats(&self, hart: usize) -> (u64, u64) {
        match (self.sched_local.get(hart), self.sched_steals.get(hart)) {
            (Some(local), Some(steals)) => (*local, *steals),
            _ => (0, 0),
        }
    }

    pub fn pick(&mut self) -> Option<ObjRef> {
        self.pick_on(0)
    }

    fn best_in_queue(&self, hart: usize, cpu: usize) -> Option<(usize, u8, u64)> {
        let mut best: Option<(usize, u8, u64)> = None;
        for index in 0..self.runq_len[hart] {
            let Some(reference) = self.runq[hart][index] else {
                continue;
            };
            let Some(tcb) = self.tcb(reference) else {
                continue;
            };
            if tcb.affinity & (1u8 << cpu) == 0 {
                continue;
            }
            let candidate = (index, tcb.priority, tcb.queue_seq);
            best = match best {
                None => Some(candidate),
                Some((_, best_priority, best_seq)) => {
                    if candidate.1 > best_priority
                        || (candidate.1 == best_priority && candidate.2 < best_seq)
                    {
                        Some(candidate)
                    } else {
                        best
                    }
                }
            };
        }
        best
    }

    pub fn pick_on(&mut self, hart: usize) -> Option<ObjRef> {
        if hart >= MAX_HARTS {
            return None;
        }
        let mut chosen = self
            .best_in_queue(hart, hart)
            .map(|(index, _, _)| (hart, index));
        let mut stole = false;
        if chosen.is_none() {
            let mut best: Option<(usize, usize, u8, u64)> = None;
            for owner in 0..MAX_HARTS {
                if owner == hart {
                    continue;
                }
                for index in 0..self.runq_len[owner] {
                    let Some(reference) = self.runq[owner][index] else {
                        continue;
                    };
                    let Some(tcb) = self.tcb(reference) else {
                        continue;
                    };
                    if tcb.affinity & (1u8 << hart) == 0 {
                        continue;
                    }
                    let candidate = (owner, index, tcb.priority, tcb.queue_seq);
                    best = match best {
                        None => Some(candidate),
                        Some((_, _, priority, seq)) => {
                            if candidate.2 > priority
                                || (candidate.2 == priority && candidate.3 < seq)
                            {
                                Some(candidate)
                            } else {
                                best
                            }
                        }
                    };
                }
            }
            if let Some((owner, index, _, _)) = best {
                chosen = Some((owner, index));
                stole = true;
            }
        }
        let (owner, index) = chosen?;
        let reference = self.runq[owner][index]?;
        let mut shift = index;
        while shift + 1 < self.runq_len[owner] {
            self.runq[owner][shift] = self.runq[owner][shift + 1];
            shift += 1;
        }
        self.runq_len[owner] -= 1;
        self.runq[owner][self.runq_len[owner]] = None;
        if let Some(tcb) = self.tcb_mut(reference) {
            tcb.queued = false;
            tcb.state = ThreadState::Running;
        }
        if stole {
            self.sched_steals[hart] = self.sched_steals[hart].wrapping_add(1);
        } else {
            self.sched_local[hart] = self.sched_local[hart].wrapping_add(1);
        }
        self.current[hart] = Some(reference);
        self.stats.context_switches += 1;
        Some(reference)
    }

    pub fn yield_current(&mut self) -> Option<ObjRef> {
        self.yield_current_on(0)
    }

    pub fn yield_current_on(&mut self, hart: usize) -> Option<ObjRef> {
        // Keep `current` set until the architecture has saved the user context.
        // The runtime's switch_blocked() saves it, then pick() replaces it.
        let current = *self.current.get(hart)?;
        let current = current?;
        let mut suspended = false;
        if let Some(tcb) = self.tcb_mut(current) {
            if tcb.state == ThreadState::Suspended {
                tcb.queued = false;
                suspended = true;
            } else if tcb.state == ThreadState::Running {
                tcb.state = ThreadState::Ready;
            }
        }
        if !suspended {
            self.enqueue(current);
        }
        Some(current)
    }

    pub fn current(&self) -> Option<ObjRef> {
        self.current_on(0)
    }

    pub fn current_on(&self, hart: usize) -> Option<ObjRef> {
        self.current.get(hart).copied().flatten()
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

    /// Slot in the faulting TCB's cspace used for its fault endpoint.
    /// Runtime fault messages expose this value so a supervisor can identify
    /// which faulting task generated the record without trusting the address.
    pub fn fault_endpoint_slot(&self, tcb: ObjRef) -> Option<u8> {
        let fault = self.tcb(tcb)?.fault_ep?;
        self.fault_endpoint(tcb)?;
        Some(fault.addr.slot)
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
        self.remove_from_runq(tcb);
        let table = self.tcb_mut(tcb).ok_or(KError::ObjectDestroyed)?;
        table.state = ThreadState::BlockedFault;
        table.queued = false;
        let _ = delivered;
        for current in self.current.iter_mut() {
            if *current == Some(tcb) {
                *current = None;
            }
        }
        Ok(())
    }

    /// Suspend the currently running TCB after a user fault and return the
    /// next runnable thread (if any).  The TCB is not destroyed so its fault
    /// can later be reported to a fault endpoint.
    pub fn fault_current(&mut self) -> Option<ObjRef> {
        self.fault_current_on(0)
    }

    pub fn fault_current_on(&mut self, hart: usize) -> Option<ObjRef> {
        let slot = self.current.get_mut(hart)?;
        let old = slot.take()?;
        if let Some(table) = self.tcb_mut(old) {
            table.state = ThreadState::Faulted;
            table.queued = false;
        }
        self.pick_on(hart)
    }

    pub fn tick(&mut self, hart: usize) -> usize {
        if let Some(counter) = self.ticks.get_mut(hart) {
            *counter = counter.wrapping_add(1);
            self.stats.ticks = *counter;
            if hart == 0 {
                self.system_ticks
                    .store(*counter, core::sync::atomic::Ordering::Release);
            }
        }
        let now = self.fast_time();
        let slept = self.wake_sleepers(now);
        let budgeted = self.wake_budgeters(now);
        let _ = self.charge_current_budget(hart, now);
        slept + budgeted
    }

    /// Move every deadline-expired `BlockedSleep` TCB to the ready queue.
    /// Returns the number of tasks woken by this call.
    fn wake_sleepers(&mut self, now: u64) -> usize {
        let mut woke = 0usize;
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            let expired = match self.objs[index].kind {
                Kind::Tcb(tcb) => {
                    tcb.state == ThreadState::BlockedSleep
                        && tcb.wake_at.is_some_and(|deadline| deadline <= now)
                }
                _ => false,
            };
            if !expired {
                continue;
            }
            if let Some(tcb) = self.tcb_mut(reference) {
                tcb.wake_at = None;
                tcb.state = ThreadState::Ready;
                tcb.resume = Some(Message::empty(0));
                tcb.resume_error = None;
            }
            self.enqueue(reference);
            woke += 1;
        }
        woke
    }

    /// Lock-free system time source.  Only the BSP updates it; all harts may
    /// read it without taking the BKL.
    pub fn fast_time(&self) -> u64 {
        self.system_ticks
            .load(core::sync::atomic::Ordering::Acquire)
    }

    /// BSP ticks.  System-wide test timing always uses hart 0 so that the AP
    /// timer cannot make the BSP measurement depend on itself.
    pub fn ticks(&self) -> u64 {
        self.ticks[0]
    }

    pub fn ticks_on(&self, hart: usize) -> u64 {
        self.ticks.get(hart).copied().unwrap_or(0)
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
        self.user_syscall_on(tcb, 0, number, a0, a1, a2, a3, a4, a5)
    }

    pub fn user_syscall_on(
        &mut self,
        tcb: ObjRef,
        hart: usize,
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
            if self.yield_current_on(hart).is_some() {
                return UserAction::Blocked;
            }
            return UserAction::Return {
                value: 0,
                error: KError::Ok,
            };
        }
        if number == SyscallId::GetTime as u64 {
            // System-wide monotonic time is the BSP tick source.  Per-hart
            // counters remain internal for the SMP harness and preemption
            // proof, but a task that migrates between harts must never see
            // wall time jump backwards or forwards.
            return UserAction::Return {
                value: self.fast_time(),
                error: KError::Ok,
            };
        }
        if number == SyscallId::ConsoleRead as u64 {
            return UserAction::ConsoleRead;
        }
        if number == SyscallId::Sleep as u64 {
            if a0 == 0 {
                return UserAction::Return {
                    value: 0,
                    error: KError::Ok,
                };
            }
            if !self.has_ready_on(hart) {
                // The runtime has no BSP idle task.  Let user space fall back
                // to cooperative yielding until the deadline instead of
                // parking with no runnable TCB.
                return UserAction::Return {
                    value: 0,
                    error: KError::WouldBlock,
                };
            }
            let deadline = self.fast_time().saturating_add(a0);
            let Some(table) = self.tcb_mut(tcb) else {
                return UserAction::Return {
                    value: 0,
                    error: KError::InvalidCap,
                };
            };
            table.state = ThreadState::BlockedSleep;
            table.wake_at = Some(deadline);
            return UserAction::Sleep;
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

        if number == SyscallId::BlockRead as u64
            || number == SyscallId::BlockWrite as u64
            || number == SyscallId::BlockFlush as u64
        {
            let flush = number == SyscallId::BlockFlush as u64;
            let write = number == SyscallId::BlockWrite as u64 || flush;
            let rights = if write { Rights::WRITE } else { Rights::READ };
            if self
                .lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::BlockDevice),
                    rights,
                )
                .is_err()
            {
                return UserAction::Return {
                    value: 0,
                    error: KError::InvalidCap,
                };
            }
            if flush {
                return UserAction::BlockFlush;
            }
            if self.validate_user_range(tcb, a2, 512, write).is_err() {
                return UserAction::Return {
                    value: 0,
                    error: KError::InvalidArg,
                };
            }
            return if write {
                UserAction::BlockWrite { lba: a1, ptr: a2 }
            } else {
                UserAction::BlockRead { lba: a1, ptr: a2 }
            };
        }

        match number {
            n if n == SyscallId::Send as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
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
                    slot_or_invalid!(a0),
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
                    slot_or_invalid!(a0),
                    Some(ObjType::Endpoint),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let endpoint = cap.obj;
                let recv_slot = slot_or_invalid!(a1);
                match self.recv(tcb, endpoint, recv_slot) {
                    Ok(Some(msg)) => message_action(KError::Ok, 0, &msg),
                    Ok(None) => UserAction::Blocked,
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::ReplyRecv as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Endpoint),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let endpoint = cap.obj;
                let recv_slot = slot_or_invalid!(a1);
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
                    slot_or_invalid!(a0),
                    Some(ObjType::Notification),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let ntfn = cap.obj;
                let bits = if a1 != 0 {
                    a1
                } else if cap.badge != 0 {
                    cap.badge
                } else {
                    1
                };
                match self.signal(ntfn, bits) {
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
                    slot_or_invalid!(a0),
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
                    slot_or_invalid!(a0),
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
            n if n == SyscallId::IoMap as u64 => {
                let domain = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Iommu),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let frame = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a2),
                    Some(ObjType::Frame),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let rights = Rights((a3 as u8) & (Rights::READ | Rights::WRITE).0);
                match self.iommu_map(domain.obj, frame.obj, a1, rights) {
                    Ok(mapping) => UserAction::Return {
                        value: mapping.iova,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::IoUnmap as u64 => {
                let domain = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Iommu),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.iommu_unmap(domain.obj, a1) {
                    Ok(mapping) => UserAction::Return {
                        value: mapping.iova,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::IrqGet as u64 => {
                // Until an IRQControl object exists, only the bootstrap root
                // may turn a logical IRQ into a capability.  Root can copy the
                // resulting cap to the driver/client that needs it.
                if self.root != Some(tcb) {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NoRights,
                    };
                }
                let dest = match slot_arg(a1) {
                    Ok(dest) if dest != 0 && (dest as usize) < self.cspace_capacity(tcb) => dest,
                    _ => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                if self
                    .cspace(tcb)
                    .and_then(|slots| slots.get(dest as usize))
                    .is_some_and(Option::is_some)
                {
                    return UserAction::Return {
                        value: 0,
                        error: KError::SlotOccupied,
                    };
                }
                let number = match u32::try_from(a0) {
                    Ok(number) => number,
                    Err(_) => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                let duplicate = self
                    .objs
                    .iter()
                    .any(|object| matches!(object.kind, Kind::Irq(irq) if irq.number == number));
                if duplicate {
                    return UserAction::Return {
                        value: 0,
                        error: KError::IrqInUse,
                    };
                }
                let object = match self.create_irq(number) {
                    Some(object) => object,
                    None => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::OutOfMemory,
                        }
                    }
                };
                let mut cap = Cap::new(
                    object,
                    ObjType::Irq,
                    Rights::READ | Rights::WRITE | Rights::GRANT,
                );
                cap.parent = None;
                match self.install_cap(tcb, dest, cap) {
                    Ok(()) => UserAction::Return {
                        value: dest as u64,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::IrqSetNotification as u64 => {
                let irq = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Irq),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let notification = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
                    Some(ObjType::Notification),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.set_irq_notification(irq.obj, notification.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::IrqAck as u64 => {
                let irq = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Irq),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.irq_ack(irq.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::Retype as u64 => {
                let untyped_slot = slot_or_invalid!(a0);
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
                    n if n == ObjType::CNode as u64 => ObjType::CNode,
                    n if n == ObjType::SchedContext as u64 => ObjType::SchedContext,
                    n if n == ObjType::Iommu as u64 => ObjType::Iommu,
                    n if n == ObjType::BlockDevice as u64 => ObjType::BlockDevice,
                    n if n == ObjType::Untyped as u64 => ObjType::Untyped,
                    _ => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                let dest = match slot_arg(a2) {
                    Ok(dest) if dest != 0 && (dest as usize) < self.cspace_capacity(tcb) => dest,
                    _ => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidArg,
                        }
                    }
                };
                if self
                    .cspace(tcb)
                    .and_then(|slots| slots.get(dest as usize))
                    .is_none()
                    || self
                        .cspace(tcb)
                        .and_then(|slots| slots.get(dest as usize))
                        .is_some_and(Option::is_some)
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
                let src_slot = slot_or_invalid!(a0);
                let dst_tcb_slot = slot_or_invalid!(a1);
                let dst_slot = slot_or_invalid!(a2);
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
            n if n == SyscallId::CapMint as u64 => {
                let src_slot = slot_or_invalid!(a0);
                let dst_tcb_slot = slot_or_invalid!(a1);
                let dst_slot = slot_or_invalid!(a2);
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
                let source = match self.lookup_cap(tcb, src_slot, None, Rights::NONE) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                if !matches!(source.ty, ObjType::Endpoint | ObjType::Notification) {
                    return UserAction::Return {
                        value: 0,
                        error: KError::WrongType,
                    };
                }
                // D6: a badge is an identity boundary.  Only an unbudged
                // source cap can be minted; no cap can be rebadged.
                if source.badge != 0 {
                    return UserAction::Return {
                        value: 0,
                        error: KError::IllegalOperation,
                    };
                }
                let rights = Rights((a3 as u8) & Rights::ALL.0) & source.rights;
                match self.copy_cap(tcb, src_slot, dest.obj, dst_slot, rights, a4) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::CNodeMint as u64 => {
                let destination = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::CNode),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let source_slot = slot_or_invalid!(a2);
                let dest_index = match slot_arg(a1) {
                    Ok(index) => index,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let rights = Rights((a3 as u8) & Rights::ALL.0);
                match self.cnode_mint(tcb, source_slot, destination.obj, dest_index, rights, a4) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::CNodeInstall as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let cnode = if a1 == 0 {
                    None
                } else {
                    match self.lookup_cap(
                        tcb,
                        slot_or_invalid!(a1),
                        Some(ObjType::CNode),
                        Rights::WRITE,
                    ) {
                        Ok(cap) => Some(cap.obj),
                        Err(error) => return UserAction::Return { value: 0, error },
                    }
                };
                match self.set_tcb_cnode(target.obj, cnode) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbDestroy as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.destroy_object(target.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::ObjectDestroy as u64 => {
                let slot = slot_or_invalid!(a0);
                match self.destroy_cap_object(tcb, slot) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::SchedContextConfigure as u64 => {
                let context = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::SchedContext),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.configure_sched_context(context.obj, a1, a2) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::SchedContextBind as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let context = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
                    Some(ObjType::SchedContext),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.bind_sched_context(target.obj, context.obj) {
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
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let vspace = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a3),
                    Some(ObjType::VSpace),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let fault_slot = slot_or_invalid!(a5);
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
            n if n == SyscallId::TcbRestart as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.restart_tcb(cap.obj, tcb) {
                    Ok((entry, stack)) => UserAction::RestartTcb {
                        tcb: cap.obj,
                        entry,
                        stack,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbBindNotification as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let notification = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
                    Some(ObjType::Notification),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.bind_notification(target.obj, notification.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbSetAffinity as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.set_tcb_affinity(cap.obj, a1 as u8) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbSuspend as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.suspend_tcb(cap.obj, tcb) {
                    Ok(hart) => UserAction::TcbSuspended { hart },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbSetPriority as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.set_tcb_priority(cap.obj, a1 as u8, tcb) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbResume as u64 => {
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
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
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let frame = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
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
            n if n == SyscallId::CapDelete as u64 => {
                let slot = slot_or_invalid!(a0);
                match self.delete_cap(tcb, slot) {
                    Ok(_) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::CapRevoke as u64 => {
                let slot = slot_or_invalid!(a0);
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
                    slot_or_invalid!(a0),
                    Some(ObjType::PageTable),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let space = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
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
                    slot_or_invalid!(a0),
                    Some(ObjType::Frame),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                // Vòng 80 (audit P0-1): trước đây arm này không kiểm tra con
                // trỏ người dùng, nên frame_write(slot, 0, <VA rác>, n) làm
                // kernel copy từ địa chỉ chưa map và chết ở EL1. Kiểm tra
                // đúng số byte runtime sẽ copy (đã kẹp USER_COPY_MAX).
                let len = (a3 as usize).min(vifast_abi::USER_COPY_MAX);
                if self.validate_user_range(tcb, a2, len, false).is_err() {
                    return UserAction::Return {
                        value: 0,
                        error: KError::InvalidArg,
                    };
                }
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
                    slot_or_invalid!(a0),
                    Some(ObjType::VSpace),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let frame = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a1),
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
                if a4 != 0 {
                    return UserAction::FrameRemap {
                        space: space.obj,
                        frame: frame.obj,
                        va: a2,
                        rights,
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
                    slot_or_invalid!(a0),
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
                    slot_or_invalid!(a2),
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

fn message_action(error: KError, _value: u64, msg: &Message) -> UserAction {
    let mut data = [0u64; 4];
    let words = msg.words();
    let mut i = 0;
    while i < data.len() && i < words.len() {
        data[i] = words[i];
        i += 1;
    }
    UserAction::Message {
        error,
        value: msg.badge,
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
            block_device: 13,
            server_elf_va: 0x1000_0000,
            server_elf_len: 4096,
            client_elf_va: 0x1000_2000,
            client_elf_len: 4096,
            sched_harts: 0b11,
        };
        let base = &info as *const BootInfo as usize;
        assert_eq!(core::mem::size_of::<BootInfo>(), 80);
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
            (32, &info.block_device as *const u16 as usize),
            (40, &info.server_elf_va as *const u64 as usize),
            (48, &info.server_elf_len as *const u64 as usize),
            (56, &info.client_elf_va as *const u64 as usize),
            (64, &info.client_elf_len as *const u64 as usize),
            (72, &info.sched_harts as *const u32 as usize),
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
    fn vspace_range_allocator_reuses_unmapped_holes() {
        let mut kernel = Kernel::new();
        let space = kernel
            .create_vspace_with_page_size(0x1000, 0x4000_0000, 0x4000, 4096)
            .unwrap();
        let frame_a = kernel.create_frame(0x9000, 4096).unwrap();
        let frame_b = kernel.create_frame(0xa000, 4096).unwrap();
        let explicit = kernel
            .map_frame(space, frame_a, 0x4000_1000, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(explicit.va, 0x4000_1000);
        let first_free = kernel
            .map_frame(space, frame_b, 0, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(first_free.va, 0x4000_0000);
        kernel.unmap_frame(space, first_free.va).unwrap();
        let reused = kernel
            .map_frame(space, frame_b, 0, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(reused.va, first_free.va);
        assert!(kernel.check_invariants().is_ok());
    }

    struct FlushOps;

    static FLUSH_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static FLUSH_ASID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static FLUSH_MASK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static FLUSH_OPS: FlushOps = FlushOps;

    impl PageTableOps for FlushOps {
        fn flush_all_asid(&self, asid: u16, targets: u32) {
            FLUSH_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            FLUSH_ASID.store(asid as usize, std::sync::atomic::Ordering::SeqCst);
            FLUSH_MASK.store(targets as usize, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn reclaim_fuzz(steps: usize) {
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x700_0000, 0x8000, 4096, 8)
            .unwrap();
        let kinds = [
            ObjType::Frame,
            ObjType::PageTable,
            ObjType::Endpoint,
            ObjType::Notification,
            ObjType::SchedContext,
            ObjType::CNode,
        ];
        let mut seed = 0x5eed_1234_5678_9abcu64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        let mut live: [Option<ObjRef>; 4] = [None; 4];
        let mut allocations = 0usize;
        let mut destroys = 0usize;
        for step in 0..steps {
            let free_slot = live.iter().position(|slot| slot.is_none());
            if next() % 100 < 72 {
                if let Some(slot) = free_slot {
                    let ty = kinds[next() % kinds.len()];
                    if let Ok(object) = kernel.retype(untyped, ty) {
                        live[slot] = Some(object);
                        allocations += 1;
                    }
                }
            } else {
                let mut index = next() % live.len();
                for _ in 0..live.len() {
                    if live[index].is_some() {
                        break;
                    }
                    index = (index + 1) % live.len();
                }
                if let Some(object) = live[index].take() {
                    kernel.destroy_object(object).unwrap();
                    destroys += 1;
                }
            }
            let region = kernel.untyped(untyped).unwrap();
            let free_sum: usize = region.free.iter().flatten().map(|f| f.len as usize).sum();
            let live_sum: usize = live
                .iter()
                .flatten()
                .map(|object| kernel.get(*object).unwrap().alloc_len as usize)
                .sum();
            // `bump` may include alignment padding below aligned objects; all
            // other bytes must be either live or on the free list.
            assert!(
                free_sum + live_sum <= region.bump,
                "free/live overlap at step {step}: bump={} free={free_sum} live={live_sum}",
                region.bump
            );
            assert!(kernel.check_invariants().is_ok(), "invariant after step");
        }
        // Free the remaining objects in LIFO order so the free list can
        // coalesce them with the watermark instead of fragmenting.
        let mut remaining: [Option<ObjRef>; 4] = [None; 4];
        for (slot, object) in live.iter_mut().enumerate() {
            remaining[slot] = object.take();
        }
        let mut ordered: [Option<ObjRef>; 4] = [None; 4];
        for index in 0..remaining.len() {
            let mut largest: Option<(usize, ObjRef)> = None;
            for slot in remaining.iter() {
                if let Some(object) = slot {
                    let offset = kernel.get(*object).unwrap().alloc_offset as usize;
                    if largest.is_none_or(|(best, _)| offset > best) {
                        largest = Some((offset, *object));
                    }
                }
            }
            if let Some((_, object)) = largest {
                ordered[index] = Some(object);
                for slot in remaining.iter_mut() {
                    if *slot == Some(object) {
                        *slot = None;
                    }
                }
            }
        }
        for object in ordered.iter_mut().filter_map(|slot| slot.take()) {
            kernel.destroy_object(object).unwrap();
        }
        let (_, bump, objects, _) = kernel.untyped_usage(untyped).unwrap();
        let region = kernel.untyped(untyped).unwrap();
        assert!(bump <= 0x8000, "bump escaped the arena: bump={bump}");
        assert_eq!(objects, 0);
        assert_eq!(region.leaked, 0, "mixed-size fuzz leaked bytes");
        assert!(kernel.check_invariants().is_ok());
        assert!(allocations > 0 && destroys > 0);
    }

    #[test]
    fn reclaim_fuzz_mixed_sizes_keeps_invariants_and_reclaims_all() {
        reclaim_fuzz(2_000);
    }

    #[test]
    #[ignore = "long seeded reclaim fuzz; run with --ignored"]
    fn reclaim_fuzz_long_keeps_invariants_and_reclaims_all() {
        reclaim_fuzz(100_000);
    }

    #[test]
    fn cap_revoke_churn_reclaims_every_round() {
        for round in 0..200u64 {
            let mut kernel = Kernel::new();
            let root = kernel.root().unwrap();
            let untyped = kernel
                .create_untyped_with_limit(0x800_0000 + round as usize * 0x20_0000, 0x2000, 4096, 4)
                .unwrap();
            kernel
                .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
                .unwrap();
            for (slot, ty) in [(2u64, ObjType::Frame), (3, ObjType::Endpoint)] {
                let action = kernel.user_syscall(
                    root,
                    SyscallId::Retype as u64,
                    1,
                    ty as u64,
                    slot,
                    0,
                    0,
                    0,
                );
                assert!(matches!(
                    action,
                    UserAction::Return {
                        error: KError::Ok,
                        ..
                    }
                ));
            }
            kernel.revoke_cap(root, 1).unwrap();
            let region = kernel.untyped(untyped).unwrap();
            assert_eq!(region.objects, 0, "round {round}");
            assert_eq!(region.leaked, 0, "round {round}");
            assert!(kernel.check_invariants().is_ok());
        }
    }

    #[test]
    fn destroying_a_server_wakes_a_client_blocked_in_reply() {
        // Vòng 86 (audit #3 P1-3): xác nhận destroy TCB server có đánh thức
        // client đang BlockedReply (detach_tcb gọi wake_error).
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 1).unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let call = Message::new(0x1234, &[7]).unwrap();
        // Server chưa recv nên send(call) đẩy vào hàng đợi và trả Ok(true)?
        // Cần call thật: server recv trước để nhận caller.
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None));
        assert_eq!(kernel.send(client, endpoint, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        kernel.destroy_object(server).unwrap();
        let client_state = kernel.tcb(client).unwrap();
        assert_eq!(client_state.state, ThreadState::Ready);
        assert_eq!(client_state.resume_error, Some(KError::ObjectDestroyed));
    }

    #[test]
    #[test]
    fn wake_error_does_not_queue_a_running_task() {
        // Vòng 98 (soi chay K1): task đang Running trên hart khác mà bị
        // wake_error sẽ vừa Running vừa queued -> có thể chạy hai lần.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("runner", 5).unwrap();
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Running;
        kernel.wake_error(tcb, KError::WouldBlock);
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::Running);
        assert!(!table.queued, "Running task must not be queued");
        assert_eq!(table.resume_error, Some(KError::WouldBlock));
    }

    #[test]
    fn wake_error_queues_a_blocked_task() {
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("blocked", 5).unwrap();
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::BlockedReply;
        kernel.wake_error(tcb, KError::ObjectDestroyed);
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert!(table.queued);
        assert_eq!(table.resume_error, Some(KError::ObjectDestroyed));
    }

    #[test]
    fn enqueue_prefers_the_least_loaded_eligible_hart() {
        // Vòng 98 (soi chay K3): trước đây luôn chọn hart chỉ số nhỏ nhất nên
        // hart 0 đầy trước khi tràn sang hart 1.
        let mut kernel = Kernel::new();
        let first = kernel.create_tcb("first", 3).unwrap();
        kernel.tcb_mut(first).unwrap().affinity = 0b11;
        kernel.enqueue(first);
        let second = kernel.create_tcb("second", 3).unwrap();
        kernel.tcb_mut(second).unwrap().affinity = 0b11;
        kernel.enqueue(second);
        assert_eq!(kernel.pick_on(1), Some(second));
        assert_eq!(kernel.pick_on(0), Some(first));
    }

    #[test]
    fn destroy_chain_wakes_every_waiting_client() {
        // Vòng 98 (soi chay K2): chuỗi A->B->C; destroy C phải đánh thức B,
        // sau đó destroy B phải đánh thức A.
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("a", 1).unwrap();
        let b = kernel.create_tcb("b", 1).unwrap();
        let c = kernel.create_tcb("c", 1).unwrap();
        let ep_b = kernel.create_endpoint().unwrap();
        let ep_c = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(b, ep_b, 4), Ok(None));
        assert_eq!(kernel.recv(c, ep_c, 4), Ok(None));
        let call = Message::new(0x11, &[1]).unwrap();
        assert_eq!(kernel.send(a, ep_b, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::BlockedReply);
        let call = Message::new(0x22, &[2]).unwrap();
        assert_eq!(kernel.send(b, ep_c, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(b).unwrap().state, ThreadState::BlockedReply);
        kernel.destroy_object(c).unwrap();
        assert_eq!(kernel.tcb(b).unwrap().state, ThreadState::Ready);
        assert_eq!(
            kernel.tcb(b).unwrap().resume_error,
            Some(KError::ObjectDestroyed)
        );
        kernel.destroy_object(b).unwrap();
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::Ready);
        assert_eq!(
            kernel.tcb(a).unwrap().resume_error,
            Some(KError::ObjectDestroyed)
        );
    }

    #[test]
    fn notification_wakes_every_waiter_in_the_queue() {
        // Vòng 88 (audit #4 P0): một slot duy nhất từng khiến waiter thứ hai
        // ghi đè waiter đầu (task đầu kẹt BlockedNotification vĩnh viễn).
        // Root cố ý park 4 worker trên cùng notification, nên hàng đợi 6 slot
        // và signal phải đánh thức TẤT CẢ.
        let mut kernel = Kernel::new();
        let names = ["w0", "w1", "w2", "w3"];
        let mut waiters = [None; 4];
        for (index, name) in names.iter().enumerate() {
            waiters[index] = Some(kernel.create_tcb(name, 5).unwrap());
        }
        // Dùng chung một notification cho cả bốn (như root park worker).
        let ntfn = kernel.create_notification().unwrap();
        for tcb in waiters.into_iter().flatten() {
            assert_eq!(kernel.wait_as(tcb, ntfn), Ok(None));
            assert_eq!(
                kernel.tcb(tcb).unwrap().state,
                ThreadState::BlockedNotification
            );
        }
        assert_eq!(kernel.signal(ntfn, 0b10), Ok(()));
        for tcb in waiters.into_iter().flatten() {
            assert_eq!(kernel.tcb(tcb).unwrap().state, ThreadState::Ready);
        }
    }

    #[test]
    fn signal_skips_waiters_that_are_not_blocked() {
        // Vòng 88 (audit #4): nếu waiter đã được đánh thức bởi đường khác
        // (state khác BlockedNotification) thì signal không được đưa nó vào
        // runqueue lần nữa, và không được nuốt bit.
        let mut kernel = Kernel::new();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(waiter, ntfn), Ok(None));
        kernel.tcb_mut(waiter).unwrap().state = ThreadState::Suspended;
        assert_eq!(kernel.signal(ntfn, 0b1000), Ok(()));
        assert_eq!(kernel.tcb(waiter).unwrap().state, ThreadState::Suspended);
        assert_eq!(kernel.poll(ntfn), Ok(0b1000));
    }

    #[test]
    fn signal_with_dead_waiter_keeps_the_bits() {
        // Vòng 86 (audit #3 P0-1): waiter đã chết không được làm mất bit.
        let mut kernel = Kernel::new();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(waiter, ntfn), Ok(None));
        kernel.destroy_object(waiter).unwrap();
        assert_eq!(kernel.signal(ntfn, 0b100), Ok(()));
        assert_eq!(kernel.poll(ntfn), Ok(0b100));
    }

    #[test]
    fn destroyed_vspace_removes_its_tcb_from_the_runqueue() {
        // Vòng 83 (audit #2 P1-6): destroy VSpace làm TCB Faulted nhưng trước
        // đây không gỡ khỏi runqueue → pick_on vẫn trả TCB không có VSpace.
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x940_0000, 0x8000, 4096, 8)
            .unwrap();
        let vspace = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let tcb = kernel.create_tcb("worker", 5).unwrap();
        kernel.set_tcb_vspace(tcb, vspace).unwrap();
        kernel.enqueue(tcb);
        kernel.destroy_object(vspace).unwrap();
        assert_eq!(kernel.tcb(tcb).unwrap().state, ThreadState::Faulted);
        assert_ne!(kernel.pick_on(0), Some(tcb));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn vspace_destroy_flushes_asid_on_loaded_harts() {
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&FLUSH_OPS);
        let untyped = kernel
            .create_untyped_with_limit(0x900_0000, 0x8000, 4096, 8)
            .unwrap();
        let vspace = kernel.retype(untyped, ObjType::VSpace).unwrap();
        kernel.vspace_mut(vspace).unwrap().harts_loaded = 0b10;
        FLUSH_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        FLUSH_ASID.store(0, std::sync::atomic::Ordering::SeqCst);
        FLUSH_MASK.store(0, std::sync::atomic::Ordering::SeqCst);
        let asid = kernel.vspace(vspace).unwrap().asid;
        kernel.destroy_object(vspace).unwrap();
        assert_eq!(FLUSH_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            FLUSH_ASID.load(std::sync::atomic::Ordering::SeqCst),
            asid as usize
        );
        assert_eq!(FLUSH_MASK.load(std::sync::atomic::Ordering::SeqCst), 0b10);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn asid_allocation_skips_live_vspaces_after_wrap() {
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x920_0000, 0x8000, 4096, 8)
            .unwrap();
        let first = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let second = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let first_asid = kernel.vspace(first).unwrap().asid;
        let second_asid = kernel.vspace(second).unwrap().asid;
        assert_ne!(first_asid, second_asid);
        // Force the counter back to the two ASIDs that are still live.
        kernel.next_asid = first_asid;
        let third = kernel.retype(untyped, ObjType::VSpace).unwrap();
        let third_asid = kernel.vspace(third).unwrap().asid;
        assert_ne!(third_asid, first_asid);
        assert_ne!(third_asid, second_asid);
        assert_ne!(third_asid, 0);
        // After destroying the second VSpace its ASID may be reused.
        kernel.destroy_object(second).unwrap();
        kernel.next_asid = second_asid;
        let fourth = kernel.retype(untyped, ObjType::VSpace).unwrap();
        assert_eq!(kernel.vspace(fourth).unwrap().asid, second_asid);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn vspace_asids_are_nonzero_and_distinct() {
        let mut kernel = Kernel::new();
        let first = kernel.create_vspace(0x1000, 0x4000_0000, 0x2000).unwrap();
        let second = kernel.create_vspace(0x3000, 0x5000_0000, 0x2000).unwrap();
        let first_asid = kernel.vspace(first).unwrap().asid;
        let second_asid = kernel.vspace(second).unwrap().asid;
        assert_ne!(first_asid, 0);
        assert_ne!(second_asid, 0);
        assert_ne!(first_asid, second_asid);
        assert_eq!(kernel.current_asid(kernel.root().unwrap()), None);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn iommu_domain_maps_and_rejects_invalid_dma() {
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x200_0000, 0x4000, 4096, 8)
            .unwrap();
        let domain = kernel.retype(untyped, ObjType::Iommu).unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let mapping = kernel
            .iommu_map(domain, frame, IOMMU_IOVA_BASE, Rights::READ | Rights::WRITE)
            .unwrap();
        assert_eq!(mapping.pa, 0x9000);
        assert_eq!(
            kernel.iommu_map(domain, frame, IOMMU_IOVA_BASE, Rights::READ),
            Err(KError::AlreadyMapped)
        );
        assert_eq!(
            kernel.iommu_map(domain, frame, IOMMU_IOVA_BASE + 0x1000, Rights::EXEC),
            Err(KError::InvalidArg)
        );
        assert!(kernel
            .iommu(domain)
            .unwrap()
            .find(IOMMU_IOVA_BASE)
            .is_some());
        assert_eq!(kernel.iommu_unmap(domain, IOMMU_IOVA_BASE), Ok(mapping));
        assert!(kernel
            .iommu(domain)
            .unwrap()
            .find(IOMMU_IOVA_BASE)
            .is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    fn test_root_with_untyped(kernel: &mut Kernel, base: usize, limit: usize) -> ObjRef {
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(base, 0x4000, 4096, limit)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        root
    }

    fn retype_at(kernel: &mut Kernel, root: ObjRef, ty: ObjType, dest: u8) -> ObjRef {
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ty as u64,
                dest as u64,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: dest as u64,
                error: KError::Ok
            }
        );
        kernel
            .cap_at(SlotAddr {
                tcb: root,
                slot: dest,
            })
            .unwrap()
            .obj
    }

    #[test]
    fn object_destroy_requires_original_cap_and_no_dependents() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x700_0000, 8);
        let endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        kernel.copy_cap(root, 2, root, 3, Rights::ALL, 0).unwrap();
        // A copy is not the root capability.
        assert_eq!(kernel.destroy_cap_object(root, 3), Err(KError::NoRights));
        // The root cap cannot be destroyed while another cap still points at it.
        assert_eq!(
            kernel.destroy_cap_object(root, 2),
            Err(KError::HasDependents)
        );
        kernel.delete_cap(root, 3).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.get(endpoint).is_none());
        assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).is_none());
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::InvalidCap));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_mapped_frame() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x710_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        let mapping = kernel.map_frame(vspace, frame, 0, Rights::READ).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::Busy));
        kernel.unmap_frame(vspace, mapping.va).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_dma_mapped_frame() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x715_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let domain = retype_at(&mut kernel, root, ObjType::Iommu, 3);
        kernel
            .iommu_map(domain, frame, IOMMU_IOVA_BASE, Rights::READ)
            .unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::Busy));
        kernel.iommu_unmap(domain, IOMMU_IOVA_BASE).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_endpoint_with_waiters() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x720_0000, 16);
        let endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        let sender = retype_at(&mut kernel, root, ObjType::Tcb, 3);
        assert_eq!(
            kernel.send(sender, endpoint, 0, Message::default(), false),
            Ok(false)
        );
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::Busy));
        assert!(kernel.recv(root, endpoint, 0).unwrap().is_some());
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_bound_notification() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x730_0000, 16);
        let notification = retype_at(&mut kernel, root, ObjType::Notification, 2);
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 3);
        kernel.bind_notification(tcb, notification).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::Busy));
        kernel.tcb_mut(tcb).unwrap().bound_ntfn = None;
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_vspace_in_use() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x740_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        let mapping = kernel.map_frame(vspace, frame, 0, Rights::READ).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 3), Err(KError::Busy));
        kernel.unmap_frame(vspace, mapping.va).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 3), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_attached_page_table() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x750_0000, 16);
        let table = retype_at(&mut kernel, root, ObjType::PageTable, 2);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        kernel.page_table_mut(table).unwrap().mapped = Some(PageTableMapping {
            vspace,
            va_base: 0x4000_0000,
            level: 2,
        });
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::Busy));
        kernel.page_table_mut(table).unwrap().mapped = None;
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_rejects_unsupported_types() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x760_0000, 16);
        // Untyped root cap is rejected even though other checks would pass.
        assert_eq!(kernel.destroy_cap_object(root, 1), Err(KError::InvalidArg));
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 2);
        assert_eq!(kernel.destroy_cap_object(root, 2), Err(KError::InvalidArg));
        assert!(kernel.get(tcb).is_some());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_allows_sched_context_and_unbinds() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x770_0000, 16);
        let context = retype_at(&mut kernel, root, ObjType::SchedContext, 2);
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 3);
        kernel.configure_sched_context(context, 2, 10).unwrap();
        kernel.bind_sched_context(tcb, context).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.tcb(tcb).unwrap().sched_context.is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_from_copy_holder_is_rejected() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x780_0000, 16);
        let endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        let other = retype_at(&mut kernel, root, ObjType::Tcb, 3);
        kernel.copy_cap(root, 2, other, 2, Rights::ALL, 0).unwrap();
        assert_eq!(kernel.destroy_cap_object(other, 2), Err(KError::NoRights));
        assert!(kernel.get(endpoint).is_some());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn destroying_frame_removes_dma_mapping() {
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x210_0000, 0x4000, 4096, 8)
            .unwrap();
        let domain = kernel.retype(untyped, ObjType::Iommu).unwrap();
        let frame = kernel.create_frame(0xa000, 4096).unwrap();
        kernel
            .iommu_map(domain, frame, IOMMU_IOVA_BASE, Rights::READ)
            .unwrap();
        kernel.destroy_object(frame).unwrap();
        assert!(kernel
            .iommu(domain)
            .unwrap()
            .find(IOMMU_IOVA_BASE)
            .is_none());
        assert!(kernel.check_invariants().is_ok());
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
        // Vòng 84: slot không hợp lệ (> 255) phải trả InvalidArg thay vì
        // lặng lẽ rơi về slot 0.
        assert_eq!(
            kernel.user_syscall(root, SyscallId::CapDelete as u64, 300, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        // Vòng 80 (audit P0-1): FrameWrite phải kiểm tra con trỏ như mọi
        // syscall copy khác. Trước bản sửa, con trỏ rác vẫn tạo ra
        // UserAction::FrameWrite và runtime copy từ địa chỉ chưa map → chết
        // kernel ở EL1.
        let frame = kernel.create_frame(0x1f00_0000, 4096).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameWrite as u64,
                1,
                0,
                0x8000_0000,
                8,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameWrite as u64,
                1,
                0,
                0x10_0000_0000,
                8,
                0,
                0
            ),
            UserAction::FrameWrite {
                frame,
                offset: 0,
                ptr: 0x10_0000_0000,
                len: 8
            }
        );
    }

    #[test]
    fn failed_retype_returns_the_carved_bytes() {
        // Vòng 82 (audit P1-1): nếu alloc_owned thất bại sau untyped_take,
        // byte đã lấy phải được trả lại; nếu không, mỗi lần retype hỏng lại
        // đốt một khoảng Untyped và làm cạn arena (đúng kiểu x86 đã gặp).
        let mut kernel = Kernel::new();
        let untyped = kernel
            .create_untyped_with_limit(0x3000_0000, 0x8000, 4096, 1)
            .unwrap();
        let before = kernel.untyped_usage(untyped).unwrap();
        assert!(kernel.retype(untyped, ObjType::Frame).is_ok());
        let after_ok = kernel.untyped_usage(untyped).unwrap();
        assert!(after_ok.1 > before.1, "frame phải tiêu byte");
        assert_eq!(
            kernel.retype(untyped, ObjType::Frame),
            Err(KError::OutOfMemory)
        );
        let after_fail = kernel.untyped_usage(untyped).unwrap();
        assert_eq!(
            after_fail.1, after_ok.1,
            "retype thất bại không được đốt thêm byte"
        );
        assert_eq!(after_fail.2, after_ok.2, "và không được tăng số object");
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
    fn revoke_untyped_rejects_child_current_on_other_hart_before_mutating() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x520_0000, 4096, 4096, 8)
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
        kernel.tcb_mut(child).unwrap().affinity = 1 << 1;
        kernel.enqueue(child);
        assert_eq!(kernel.pick_on(1), Some(child));
        let usage_before = kernel.untyped_usage(untyped).unwrap();
        let cap_before = kernel.cap_at(SlotAddr { tcb: root, slot: 1 });
        assert_eq!(kernel.revoke_cap(root, 1), Err(KError::IllegalOperation));
        assert_eq!(kernel.untyped_usage(untyped).unwrap(), usage_before);
        assert!(kernel.get(child).is_some());
        assert_eq!(kernel.cap_at(SlotAddr { tcb: root, slot: 1 }), cap_before);
        assert!(kernel.check_invariants().is_ok());
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
    static ZERO_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static ZERO_OPS: ZeroOps = ZeroOps;

    impl PageTableOps for ZeroOps {
        fn zero_frame(&self, _pa: u64, _size: usize) {
            ZERO_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn object_destroy_reclaims_top_untyped_bytes() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x800_0000, 8);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        let endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 256);
        assert_eq!(kernel.untyped(untyped).unwrap().objects, 1);
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 0);
        assert_eq!(kernel.untyped(untyped).unwrap().objects, 0);
        assert!(kernel
            .untyped(untyped)
            .unwrap()
            .free
            .iter()
            .all(Option::is_none));
        assert!(kernel.get(endpoint).is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_reuses_exact_free_range() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x810_0000, 16);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let _endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 3);
        let old_pa = kernel.frame(frame).unwrap().pa;
        let bump = kernel.untyped(untyped).unwrap().bump;
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert_eq!(kernel.untyped(untyped).unwrap().bump, bump);
        let frame2 = retype_at(&mut kernel, root, ObjType::Frame, 4);
        assert_eq!(kernel.frame(frame2).unwrap().pa, old_pa);
        assert_eq!(kernel.get(frame2).unwrap().alloc_offset, 0);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, bump);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn untyped_reclaim_coalesces_adjacent_ranges() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x820_0000, 16);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        let _e0 = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        let _e1 = retype_at(&mut kernel, root, ObjType::Endpoint, 3);
        let _e2 = retype_at(&mut kernel, root, ObjType::Endpoint, 4);
        let _e3 = retype_at(&mut kernel, root, ObjType::Endpoint, 5);
        assert_eq!(kernel.destroy_cap_object(root, 4), Ok(()));
        assert_eq!(kernel.destroy_cap_object(root, 3), Ok(()));
        assert_eq!(
            kernel.untyped(untyped).unwrap().free[0],
            Some(FreeRegion {
                offset: 256,
                len: 512
            })
        );
        let e4 = retype_at(&mut kernel, root, ObjType::Endpoint, 6);
        assert_eq!(kernel.get(e4).unwrap().alloc_offset, 256);
        let e5 = retype_at(&mut kernel, root, ObjType::Endpoint, 7);
        assert_eq!(kernel.get(e5).unwrap().alloc_offset, 512);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn untyped_free_list_full_counts_leak() {
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x830_0000, 48);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        // 35 endpoints: offsets 0, 256, ..., 8704.
        for slot in 2..37 {
            retype_at(&mut kernel, root, ObjType::Endpoint, slot);
        }
        // Destroy every other endpoint so no two freed ranges are adjacent;
        // the 17th isolated free range does not fit the 16-entry list.
        for index in 0..17 {
            let slot = 2 + index * 2;
            assert_eq!(kernel.destroy_cap_object(root, slot), Ok(()));
        }
        let region = kernel.untyped(untyped).unwrap();
        assert_eq!(region.objects, 18);
        assert_eq!(region.leaked, 256);
        assert_eq!(region.free.iter().flatten().count(), MAX_UNTYPED_FREE);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn reused_frame_is_zeroed_and_keeps_pa() {
        let _guard = ZERO_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut kernel = Kernel::new();
        kernel.set_page_table_ops(&ZERO_OPS);
        let root = test_root_with_untyped(&mut kernel, 0x840_0000, 16);
        ZERO_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let _endpoint = retype_at(&mut kernel, root, ObjType::Endpoint, 3);
        let old_pa = kernel.frame(frame).unwrap().pa;
        assert_eq!(ZERO_CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        let frame2 = retype_at(&mut kernel, root, ObjType::Frame, 4);
        assert_eq!(kernel.frame(frame2).unwrap().pa, old_pa);
        assert_eq!(ZERO_CALLS.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn boot_chain_reset_retypes_and_configures_two_tcbs() {
        let mut kernel = Kernel::new();
        // Runtime initializes the kernel, runs the self-test kernel, then
        // resets the final kernel; model the reset boundary explicitly.
        kernel.reset();
        let root = kernel.root().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x8a0_0000, 0x8000, 4096, 8)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(untyped, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let mut tcbs = std::vec::Vec::new();
        let mut spaces = std::vec::Vec::new();
        for slot in [2u64, 3] {
            let action = kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::Tcb as u64,
                slot,
                0,
                0,
                0,
            );
            assert!(matches!(
                action,
                UserAction::Return {
                    error: KError::Ok,
                    ..
                }
            ));
            let tcb = kernel
                .cap_at(SlotAddr {
                    tcb: root,
                    slot: slot as u8,
                })
                .unwrap()
                .obj;
            tcbs.push(tcb);
            let space_action = kernel.user_syscall(
                root,
                SyscallId::Retype as u64,
                1,
                ObjType::VSpace as u64,
                slot + 2,
                0,
                0,
                0,
            );
            assert!(matches!(
                space_action,
                UserAction::Return {
                    error: KError::Ok,
                    ..
                }
            ));
            let space = kernel
                .cap_at(SlotAddr {
                    tcb: root,
                    slot: slot as u8 + 2,
                })
                .unwrap()
                .obj;
            spaces.push(space);
        }
        for (index, tcb) in tcbs.iter().enumerate() {
            kernel
                .configure_tcb(*tcb, 0x1000_0000, 0x2000_0000, spaces[index], 5)
                .unwrap();
            kernel.resume_tcb(*tcb).unwrap();
        }
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn tcb_context_slots_are_unique_and_reused() {
        let mut kernel = Kernel::new();
        let mut tcbs = std::vec::Vec::new();
        while kernel.tcb_count() < MAX_TCBS {
            tcbs.push(kernel.create_tcb("ctx", 1).unwrap());
        }
        assert!(kernel.create_tcb("overflow", 1).is_none());
        let mut slots: std::vec::Vec<u8> = tcbs
            .iter()
            .map(|tcb| kernel.context_slot(*tcb).unwrap())
            .collect();
        let freed = slots[2];
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), MAX_TCBS - 1);
        kernel.destroy_object(tcbs[2]).unwrap();
        let replacement = kernel.create_tcb("reuse", 1).unwrap();
        assert_eq!(kernel.context_slot(replacement), Some(freed));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_variant_sizes_for_pool_design() {
        eprintln!(
            "VARIANT Object={} Tcb={} CNode={} VSpace={} Endpoint={} Notification={} Frame={} Untyped={} Irq={} Iommu={} BlockDevice={} Cap={} Kernel={}",
            core::mem::size_of::<Object>(),
            core::mem::size_of::<Tcb>(),
            core::mem::size_of::<CNode>(),
            core::mem::size_of::<VSpace>(),
            core::mem::size_of::<Endpoint>(),
            core::mem::size_of::<Notification>(),
            core::mem::size_of::<Frame>(),
            core::mem::size_of::<Untyped>(),
            core::mem::size_of::<Irq>(),
            core::mem::size_of::<IommuDomain>(),
            core::mem::size_of::<BlockDevice>(),
            core::mem::size_of::<Cap>(),
            core::mem::size_of::<Kernel>(),
        );
        eprintln!(
            "LAYOUT objs={} root_cspace={} current={} size={}",
            core::mem::offset_of!(Kernel, objs),
            core::mem::offset_of!(Kernel, root_cspace),
            core::mem::offset_of!(Kernel, current),
            core::mem::size_of::<Kernel>(),
        );
        assert!(core::mem::size_of::<Object>() <= 4752);
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
        let _guard = ZERO_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
    fn tcb_destroy_frees_pool_slot_and_rejects_current_root() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let child = kernel.create_tcb("doomed", 1).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(child, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbDestroy as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok,
            }
        );
        assert!(kernel.get(child).is_none());
        assert!(kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).is_none());
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbDestroy as u64, 0, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap,
            }
        );
        assert_eq!(kernel.destroy_object(root), Err(KError::IllegalOperation));
        while kernel.tcb_count() < MAX_TCBS {
            assert!(kernel.create_tcb("replacement", 1).is_some());
        }
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
        kernel.current[0] = Some(root);
        assert_eq!(kernel.revoke_cap(root, 1), Err(KError::IllegalOperation));
        kernel.current[0] = None;

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
    fn set_affinity_rejects_empty_and_unknown_harts() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let tcb = kernel.create_tcb("aff", 1).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(tcb, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbSetAffinity as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbSetAffinity as u64, 1, 0x80, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbSetAffinity as u64, 1, 0b10, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(tcb).unwrap().affinity, 0b10);
    }

    #[test]
    fn affinity_filters_pick_on_hart() {
        let mut kernel = Kernel::new();
        let only_one = kernel.create_tcb("only1", 1).unwrap();
        let either = kernel.create_tcb("either", 1).unwrap();
        kernel.tcb_mut(only_one).unwrap().affinity = 1 << 1;
        kernel.tcb_mut(either).unwrap().affinity = 0b11;
        kernel.enqueue(only_one);
        kernel.enqueue(either);
        assert!(kernel.has_ready_on(1));
        assert_eq!(kernel.pick_on(0), Some(either));
        assert_eq!(kernel.pick_on(1), Some(only_one));
        assert_eq!(kernel.pick_on(2), None);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn current_tcb_is_per_hart_and_running_is_unique() {
        let mut kernel = Kernel::new();
        let hart0 = kernel.create_tcb("hart0", 1).unwrap();
        let hart1 = kernel.create_tcb("hart1", 1).unwrap();
        kernel.tcb_mut(hart1).unwrap().affinity = 1 << 1;
        kernel.enqueue(hart0);
        kernel.enqueue(hart1);
        assert_eq!(kernel.pick_on(0), Some(hart0));
        assert_eq!(kernel.pick_on(1), Some(hart1));
        assert_eq!(kernel.current_on(0), Some(hart0));
        assert_eq!(kernel.current_on(1), Some(hart1));
        assert_eq!(kernel.pick_on(2), None);
        assert!(kernel.check_invariants().is_ok());
        assert_eq!(kernel.destroy_object(hart0), Err(KError::IllegalOperation));
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

    fn syscall_fuzz_case(steps: usize) {
        let _guard = WALK_LOCK.lock().unwrap();
        for format in [PteFormat::Riscv, PteFormat::X86, PteFormat::AArch] {
            let mut seed = 0x5eed_1234_abcd_9876u64;
            let mut next = || {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                seed
            };
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
            let child = kernel.create_tcb("fuzz", 5).unwrap();
            let endpoint = kernel.retype(untyped, ObjType::Endpoint).unwrap();
            let block = kernel.retype(untyped, ObjType::BlockDevice).unwrap();
            for (slot, object, ty, rights) in [
                (1, untyped, ObjType::Untyped, Rights::ALL),
                (2, child, ObjType::Tcb, Rights::ALL),
                (3, endpoint, ObjType::Endpoint, Rights::ALL),
                (4, space, ObjType::VSpace, Rights::ALL),
                (5, frame, ObjType::Frame, Rights::ALL),
                (6, table, ObjType::PageTable, Rights::ALL),
                (
                    32,
                    block,
                    ObjType::BlockDevice,
                    Rights::READ | Rights::WRITE,
                ),
            ] {
                kernel
                    .install_cap(root, slot, Cap::new(object, ty, rights))
                    .unwrap();
            }
            kernel.set_tcb_vspace(root, space).unwrap();
            kernel.note_vspace_hart_loaded(root, 0);
            kernel.note_vspace_hart_loaded(root, 1);
            let types = [
                ObjType::Frame,
                ObjType::PageTable,
                ObjType::VSpace,
                ObjType::Tcb,
                ObjType::Endpoint,
                ObjType::Notification,
                ObjType::Untyped,
                ObjType::BlockDevice,
            ];
            let mut successes = 0usize;
            for step in 0..steps {
                let choice = next() % 12;
                let slot = 7 + (next() as usize % 25);
                let other = 7 + (next() as usize % 25);
                let va = 0x10_0000_0000u64 + (next() & 0x3f) * 0x1000;
                let rights = Rights((next() as u8) & Rights::ALL.0);
                let user_ptr = 0x10_0000_0000u64 + (next() & 0x3) * 0x1000;
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
                        1 + (next() % 6) as u64,
                        other as u64,
                        slot as u64,
                        rights.0 as u64,
                        0,
                        0,
                    ),
                    2 => (SyscallId::CapDelete as u64, slot as u64, 0, 0, 0, 0, 0),
                    3 => (
                        SyscallId::CapRevoke as u64,
                        1 + (next() % 6) as u64,
                        0,
                        0,
                        0,
                        0,
                        0,
                    ),
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
                    6 => (
                        SyscallId::PageTableMap as u64,
                        if next() & 1 == 0 { 6 } else { other as u64 },
                        4,
                        va,
                        0,
                        0,
                        0,
                    ),
                    7 => (
                        SyscallId::CapMint as u64,
                        3,
                        2,
                        slot as u64,
                        rights.0 as u64,
                        (next() & 1) * 0xffff,
                        0,
                    ),
                    8 => (
                        SyscallId::TcbSetAffinity as u64,
                        2,
                        [0u64, 1, 2, 3, 0x80][(next() as usize) % 5],
                        0,
                        0,
                        0,
                        0,
                    ),
                    9 => (
                        SyscallId::BlockRead as u64,
                        if next() & 1 == 0 { 32 } else { other as u64 },
                        next() & 7,
                        user_ptr,
                        0,
                        0,
                        0,
                    ),
                    10 => (
                        SyscallId::BlockWrite as u64,
                        if next() & 1 == 0 { 32 } else { other as u64 },
                        next() & 7,
                        user_ptr,
                        0,
                        0,
                        0,
                    ),
                    _ => (
                        SyscallId::CapCopy as u64,
                        3,
                        2,
                        slot as u64,
                        Rights::READ.0 as u64,
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
                if step % 1000 == 0 || steps <= 10_000 {
                    assert_eq!(
                        kernel.check_invariants(),
                        Ok(()),
                        "format {:?} failed at step {}",
                        format,
                        step
                    );
                }
            }
            assert_eq!(kernel.check_invariants(), Ok(()));
            assert!(
                successes > 100,
                "format {:?} only produced {} successful syscalls",
                format,
                successes
            );
        }
    }

    #[test]
    fn syscall_fuzz_three_pte_formats_keeps_invariants() {
        syscall_fuzz_case(10_000);
    }

    #[test]
    #[ignore = "long seeded syscall fuzz; run with --ignored"]
    fn syscall_fuzz_long_100k_keeps_invariants() {
        syscall_fuzz_case(100_000);
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
    #[test]
    fn syscall_status_matches_notimplemented_fallback() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        for (id, implemented) in SYSCALL_STATUS {
            let action = kernel.user_syscall(root, *id as u64, 0, 0, 0, 0, 0, 0);
            let not_implemented = matches!(
                action,
                UserAction::Return {
                    error: KError::NotImplemented,
                    ..
                }
            );
            assert_eq!(
                not_implemented, !*implemented,
                "syscall {:?} status does not match dispatch",
                id
            );
        }
    }

    #[test]
    fn block_syscalls_require_a_block_device_capability() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let caps = kernel
            .create_untyped_with_limit(0x2000, 0x1000, 4096, 4)
            .unwrap();
        let block = kernel.retype(caps, ObjType::BlockDevice).unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(block, ObjType::BlockDevice, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        let endpoint = kernel.retype(caps, ObjType::Endpoint).unwrap();
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        let ptr = 0u64;
        assert_eq!(
            kernel.user_syscall(root, SyscallId::BlockRead as u64, 0, 0, ptr, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::BlockWrite as u64, 2, 0, ptr, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );
        assert!(kernel
            .lookup_cap(root, 1, Some(ObjType::BlockDevice), Rights::READ)
            .is_ok());
        assert!(kernel
            .lookup_cap(root, 1, Some(ObjType::BlockDevice), Rights::WRITE)
            .is_ok());
    }

    #[test]
    fn capmint_copies_endpoint_with_badge() {
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
        let child = kernel.create_tcb("child", 5).unwrap();
        kernel
            .install_cap(root, 3, Cap::new(child, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CapMint as u64,
                1,
                3,
                1,
                Rights::READ.0 as u64,
                7,
                0,
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        let cap = kernel
            .lookup_cap(child, 1, Some(ObjType::Endpoint), Rights::READ)
            .unwrap();
        assert_eq!(cap.badge, 7);
    }

    #[test]
    fn capmint_rejects_rebadging_an_already_badged_cap() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel.copy_cap(root, 1, root, 2, Rights::ALL, 2).unwrap();
        let child = kernel.create_tcb("child", 5).unwrap();
        kernel
            .install_cap(root, 3, Cap::new(child, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CapMint as u64,
                2,
                3,
                1,
                Rights::READ.0 as u64,
                0,
                0,
            ),
            UserAction::Return {
                value: 0,
                error: KError::IllegalOperation
            }
        );
        assert!(kernel.lookup_cap(child, 1, None, Rights::NONE).is_err());
    }

    #[test]
    fn kernel_overwrites_sender_badge_on_direct_and_queued_delivery() {
        let mut kernel = Kernel::new();
        let sender = kernel.create_tcb("sender", 5).unwrap();
        let receiver = kernel.create_tcb("receiver", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();

        // Receiver waits: direct delivery path.
        assert_eq!(kernel.recv(receiver, endpoint, 4), Ok(None));
        let mut forged = Message::new(0xAA, &[7]).unwrap();
        forged.badge = 0xFFFF;
        assert_eq!(kernel.send(sender, endpoint, 2, forged, false), Ok(true));
        let delivered = kernel.take_resume(receiver).expect("direct message");
        assert_eq!(delivered.badge, 2);
        assert_eq!(
            delivered.badge, 2,
            "sender-supplied badge was not overwritten"
        );

        // No receiver: sender queues; recv must still stamp the cap badge.
        let receiver2 = kernel.create_tcb("receiver2", 5).unwrap();
        let endpoint2 = kernel.create_endpoint().unwrap();
        let mut forged = Message::new(0xBB, &[9]).unwrap();
        forged.badge = 0;
        assert_eq!(kernel.send(sender, endpoint2, 2, forged, false), Ok(false));
        let queued = kernel
            .recv(receiver2, endpoint2, 4)
            .unwrap()
            .expect("queued");
        assert_eq!(queued.badge, 2);
    }

    #[test]
    fn gettime_is_system_wide_bsp_time_not_per_hart() {
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("time-a", 5).unwrap();
        let b = kernel.create_tcb("time-b", 5).unwrap();
        kernel.tick(0);
        kernel.tick(0);
        kernel.tick(1);
        kernel.tick(1);
        kernel.tick(1);
        let a1 = kernel.user_syscall_on(a, 0, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0);
        let b1 = kernel.user_syscall_on(b, 1, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0);
        let UserAction::Return { value: av, .. } = a1 else {
            panic!("GetTime did not return");
        };
        let UserAction::Return { value: bv, .. } = b1 else {
            panic!("GetTime did not return");
        };
        assert_eq!(av, 2);
        assert_eq!(bv, 2);
        kernel.tick(1);
        let b2 = kernel.user_syscall_on(b, 1, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0);
        let UserAction::Return { value: bv2, .. } = b2 else {
            panic!("GetTime did not return");
        };
        assert_eq!(bv2, 2, "AP ticks must not leak into system time");
    }

    #[test]
    fn vspace_hart_mask_tracks_loaded_harts_and_full_flush_clear() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_vspace(0x1000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();
        kernel.note_vspace_hart_loaded(root, 0);
        kernel.note_vspace_hart_loaded(root, 1);
        assert_eq!(kernel.vspace_harts_loaded(space), 0b11);
        // Switching another VSpace into hart 1 must not clear hart 1's stale
        // V entries; only a full-ASID flush may do that.
        let other = kernel
            .create_vspace(0x2000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel.set_tcb_vspace(root, other).unwrap();
        kernel.note_vspace_hart_loaded(root, 1);
        assert_eq!(kernel.vspace_harts_loaded(space), 0b11);
        assert_eq!(kernel.vspace_harts_loaded(other), 0b10);
        // Full ASID flush of the first space clears only the flushed hart.
        assert_eq!(kernel.clear_vspace_harts(space, 1 << 0), 0b10);
        assert_eq!(kernel.vspace_harts_loaded(space), 0b10);
    }

    #[test]
    fn layout_sizes_for_status() {
        println!(
            "LAYOUT kernel={} object={} tcb={} cap={} option_cap={}",
            core::mem::size_of::<Kernel>(),
            core::mem::size_of::<Object>(),
            core::mem::size_of::<Tcb>(),
            core::mem::size_of::<Cap>(),
            core::mem::size_of::<Option<Cap>>()
        );
    }

    #[test]
    fn capdelete_syscall_removes_leaf_and_reuses_slot() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::CapDelete as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert!(kernel.lookup_cap(root, 1, None, Rights::NONE).is_err());
        assert!(kernel.get(frame).is_some());
        kernel
            .install_cap(root, 1, Cap::new(frame, ObjType::Frame, Rights::READ))
            .unwrap();
        assert!(kernel
            .lookup_cap(root, 1, Some(ObjType::Frame), Rights::READ)
            .is_ok());
    }

    #[test]
    fn capdelete_refuses_child_caps_and_self_tcb_or_vspace() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let child = kernel.create_tcb("child", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel.copy_cap(root, 1, child, 1, Rights::READ, 0).unwrap();
        assert_eq!(kernel.delete_cap(root, 1), Err(KError::IllegalOperation));
        assert!(kernel
            .lookup_cap(child, 1, Some(ObjType::Endpoint), Rights::READ)
            .is_ok());

        kernel
            .install_cap(root, 2, Cap::new(root, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(kernel.delete_cap(root, 2), Err(KError::IllegalOperation));
        let space = kernel
            .create_vspace(0x8000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel.set_tcb_vspace(root, space).unwrap();
        kernel
            .install_cap(root, 3, Cap::new(space, ObjType::VSpace, Rights::ALL))
            .unwrap();
        assert_eq!(kernel.delete_cap(root, 3), Err(KError::IllegalOperation));
    }

    #[test]
    fn capdelete_does_not_strand_queued_sender() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let sender = kernel.create_tcb("sender", 1).unwrap();
        let receiver = kernel.create_tcb("receiver", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        let sent = kernel
            .send(
                sender,
                endpoint,
                0,
                Message::new(0x42, &[7]).unwrap(),
                false,
            )
            .unwrap();
        assert!(!sent);
        assert!(kernel.delete_cap(root, 1).is_ok());
        let received = kernel.recv(receiver, endpoint, 4).unwrap().unwrap();
        assert_eq!(received.label, 0x42);
        assert_eq!(received.words()[0], 7);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn tcb_set_priority_only_lowers_within_caller_priority() {
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("target", 10).unwrap();
        kernel
            .install_cap(caller, 1, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(caller, SyscallId::TcbSetPriority as u64, 1, 7, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::PrioTooHigh
            }
        );
        assert_eq!(
            kernel.user_syscall(caller, SyscallId::TcbSetPriority as u64, 1, 5, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(target).unwrap().priority, 5);
        assert_eq!(
            kernel.user_syscall(caller, SyscallId::TcbSetPriority as u64, 1, 6, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::PrioTooHigh
            }
        );
    }

    #[test]
    fn tcb_suspend_refuses_root_and_handles_ready_and_running() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("target", 5).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(root, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.suspend_tcb(root, root),
            Err(KError::IllegalOperation)
        );
        let space = kernel
            .create_vspace(0x8000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel.set_tcb_vspace(target, space).unwrap();
        kernel
            .configure_tcb(target, 0x1000, 0x9000, space, 5)
            .unwrap();
        kernel.set_tcb_affinity(target, 0b10).unwrap();
        kernel.resume_tcb(target).unwrap();
        assert_eq!(kernel.suspend_tcb(target, root), Ok(None));
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Suspended);
        assert!(!kernel.has_ready_on(1));
        kernel.resume_tcb(target).unwrap();
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Ready);
        assert!(kernel.has_ready_on(1));
        let picked = kernel.pick_on(1).unwrap();
        assert_eq!(picked, target);
        assert_eq!(kernel.suspend_tcb(target, root), Ok(Some(1)));
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Suspended);
        // Emulate the timer switching away from the suspended current TCB.
        kernel.current[1] = None;
        kernel.resume_tcb(target).unwrap();
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Ready);
    }

    #[test]
    fn tcb_suspend_removes_blocked_receiver_from_endpoint_queue() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("target", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        let space = kernel
            .create_vspace(0x8000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel
            .configure_tcb(target, 0x1000, 0x9000, space, 5)
            .unwrap();
        assert_eq!(kernel.recv(target, endpoint, 4), Ok(None));
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::BlockedRecv);
        assert_eq!(kernel.suspend_tcb(target, root), Ok(None));
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Suspended);
        assert!(kernel.endpoint(endpoint).unwrap().recv.is_empty());
        assert_eq!(kernel.take_resume_error(target), Some(KError::WouldBlock));
        kernel.resume_tcb(target).unwrap();
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Ready);
    }

    #[test]
    fn tcb_bind_notification_and_restart() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("target", 5).unwrap();
        let notification = kernel.create_notification().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(notification, ObjType::Notification, Rights::ALL),
            )
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::TcbBindNotification as u64,
                1,
                2,
                0,
                0,
                0,
                0,
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(target).unwrap().bound_ntfn, Some(notification));

        let space = kernel
            .create_vspace(0x8000, 0x10_0000_0000, 0x10_0000)
            .unwrap();
        kernel
            .configure_tcb(target, 0x1234, 0x9000, space, 5)
            .unwrap();
        kernel.tcb_mut(target).unwrap().state = ThreadState::Suspended;
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbRestart as u64, 1, 0, 0, 0, 0, 0),
            UserAction::RestartTcb {
                tcb: target,
                entry: 0x1234,
                stack: 0x9000
            }
        );
        let restarted = kernel.tcb(target).unwrap();
        assert_eq!(restarted.state, ThreadState::Ready);
        assert!(restarted.queued);
    }

    #[test]
    fn sleep_blocks_until_bsp_tick_deadline() {
        let mut kernel = Kernel::new();
        let sleeper = kernel.create_tcb("sleeper", 5).unwrap();
        let idle = kernel.create_tcb("idle", 1).unwrap();
        kernel.enqueue(idle);
        kernel.enqueue(sleeper);
        assert_eq!(kernel.pick_on(0), Some(sleeper));
        assert_eq!(kernel.tick(0), 0); // system time is now 1
        assert_eq!(
            kernel.user_syscall_on(sleeper, 0, SyscallId::Sleep as u64, 3, 0, 0, 0, 0, 0),
            UserAction::Sleep
        );
        assert_eq!(
            kernel.tcb(sleeper).unwrap().state,
            ThreadState::BlockedSleep
        );
        assert_eq!(kernel.tcb(sleeper).unwrap().wake_at, Some(4));
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(
            kernel.tcb(sleeper).unwrap().state,
            ThreadState::BlockedSleep
        );
        assert_eq!(kernel.tick(0), 1);
        assert_eq!(kernel.tcb(sleeper).unwrap().state, ThreadState::Ready);
        assert!(kernel.tcb(sleeper).unwrap().queued);
        assert_eq!(kernel.pick_on(0), Some(sleeper));
        assert!(kernel.take_resume(sleeper).is_some());
        assert_eq!(kernel.tick(0), 0);
    }

    #[test]
    fn cnode_mint_and_install_switches_tcb_cspace() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("cnode-target", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CNodeMint as u64,
                1,
                2,
                2,
                Rights::READ.0 as u64,
                7,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        let minted = kernel.cnode(cnode).unwrap().slots[2].unwrap();
        assert_eq!(minted.ty, ObjType::Endpoint);
        assert_eq!(minted.badge, 7);
        assert_eq!(minted.rights, Rights::READ);

        assert_eq!(
            kernel.user_syscall(root, SyscallId::CNodeInstall as u64, 3, 1, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(target).unwrap().cnode, Some(cnode));
        assert_eq!(
            kernel.lookup_cap(target, 2, Some(ObjType::Endpoint), Rights::READ),
            Ok(minted)
        );
        assert!(kernel.check_invariants().is_ok());
        assert_eq!(kernel.destroy_object(cnode), Err(KError::IllegalOperation));

        assert_eq!(
            kernel.user_syscall(root, SyscallId::CNodeInstall as u64, 3, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(target).unwrap().cnode, None);
        assert_eq!(
            kernel.lookup_cap(target, 2, Some(ObjType::Endpoint), Rights::READ),
            Err(KError::InvalidCap)
        );
        kernel.destroy_object(cnode).unwrap();
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn cnode_caps_participate_in_delete_and_revoke() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("cnode-target", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CNodeMint as u64,
                1,
                1,
                2,
                Rights::READ.0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::CNodeInstall as u64, 3, 1, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.delete_cap(root, 2), Err(KError::IllegalOperation));
        assert!(kernel.revoke_cap(root, 2).unwrap() >= 1);
        assert!(kernel.cnode(cnode).unwrap().slots[1].is_none());
        assert_eq!(
            kernel.lookup_cap(target, 1, Some(ObjType::Endpoint), Rights::READ),
            Err(KError::InvalidCap)
        );
        assert_eq!(kernel.destroy_object(cnode), Err(KError::IllegalOperation));
        kernel.user_syscall(root, SyscallId::CNodeInstall as u64, 3, 0, 0, 0, 0, 0);
        kernel.destroy_object(cnode).unwrap();
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn cnode_mint_rejects_occupied_and_bad_destinations() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        kernel
            .install_cap(root, 1, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CNodeMint as u64,
                1,
                1,
                2,
                Rights::READ.0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CNodeMint as u64,
                1,
                1,
                2,
                Rights::READ.0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::SlotOccupied
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::CNodeMint as u64,
                1,
                2,
                99,
                Rights::READ.0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );
    }

    #[test]
    fn irq_get_delivers_timer_edge_and_ack_rearms_it() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let notification = kernel.create_notification().unwrap();
        kernel
            .install_cap(
                root,
                2,
                Cap::new(notification, ObjType::Notification, Rights::ALL),
            )
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::IrqGet as u64,
                IRQ_TIMER as u64,
                1,
                0,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 1,
                error: KError::Ok
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::IrqSetNotification as u64, 1, 2, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::IrqGet as u64,
                IRQ_TIMER as u64,
                3,
                0,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::IrqInUse
            }
        );
        let waiter = kernel.create_tcb("irq-waiter", 5).unwrap();
        assert_eq!(kernel.wait_as(waiter, notification), Ok(None));
        assert!(kernel.irq_raise(IRQ_TIMER));
        assert_eq!(kernel.tcb(waiter).unwrap().state, ThreadState::Ready);
        let message = kernel.take_resume(waiter).unwrap();
        assert_eq!(message.label, LABEL_NOTIFICATION);
        assert_eq!(message.words()[0], 1);
        assert!(!kernel.irq_raise(IRQ_TIMER));
        assert_eq!(
            kernel.user_syscall(root, SyscallId::IrqAck as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert!(kernel.irq_raise(IRQ_TIMER));
    }

    #[test]
    fn irq_get_rejects_non_root_and_bad_slots() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let other = kernel.create_tcb("other", 5).unwrap();
        assert_eq!(
            kernel.user_syscall(
                other,
                SyscallId::IrqGet as u64,
                IRQ_TIMER as u64,
                1,
                0,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights
            }
        );
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::IrqGet as u64,
                IRQ_TIMER as u64,
                0,
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
            kernel.user_syscall(root, SyscallId::IrqAck as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::IrqSetNotification as u64, 1, 2, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidCap
            }
        );
    }

    #[test]
    fn sched_context_configures_binds_and_charges_budget() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("budgeted", 5).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let context = kernel.retype(untyped, ObjType::SchedContext).unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(context, ObjType::SchedContext, Rights::ALL),
            )
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();

        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::SchedContextConfigure as u64,
                1,
                0,
                5,
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
            kernel.user_syscall(
                root,
                SyscallId::SchedContextConfigure as u64,
                1,
                2,
                5,
                0,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(
            kernel.user_syscall(root, SyscallId::SchedContextBind as u64, 2, 1, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert_eq!(kernel.tcb(target).unwrap().sched_context, Some(context));
        assert_eq!(kernel.sched_context(context).unwrap().bound, Some(target));

        kernel.current[0] = Some(target);
        kernel.tcb_mut(target).unwrap().state = ThreadState::Running;
        kernel.tcb_mut(target).unwrap().queued = false;
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.sched_context(context).unwrap().remaining, 1);
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(
            kernel.tcb(target).unwrap().state,
            ThreadState::BlockedBudget
        );
        assert_eq!(kernel.tcb(target).unwrap().wake_at, Some(5));
        assert!(!kernel.tcb(target).unwrap().queued);
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.tick(0), 1);
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Ready);
        assert_eq!(kernel.sched_context(context).unwrap().remaining, 2);
        assert_eq!(kernel.sched_context(context).unwrap().period_start, 5);
        assert!(kernel.take_resume(target).is_none());
        assert_eq!(kernel.pick_on(0), Some(target));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn sched_context_destroy_unbinds_and_wakes_budget_blocked_tcb() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("budgeted", 5).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let context = kernel.retype(untyped, ObjType::SchedContext).unwrap();
        kernel
            .install_cap(
                root,
                1,
                Cap::new(context, ObjType::SchedContext, Rights::ALL),
            )
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(target, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::SchedContextBind as u64, 2, 1, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::InvalidArg
            }
        );
        kernel.configure_sched_context(context, 1, 3).unwrap();
        kernel.bind_sched_context(target, context).unwrap();
        kernel.current[0] = Some(target);
        kernel.tcb_mut(target).unwrap().state = ThreadState::Running;
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(
            kernel.tcb(target).unwrap().state,
            ThreadState::BlockedBudget
        );
        kernel.destroy_object(context).unwrap();
        let target_table = kernel.tcb(target).unwrap();
        assert_eq!(target_table.sched_context, None);
        assert_eq!(target_table.state, ThreadState::Ready);
        assert!(target_table.wake_at.is_none());
        assert!(target_table.queued);
        kernel.current[0] = None;
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn per_hart_runqueue_home_assignment_steal_and_affinity_move() {
        let mut kernel = Kernel::new();
        let local0 = kernel.create_tcb("local0", 1).unwrap();
        let local1 = kernel.create_tcb("local1", 1).unwrap();
        let both = kernel.create_tcb("both", 1).unwrap();
        kernel.tcb_mut(local0).unwrap().affinity = 0b01;
        kernel.tcb_mut(local1).unwrap().affinity = 0b10;
        kernel.tcb_mut(both).unwrap().affinity = 0b11;
        kernel.enqueue(local0);
        kernel.enqueue(local1);
        kernel.enqueue(both);
        assert_eq!(kernel.runq_depth(0), 2);
        assert_eq!(kernel.runq_depth(1), 1);
        assert_eq!(kernel.pick_on(1), Some(local1));
        assert_eq!(kernel.scheduler_stats(1), (1, 0));
        assert_eq!(kernel.runq_depth(1), 0);
        assert!(kernel.check_invariants().is_ok());

        let mut steal = Kernel::new();
        let mobile = steal.create_tcb("mobile", 1).unwrap();
        steal.tcb_mut(mobile).unwrap().affinity = 0b11;
        steal.enqueue(mobile);
        assert_eq!(steal.runq_depth(0), 1);
        assert_eq!(steal.runq_depth(1), 0);
        assert_eq!(steal.pick_on(1), Some(mobile));
        assert_eq!(steal.scheduler_stats(1), (0, 1));
        assert_eq!(steal.runq_depth(0), 0);
        assert!(steal.check_invariants().is_ok());

        let mut moved = Kernel::new();
        let task = moved.create_tcb("moved", 1).unwrap();
        moved.tcb_mut(task).unwrap().affinity = 0b10;
        moved.enqueue(task);
        assert_eq!(moved.runq_depth(1), 1);
        moved.set_tcb_affinity(task, 0b01).unwrap();
        assert_eq!(moved.runq_depth(1), 0);
        assert_eq!(moved.runq_depth(0), 1);
        assert_eq!(moved.pick_on(0), Some(task));
        assert!(moved.check_invariants().is_ok());
    }
}
