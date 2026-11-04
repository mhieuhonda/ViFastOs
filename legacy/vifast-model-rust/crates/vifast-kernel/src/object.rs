//! Kernel object, capability, slot và bảng object.
//!
//! Nguyên tắc:
//! - Không object nào giữ tham chiếu Rust tới object khác, chỉ giữ [`ObjRef`]
//!   (chỉ số + generation). Object bị huỷ thì generation tăng, mọi `ObjRef` cũ
//!   tự động vô hiệu: không có use-after-free, không cần lifetime phức tạp.
//! - Object sinh ra từ Untyped và có địa chỉ vật lý thật (`Header::paddr`).
//!
//! Đơn giản hoá có chủ đích ở bản này: siêu dữ liệu (hàng đợi, bảng tra) dùng
//! `alloc` (Vec, VecDeque, BTreeMap). Bản chạy trên phần cứng nên thay bằng danh
//! sách liên kết intrusive và mảng cố định nằm trong chính vùng nhớ của object.
//! Xem docs/03-diem-chu-y.md.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use vifast_abi::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ObjRef {
    pub index: u32,
    pub gen: u32,
}

/// Địa chỉ một slot: CNode nào, ô thứ mấy.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SlotAddr {
    pub cnode: ObjRef,
    pub index: usize,
}

/// Cap của frame/page table đang được map ở đâu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mapping {
    Frame { vspace: ObjRef, vaddr: u64 },
    Table { vspace: ObjRef, base: u64, level: u8 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cap {
    pub obj_type: ObjType,
    pub obj: Option<ObjRef>,
    pub rights: Rights,
    pub badge: u64,
    pub irq: Option<u32>,
    /// Frame/PageTable: đang map ở đâu. Mỗi cap map được đúng một lần.
    pub mapped: Option<Mapping>,
    /// Frame: đang map vào IOSpace nào, ở iova nào.
    pub io_mapped: Option<(ObjRef, u64)>,
    /// CNode cap: guard dùng khi phân giải cptr.
    pub guard: u64,
    pub guard_bits: u8,
}

impl Cap {
    pub fn new(obj_type: ObjType, obj: ObjRef, rights: Rights) -> Cap {
        Cap {
            obj_type,
            obj: Some(obj),
            rights,
            badge: 0,
            irq: None,
            mapped: None,
            io_mapped: None,
            guard: 0,
            guard_bits: 0,
        }
    }

    /// Tạo cap con: quyền chỉ giảm, badge chỉ gán được một lần, không mang theo mapping.
    pub fn derive(&self, rights: Rights, badge: u64) -> KResult<Cap> {
        let mut new_badge = self.badge;
        if badge != 0 {
            if !matches!(self.obj_type, ObjType::Endpoint | ObjType::Notification) {
                return Err(KError::InvalidArg);
            }
            if self.badge != 0 {
                return Err(KError::NoRights);
            }
            new_badge = badge;
        }
        Ok(Cap {
            rights: self.rights & rights,
            badge: new_badge,
            mapped: None,
            io_mapped: None,
            ..*self
        })
    }
}

/// Một ô capability và vị trí của nó trong cây dẫn xuất (CDT).
#[derive(Clone, Debug, Default)]
pub struct Slot {
    pub cap: Option<Cap>,
    pub parent: Option<SlotAddr>,
    pub children: Vec<SlotAddr>,
}

// ---------------------------------------------------------------------------
// Các loại object
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
    /// Số cap trỏ tới object. Về 0 thì object bị huỷ.
    pub refs: u32,
    pub parent_untyped: Option<ObjRef>,
    /// Đang bị huỷ (hoặc là CNode zombie đang được dọn).
    pub dying: bool,
    /// Địa chỉ vật lý của object.
    pub paddr: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Untyped {
    pub size: u64,
    pub watermark: u64,
    pub children: BTreeSet<ObjRef>,
}

#[derive(Clone, Debug, Default)]
pub struct Endpoint {
    pub send_queue: VecDeque<ObjRef>,
    pub recv_queue: VecDeque<ObjRef>,
}

#[derive(Clone, Debug, Default)]
pub struct Notification {
    pub word: u64,
    pub waiters: VecDeque<ObjRef>,
    pub bound_tcb: Option<ObjRef>,
}

#[derive(Clone, Debug, Default)]
pub struct CNode {
    pub slots: Vec<Slot>,
    /// Tiến độ dọn khi CNode thành zombie.
    pub reap_cursor: usize,
}

impl CNode {
    pub fn with_bits(bits: u32) -> CNode {
        CNode { slots: (0..(1usize << bits)).map(|_| Slot::default()).collect(), reap_cursor: 0 }
    }

    /// Số bit chỉ số (số slot luôn là luỹ thừa của 2).
    pub fn radix(&self) -> u32 {
        self.slots.len().trailing_zeros()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// Frame MMIO của thiết bị (không thuộc RAM, không xoá, không IoMap được).
    pub device: bool,
}

/// Page table gốc Sv39 nằm ở `Header::paddr`.
/// `frames`/`tables` chỉ là bản đồ ngược để huỷ cho đúng.
#[derive(Clone, Debug, Default)]
pub struct VSpace {
    pub asid: u32,
    /// vpn -> slot của frame cap.
    pub frames: BTreeMap<u64, SlotAddr>,
    /// (base, level) -> slot của page table cap.
    pub tables: BTreeMap<(u64, u8), SlotAddr>,
}

#[derive(Clone, Copy, Debug)]
pub struct IoMapping {
    pub paddr: u64,
    pub rights: Rights,
    pub slot: SlotAddr,
}

/// Bảng dịch IOMMU của một thiết bị: trang iova -> trang vật lý.
#[derive(Clone, Debug, Default)]
pub struct IoSpace {
    pub device: u32,
    pub pages: BTreeMap<u64, IoMapping>,
}

#[derive(Clone, Debug, Default)]
pub struct SchedContext {
    pub budget: u64,
    pub period: u64,
    pub remaining: i64,
    pub configured: bool,
    pub bound_tcb: Option<ObjRef>,
    /// Đổi cấu hình thì tăng: các sự kiện nạp lại cũ bị bỏ qua.
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PendingSend {
    pub badge: u64,
    pub msg: Message,
    pub is_call: bool,
    pub can_grant: bool,
}

/// Thanh ghi user được lưu khi trap. Bản thật: `#[repr(C)]`, assembly lưu/khôi phục.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrapFrame {
    pub pc: u64,
    pub a: [u64; 8],
}

#[derive(Clone, Debug)]
pub struct Tcb {
    pub name: String,
    pub state: ThreadState,
    pub priority: u8,
    pub mcp: u8,
    /// Hart chạy thread này.
    pub affinity: usize,
    pub cspace: Option<ObjRef>,
    pub cspace_guard: u64,
    pub cspace_guard_bits: u8,
    pub vspace: Option<ObjRef>,
    pub fault_ep: Option<ObjRef>,
    pub fault_badge: u64,
    pub image: Option<u32>,
    pub args: [u64; NUM_ARGS],
    pub regs: TrapFrame,
    /// Tăng mỗi lần ngữ cảnh bị xoá (restart, huỷ). Bên thực thi user
    /// (CPU thật hoặc simulator) dùng nó để biết phải chạy lại từ đầu.
    pub context_epoch: u64,
    /// Kết quả syscall đang chờ giao cho thread khi nó chạy lại.
    pub resume: Option<SysResult>,
    pub fault_kind: Option<FaultKind>,
    // IPC
    pub pending: Option<PendingSend>,
    pub blocked_on: Option<ObjRef>,
    pub recv_slot: u64,
    /// (thread đang chờ reply, reply_epoch lúc đó).
    pub caller: Option<(ObjRef, u64)>,
    pub reply_epoch: u64,
    pub bound_ntfn: Option<ObjRef>,
    // Lập lịch
    pub timeslice: i32,
    pub sc: Option<ObjRef>,
    /// Hết budget, chờ chu kỳ sau.
    pub throttled: bool,
}

impl Tcb {
    pub fn new() -> Tcb {
        Tcb {
            name: String::new(),
            state: ThreadState::Inactive,
            priority: 0,
            mcp: 0,
            affinity: 0,
            cspace: None,
            cspace_guard: 0,
            cspace_guard_bits: 0,
            vspace: None,
            fault_ep: None,
            fault_badge: 0,
            image: None,
            args: [0; NUM_ARGS],
            regs: TrapFrame::default(),
            context_epoch: 0,
            resume: None,
            fault_kind: None,
            pending: None,
            blocked_on: None,
            recv_slot: 0,
            caller: None,
            reply_epoch: 0,
            bound_ntfn: None,
            timeslice: DEFAULT_TIMESLICE,
            sc: None,
            throttled: false,
        }
    }
}

impl Default for Tcb {
    fn default() -> Self {
        Tcb::new()
    }
}

#[derive(Clone, Debug)]
pub enum Kind {
    Untyped(Untyped),
    Tcb(Tcb),
    Endpoint(Endpoint),
    Notification(Notification),
    CNode(CNode),
    Frame(Frame),
    PageTable,
    VSpace(VSpace),
    IoSpace(IoSpace),
    IrqControl,
    IrqHandler { irq: u32 },
    SchedControl,
    SchedContext(SchedContext),
}

impl Kind {
    pub fn obj_type(&self) -> ObjType {
        match self {
            Kind::Untyped(_) => ObjType::Untyped,
            Kind::Tcb(_) => ObjType::Tcb,
            Kind::Endpoint(_) => ObjType::Endpoint,
            Kind::Notification(_) => ObjType::Notification,
            Kind::CNode(_) => ObjType::CNode,
            Kind::Frame(_) => ObjType::Frame,
            Kind::PageTable => ObjType::PageTable,
            Kind::VSpace(_) => ObjType::VSpace,
            Kind::IoSpace(_) => ObjType::IoSpace,
            Kind::IrqControl => ObjType::IrqControl,
            Kind::IrqHandler { .. } => ObjType::IrqHandler,
            Kind::SchedControl => ObjType::SchedControl,
            Kind::SchedContext(_) => ObjType::SchedContext,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Object {
    pub hdr: Header,
    pub kind: Kind,
}

impl Object {
    pub fn new(kind: Kind) -> Object {
        Object { hdr: Header::default(), kind }
    }
}

/// Tạo phần thân object mới khi retype.
pub fn new_kind(obj_type: ObjType, size_bits: u32) -> KResult<Kind> {
    Ok(match obj_type {
        ObjType::Untyped => Kind::Untyped(Untyped { size: 1u64 << size_bits, ..Untyped::default() }),
        ObjType::Tcb => Kind::Tcb(Tcb::new()),
        ObjType::Endpoint => Kind::Endpoint(Endpoint::default()),
        ObjType::Notification => Kind::Notification(Notification::default()),
        ObjType::CNode => Kind::CNode(CNode::with_bits(size_bits)),
        ObjType::Frame => Kind::Frame(Frame::default()),
        ObjType::VSpace => Kind::VSpace(VSpace::default()),
        ObjType::PageTable => Kind::PageTable,
        ObjType::SchedContext => Kind::SchedContext(SchedContext::default()),
        _ => return Err(KError::InvalidArg),
    })
}

// ---------------------------------------------------------------------------
// Bảng object (arena có generation)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Entry {
    gen: u32,
    obj: Option<Object>,
}

/// Arena có generation. Bản phần cứng: mảng cố định cấp lúc boot + free list.
#[derive(Clone, Debug, Default)]
pub struct ObjectTable {
    entries: Vec<Entry>,
    free: Vec<u32>,
}

macro_rules! typed_accessors {
    ($($get:ident, $get_mut:ident, $variant:ident, $ty:ty;)*) => {
        $(
            pub fn $get(&self, r: ObjRef) -> KResult<&$ty> {
                match &self.get(r)?.kind {
                    Kind::$variant(x) => Ok(x),
                    _ => Err(KError::WrongType),
                }
            }

            pub fn $get_mut(&mut self, r: ObjRef) -> KResult<&mut $ty> {
                match &mut self.get_mut(r)?.kind {
                    Kind::$variant(x) => Ok(x),
                    _ => Err(KError::WrongType),
                }
            }
        )*
    };
}

impl ObjectTable {
    pub fn new() -> ObjectTable {
        ObjectTable::default()
    }

    pub fn insert(&mut self, obj: Object) -> ObjRef {
        if let Some(i) = self.free.pop() {
            let e = &mut self.entries[i as usize];
            e.obj = Some(obj);
            ObjRef { index: i, gen: e.gen }
        } else {
            let i = self.entries.len() as u32;
            self.entries.push(Entry { gen: 0, obj: Some(obj) });
            ObjRef { index: i, gen: 0 }
        }
    }

    pub fn try_get(&self, r: ObjRef) -> Option<&Object> {
        let e = self.entries.get(r.index as usize)?;
        if e.gen != r.gen {
            return None;
        }
        e.obj.as_ref()
    }

    pub fn try_get_mut(&mut self, r: ObjRef) -> Option<&mut Object> {
        let e = self.entries.get_mut(r.index as usize)?;
        if e.gen != r.gen {
            return None;
        }
        e.obj.as_mut()
    }

    pub fn get(&self, r: ObjRef) -> KResult<&Object> {
        self.try_get(r).ok_or(KError::ObjectDestroyed)
    }

    pub fn get_mut(&mut self, r: ObjRef) -> KResult<&mut Object> {
        self.try_get_mut(r).ok_or(KError::ObjectDestroyed)
    }

    pub fn alive(&self, r: Option<ObjRef>) -> bool {
        r.map_or(false, |r| self.try_get(r).is_some())
    }

    pub fn remove(&mut self, r: ObjRef) {
        if let Some(e) = self.entries.get_mut(r.index as usize) {
            if e.gen == r.gen && e.obj.is_some() {
                e.gen = e.gen.wrapping_add(1);
                e.obj = None;
                self.free.push(r.index);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (ObjRef, &Object)> + '_ {
        self.entries.iter().enumerate().filter_map(|(i, e)| {
            e.obj.as_ref().map(|o| (ObjRef { index: i as u32, gen: e.gen }, o))
        })
    }

    typed_accessors! {
        untyped, untyped_mut, Untyped, Untyped;
        tcb, tcb_mut, Tcb, Tcb;
        endpoint, endpoint_mut, Endpoint, Endpoint;
        notification, notification_mut, Notification, Notification;
        cnode, cnode_mut, CNode, CNode;
        frame, frame_mut, Frame, Frame;
        vspace, vspace_mut, VSpace, VSpace;
        iospace, iospace_mut, IoSpace, IoSpace;
        sched_context, sched_context_mut, SchedContext, SchedContext;
    }
}
