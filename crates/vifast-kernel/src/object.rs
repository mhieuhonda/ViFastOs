//! Fixed-capacity object model for the portable kernel.
//!
//! No heap is used here.  The table is intentionally small; it is a reference
//! implementation for the architecture ports and for host tests, not the final
//! production object allocator.

use vifast_abi::*;

pub const MAX_OBJECTS: usize = 128;
pub const MAX_TCBS: usize = 8;
pub const MAX_CAPS: usize = 64;
pub const MAX_QUEUE: usize = 8;
pub const MAX_MAPPINGS: usize = 32;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap {
    pub obj: ObjRef,
    pub ty: ObjType,
    pub rights: Rights,
    pub badge: u64,
    pub parent: Option<SlotAddr>,
}

impl Cap {
    pub const fn new(obj: ObjRef, ty: ObjType, rights: Rights) -> Self {
        Cap {
            obj,
            ty,
            rights,
            badge: 0,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub pa: usize,
    pub size: usize,
}

/// One user mapping tracked by a VSpace object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub va: u64,
    pub pa: u64,
    pub len: u64,
    pub rights: Rights,
}

impl Default for Mapping {
    fn default() -> Self {
        Mapping {
            va: 0,
            pa: 0,
            len: 0,
            rights: Rights::NONE,
        }
    }
}

/// A user address space.  `root` is interpreted by the architecture port
/// (Sv39 root, TTBR0 root, CR3).  `mappings` is the portable shadow table used
/// for validation, W^X enforcement and unmap bookkeeping.
#[derive(Clone, Copy, Debug)]
pub struct VSpace {
    pub root: u64,
    pub page_size: u64,
    pub base: u64,
    pub next_va: u64,
    pub end: u64,
    /// Read/execute user code window, used for pointer validation.
    pub code_base: u64,
    pub code_len: u64,
    /// Read/write user stack window, used for pointer validation.
    pub stack_base: u64,
    pub stack_len: u64,
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
            page_size,
            base,
            next_va: base,
            end,
            code_base: 0,
            code_len: 0,
            stack_base: 0,
            stack_len: 0,
            mappings: [None; MAX_MAPPINGS],
        }
    }

    pub fn with_user_layout(
        root: u64,
        base: u64,
        size: u64,
        page_size: u64,
        code_base: u64,
        code_len: u64,
        stack_base: u64,
        stack_len: u64,
    ) -> Self {
        let mut space = Self::with_page_size(root, base, size, page_size);
        space.code_base = code_base;
        space.code_len = code_len;
        space.stack_base = stack_base;
        space.stack_len = stack_len;
        space
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

    /// Check a user range against code, stack and explicit mappings.
    pub fn validate_range(&self, va: u64, len: usize, need_write: bool) -> bool {
        if len == 0 {
            return false;
        }
        let Some(end) = va.checked_add(len as u64) else {
            return false;
        };
        let in_region = |base: u64, region_len: u64| -> bool {
            region_len != 0 && va >= base && end <= base.saturating_add(region_len)
        };
        if !need_write && in_region(self.code_base, self.code_len) {
            return true;
        }
        if in_region(self.stack_base, self.stack_len) {
            return true;
        }
        // Every touched page must have a mapping with the required rights.
        let page = self.page_size;
        if page == 0 {
            return false;
        }
        let mut cursor = va & !(page - 1);
        while cursor < end {
            let Some(mapping) = self.find(cursor) else {
                return false;
            };
            if mapping.va != cursor || mapping.len != page {
                return false;
            }
            if need_write && !mapping.rights.contains(Rights::WRITE) {
                return false;
            }
            if !mapping.rights.contains(Rights::READ) {
                return false;
            }
            cursor = cursor.saturating_add(page);
            if cursor == 0 {
                return false;
            }
        }
        true
    }

    pub(crate) fn contains(&self, va: u64, len: u64) -> bool {
        self.mappings.iter().flatten().any(|mapping| {
            va < mapping.va.saturating_add(mapping.len) && mapping.va < va.saturating_add(len)
        })
    }
}

/// Minimal page-table object.  Ports that use multi-level page tables can
/// attach one object to a VSpace; the current bring-up ports use static roots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageTable {
    pub root: u64,
    pub level: u8,
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
    pub cspace: [Option<Cap>; MAX_CAPS],
    /// Client waiting for this TCB to reply: (client, reply epoch).
    pub caller: Option<(ObjRef, u8)>,
    pub reply_epoch: u8,
    pub recv_slot: u8,
    pub bound_ntfn: Option<ObjRef>,
    pub pending: Option<PendingSend>,
    pub resume: Option<Message>,
    pub resume_error: Option<KError>,
    pub fds: [Option<Fd>; MAX_FDS],
    pub queued: bool,
    pub queue_seq: u64,
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
            cspace: [None; MAX_CAPS],
            caller: None,
            reply_epoch: 0,
            recv_slot: 0,
            bound_ntfn: None,
            pending: None,
            resume: None,
            resume_error: None,
            fds: [None; MAX_FDS],
            queued: false,
            queue_seq: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Untyped {
    pub base: usize,
    pub size: usize,
    pub bump: usize,
    pub page_size: usize,
    pub max_objects: usize,
    pub objects: usize,
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
}

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
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Object {
    pub gen: u32,
    pub kind: Kind,
    /// Untyped that owns this object, if it was created by retype.
    pub owner: Option<ObjRef>,
}

impl Object {
    pub const fn free() -> Object {
        Object {
            gen: 0,
            kind: Kind::Free,
            owner: None,
        }
    }

    pub fn is_free(&self) -> bool {
        matches!(self.kind, Kind::Free)
    }
}
