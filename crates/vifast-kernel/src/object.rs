//! Fixed-capacity object model for the portable kernel.
//!
//! No heap is used here.  The table is intentionally small; it is a reference
//! implementation for the architecture ports and for host tests, not the final
//! production object allocator.

use vifast_abi::*;

pub const MAX_OBJECTS: usize = 128;
pub const MAX_TCBS: usize = 16;
/// Root task cspace capacity.  Root owns the large slot range used while it
/// builds the initial task graph.
pub const MAX_CAPS_ROOT: usize = 96;
/// Ordinary TCB cspace capacity.  Keeping this compact is important: `Tcb`
/// is the largest `Object` variant, so every unused slot costs
/// `MAX_OBJECTS * sizeof(Option<Cap>)` bytes of the static kernel image.
pub const MAX_CAPS_TASK: usize = 64;
/// Historical name used by the BootInfo slot range (root cspace).
pub const MAX_CAPS: usize = MAX_CAPS_ROOT;
pub const MAX_QUEUE: usize = 8;
pub const MAX_MAPPINGS: usize = 64;
pub const MAX_IOMMU_MAPPINGS: usize = 32;
pub const MAX_FDS: usize = 8;
pub const MAX_PRIORITY: u8 = 15;
const _: () = assert!(MAX_OBJECTS <= 256);
const _: () = assert!(MAX_CAPS <= u8::MAX as usize);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ObjRef {
    pub index: u8,
    pub gen: u32,
}

impl ObjRef {
    pub const fn invalid() -> Self {
        ObjRef {
            index: u8::MAX,
            gen: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.index != u8::MAX
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SlotAddr {
    pub tcb: ObjRef,
    pub slot: u8,
}

/// Exact capability identity.  The slot part is only a lookup location; the
/// monotonically increasing id prevents a reused slot from inheriting the
/// old capability's children or creator relationships.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CapRef {
    pub addr: SlotAddr,
    pub id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap {
    pub obj: ObjRef,
    pub ty: ObjType,
    pub rights: Rights,
    pub badge: u64,
    pub id: u64,
    /// True only for a cap originally installed without a parent (runtime
    /// root cap). Reparented child caps with no surviving parent keep false.
    pub root: bool,
    pub parent: Option<CapRef>,
}

impl Cap {
    pub const fn new(obj: ObjRef, ty: ObjType, rights: Rights) -> Self {
        Cap {
            obj,
            ty,
            rights,
            badge: 0,
            id: 0,
            root: false,
            parent: None,
        }
    }

    pub fn derive(&self, rights: Rights, badge: u64) -> KResult<Cap> {
        if badge != 0 {
            if !matches!(self.ty, ObjType::Endpoint | ObjType::Notification) {
                return Err(KError::InvalidArg);
            }
            if self.badge != 0 {
                return Err(KError::NoRights);
            }
        }
        Ok(Cap {
            obj: self.obj,
            ty: self.ty,
            rights: Rights(self.rights.0 & rights.0),
            badge: if badge != 0 { badge } else { self.badge },
            id: 0,
            root: false,
            parent: None,
        })
    }
}

/// Small FIFO used by endpoints.  It never allocates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Queue {
    entries: [Option<ObjRef>; MAX_QUEUE],
    head: u8,
    len: u8,
}

impl Queue {
    pub fn push(&mut self, value: ObjRef) -> bool {
        if self.len as usize >= MAX_QUEUE {
            return false;
        }
        let index = (self.head as usize + self.len as usize) % MAX_QUEUE;
        self.entries[index] = Some(value);
        self.len += 1;
        true
    }

    pub fn front(&self) -> Option<ObjRef> {
        if self.len == 0 {
            None
        } else {
            self.entries[self.head as usize]
        }
    }

    pub fn pop(&mut self) -> Option<ObjRef> {
        if self.len == 0 {
            return None;
        }
        let value = self.entries[self.head as usize].take();
        self.head = (self.head + 1) % MAX_QUEUE as u8;
        self.len -= 1;
        value
    }

    pub fn remove(&mut self, value: ObjRef) {
        let mut out = Queue::default();
        while let Some(item) = self.pop() {
            if item != value {
                let _ = out.push(item);
            }
        }
        *self = out;
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, value: ObjRef) -> bool {
        for i in 0..self.len as usize {
            let index = (self.head as usize + i) % MAX_QUEUE;
            if self.entries[index] == Some(value) {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PendingSend {
    pub badge: u64,
    pub msg: Message,
    pub is_call: bool,
    pub can_grant: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Endpoint {
    pub send: Queue,
    pub recv: Queue,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Notification {
    pub word: u64,
    pub waiter: Option<ObjRef>,
}

/// A physical memory frame that can be mapped into a virtual address space.
/// The portable core only records the physical address and size; the
/// architecture port owns the actual page-table entry.
///
/// `mapped_w`/`mapped_x` are the number of live mappings of this exact Frame
/// object carrying WRITE/EXEC.  W^X is enforced over these counters, not only
/// over one mapping's rights, so the same Frame cannot be RW at one VA and RX
/// at another.  They are derived state: `check_invariants` compares them with
/// the VSpace reverse maps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub pa: usize,
    pub size: usize,
    pub mapped_w: u16,
    pub mapped_x: u16,
}

/// One user mapping tracked by a VSpace object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub va: u64,
    pub pa: u64,
    pub len: u64,
    pub rights: Rights,
    /// Exact Frame object that owns this leaf.  Storing it makes counter
    /// bookkeeping exact even if two Frame objects ever name the same PA.
    pub frame: ObjRef,
}

impl Default for Mapping {
    fn default() -> Self {
        Mapping {
            va: 0,
            pa: 0,
            len: 0,
            rights: Rights::NONE,
            frame: ObjRef::invalid(),
        }
    }
}

/// One IOMMU/DMA mapping.  The portable model owns the reverse map; a
/// hardware IOMMU driver can consume the same entries later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IommuMapping {
    pub iova: u64,
    pub pa: u64,
    pub len: u64,
    pub rights: Rights,
    pub frame: ObjRef,
}

impl Default for IommuMapping {
    fn default() -> Self {
        Self {
            iova: 0,
            pa: 0,
            len: 0,
            rights: Rights::NONE,
            frame: ObjRef::invalid(),
        }
    }
}

/// A software IOMMU domain.  It is deliberately independent of a VSpace:
/// device-visible IOVAs are not user VAs and are validated separately.
#[derive(Clone, Copy, Debug)]
pub struct IommuDomain {
    pub mappings: [Option<IommuMapping>; MAX_IOMMU_MAPPINGS],
}

impl Default for IommuDomain {
    fn default() -> Self {
        Self {
            mappings: [None; MAX_IOMMU_MAPPINGS],
        }
    }
}

impl IommuDomain {
    pub fn find(&self, iova: u64) -> Option<IommuMapping> {
        self.mappings
            .iter()
            .flatten()
            .find(|mapping| iova >= mapping.iova && iova < mapping.iova.saturating_add(mapping.len))
            .copied()
    }

    pub fn contains(&self, iova: u64, len: u64) -> bool {
        self.mappings.iter().flatten().any(|mapping| {
            iova < mapping.iova.saturating_add(mapping.len)
                && mapping.iova < iova.saturating_add(len)
        })
    }

    pub fn insert(&mut self, mapping: IommuMapping) -> KResult<()> {
        if self.contains(mapping.iova, mapping.len) {
            return Err(KError::AlreadyMapped);
        }
        for slot in self.mappings.iter_mut() {
            if slot.is_none() {
                *slot = Some(mapping);
                return Ok(());
            }
        }
        Err(KError::OutOfMemory)
    }

    pub fn remove(&mut self, iova: u64) -> KResult<IommuMapping> {
        for slot in self.mappings.iter_mut() {
            if slot.is_some_and(|mapping| {
                mapping.iova <= iova && iova < mapping.iova.saturating_add(mapping.len)
            }) {
                return slot.take().ok_or(KError::NotMapped);
            }
        }
        Err(KError::NotMapped)
    }

    pub fn remove_frame(&mut self, frame: ObjRef) {
        for slot in self.mappings.iter_mut() {
            if slot.is_some_and(|mapping| mapping.frame == frame) {
                *slot = None;
            }
        }
    }
}

/// A user address space.  `root` is interpreted by the architecture port
/// (Sv39 root, TTBR0 root, CR3).  `mappings` is the reverse map used for
/// validation, W^X enforcement and unmap bookkeeping.
#[derive(Clone, Copy, Debug)]
pub struct VSpace {
    pub root: u64,
    /// Private PDPT/L1 page used by x86/AArch64 retyped roots.
    pub aux_pa: Option<u64>,
    /// Hardware address-space tag assigned by the portable kernel.  Values
    /// are nonzero and fit in x86's 12-bit PCID field; RISC-V/AArch64 use the
    /// same value in their wider ASID fields.
    pub asid: u16,
    pub page_size: u64,
    pub base: u64,
    pub next_va: u64,
    pub end: u64,
    /// Harts whose TLB may still hold entries tagged with this VSpace ASID.
    pub harts_loaded: u32,
    pub mappings: [Option<Mapping>; MAX_MAPPINGS],
}

impl VSpace {
    pub fn new(root: u64, base: u64, size: u64) -> Self {
        Self::with_page_size(root, base, size, DEFAULT_PAGE_SIZE)
    }

    pub fn with_page_size(root: u64, base: u64, size: u64, page_size: u64) -> Self {
        let end = base.saturating_add(size);
        VSpace {
            root,
            aux_pa: None,
            asid: 0,
            page_size,
            base,
            next_va: base,
            end,
            harts_loaded: 0,
            mappings: [None; MAX_MAPPINGS],
        }
    }

    pub fn find(&self, va: u64) -> Option<Mapping> {
        for mapping in self.mappings.iter().flatten() {
            if va >= mapping.va && va < mapping.va.saturating_add(mapping.len) {
                return Some(*mapping);
            }
        }
        None
    }

    pub fn mapping_index(&self, va: u64) -> Option<usize> {
        for (i, mapping) in self.mappings.iter().enumerate() {
            if let Some(mapping) = mapping {
                if mapping.va == va {
                    return Some(i);
                }
            }
        }
        None
    }

    pub(crate) fn insert(&mut self, mapping: Mapping) -> KResult<()> {
        for slot in self.mappings.iter_mut() {
            if slot.is_none() {
                *slot = Some(mapping);
                let next = mapping.va.saturating_add(mapping.len);
                if next > self.next_va {
                    self.next_va = next;
                }
                return Ok(());
            }
        }
        Err(KError::OutOfMemory)
    }

    pub(crate) fn contains(&self, va: u64, len: u64) -> bool {
        self.mappings.iter().flatten().any(|mapping| {
            va < mapping.va.saturating_add(mapping.len) && mapping.va < va.saturating_add(len)
        })
    }

    /// First-fit page-range allocation inside this VSpace.  Walking the
    /// (small) reverse map makes unmapped holes reusable without a separate
    /// heap or interval tree.
    pub(crate) fn find_free_va(&self, len: u64) -> Option<u64> {
        let page = self.page_size;
        if page == 0 || len == 0 || len % page != 0 {
            return None;
        }
        let mut candidate = self.base;
        while candidate < self.end {
            let end = candidate.checked_add(len)?;
            if end > self.end {
                return None;
            }
            let mut moved = false;
            for mapping in self.mappings.iter().flatten() {
                let mapping_end = mapping.va.saturating_add(mapping.len);
                if candidate < mapping_end && mapping.va < end {
                    let aligned = mapping_end
                        .checked_add(page - 1)?
                        .checked_div(page)?
                        .checked_mul(page)?;
                    if aligned <= candidate {
                        return None;
                    }
                    candidate = aligned;
                    moved = true;
                }
            }
            if !moved {
                return Some(candidate);
            }
        }
        None
    }
}

/// Where a PageTable object is attached.  A page-table page may be installed
/// exactly once; the location records the parent VSpace and the VA base of the
/// range covered by this level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageTableMapping {
    pub vspace: ObjRef,
    pub va_base: u64,
    pub level: u8,
}

/// Minimal page-table object.  Ports that use multi-level page tables attach
/// one object to a VSpace with `PageTableMap`; level is assigned by the kernel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageTable {
    pub root: u64,
    pub mapped: Option<PageTableMapping>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcBuffer {
    pub frame: ObjRef,
    pub va: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fd {
    pub file: u8,
    pub offset: usize,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Tcb {
    pub name: &'static str,
    pub state: ThreadState,
    pub priority: u8,
    pub entry: u64,
    pub stack: u64,
    pub vspace: Option<ObjRef>,
    pub cspace: [Option<Cap>; MAX_CAPS_TASK],
    /// Client waiting for this TCB to reply: (client, reply epoch).
    pub caller: Option<(ObjRef, u8)>,
    pub reply_epoch: u8,
    pub recv_slot: u8,
    pub bound_ntfn: Option<ObjRef>,
    pub fault_ep: Option<CapRef>,
    pub ipc_buffer: Option<IpcBuffer>,
    pub pending: Option<PendingSend>,
    pub resume: Option<Message>,
    pub resume_error: Option<KError>,
    pub fds: [Option<Fd>; MAX_FDS],
    pub queued: bool,
    pub queue_seq: u64,
    /// Bitmask of harts allowed to run this TCB.  Default: hart 0 only.
    pub affinity: u8,
}

impl Tcb {
    pub fn new(name: &'static str, priority: u8) -> Self {
        Tcb {
            name,
            state: ThreadState::Inactive,
            priority: priority.min(MAX_PRIORITY),
            entry: 0,
            stack: 0,
            vspace: None,
            cspace: [None; MAX_CAPS_TASK],
            caller: None,
            reply_epoch: 0,
            recv_slot: 0,
            bound_ntfn: None,
            fault_ep: None,
            ipc_buffer: None,
            pending: None,
            resume: None,
            resume_error: None,
            fds: [None; MAX_FDS],
            queued: false,
            queue_seq: 0,
            affinity: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Untyped {
    pub base: usize,
    pub size: usize,
    pub bump: usize,
    pub page_size: usize,
    pub max_objects: usize,
    pub objects: usize,
    /// Sum of `max_objects` of live direct child Untypeds.  This quota is held
    /// so siblings cannot consume the child's future object budget.
    pub reserved: usize,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Free,
    Endpoint(Endpoint),
    Notification(Notification),
    Tcb(Tcb),
    Untyped(Untyped),
    Frame(Frame),
    VSpace(VSpace),
    PageTable(PageTable),
    Iommu(IommuDomain),
    BlockDevice(BlockDevice),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockDevice;

impl Kind {
    pub fn obj_type(&self) -> Option<ObjType> {
        match self {
            Kind::Free => None,
            Kind::Endpoint(_) => Some(ObjType::Endpoint),
            Kind::Notification(_) => Some(ObjType::Notification),
            Kind::Tcb(_) => Some(ObjType::Tcb),
            Kind::Untyped(_) => Some(ObjType::Untyped),
            Kind::Frame(_) => Some(ObjType::Frame),
            Kind::VSpace(_) => Some(ObjType::VSpace),
            Kind::PageTable(_) => Some(ObjType::PageTable),
            Kind::Iommu(_) => Some(ObjType::Iommu),
            Kind::BlockDevice(_) => Some(ObjType::BlockDevice),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Object {
    pub gen: u32,
    pub kind: Kind,
    /// Untyped that owns this object, if it was created by retype.
    pub owner: Option<ObjRef>,
    /// Exact capability through which retype created this object.  Revoking a
    /// different copy of the same Untyped must not destroy it.
    pub creator: Option<CapRef>,
}

impl Object {
    pub const fn free() -> Object {
        Object {
            gen: 0,
            kind: Kind::Free,
            owner: None,
            creator: None,
        }
    }

    pub fn is_free(&self) -> bool {
        matches!(self.kind, Kind::Free)
    }
}
