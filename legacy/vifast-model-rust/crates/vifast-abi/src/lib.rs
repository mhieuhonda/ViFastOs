//! ABI dùng chung giữa kernel ViFastOs và user space.
//!
//! Crate này `no_std`, không có `unsafe`. Nó định nghĩa mọi thứ mà hai bên của
//! ranh giới syscall phải thống nhất: hằng số, mã lỗi, quyền, loại object,
//! message IPC, kết quả syscall và danh sách syscall.
//!
//! Tương ứng với `ktypes.py` + `abi.py` trong mô hình Python.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use core::ops::{BitAnd, BitOr, BitOrAssign};

// ---------------------------------------------------------------------------
// Hằng số
// ---------------------------------------------------------------------------

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;
pub const WORD_BITS: u32 = 64;
/// Số word tối đa trong một message (truyền qua thanh ghi).
pub const MSG_MAX_WORDS: usize = 8;
pub const NUM_PRIORITIES: usize = 256;
/// Số tick trước khi timer ngắt để xoay vòng.
pub const DEFAULT_TIMESLICE: i32 = 5;
/// Số cap tối đa xoá trong một lần chạy revoke (preemption point).
pub const REVOKE_BATCH: usize = 8;
/// Số slot của CNode zombie được dọn mỗi tick.
pub const REAP_BUDGET: usize = 16;
pub const MAX_ASID: u32 = 1 << 16;
/// Số thanh ghi tham số khi khởi động một thread (a0..a3).
pub const NUM_ARGS: usize = 4;

/// Label của message kernel gửi tới fault handler.
pub const LABEL_FAULT: u64 = 0xFFFF_0001;
/// Label của message sinh ra từ bound notification.
pub const LABEL_NOTIFICATION: u64 = 0xFFFF_0002;

// ---------------------------------------------------------------------------
// Mã lỗi
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KError {
    #[default]
    Ok = 0,
    /// cptr không phân giải được hoặc slot rỗng.
    InvalidCap = 1,
    WrongType = 2,
    NoRights = 3,
    SlotOccupied = 4,
    /// Untyped hoặc ASID không đủ. Chỉ ảnh hưởng người gọi.
    OutOfMemory = 5,
    InvalidArg = 6,
    Alignment = 7,
    AlreadyMapped = 8,
    NotMapped = 9,
    NoCaller = 10,
    ObjectDestroyed = 11,
    IrqInUse = 12,
    /// Vượt MCP của người gọi.
    PrioTooHigh = 13,
    /// Đối tác bị restart, suspend hoặc huỷ.
    Cancelled = 14,
    MissingPageTable = 15,
    /// Ví dụ map trang vừa ghi vừa thực thi (vi phạm W^X).
    IllegalOperation = 16,
}

impl KError {
    pub const fn name(self) -> &'static str {
        match self {
            KError::Ok => "OK",
            KError::InvalidCap => "INVALID_CAP",
            KError::WrongType => "WRONG_TYPE",
            KError::NoRights => "NO_RIGHTS",
            KError::SlotOccupied => "SLOT_OCCUPIED",
            KError::OutOfMemory => "OUT_OF_MEMORY",
            KError::InvalidArg => "INVALID_ARG",
            KError::Alignment => "ALIGNMENT",
            KError::AlreadyMapped => "ALREADY_MAPPED",
            KError::NotMapped => "NOT_MAPPED",
            KError::NoCaller => "NO_CALLER",
            KError::ObjectDestroyed => "OBJECT_DESTROYED",
            KError::IrqInUse => "IRQ_IN_USE",
            KError::PrioTooHigh => "PRIO_TOO_HIGH",
            KError::Cancelled => "CANCELLED",
            KError::MissingPageTable => "MISSING_PAGE_TABLE",
            KError::IllegalOperation => "ILLEGAL_OPERATION",
        }
    }
}

/// Kết quả nội bộ của kernel. `Err` không bao giờ chứa `KError::Ok`.
pub type KResult<T> = Result<T, KError>;

// ---------------------------------------------------------------------------
// Quyền
// ---------------------------------------------------------------------------

/// Tập quyền của một capability (bitflags viết tay để không cần dependency).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rights(pub u8);

impl Rights {
    pub const NONE: Rights = Rights(0);
    /// endpoint: recv, notification: wait, frame: đọc.
    pub const READ: Rights = Rights(1);
    /// endpoint: send/call, notification: signal, frame: ghi.
    pub const WRITE: Rights = Rights(2);
    pub const EXEC: Rights = Rights(4);
    /// Được chuyển capability qua IPC.
    pub const GRANT: Rights = Rights(8);
    pub const ALL: Rights = Rights(15);
    pub const RW: Rights = Rights(3);

    pub const fn contains(self, other: Rights) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Rights) -> bool {
        self.0 & other.0 != 0
    }

    /// Đọc chuỗi kiểu "RWXG".
    pub fn parse(text: &str) -> Option<Rights> {
        let mut r = Rights::NONE;
        for ch in text.chars() {
            r |= match ch.to_ascii_uppercase() {
                'R' => Rights::READ,
                'W' => Rights::WRITE,
                'X' => Rights::EXEC,
                'G' => Rights::GRANT,
                _ => return None,
            };
        }
        Some(r)
    }
}

impl BitAnd for Rights {
    type Output = Rights;
    fn bitand(self, rhs: Rights) -> Rights {
        Rights(self.0 & rhs.0)
    }
}

impl BitOr for Rights {
    type Output = Rights;
    fn bitor(self, rhs: Rights) -> Rights {
        Rights(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Rights) {
        self.0 |= rhs.0;
    }
}

// ---------------------------------------------------------------------------
// Loại object
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ObjType {
    Untyped = 0,
    Tcb = 1,
    Endpoint = 2,
    Notification = 3,
    CNode = 4,
    Frame = 5,
    /// Page table gốc Sv39 + ASID.
    VSpace = 6,
    IrqControl = 7,
    IrqHandler = 8,
    PageTable = 9,
    /// Bảng dịch IOMMU của một thiết bị (cấp lúc boot).
    IoSpace = 10,
    /// Ngân sách CPU budget/period (kiểu seL4 MCS).
    SchedContext = 11,
    /// Quyền cấu hình SchedContext.
    SchedControl = 12,
}

impl ObjType {
    /// Loại object có thể tạo bằng Retype từ Untyped.
    pub const fn is_retypeable(self) -> bool {
        matches!(
            self,
            ObjType::Untyped
                | ObjType::Tcb
                | ObjType::Endpoint
                | ObjType::Notification
                | ObjType::CNode
                | ObjType::Frame
                | ObjType::VSpace
                | ObjType::PageTable
                | ObjType::SchedContext
        )
    }
}

pub const CNODE_SLOT_SIZE: u64 = 32;

/// Kích thước object khi retype từ untyped (byte).
pub fn object_size(obj_type: ObjType, size_bits: u32) -> KResult<u64> {
    if !obj_type.is_retypeable() {
        return Err(KError::InvalidArg);
    }
    Ok(match obj_type {
        ObjType::Tcb => 1024,
        ObjType::Endpoint => 16,
        ObjType::Notification => 32,
        ObjType::Frame | ObjType::VSpace | ObjType::PageTable => PAGE_SIZE,
        ObjType::SchedContext => 256,
        ObjType::CNode => {
            if !(1..=12).contains(&size_bits) {
                return Err(KError::InvalidArg);
            }
            (1u64 << size_bits) * CNODE_SLOT_SIZE
        }
        ObjType::Untyped => {
            if !(4..=40).contains(&size_bits) {
                return Err(KError::InvalidArg);
            }
            1u64 << size_bits
        }
        _ => return Err(KError::InvalidArg),
    })
}

// ---------------------------------------------------------------------------
// Trạng thái thread và loại fault
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThreadState {
    #[default]
    Inactive,
    Ready,
    Running,
    BlockedSend,
    BlockedRecv,
    BlockedReply,
    BlockedNotification,
    /// Lỗi mà không có fault handler, chờ restart.
    Faulted,
}

impl ThreadState {
    pub const fn is_blocked(self) -> bool {
        matches!(
            self,
            ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
        )
    }
}

#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultKind {
    /// Truy cập bộ nhớ không hợp lệ (sửa được rồi chạy tiếp).
    VmFault = 1,
    /// Panic hoặc lệnh không hợp lệ (phải restart).
    UserException = 2,
    /// Chương trình kết thúc.
    Exited = 3,
    BadSyscall = 4,
}

impl FaultKind {
    pub const fn name(self) -> &'static str {
        match self {
            FaultKind::VmFault => "VM_FAULT",
            FaultKind::UserException => "USER_EXCEPTION",
            FaultKind::Exited => "EXITED",
            FaultKind::BadSyscall => "BAD_SYSCALL",
        }
    }

    pub const fn from_u64(v: u64) -> Option<FaultKind> {
        match v {
            1 => Some(FaultKind::VmFault),
            2 => Some(FaultKind::UserException),
            3 => Some(FaultKind::Exited),
            4 => Some(FaultKind::BadSyscall),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Message và kết quả syscall
// ---------------------------------------------------------------------------

/// Message IPC: label + tối đa 8 word + tối đa 1 capability.
///
/// Phía gửi: `cap` là cptr cần chuyển. Phía nhận: `cap` là slot đã nhận
/// (hoặc `None` nếu không chuyển được).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Message {
    pub label: u64,
    pub len: u8,
    pub data: [u64; MSG_MAX_WORDS],
    pub cap: Option<u64>,
}

impl Message {
    /// Tạo message. Panic nếu quá 8 word (lỗi lập trình phía user).
    pub fn new(label: u64, words: &[u64]) -> Message {
        assert!(words.len() <= MSG_MAX_WORDS, "message quá lớn");
        let mut data = [0u64; MSG_MAX_WORDS];
        data[..words.len()].copy_from_slice(words);
        Message { label, len: words.len() as u8, data, cap: None }
    }

    pub fn empty(label: u64) -> Message {
        Message::new(label, &[])
    }

    pub fn with_cap(mut self, cptr: u64) -> Message {
        self.cap = Some(cptr);
        self
    }

    pub fn words(&self) -> &[u64] {
        &self.data[..self.len as usize]
    }
}

/// Kết quả syscall. Quy ước thanh ghi: a0 = mã lỗi, a1 = value hoặc badge.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SysResult {
    pub err: KError,
    pub value: u64,
    pub badge: u64,
    pub msg: Option<Message>,
}

impl SysResult {
    pub const fn ok() -> SysResult {
        SysResult { err: KError::Ok, value: 0, badge: 0, msg: None }
    }

    pub const fn error(err: KError) -> SysResult {
        SysResult { err, value: 0, badge: 0, msg: None }
    }

    pub const fn value(value: u64) -> SysResult {
        SysResult { err: KError::Ok, value, badge: 0, msg: None }
    }

    pub const fn is_ok(&self) -> bool {
        matches!(self.err, KError::Ok)
    }
}

// ---------------------------------------------------------------------------
// Syscall
// ---------------------------------------------------------------------------

/// Toàn bộ syscall. Mọi cptr được phân giải qua CSpace nhiều cấp có guard.
/// `dest_cnode = 0` nghĩa là CNode gốc của chính người gọi.
///
/// Load/Store/Fetch KHÔNG có ở đây: chúng là lệnh máy đi qua MMU.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Syscall {
    // IPC
    Send { ep: u64, msg: Message },
    Call { ep: u64, msg: Message, recv_slot: u64 },
    Recv { ep: u64, recv_slot: u64 },
    Reply { msg: Message },
    ReplyRecv { ep: u64, msg: Message, recv_slot: u64 },
    Signal { ntfn: u64 },
    Wait { ntfn: u64 },
    Poll { ntfn: u64 },
    Yield,
    // Capability
    Retype { untyped: u64, obj_type: ObjType, size_bits: u32, dest_index: u64, count: u64, dest_cnode: u64 },
    CapCopy { src: u64, dest_index: u64, rights: Rights, dest_cnode: u64 },
    CapMint { src: u64, dest_index: u64, rights: Rights, badge: u64, dest_cnode: u64 },
    CNodeMint { src: u64, dest_index: u64, guard: u64, guard_bits: u32, dest_cnode: u64 },
    CapDelete { slot: u64 },
    CapRevoke { slot: u64 },
    // TCB
    TcbConfigure {
        tcb: u64,
        cspace: u64,
        vspace: u64,
        priority: u32,
        mcp: u32,
        /// Mã image (tương đương địa chỉ điểm vào trong hệ thật).
        image: u32,
        /// Giá trị thanh ghi a0..a3 lúc khởi động.
        args: [u64; NUM_ARGS],
        /// 0 = không có fault handler. Badge lỗi = badge của cap này.
        fault_ep: u64,
    },
    TcbResume { tcb: u64 },
    TcbSuspend { tcb: u64 },
    TcbRestart { tcb: u64 },
    TcbSetPriority { tcb: u64, priority: u32 },
    TcbSetAffinity { tcb: u64, hart: u32 },
    TcbBindNotification { tcb: u64, ntfn: u64 },
    // Lập lịch theo ngân sách (MCS)
    SchedContextConfigure { sched_control: u64, sc: u64, budget: u64, period: u64 },
    SchedContextBind { sc: u64, tcb: u64 },
    // Bộ nhớ ảo
    PageTableMap { page_table: u64, vspace: u64, vaddr: u64 },
    FrameMap { frame: u64, vspace: u64, vaddr: u64, rights: Rights },
    FrameUnmap { frame: u64 },
    // IOMMU
    IoMap { frame: u64, iospace: u64, iova: u64, rights: Rights },
    IoUnmap { frame: u64 },
    // Ngắt
    IrqGet { control: u64, irq: u32, dest_index: u64, dest_cnode: u64 },
    IrqSetNotification { handler: u64, ntfn: u64 },
    IrqAck { handler: u64 },
    // Khác
    DebugPrint { text: String },
    GetTime,
}

impl Syscall {
    /// Số hiệu syscall (giá trị thanh ghi a7).
    pub fn number(&self) -> u64 {
        use Syscall::*;
        match self {
            Send { .. } => 0,
            Call { .. } => 1,
            Recv { .. } => 2,
            Reply { .. } => 3,
            ReplyRecv { .. } => 4,
            Signal { .. } => 5,
            Wait { .. } => 6,
            Poll { .. } => 7,
            Yield => 8,
            Retype { .. } => 9,
            CapCopy { .. } => 10,
            CapMint { .. } => 11,
            CNodeMint { .. } => 12,
            CapDelete { .. } => 13,
            CapRevoke { .. } => 14,
            TcbConfigure { .. } => 15,
            TcbResume { .. } => 16,
            TcbSuspend { .. } => 17,
            TcbRestart { .. } => 18,
            TcbSetPriority { .. } => 19,
            TcbSetAffinity { .. } => 20,
            TcbBindNotification { .. } => 21,
            SchedContextConfigure { .. } => 22,
            SchedContextBind { .. } => 23,
            PageTableMap { .. } => 24,
            FrameMap { .. } => 25,
            FrameUnmap { .. } => 26,
            IoMap { .. } => 27,
            IoUnmap { .. } => 28,
            IrqGet { .. } => 29,
            IrqSetNotification { .. } => 30,
            IrqAck { .. } => 31,
            DebugPrint { .. } => 32,
            GetTime => 33,
        }
    }
}
