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
        argc: u64,
        argv: u64,
    },
    /// Rebuild a TCB context at its configured entry/stack.
    RestartTcb {
        tcb: ObjRef,
        entry: u64,
        stack: u64,
        argc: u64,
        argv: u64,
    },
    /// D15 (r246): ghi `argc`/`argv` vào context đã dựng của TCB.
    SetTcbArgs {
        tcb: ObjRef,
        argc: u64,
        argv: u64,
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

/// Bất biến sched ở RANH GIỚI trả về user (D19, r259).
///
/// Chỉ kiểm ở ranh giới — nơi hạt nhân vừa chọn TCB kế tiếp và sắp trả về
/// user — chứ không kiểm trên mọi lời gọi syscall: chi phí O(MAX_TCBS) chỉ
/// trả một lần cho mỗi lần chuyển ngữ cảnh, và ở ranh giới thì trạng thái
/// "đang chạy" là thứ vừa được thiết lập xong.
///
/// Hợp đồng: `current[hart]` tồn tại, TCB đó đang `Running`, và nó KHÔNG
/// còn nằm trong ready queue. Vi phạm nghĩa là TCB có thể chạy hai lần
/// hoặc bị chọn trong khi đang chạy — đúng lớp lỗi K40.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryError {
    /// Hart ở ranh giới nhưng không có `current` (không thể quay về user).
    NoCurrent,
    /// `current` không ở trạng thái Running.
    CurrentNotRunning,
    /// TCB vừa Running vừa nằm trong ready queue (sẽ chạy hai lần).
    RunningAndQueued,
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
    /// Vòng 160: trang staging module (runtime map read-only vào root vspace).
    staging: [Option<(ObjRef, u64, ObjRef)>; 32],
    /// Vòng 156: nhịp timer (tick/giây) do platform đẩy vào lúc init.
    timer_hz: u32,
    /// Root's cspace is separate so ordinary TCBs stay compact.
    pub(crate) root_cspace: [Option<Cap>; MAX_CAPS_ROOT],
    pub(crate) next_cap_id: u64,
    pub(crate) next_asid: u16,
    /// (#2a) số TCB đang BlockedSleep — fast-path cho `tick()` bỏ qua
    /// `wake_sleepers` (quét 168 object) khi bằng 0. Tăng khi `Sleep` park
    /// thành công, giảm khi wake (hết deadline / make_ready / wake_error /
    /// suspend / restart / exit) hoặc destroy/xoá TCB đang ngủ. Sửa trực tiếp
    /// `tcb.state` trong test KHÔNG qua helper nên counter có thể lệch ở test
    /// — `tick()` luôn đối chiếu `sleepers_active > 0` trước khi quét và
    /// `wake_sleepers` đồng bộ lại về số TCB BlockedSleep thật khi thấy.
    sleepers_active: u32,
    /// (#2a) tương tự cho BlockedBudget — fast-path cho `wake_budgeters`.
    budgeters_active: u32,
    /// (#2b) map number IRQ (0..64) -> object: `irq_raise` O(1) thay vì quét
    /// 168 object. `create_irq` cấm number >= 64 nên index luôn trong biên.
    /// Entry được dọn ở `destroy_object` (Kind::Irq) và giữ đồng bộ với gen.
    irq_by_number: [Option<ObjRef>; 64],
    /// (#3a) gợi ý slot trống cho `alloc_owned`: thử hint trước, chỉ quét
    /// tuyến tính khi hint bẩn. `destroy_object` đẩy slot vừa free về hint.
    next_free_hint: u8,
    /// (#3b) số TCB sống — `tcb_count()` O(1) thay vì quét 168 object.
    /// Tăng ở `create_tcb`/retype TCB thành công, giảm ở `destroy_object`.
    live_tcb_count: usize,
    /// K60 (r260): mask các hart ĐÃ THẬT SỰ chạy (tick, được chọn làm
    /// `current`, hoặc nạp ASID). `check_invariants` dùng nó để bắt
    /// `harts_loaded` "ma" — bit cho hart chưa từng chạy thì không thể có
    /// ASID nạp thật, tức là sổ sách flush đã hỏng.
    harts_seen: u32,
    /// (#3b) bitmask slot context đang dùng — `free_context_slot` O(1) thay
    /// vì O(24×168). Bit i = slot i đã cấp. Đồng bộ ở mọi nơi gán/xoá
    /// `context_slot` (create/retype/destroy).
    context_slot_used: u32,
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
            staging: [None; 32],
            timer_hz: 100,
            root_cspace: [None; MAX_CAPS_ROOT],
            next_cap_id: 1,
            next_asid: 1,
            sleepers_active: 0,
            budgeters_active: 0,
            irq_by_number: [None; 64],
            next_free_hint: 0,
            live_tcb_count: 0,
            harts_seen: 0,
            context_slot_used: 0,
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
        self.sleepers_active = 0;
        self.budgeters_active = 0;
        self.irq_by_number = [None; 64];
        self.next_free_hint = 0;
        self.live_tcb_count = 0;
        self.context_slot_used = 0;
        self.init_fs();
        if let Some(root) = self.create_tcb("root", 1) {
            self.root = Some(root);
        }
        // Rebuild các counter từ object sống sau bring-up (root TCB):
        // create_tcb trong reset đã tăng counter nhưng để chắc chắn với mọi
        // đường tạo TCB ngoài alloc (hiện không có, phòng thủ sâu).
        self.rebuild_tcb_caches();
        debug_assert!(self.live_tcb_count >= 1);
    }

    /// Quét một lần để dựng lại `live_tcb_count`/`context_slot_used` từ object
    /// sống. Chỉ dùng ở bring-up/test-repair, không dùng ở đường nóng.
    pub(crate) fn rebuild_tcb_caches(&mut self) {
        let mut count = 0usize;
        let mut mask = 0u32;
        for object in self.objs.iter() {
            if let Kind::Tcb(tcb) = object.kind {
                count += 1;
                if (tcb.context_slot as usize) < MAX_TCBS {
                    mask |= 1u32 << tcb.context_slot;
                }
            }
        }
        self.live_tcb_count = count;
        self.context_slot_used = mask;
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
        // Vòng 101 (K55): DMA-WRITE cũng là WRITE — phải cộng mapped_w để
        // W^X chéo CPU/DMA khép kín. Trước đây iommu_map(WRITE) lúc
        // mapped_x == 0 lọt mà counters 0/0, rồi map_frame(EXEC) thấy
        // mapped_w == 0 cũng lọt => 1 PA vừa CPU-X vừa DMA-W.
        let next_w = if rights.contains(Rights::WRITE) {
            frame.mapped_w.checked_add(1).ok_or(KError::OutOfMemory)?
        } else {
            frame.mapped_w
        };
        let mapping = IommuMapping {
            iova,
            pa: frame.pa as u64,
            len: DEFAULT_PAGE_SIZE,
            rights,
            frame: frame_obj,
        };
        {
            let entry = self.iommu_mut(domain).ok_or(KError::WrongType)?;
            entry.insert(mapping)?;
            entry.flush_iotlb();
        }
        self.set_frame_mapping_counts(frame_obj, next_w, frame.mapped_x);
        Ok(mapping)
    }

    pub fn iommu_unmap(&mut self, domain: ObjRef, iova: u64) -> KResult<IommuMapping> {
        let entry = self.iommu(domain).ok_or(KError::WrongType)?;
        let mapping = entry
            .mappings
            .iter()
            .flatten()
            .find(|m| m.iova == iova)
            .copied()
            .ok_or(KError::NotMapped)?;
        let frame_obj = mapping.frame;
        let is_write = mapping.rights.contains(Rights::WRITE);
        let entry = self.iommu_mut(domain).ok_or(KError::WrongType)?;
        let mapping = entry.remove(iova)?;
        entry.flush_iotlb();
        // Vòng 101 (K55): đối ứng với cộng mapped_w lúc map.
        if is_write {
            let counts = self.frame(frame_obj).map(|f| (f.mapped_w, f.mapped_x));
            if let Some((w, x)) = counts {
                self.set_frame_mapping_counts(frame_obj, w.saturating_sub(1), x);
            }
        }
        Ok(mapping)
    }

    fn unmap_iommu_frame_everywhere(&mut self, frame: ObjRef) {
        let mut removed_w = 0u16;
        for index in 0..MAX_OBJECTS {
            let obj = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if let Some(domain) = self.iommu_mut(obj) {
                // Vòng 101 (K55): đếm số mapping WRITE bị gỡ để trừ mapped_w.
                let before = domain
                    .mappings
                    .iter()
                    .flatten()
                    .filter(|m| m.frame == frame && m.rights.contains(Rights::WRITE))
                    .count();
                domain.remove_frame(frame);
                domain.flush_iotlb();
                removed_w = removed_w.saturating_add(before.min(u16::MAX as usize) as u16);
            }
        }
        if removed_w != 0 {
            let counts = self.frame(frame).map(|f| (f.mapped_w, f.mapped_x));
            if let Some((w, x)) = counts {
                self.set_frame_mapping_counts(frame, w.saturating_sub(removed_w), x);
            }
        }
    }

    pub fn set_page_table_ops(&mut self, ops: &'static dyn PageTableOps) {
        self.page_table_ops = Some(ops);
    }

    /// Vòng 160: ghi nhận một trang staging để thu hồi sau khi root parse
    /// xong module (xem `release_staging`).
    pub fn register_staging(&mut self, vspace: ObjRef, va: u64, frame: ObjRef) {
        for entry in self.staging.iter_mut() {
            if entry.is_none() {
                *entry = Some((vspace, va, frame));
                return;
            }
        }
    }

    /// Vòng 160: bỏ mapping và huỷ mọi trang staging đã ghi nhận; trả số trang
    /// đã thu hồi. Trang huỷ ⇒ vùng nhớ về `Untyped.free` của arena.
    pub fn release_staging(&mut self) -> usize {
        let mut released = 0usize;
        for index in 0..self.staging.len() {
            let Some((vspace, va, frame)) = self.staging[index] else {
                continue;
            };
            self.staging[index] = None;
            let _ = self.unmap_frame(vspace, va);
            if self.destroy_object(frame).is_ok() {
                released += 1;
            }
        }
        released
    }

    /// Vòng 156: đặt nhịp timer (tick/giây).
    pub fn set_timer_hz(&mut self, hz: u32) {
        self.timer_hz = hz.max(1);
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
        self.harts_seen |= 1 << hart;
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
        // (#3c, correctness-first): giữ logic skip wrap-around hiện tại (an
        // toàn, đã có test `asid_allocation_skips_live_vspaces_after_wrap`).
        // Tối ưu duy nhất: kiểm tra nhanh trước — `next_asid` không bận thì
        // return ngay với 1 lần check O(1)-amortized-ish thay vì vào vòng
        // lặp. `asid_in_use` vẫn quét 168 object nhưng chỉ 1 lần trong case
        // phổ biến (không wrap), thay vì worst-case 4095×168 khi nghẽn.
        // Không thêm refcount `asid_live` vì phải đồng bộ ở mọi đường
        // create/destroy/retype VSpace — rủi ro stale cao hơn lợi ích.
        let first = self.next_asid;
        if first >= 1 && first < 0x0fff && !self.asid_in_use(first) {
            self.next_asid = if first >= 0x0fff { 1 } else { first + 1 };
            return first;
        }
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

    /// (#3a) cấp object với gợi ý `next_free_hint`: thử hint trước (O(1)
    /// khi hint sạch — case phổ biến sau create/destroy xen kẽ), chỉ quét
    /// tuyến tính khi hint bẩn. Mọi đường giải phóng object (destroy +
    /// create_tcb-rollback) đều đẩy slot vừa free về hint nên hint ít bẩn.
    fn alloc_owned(
        &mut self,
        kind: Kind,
        owner: Option<ObjRef>,
        creator: Option<CapRef>,
    ) -> Option<ObjRef> {
        let hint = (self.next_free_hint as usize).min(MAX_OBJECTS - 1);
        if self.objs[hint].is_free() {
            return Some(self.alloc_at(hint, kind, owner, creator));
        }
        for i in 0..MAX_OBJECTS {
            if self.objs[i].is_free() {
                self.next_free_hint = i as u8;
                return Some(self.alloc_at(i, kind, owner, creator));
            }
        }
        None
    }

    /// Ghi object vào slot đã biết trống. Caller đảm bảo slot free.
    fn alloc_at(
        &mut self,
        index: usize,
        kind: Kind,
        owner: Option<ObjRef>,
        creator: Option<CapRef>,
    ) -> ObjRef {
        let gen = self.objs[index].gen.wrapping_add(1);
        let gen = if gen == 0 { 1 } else { gen };
        self.objs[index].gen = gen;
        self.objs[index].kind = kind;
        self.objs[index].owner = owner;
        self.objs[index].creator = creator;
        ObjRef {
            index: index as u8,
            gen,
        }
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
        // Vòng 101 (K47): số IRQ cắt `& 63` khi ánh xạ bit — 2 IRQ khác nhau
        // (vd. 7 và 71) sẽ alias cùng bit, ack(7) xóa luôn cạnh của 71. Cấm
        // number >= 64 ngay lúc tạo thay vì sửa mọi đường signal/ack/raise.
        if number >= 64 {
            return None;
        }
        // (#2b) check trùng O(1) qua map thay vì quét 168 object.
        if self.irq_by_number[number as usize].is_some() {
            return None;
        }
        let object = self.alloc(Kind::Irq(Irq {
            number,
            notification: None,
            pending: false,
        }))?;
        // Map giữ ObjRef có gen — raise qua map nhưng verify gen chống stale.
        self.irq_by_number[number as usize] = Some(object);
        Some(object)
    }

    /// (#3b) số TCB sống — O(1) qua counter thay vì quét 168 object.
    fn tcb_count(&self) -> usize {
        self.live_tcb_count
    }

    pub fn create_tcb(&mut self, name: &'static str, priority: u8) -> Option<ObjRef> {
        if self.tcb_count() >= MAX_TCBS {
            return None;
        }
        // Vòng 100 (K27): giành object TRƯỚC context slot. Trước đây lấy slot
        // rồi mới alloc; alloc fail thì slot bị rò (retype hỏng liên tiếp làm
        // cạn slot dù không tạo được TCB nào).
        let object = self.alloc(Kind::Tcb(Tcb::new(name, priority)))?;
        let Some(slot) = self.free_context_slot() else {
            // Trả object vừa cấp để không rò (cùng cách destroy_object giải
            // phóng: đặt lại Object::free và bump gen).
            if let Some(entry) = self.objs.get_mut(object.index as usize) {
                *entry = Object::free();
            }
            // (#3a) slot vừa free phải về hint để alloc sau dùng ngay.
            self.next_free_hint = object.index;
            return None;
        };
        if let Some(tcb) = self.tcb_mut(object) {
            tcb.context_slot = slot;
        }
        // (#3b) duy trì counter + bitmask sau khi gán slot thành công.
        self.live_tcb_count = self.live_tcb_count.saturating_add(1);
        self.context_slot_used |= 1u32 << slot;
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
        // Vòng 101 (K59): 2 Frame cùng PA thì unmap_frame_everywhere quét
        // theo PA sẽ xóa nhầm mapping của anh em. Từ chối overlap ngay lúc
        // tạo (giữ đồng nhất với invariant coi overlap là lỗi).
        if size == 0 {
            return None;
        }
        let (start, end) = (pa as u64, (pa as u64).saturating_add(size as u64));
        for object in self.objs.iter() {
            if let Kind::Frame(other) = object.kind {
                let (o_start, o_end) = (
                    other.pa as u64,
                    (other.pa as u64).saturating_add(other.size as u64),
                );
                if start < o_end && o_start < end {
                    return None;
                }
            }
        }
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
        // D15: cấu hình lại nghĩa là chương trình mới ⇒ xoá tham số cũ. Ai
        // muốn truyền tham số thì gọi `TcbSetArgs` SAU bước này.
        table.argc = 0;
        table.argv = 0;
        Ok(())
    }

    /// D15 (r246): đặt `argc`/`argv` khởi động cho một TCB đã configure
    /// nhưng chưa chạy. Chỉ nhận TCB `Inactive`/`Exited` để không ghi đè
    /// context của task đang sống.
    pub fn set_tcb_args(&mut self, tcb: ObjRef, argc: u64, argv: u64) -> KResult<()> {
        if argc > vifast_abi::user_layout::EXEC_ARGS_MAX as u64 {
            return Err(KError::InvalidArg);
        }
        if argc != 0 && argv == 0 {
            return Err(KError::InvalidArg);
        }
        if argv % 8 != 0 {
            return Err(KError::InvalidArg);
        }
        let table = self.tcb_mut(tcb).ok_or(KError::WrongType)?;
        if table.state != ThreadState::Inactive && table.state != ThreadState::Exited {
            return Err(KError::IllegalOperation);
        }
        table.argc = argc;
        table.argv = argv;
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
        // Vòng 100 (K21): ack phải xoá cả bit tương ứng trong notification đã
        // bind. Nếu chỉ xoá `pending`, bit cũ còn trong `word` sẽ làm lần raise
        // kế tiếp "dính" vào cùng bit (word |= bit) và người nhận chỉ thấy một
        // wakeup cho hai edge — đúng kiểu mất ngắt.
        let (notification, number) = {
            let irq = self.irq_mut(irq_ref).ok_or(KError::WrongType)?;
            irq.pending = false;
            (irq.notification, irq.number)
        };
        if let Some(notification) = notification {
            if let Some(ntfn) = self.notification_mut(notification) {
                ntfn.word &= !(1u64 << (number & 63));
            }
        }
        Ok(())
    }

    /// Raise a logical IRQ from a platform interrupt handler.  Edges are
    /// coalesced until user space acknowledges the object.  Returns true when
    /// a new pending edge was recorded.
    pub fn irq_raise(&mut self, number: u32) -> bool {
        // (#2b) O(1) qua map thay vì quét 168 object. number >= 64 không bao
        // giờ có object (create_irq cấm) nên từ chối ngay.
        let index = u32::try_from(number).ok().map(|n| n as usize);
        let Some(index) = index.filter(|i| *i < 64) else {
            return false;
        };
        let Some(irq_ref) = self.irq_by_number[index] else {
            return false;
        };
        // Verify gen chống stale: map và object phải cùng gen, object phải
        // còn là Irq đúng number (destroy/reuse slot không để lại entry cũ).
        let (notification, pending) = match self.irq(irq_ref) {
            Some(irq) if irq.number == number => (irq.notification, irq.pending),
            _ => {
                // Entry stale (object đã destroy nhưng map chưa dọn — không
                // nên xảy ra vì destroy dọn map; dọn phòng thủ tại đây).
                if self.get(irq_ref).is_none() {
                    self.irq_by_number[index] = None;
                }
                return false;
            }
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

    /// (#12, trong phạm vi lib.rs): đánh thức ĐÚNG MỘT waiter đang
    /// BlockedNotification (FIFO theo thứ tự slot), giữ bit cho waiter còn
    /// lại thay vì wake tất cả như `signal`. Dùng khi notification được dùng
    /// như hàng đợi (tránh thundering herd: N waiter tranh 1 event).
    /// Trả true khi đã đánh thức một waiter, false khi không có waiter nào
    /// đang chờ (bit giữ lại trong word).
    pub fn signal_one(&mut self, notification: ObjRef, bits: u64) -> KResult<bool> {
        let bits = if bits == 0 { 1 } else { bits };
        let pending = {
            let ntfn = self.notification(notification).ok_or(KError::InvalidCap)?;
            ntfn.waiters
        };
        // Tìm waiter đầu tiên còn BlockedNotification (FIFO theo slot).
        let mut target = None;
        let mut target_index = 0usize;
        for (index, waiter) in pending.into_iter().enumerate() {
            let Some(waiter) = waiter else { continue };
            let blocked = self
                .tcb(waiter)
                .is_some_and(|table| table.state == ThreadState::BlockedNotification);
            if blocked {
                target = Some(waiter);
                target_index = index;
                break;
            }
        }
        let Some(waiter) = target else {
            // Không ai chờ — cộng bit vào word như signal.
            if let Some(ntfn) = self.notification_mut(notification) {
                ntfn.word |= bits;
            }
            self.stats.notifications += 1;
            return Ok(false);
        };
        // Gỡ đúng slot của waiter được chọn, giữ nguyên các waiter khác.
        // Word cộng bit rồi trả cho waiter này; các waiter còn lại chờ event
        // sau (bit của họ không mất vì word đã cộng dồn trước khi clear).
        let value = {
            let ntfn = self
                .notification_mut(notification)
                .ok_or(KError::InvalidCap)?;
            let value = ntfn.word | bits;
            ntfn.waiters[target_index] = None;
            ntfn.word = 0;
            value
        };
        if let Ok(msg) = Message::new(LABEL_NOTIFICATION, &[value]) {
            if let Some(table) = self.tcb_mut(waiter) {
                table.resume = Some(msg);
                table.resume_error = None;
            }
        }
        self.make_ready(waiter);
        self.stats.notifications += 1;
        Ok(true)
    }

    /// (#2b) đồng bộ map IRQ sau khi object IRQ đổi trạng thái sống/chết.
    /// Gọi ở destroy path; create path ghi trực tiếp vì đã có ObjRef mới.
    fn irq_map_remove(&mut self, number: u32, reference: ObjRef) {
        if (number as usize) < 64 && self.irq_by_number[number as usize] == Some(reference) {
            self.irq_by_number[number as usize] = None;
        }
    }

    /// (#2a) tăng sleeper counter, saturating để không bao giờ wrap.
    fn sleepers_inc(&mut self) {
        self.sleepers_active = self.sleepers_active.saturating_add(1);
    }

    /// (#2a) giảm sleeper counter, saturating để destroy path kép không panic.
    fn sleepers_dec(&mut self) {
        self.sleepers_active = self.sleepers_active.saturating_sub(1);
    }

    /// (#2a) tăng budgeter counter.
    fn budgeters_inc(&mut self) {
        self.budgeters_active = self.budgeters_active.saturating_add(1);
    }

    /// (#2a) giảm budgeter counter.
    fn budgeters_dec(&mut self) {
        self.budgeters_active = self.budgeters_active.saturating_sub(1);
    }

    /// (#2a) rời trạng thái BlockedSleep: xoá wake_at + giảm counter đúng
    /// một lần. Trả true nếu TCB thật sự đang ngủ (để caller quyết định tiếp).
    /// Mọi đường rời sleep (wake hết deadline, suspend, wake_error,
    /// make_ready, restart, exit, destroy) đều đi qua đây.
    fn leave_sleep(&mut self, tcb: ObjRef) -> bool {
        let sleeping = self
            .tcb(tcb)
            .is_some_and(|table| table.state == ThreadState::BlockedSleep);
        if !sleeping {
            return false;
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.wake_at = None;
        }
        self.sleepers_dec();
        true
    }

    /// (#2a) rời trạng thái BlockedBudget, đối xứng với `leave_sleep`.
    fn leave_budget(&mut self, tcb: ObjRef) -> bool {
        let blocked = self
            .tcb(tcb)
            .is_some_and(|table| table.state == ThreadState::BlockedBudget);
        if !blocked {
            return false;
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.wake_at = None;
        }
        self.budgeters_dec();
        true
    }

    /// (#2a) self-heal đối xứng với wake_sleepers: quét mà không thấy
    /// budgeter thật thì đồng bộ counter về 0.
    fn wake_budgeters(&mut self, now: u64) -> usize {
        // (#2a) fast-path: không có budgeter thì không quét 168 object.
        if self.budgeters_active == 0 {
            return 0;
        }
        let mut woke = 0usize;
        let mut seen = 0u32;
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            let expired = match self.objs[index].kind {
                Kind::Tcb(tcb) => {
                    if tcb.state == ThreadState::BlockedBudget {
                        seen = seen.saturating_add(1);
                        tcb.wake_at.is_some_and(|deadline| deadline <= now)
                    } else {
                        false
                    }
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
            self.leave_budget(reference);
            if let Some(tcb) = self.tcb_mut(reference) {
                tcb.state = ThreadState::Ready;
                tcb.resume = None;
                tcb.resume_error = None;
            }
            self.enqueue(reference);
            woke += 1;
        }
        if seen == 0 {
            self.budgeters_active = 0;
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
            // (#2a) vào BlockedBudget — tăng counter để tick() biết phải quét.
            self.budgeters_inc();
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
        // Vòng 101 (K62): cấm giấu vé nhúng sau CNode — bind CNode khi mảng
        // nhúng còn vé sống sẽ tạo "vé ma": revoke/cap_at/collect/delete/
        // invariant/remove_caps_to đi theo view hiện tại (CNode) nên vé ẩn
        // không bao giờ bị đánh dấu/thu. Muốn bind CNode thì xóa sạch vé
        // nhúng trước (delete từng slot); unbind (None) luôn cho phép.
        if cnode.is_some() {
            let has_live_embedded = self
                .tcb(target)
                .map(|table| table.cspace.iter().any(Option::is_some))
                .unwrap_or(false);
            if has_live_embedded {
                return Err(KError::IllegalOperation);
            }
            if self.cnode(cnode.unwrap()).is_none() {
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
            cnode: self.table_of(caller),
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
            ThreadState::Inactive
            | ThreadState::Faulted
            | ThreadState::BlockedFault
            | ThreadState::Exited => return Err(KError::IllegalOperation),
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
                // (#2a) suspend gỡ khỏi sleep — giảm counter qua helper.
                self.leave_sleep(target);
                let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
                table.resume = Some(Message::empty(0));
                table.resume_error = Some(KError::WouldBlock);
            }
            ThreadState::BlockedBudget => {
                self.remove_from_runq(target);
                // (#2a) suspend gỡ khỏi budget — giảm counter qua helper.
                self.leave_budget(target);
                let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
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
        // Vòng 101 (K32/K33/K34): resume chỉ cho Inactive (mở máy),
        // Suspended (tiếp tục sau suspend), Ready (lũy đẳng) và BlockedFault
        // (fault handler xử lý xong cho chạy lại — pager flow dựa vào đường
        // này). Mọi Blocked* khác/Running đều từ chối — trước đây resume
        // set Ready mù quáng mà không gỡ
        // endpoint.send/recv hay notification.waiters (K32, thành viên
        // kép), ép Running thành Ready trong khi current[hart] còn trỏ
        // tới nó (K33, alias current+queued), và resume ngay sau
        // suspend-Running khi runtime chưa kịp dọn current (K34).
        // BlockedReply cũng cấm như suspend để server không ôm caller cũ.
        if !matches!(
            state,
            ThreadState::Inactive
                | ThreadState::Suspended
                | ThreadState::Ready
                | ThreadState::BlockedFault
        ) {
            return Err(KError::IllegalOperation);
        }
        if self.current.contains(&Some(tcb)) {
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
    pub fn restart_tcb(&mut self, target: ObjRef, caller: ObjRef) -> KResult<(u64, u64, u64, u64)> {
        if target == caller {
            return Err(KError::IllegalOperation);
        }
        // Mốc 1 (vòng 64): cấm restart task đang là current trên bất kỳ
        // hart nào — nếu không hart đó quay lại user với entry/stack cũ
        // trong khi TCB đã bị reset (giống K33 của resume, alias current).
        if self.current.contains(&Some(target)) {
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
        let argc = table.argc;
        let argv = table.argv;
        let sched = table.sched_context;
        // Vòng 101 (K36): cũng cấm restart một client đang BlockedReply —
        // nếu không server ôm caller cũ mà `reply` đòi BlockedReply+epoch
        // nên server treo với caller ma. Quét mọi server và gỡ caller
        // trỏ tới target để reply sau này trả NoCaller sạch sẽ.
        if self
            .tcb(target)
            .map(|t| t.state)
            .is_some_and(|s| s == ThreadState::BlockedReply)
        {
            return Err(KError::IllegalOperation);
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(table) = self.tcb_mut(r) {
                if table.caller.is_some_and(|(client, _)| client == target) {
                    table.caller = None;
                }
            }
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(endpoint) = self.endpoint_mut(r) {
                endpoint.send.remove(target);
                endpoint.recv.remove(target);
            }
            if let Some(notification) = self.notification_mut(r) {
                for slot in notification.waiters.iter_mut() {
                    if *slot == Some(target) {
                        *slot = None;
                    }
                }
            }
        }
        self.remove_from_runq(target);
        if let Some(sched) = sched {
            let now = self.fast_time();
            if let Some(context) = self.sched_context_mut(sched) {
                context.remaining = context.budget;
                context.period_start = now;
            }
        }
        // (#2a) restart rời sleep/budget cũ (nếu có) trước khi Ready —
        // dùng helper để counter giảm đúng một lần, kể cả khi TCB đang
        // BlockedSleep/BlockedBudget. wake_at luôn xoá (kể cả khi set tay
        // ngoài Blocked* như test round103 bẩn hoá wake_at=999 lúc Running).
        self.leave_sleep(target);
        self.leave_budget(target);
        let table = self.tcb_mut(target).ok_or(KError::ObjectDestroyed)?;
        table.state = ThreadState::Ready;
        table.pending = None;
        table.resume = None;
        table.resume_error = None;
        table.recv_slot = 0;
        table.wake_at = None;
        table.fds = [None; MAX_FDS];
        self.enqueue(target);
        Ok((entry, stack, argc, argv))
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

    /// (#3b) slot context trống đầu tiên — O(1) qua bitmask thay vì
    /// O(24×168). MAX_TCBS = 24 vừa trong u32.
    fn free_context_slot(&self) -> Option<u8> {
        for slot in 0..MAX_TCBS {
            if self.context_slot_used & (1u32 << slot) == 0 {
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

    /// Leaked bytes for an Untyped by physical base: ranges returned while
    /// the free list was full (vòng 36: quan sát leak tích lũy qua serial).
    pub fn untyped_leak_by_base(&self, base: usize) -> Option<u32> {
        self.objs.iter().find_map(|object| match object.kind {
            Kind::Untyped(ref region) if region.base == base => Some(region.leaked),
            _ => None,
        })
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
                Some(self.cap_ref(addr, cap.id))
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
        // (#3b) retype TCB thành công cũng duy trì counter + bitmask như
        // create_tcb (check quota + giữ slot ở trên đã qua).
        if ty == ObjType::Tcb {
            self.live_tcb_count = self.live_tcb_count.saturating_add(1);
            if let Some(slot) = context_slot {
                self.context_slot_used |= 1u32 << slot;
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
                Some(self.cap_ref(addr, cap.id))
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
                    // Vòng 101 (K54): entry cha phủ cả span — flush local
                    // không đủ, hart khác giữ page-walk cache cũ. Dùng
                    // flush_vspace_range theo mask như map_frame/remap.
                    let span_pages =
                        ((span / page_size as u64).max(1).min(usize::MAX as u64)) as usize;
                    // `flush_vspace_range` cần &mut nhưng PTE đã ghi xong;
                    // gọi flush trực tiếp qua ops với mask hiện tại.
                    if let Some(space) = self.vspace(vspace) {
                        let (asid, mask) = (space.asid, space.harts_loaded);
                        if mask != 0 {
                            if span_pages > 16 {
                                ops.flush_all_asid(asid, mask);
                            } else {
                                ops.flush_va_asid(asid, va_base, span_pages, mask);
                            }
                        }
                    }
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
            // Vòng 101 (K53): KHÔNG xóa harts_loaded sau full-ASID flush.
            // Hart vẫn chạy VSpace đó không set lại mask tới lần switch
            // tiếp theo — mask rỗng làm mọi unmap/map sau bỏ shootdown
            // (thiếu flush, nguy hiểm hơn flush thừa).
            ops.flush_all_asid(asid, mask);
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
                    // Vòng 99: `Ready` mà không nằm trong runqueue là TCB chết
                    // (không ai chọn lại) — đúng lớp lỗi K1/K4. Ngược lại,
                    // `queued` phải kéo theo state Ready.
                    if tcb.state == ThreadState::Ready && !tcb.queued {
                        return Err(KError::IllegalOperation);
                    }
                    if tcb.queued && tcb.state != ThreadState::Ready {
                        return Err(KError::IllegalOperation);
                    }
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
                    // K60 (r260): bit cho hart CHƯA TỪNG chạy là mask ma —
                    // không có đường nào nạp ASID trên hart đó, nên sổ sách
                    // flush đang sai (shootdown gửi tới hart không tồn tại,
                    // hoặc bit bị đặt tay). Bắt ngay thay vì im lặng.
                    if space.harts_loaded & !self.harts_seen != 0 {
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
        // 290.3 (r290): `cap.parent` phải phân giải được ở MỌI nhánh — trước
        // đây chỉ kiểm khi `badge != 0`, nên một parent trỏ vào bảng đã chết
        // (TCB bị huỷ / CNode bị tháo) sống sót qua mọi lần kiểm. Hệ quả: cap
        // mồ côi ⇒ `revoke` không tới ⇒ object thành bất tử (P0-3).
        let mut broken_parent = false;
        self.for_each_cap(|_table, _slot, cap| {
            if let Some(parent) = cap.parent {
                let resolves = self
                    .resolve_cap_ref(parent)
                    .is_some_and(|source| source.id == parent.id);
                if !resolves {
                    broken_parent = true;
                }
            }
        });
        if broken_parent {
            return Err(KError::IllegalOperation);
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

    /// 290.1 (r290): bảng VẬT LÝ chứa một slot của `tcb` — `Some(cnode)` khi
    /// TCB dùng CNode, `None` khi cap nằm trong bảng nhúng của TCB.
    pub(crate) fn table_of(&self, tcb: ObjRef) -> Option<ObjRef> {
        if self.root == Some(tcb) {
            None
        } else {
            self.tcb(tcb).and_then(|table| table.cnode)
        }
    }

    /// Dựng `CapRef` cho một slot, ghi kèm bảng vật lý (290.1).
    pub(crate) fn cap_ref(&self, addr: SlotAddr, id: u64) -> CapRef {
        CapRef {
            addr,
            id,
            cnode: self.table_of(addr.tcb),
        }
    }

    /// Phân giải `CapRef` về cap thật, đi qua bảng vật lý đã ghi.
    ///
    /// 290.1: nhờ `cnode`, cap trong CNode **đã tháo khỏi TCB** vẫn phân giải
    /// được — trước đây chỉ có `SlotAddr{tcb, slot}` nên mọi bước nhảy qua
    /// CNode mồ côi đều thất bại và `revoke` trả `Ok(0)` (P0-3).
    /// 290.1 (r290): bộ duyệt MỘT bảng chìa vật lý dùng chung.
    ///
    /// Đi qua **mọi** bảng đang tồn tại: `root_cspace`, bảng nhúng của từng
    /// TCB, và `slots` của **mọi** CNode — kể cả CNode đã tháo khỏi TCB (đó
    /// là chỗ bốn bộ duyệt cũ bỏ sót, gốc của P0-3). Không cấp phát.
    ///
    /// `f(nhãn bảng, chỉ số slot, &cap)`; slot trống không gọi.
    pub fn for_each_cap<F: FnMut(TableId, usize, &Cap)>(&self, mut f: F) {
        if self.root.is_some() {
            for (slot, cap) in self.root_cspace.iter().enumerate() {
                if let Some(cap) = cap {
                    f(TableId::Root, slot, cap);
                }
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
            match self.get(reference).map(|object| object.kind) {
                Some(Kind::Tcb(_)) => {
                    let Some(table) = self.tcb(reference) else {
                        continue;
                    };
                    if table.cnode.is_none() {
                        for (slot, cap) in table.cspace.iter().enumerate() {
                            if let Some(cap) = cap {
                                f(TableId::Tcb(reference), slot, cap);
                            }
                        }
                    }
                }
                Some(Kind::CNode(_)) => {
                    let Some(node) = self.cnode(reference) else {
                        continue;
                    };
                    for (slot, cap) in node.slots.iter().enumerate() {
                        if let Some(cap) = cap {
                            f(TableId::CNode(reference), slot, cap);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Số slot mà `for_each_cap` sẽ đi qua — dùng cho test "đếm bằng tổng
    /// dung lượng các bảng đang tồn tại".
    pub fn cap_slot_capacity(&self) -> usize {
        let mut total = 0usize;
        if self.root.is_some() {
            total += self.root_cspace.len();
        }
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            if self.root == Some(reference) {
                continue;
            }
            match self.get(reference).map(|object| object.kind) {
                Some(Kind::Tcb(_)) => {
                    if self
                        .tcb(reference)
                        .is_some_and(|table| table.cnode.is_none())
                    {
                        total += MAX_CAPS_TASK;
                    }
                }
                Some(Kind::CNode(_)) => total += MAX_CAPS_TASK,
                _ => {}
            }
        }
        total
    }

    pub fn resolve_cap_ref(&self, reference: CapRef) -> Option<Cap> {
        let slot = reference.addr.slot as usize;
        match reference.cnode {
            Some(cnode) => self
                .cnode(cnode)
                .and_then(|node| node.slots.get(slot).copied().flatten()),
            None => self.cap_at(reference.addr),
        }
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
        let target = self.cap_ref(addr, cap.id);
        // Vòng 101 (K69): build_task_space retype VSpace/PageTable/Frame
        // với creator=None — sống, tốn byte, 0 vé. Xóa vé Untyped cuối thì
        // mất mọi đường retype/revoke chạm tới (collect chỉ lấy
        // None-creator khi target root). Cấm khi còn object owner==cap.obj
        // bất kể creator.
        let has_owned_objects = self.objs.iter().any(|object| {
            if object.is_free() || object.owner != Some(cap.obj) {
                return false;
            }
            if object
                .creator
                .is_some_and(|creator| self.cap_ref_is_descendant(creator, target))
            {
                return true;
            }
            // creator=None nhưng object sống và không phải chính Untyped
            // đang bị xóa vé: đó là object mồ côi cần giữ đường chạm tới.
            !matches!(object.kind, Kind::Untyped(_))
        });
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
        if has_live_objects || has_owned_objects || has_child_caps || has_cnode_child {
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
            cnode: self.table_of(src_tcb),
        });
        self.install_cap(dst_tcb, dst_slot, derived)
    }

    /// T27 (r264, G2 Phần B): uỷ quyền một cap sang cspace của TCB khác.
    ///
    /// Hợp đồng (mỗi điều là một test đỏ-trước):
    /// 1. Caller phải có cap WRITE tới TCB đích — không ai nhét cap vào cspace
    ///    của task khác mà không có quyền cấu hình nó.
    /// 2. Chỉ chuyển được cap caller THẬT SỰ giữ, và `dst_slot` phải trống.
    /// 3. Quyền bị THU HẸP: kết quả = `mask & cap.rights & !GRANT`. Không bao
    ///    giờ chuyển được GRANT (đúng nguyên tắc của IPC grant — tránh rửa
    ///    quyền qua nhiều bước).
    /// 4. Badge GIỮ NGUYÊN từ cap nguồn: người nhận không tự đặt danh tính
    ///    khác (badge là uid của FS server).
    /// 5. Không chuyển cap Untyped gốc (`root`) — cap đó là nguồn của mọi
    ///    object; chuyển nó là trao cả không gian bộ nhớ.
    ///
    /// Caller KHÔNG mất cap nguồn (đây là derive, không phải move).
    pub fn grant_cap_to_tcb(
        &mut self,
        caller: ObjRef,
        target: ObjRef,
        src_slot: u8,
        dst_slot: u8,
        mask: Rights,
    ) -> KResult<()> {
        let source = self.lookup_cap(caller, src_slot, None, Rights::NONE)?;
        if source.root && source.ty == ObjType::Untyped {
            return Err(KError::InvalidArg);
        }
        // a3 == 0 nghĩa là "giữ nguyên, trừ GRANT" (đúng như syscall mô tả);
        // nhờ vậy bên gọi không phải liệt kê lại quyền của cap nguồn.
        let mask = if mask.0 == 0 { Rights::ALL } else { mask };
        let rights = Rights(mask.0 & source.rights.0 & !Rights::GRANT.0);
        // badge = 0 nghĩa "giữ nguyên badge của cap nguồn" (xem `Cap::derive`);
        // truyền một badge khác sẽ bị từ chối khi cap nguồn đã có badge.
        self.copy_cap(caller, src_slot, target, dst_slot, rights, 0)
    }

    fn cap_ref_is_descendant(&self, creator: CapRef, target: CapRef) -> bool {
        if creator == target {
            return true;
        }
        // 290.2: phân giải qua BẢNG VẬT LÝ mà `CapRef` ghi (CNode đã tháo
        // khỏi TCB vẫn phân giải được) — trước đây chỉ có `SlotAddr` nên bước
        // nhảy qua CNode mồ côi luôn thất bại (P0-3).
        let Some(cap) = self.resolve_cap_ref(creator) else {
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
            let Some(parent_cap) = self.resolve_cap_ref(parent_ref) else {
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
        // Vòng 65: BlockedBudget (hết ngân sách sched_context) cũng là
        // Blocked* — wake_error phải Ready + queued, nếu không task treo
        // (có resume_error nhưng không ai pick).
        let was_blocked = matches!(
            state,
            ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
                | ThreadState::BlockedSleep
                | ThreadState::BlockedFault
                | ThreadState::BlockedBudget
        );
        // (#2a) wake_error rời sleep/budget — giảm counter đúng một lần qua
        // helper TRƯỚC khi đổi state (helper check state cũ).
        if state == ThreadState::BlockedSleep {
            self.leave_sleep(tcb);
        } else if state == ThreadState::BlockedBudget {
            self.leave_budget(tcb);
        }
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
        let cap = self.resolve_cap_ref(cap_ref)?;
        if cap.id != cap_ref.id {
            return None;
        }
        let mut parent = cap.parent;
        let mut depth = 0usize;
        while let Some(parent_ref) = parent {
            if !dying.contains(&parent_ref) {
                return self
                    .resolve_cap_ref(parent_ref)
                    .is_some_and(|candidate| candidate.id == parent_ref.id)
                    .then_some(parent_ref);
            }
            let parent_cap = self.resolve_cap_ref(parent_ref)?;
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
        // 290.2 (r290): KHÔNG return sớm khi TCB dùng CNode. Bản cũ bỏ qua
        // đường này vì "CNode là object độc lập", nhưng vòng lặp sửa
        // `cap.parent` chỉ đi qua `cspace_mut`, mà `cspace_mut` chỉ với tới
        // CNode THÔNG QUA TCB đang dùng nó. CNode đã tháo khỏi TCB thì không
        // ai sửa `parent` của nó ⇒ cap mồ côi ⇒ `revoke` không tới ⇒ object
        // thành bất tử (P0-3). Nay `CapRef.cnode` cho biết bảng vật lý nên
        // đường này đi được cả CNode.
        let table_is_cnode = self.table_of(obj);
        let Some(slots) = self.cspace(obj) else {
            return;
        };
        let mut dying = [CapRef::default(); MAX_CAPS_ROOT];
        let mut survivors = [None; MAX_CAPS_ROOT];
        let mut count = 0usize;
        for (slot, cap) in slots.iter().enumerate() {
            if let Some(cap) = cap {
                // 290.1: `slots` ở đây là bảng VẬT LÝ của `obj` — có thể là
                // CNode (khi TCB dùng CNode) nên phải ghi kèm `cnode`.
                dying[count] = CapRef {
                    addr: SlotAddr {
                        tcb: obj,
                        slot: slot as u8,
                    },
                    id: cap.id,
                    cnode: table_is_cnode,
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

        // 290.2: sửa `parent` trong MỌI bảng vật lý — root_cspace, bảng nhúng
        // của từng TCB, và slots của MỌI CNode (kể cả CNode đã tháo khỏi TCB).
        // Bản cũ chỉ đi qua `cspace_mut(r)`, mà hàm đó chỉ với tới CNode thông
        // qua TCB đang dùng nó ⇒ CNode đã tháo không bao giờ được sửa (P0-3).
        let fix = |cap: &mut Cap| {
            let Some(parent) = cap.parent else {
                return;
            };
            if let Some(index) = dying[..count].iter().position(|c| *c == parent) {
                cap.parent = survivors[index];
            }
        };
        for cap in self.root_cspace.iter_mut().flatten() {
            fix(cap);
        }
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            match self.get(r).map(|object| object.kind) {
                Some(Kind::Tcb(_)) => {
                    if self.root == Some(r) {
                        continue;
                    }
                    let uses_cnode = self.tcb(r).is_some_and(|table| table.cnode.is_some());
                    if !uses_cnode {
                        if let Some(table) = self.tcb_mut(r) {
                            for cap in table.cspace.iter_mut().flatten() {
                                fix(cap);
                            }
                        }
                    }
                }
                Some(Kind::CNode(_)) => {
                    if let Some(node) = self.cnode_mut(r) {
                        for cap in node.slots.iter_mut().flatten() {
                            fix(cap);
                        }
                    }
                }
                _ => {}
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
        // Vòng 100 (K23): TCB chết không được giữ bind notification cũ — nếu
        // object được tái dùng cho TCB khác, "Wait trên bound" sẽ nhận tín
        // hiệu của đời trước.
        if let Some(table) = self.tcb_mut(obj) {
            table.bound_ntfn = None;
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
            let blocked = self.tcb(reference).is_some_and(|table| {
                table.sched_context == Some(obj) && table.state == ThreadState::BlockedBudget
            });
            // (#2a) rời budget TRƯỚC khi đổi state — leave_budget check state
            // cũ nên phải gọi khi còn BlockedBudget, rồi mới set Ready.
            if blocked {
                self.leave_budget(reference);
            }
            let Some(table) = self.tcb_mut(reference) else {
                continue;
            };
            if table.sched_context == Some(obj) {
                table.sched_context = None;
                if blocked {
                    table.state = ThreadState::Ready;
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
        // Vòng 101 (K67): destroy mà không gỡ bound_ntfn thì TCB ôm
        // Some(chết), bind mới mãi AlreadyMapped, không có syscall unbind.
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if self
                .tcb(r)
                .is_some_and(|table| table.bound_ntfn == Some(obj))
            {
                if let Some(table) = self.tcb_mut(r) {
                    table.bound_ntfn = None;
                }
            }
            // Vòng 101 (K68): destroy mà không gỡ irq.notification thì IRQ
            // ôm Some(chết), rebind bị AlreadyMapped, raise cạnh rớt vào
            // signal(chết) mất hút.
            if let Some(irq) = self.irq_mut(r) {
                if irq.notification == Some(obj) {
                    irq.notification = None;
                    irq.pending = false;
                }
            }
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

    /// Remove one leaf mapping through the architecture page-table walk.
    /// Private (vòng 101, K58): entry cha và leaf đều đã được flush local,
    /// nhưng hart khác vẫn giữ bản dịch cũ. Mọi caller nội bộ (`unmap_frame`,
    /// `map_frame`-rollback) đều tự `flush_vspace_range` theo mask sau đó —
    /// caller ngoài gọi trực tiếp sẽ stale âm thầm. Giữ private để mọi
    /// đường unmap đều đi qua shootdown theo mask.
    fn unmap_page_table(&self, root: u64, va: u64) -> KResult<()> {
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
                // Vòng 100 (K10): destroy Frame trước đây chỉ gỡ PTE mà không
                // flush, nên hart khác còn TLB trỏ tới trang đã bị thu hồi
                // (rồi retype cho object khác) => đọc/ghi nhầm.
                self.note_frame_unmapping(*mapping);
                self.flush_vspace_range(r, mapping.va, 1);
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
            // Vòng 100 (K10): flush cả khi cả VSpace bị destroy — hart khác có
            // thể đã chạy trong không gian này và còn entry TLB.
            self.note_frame_unmapping(*mapping);
            self.flush_vspace_range(obj, mapping.va, 1);
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
            // Vòng 101 (K66): phòng thủ sâu — quét cả mảng nhúng trực tiếp,
            // không phụ thuộc view hiện tại. K62 đã cấm giấu vé sống sau
            // CNode nên nhánh này bình thường rỗng, nhưng nếu TCB chết khi
            // đang bound rồi object tái dùng, vé ma không được hồi sinh.
            if let Kind::Tcb(ref mut table) = self.objs[i].kind {
                for slot in table.cspace.iter_mut() {
                    if slot.is_some_and(|cap| cap.obj == obj) {
                        *slot = None;
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
    /// Vòng 101 (K61): zero bảng con mà không flush span thì hart khác giữ
    /// entry cha cũ trỏ tới bảng đã zero — fault oan/đọc 0. Flush cả span
    /// theo mask sau khi zero.
    fn orphan_child_tables(&mut self, vspace: ObjRef, start: u64, end: u64, parent_level: u8) {
        let (page_size, mask, asid) = match self.vspace(vspace) {
            Some(space) => (space.page_size, space.harts_loaded, space.asid),
            None => return,
        };
        let mut orphaned = false;
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
                if let (Some(ops), Some(size)) = (self.page_table_ops, Some(page_size)) {
                    ops.zero_frame(root, size as usize);
                    orphaned = true;
                }
            }
        }
        // Flush span bao phủ mọi bảng con vừa zero — hart khác không giữ
        // entry cha cũ trỏ tới bảng rỗng.
        if orphaned {
            if let Some(ops) = self.page_table_ops {
                if mask != 0 && end > start {
                    let span_pages = ((end - start) / page_size.max(1)).max(1).min(1024) as usize;
                    if span_pages > 16 {
                        ops.flush_all_asid(asid, mask);
                    } else {
                        ops.flush_va_asid(asid, start, span_pages, mask);
                    }
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
        // Vòng 101 (K52): thứ tự cũ flush-ASID TRƯỚC khi gỡ PTE rồi xóa
        // mask ngay — hart khác refill TLB giữa flush và lúc gỡ thì không
        // bao giờ flush lại (unmap_space gọi flush_vspace_range nhưng
        // early-return vì mask == 0). Giờ: gỡ PTE trước (unmap_space vẫn
        // thấy mask đầy đủ), flush cuối bằng flush_all_asid(asid, mask),
        // rồi mới xóa mask.
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
            // Flush cuối theo đúng mask đã load (không phải flush_all_user
            // local như trước — trên x86 nó chỉ reload CR3 cục bộ).
            if let Some(space) = self.vspace(obj) {
                let (asid, mask) = (space.asid, space.harts_loaded);
                if mask != 0 {
                    ops.flush_all_asid(asid, mask);
                }
            }
            if let Some(space) = self.vspace_mut(obj) {
                space.harts_loaded = 0;
            }
        } else if let Some(space) = self.vspace_mut(obj) {
            space.harts_loaded = 0;
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
            // Vòng 200: BỎ điều kiện "vspace còn mapping thì không huỷ được".
            // Giữ nó thì không VSpace nào của task exec huỷ được (vì luôn còn
            // mapping của code/stack) ⇒ frame + page table kẹt theo ⇒ rò rỉ 5
            // object mỗi chu kỳ. Huỷ address space phải TỰ gỡ mapping của nó
            // (nhánh `Kind::VSpace` trong `destroy_object` đã làm việc này).
            // Vòng 200: bỏ cả điều kiện "còn page table trỏ tới". Các page
            // table này chính là đối tượng mà `destroy_vspace_page_tables` sẽ
            // gỡ; giữ điều kiện này thì VSpace của task exec vẫn Busy vĩnh viễn
            // (mỗi lần chạy đều có 2 page table).
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
            // Vòng 194: TCB KHÔNG được rơi vào nhánh `_ => true`. Trước đây nó
            // luôn bị coi là "in use" ⇒ `destroy_cap_object` trả `Busy` vĩnh
            // viễn ⇒ TCB của task exec không bao giờ bị huỷ ⇒ VSpace luôn
            // `vspace_in_use` ⇒ frame luôn còn mapping ⇒ rò rỉ 6 object mỗi chu
            // kỳ (`DESTROY DIAG stuck=6`). `destroy_object` đã tự kiểm tra
            // current/root và tự gỡ khỏi runqueue/endpoint/notification.
            ObjType::Tcb => false,
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
            // Vòng 199: TCB phải huỷ được qua đường cap. Thiếu nó ở đây khiến
            // `destroy_cap_object` trả `InvalidArg` cho MỌI TCB ⇒ TCB của task
            // exec không bao giờ chết ⇒ VSpace luôn `vspace_in_use` ⇒ frame
            // không giải phóng (rò rỉ 6 object mỗi chu kỳ, đúng `DESTROY DIAG`).
            | ObjType::Tcb
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

    /// Kiểm tra các điều kiện tiên quyết của `destroy_object` mà KHÔNG phá gì
    /// (vòng 100, K8): `revoke_cap` cần biết chắc mọi object sẽ destroy được
    /// trước khi gỡ cap, nếu không rơi vào trạng thái nửa vời (cap mất, object
    /// còn sống, không ai chạm tới).
    /// T32 (r269): TCB đã `Exited` mà vẫn là `current` của một hart idle.
    ///
    /// `current` cố ý giữ TCB đã park cho tới khi runtime lưu context (K37),
    /// nhưng khi task đã exit thì giữ nó lại chỉ làm object không huỷ được:
    /// `destroy_object` từ chối, VSpace của nó kẹt `Busy`, và vòng exec sau
    /// hết slot (`DESTROY DIAG stuck=4` + `BIN EXEC FAIL`, soak 100 vòng 62).
    fn tcb_is_exited(&self, obj: ObjRef) -> bool {
        self.tcb(obj)
            .is_some_and(|tcb| tcb.state == ThreadState::Exited)
    }

    fn can_destroy(&self, obj: ObjRef) -> bool {
        let Some(kind) = self.get(obj).map(|object| object.kind) else {
            return true;
        };
        if (self.current.contains(&Some(obj)) && !self.tcb_is_exited(obj)) || self.root == Some(obj)
        {
            return false;
        }
        if matches!(kind, Kind::VSpace(_))
            && self
                .current
                .iter()
                .flatten()
                .any(|current| self.tcb(*current).and_then(|tcb| tcb.vspace) == Some(obj))
        {
            return false;
        }
        if matches!(kind, Kind::CNode(_))
            && (0..MAX_OBJECTS).any(|index| {
                let reference = ObjRef {
                    index: index as u8,
                    gen: self.objs[index].gen,
                };
                self.tcb(reference).and_then(|tcb| tcb.cnode) == Some(obj)
            })
        {
            return false;
        }
        // Vòng 101 (K56): Iommu domain còn mapping sống thì không destroy —
        // nếu không PA tái cấp trong khi IOTLB cũ còn trỏ (revoke giết
        // domain sống vì can_destroy cũ luôn true với Iommu).
        if matches!(kind, Kind::Iommu(_))
            && self
                .iommu(obj)
                .is_some_and(|domain| domain.mappings.iter().any(Option::is_some))
        {
            return false;
        }
        true
    }

    fn destroy_object(&mut self, obj: ObjRef) -> KResult<()> {
        let Some(kind) = self.get(obj).map(|object| object.kind) else {
            return Ok(());
        };
        if self.current.contains(&Some(obj)) {
            if self.tcb_is_exited(obj) {
                // T32: task đã chết thì `current` chỉ là tàn dư — dọn nó rồi
                // cho phép huỷ, thay vì để object không bao giờ giải phóng.
                for slot in self.current.iter_mut() {
                    if *slot == Some(obj) {
                        *slot = None;
                    }
                }
            } else {
                return Err(KError::IllegalOperation);
            }
        }
        if self.root == Some(obj) {
            return Err(KError::IllegalOperation);
        }
        // Vòng 101 (K41): TCB bị Faulted khi destroy VSpace không được giữ
        // trong hàng chờ — path cũ chỉ remove_from_runq mà không gỡ
        // endpoint.send/recv hay notification.waiters, nên entry cũ không
        // bao giờ được dọn (Faulted bị make_ready/wake_error loại trừ).
        fn fault_tcb_off_vspace(this: &mut Kernel, t: ObjRef) {
            for i in 0..MAX_OBJECTS {
                let r = ObjRef {
                    index: i as u8,
                    gen: this.objs[i].gen,
                };
                if let Some(endpoint) = this.endpoint_mut(r) {
                    endpoint.send.remove(t);
                    endpoint.recv.remove(t);
                }
                if let Some(notification) = this.notification_mut(r) {
                    for slot in notification.waiters.iter_mut() {
                        if *slot == Some(t) {
                            *slot = None;
                        }
                    }
                }
            }
            this.remove_from_runq(t);
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
                // Vòng 199: gỡ từng mapping trước khi huỷ bảng trang để
                // `mapped_w`/`mapped_x` của frame được giảm. Không có bước này,
                // frame vẫn bị coi là "còn map" ⇒ `destroy` trả `Busy` mãi
                // (đo: còn 5 object kẹt sau khi đã sửa TCB).
                let mut guard = 0usize;
                while guard < MAX_MAPPINGS {
                    guard += 1;
                    let next = self
                        .vspace(obj)
                        .and_then(|space| space.mappings.iter().flatten().next().copied());
                    let Some(mapping) = next else {
                        break;
                    };
                    let _ = self.unmap_frame(obj, mapping.va);
                }
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
                    // chạy với vspace = None. Vòng 101 (K41): đồng thời gỡ
                    // khỏi mọi endpoint/notification queue — Faulted bị
                    // make_ready/wake_error loại trừ nên entry cũ không bao
                    // giờ được dọn nếu bỏ lại.
                    if self.tcb(r).map(|tcb| tcb.state) == Some(ThreadState::Faulted) {
                        fault_tcb_off_vspace(self, r);
                    }
                }
            }
            Kind::Irq(Irq { number, .. }) => {
                // (#2b) destroy IRQ phải dọn map để number tái dùng được và
                // raise sau này không trỏ vào object chết.
                self.irq_map_remove(number, obj);
            }
            Kind::Iommu(_) => {
                // Vòng 101 (K56): destroy domain phải gỡ mọi DMA mapping và
                // flush IOTLB — nếu không PA tái cấp trong khi IOTLB cũ còn
                // trỏ. `can_destroy` đã chặn revoke domain còn mapping sống;
                // đây là phòng thủ sâu: quét toàn bộ frame để trừ mapped_w
                // đã cộng lúc iommu_map(WRITE) của chính domain này.
                let mut removed: [(ObjRef, u16); MAX_OBJECTS] =
                    [(ObjRef::invalid(), 0); MAX_OBJECTS];
                let mut removed_count = 0usize;
                if let Some(domain) = self.iommu_mut(obj) {
                    for mapping in domain.mappings.iter().flatten() {
                        if mapping.rights.contains(Rights::WRITE) {
                            let mut found = false;
                            for entry in removed.iter_mut().take(removed_count) {
                                if entry.0 == mapping.frame {
                                    entry.1 = entry.1.saturating_add(1);
                                    found = true;
                                    break;
                                }
                            }
                            if !found && removed_count < removed.len() {
                                removed[removed_count] = (mapping.frame, 1);
                                removed_count += 1;
                            }
                        }
                    }
                    domain.mappings = [None; MAX_IOMMU_MAPPINGS];
                    domain.flush_iotlb();
                }
                for (frame_obj, count) in removed.iter().take(removed_count) {
                    let counts = self.frame(*frame_obj).map(|fr| (fr.mapped_w, fr.mapped_x));
                    if let Some((w, x)) = counts {
                        self.set_frame_mapping_counts(*frame_obj, w.saturating_sub(*count), x);
                    }
                }
            }
            Kind::BlockDevice(_) | Kind::Untyped(_) | Kind::Free => {}
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
        // (#2a/#3b) snapshot trạng thái TCB trước khi free — borrow kết thúc
        // ngay sau dòng này nên counter bên dưới không xung đột borrow.
        let was_tcb_sleeping = self.get(obj).is_some_and(|object| {
            matches!(object.kind, Kind::Tcb(tcb) if tcb.state == ThreadState::BlockedSleep)
        });
        let was_tcb_budget = self.get(obj).is_some_and(|object| {
            matches!(object.kind, Kind::Tcb(tcb) if tcb.state == ThreadState::BlockedBudget)
        });
        let was_tcb = self
            .get(obj)
            .is_some_and(|object| matches!(object.kind, Kind::Tcb(_)));
        let freed_slot = self.tcb(obj).map(|tcb| tcb.context_slot);
        if was_tcb_sleeping {
            self.sleepers_dec();
        } else if was_tcb_budget {
            self.budgeters_dec();
        }
        if was_tcb {
            self.live_tcb_count = self.live_tcb_count.saturating_sub(1);
        }
        if let Some(slot) = freed_slot {
            if (slot as usize) < MAX_TCBS {
                self.context_slot_used &= !(1u32 << slot);
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
        // (#3a) slot vừa free về hint để alloc sau dùng ngay O(1).
        self.next_free_hint = obj.index;
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
        let target_ref = self.cap_ref(target, target_cap.id);
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
            // Vòng 100 (K8, audit #11): kiểm tiên quyết destroy TRƯỚC khi gỡ
            // cap. Trước đây pha 1 gỡ sạch cap rồi pha 2 `destroy_object?` có
            // thể fail (vd. chính VSpace của task đang chạy) => cap mất,
            // object còn sống, không ai chạm tới được — trái hợp đồng "Err =
            // không làm gì".
            for object in objects.iter().take(object_count) {
                if !self.can_destroy(*object) {
                    return Err(KError::IllegalOperation);
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
        // Vòng 100 (K16): badge là ranh giới danh tính. Chỉ cho phép 0 -> badge
        // (đặt lần đầu). KHÔNG cho badge -> 0 hay badge -> badge: nếu cho xoá
        // về 0 thì task có thể "rửa" vé rồi mint lại badge tuỳ ý qua CapMint
        // (CapMint chỉ chặn mint từ nguồn đã có badge). Hiện syscall
        // SetCapBadge chưa được nối ra user space nên đây là lỗ tiềm ẩn, nhưng
        // siết ngay để lần nối sau không thành lỗ thật.
        if cap.badge != 0 {
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

    /// Có TCB nào `pick_on(hart)` THẬT SỰ nhặt được không.
    ///
    /// T33a (r288): phải kiểm cả `state == Ready`, không chỉ affinity. Bản cũ
    /// bỏ sót điều kiện này nên một TCB còn trong hàng chờ nhưng đã chuyển
    /// trạng thái (Running/Blocked/Exited) vẫn làm hàm trả `true`; hart phụ
    /// thức dậy, `pick_on` trả `None`, và runtime gọi `A::exit(1)` — hart phụ
    /// CHẾT, boot treo trước `PREEMPT OK` (đo ~1% trên x86_64).
    pub fn has_ready_on(&self, hart: usize) -> bool {
        if hart >= MAX_HARTS {
            return false;
        }
        for owner in 0..MAX_HARTS {
            for queued in self.runq[owner][..self.runq_len[owner]].iter().flatten() {
                if self.tcb(*queued).is_some_and(|tcb| {
                    tcb.affinity & (1u8 << hart) != 0 && tcb.state == ThreadState::Ready
                }) {
                    return true;
                }
            }
        }
        false
    }

    /// Có TCB `Ready` nào cho BẤT KỲ hart nào không (T33b, r288).
    ///
    /// Dùng ở đường `Exit`: khi hart hiện tại không còn task kế tiếp, chỉ được
    /// kết thúc cả máy nếu KHÔNG còn task nào ở bất kỳ hart nào — nếu không thì
    /// một worker thoát trên hart phụ sẽ giết cả hệ thống (triệu chứng: guest
    /// in `USER EXIT` ngay sau `TCB REAP OK`, boot dừng giữa đường).
    pub fn has_any_ready(&self) -> bool {
        for owner in 0..MAX_HARTS {
            for queued in self.runq[owner][..self.runq_len[owner]].iter().flatten() {
                if self
                    .tcb(*queued)
                    .is_some_and(|tcb| tcb.state == ThreadState::Ready)
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
            // r251 (K40): runqueue chỉ được chứa TCB `Ready`. Entry mang
            // trạng thái khác là rác của một lần suspend/park bỏ sót — chọn
            // nó sẽ chạy một task đang Blocked/Suspended. Bỏ qua, không chọn.
            if tcb.state != ThreadState::Ready {
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
                    if tcb.state != ThreadState::Ready {
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
        self.harts_seen |= 1u32 << hart;
        Some(reference)
    }

    /// D19 (r259): kiểm bất biến sched ở ranh giới trả về user.
    ///
    /// Trả `Ok(())` khi `current[hart]` tồn tại, đang `Running`, không còn
    /// queued, và không TCB nào trong ready queue đang `Running`.
    pub fn check_boundary(&self, hart: usize) -> Result<(), BoundaryError> {
        let reference = self
            .current
            .get(hart)
            .copied()
            .flatten()
            .ok_or(BoundaryError::NoCurrent)?;
        let tcb = self.tcb(reference).ok_or(BoundaryError::NoCurrent)?;
        if tcb.state != ThreadState::Running {
            return Err(BoundaryError::CurrentNotRunning);
        }
        if tcb.queued {
            return Err(BoundaryError::RunningAndQueued);
        }
        for owner in 0..MAX_HARTS {
            let len = self.runq_len[owner];
            for slot in self.runq[owner].iter().take(len) {
                let Some(entry) = slot else {
                    continue;
                };
                if let Some(queued) = self.tcb(*entry) {
                    if queued.state == ThreadState::Running {
                        return Err(BoundaryError::RunningAndQueued);
                    }
                }
            }
        }
        Ok(())
    }

    /// Số TCB đang `Running` trên toàn hệ thống (kiểm ở test-exit, T21).
    ///
    /// Ở trạng thái nhất quán, mỗi hart đang chạy user phải có đúng một TCB
    /// Running, nên đại lượng này bằng `current_harts()`. Lệch nghĩa là có
    /// TCB Running mà không hart nào sở hữu (hoặc ngược lại).
    pub fn running_tcbs(&self) -> usize {
        self.objs
            .iter()
            .filter(|object| {
                matches!(&object.kind, Kind::Tcb(tcb) if tcb.state == ThreadState::Running)
            })
            .count()
    }

    /// Số hart đang giữ `current` (kiểm ở test-exit, T21).
    pub fn current_harts(&self) -> usize {
        self.current.iter().filter(|slot| slot.is_some()).count()
    }

    /// Tổng hợp cho phép kiểm ở test-exit: `(running, harts_có_current,
    /// harts_đang_chạy_user)`.
    ///
    /// Bất biến toàn cục đúng đắn là `running == đang_chạy_user` và
    /// `running <= harts_có_current`: mỗi TCB Running phải thuộc về đúng một
    /// hart, còn hart idle vẫn giữ `current` (TCB đang park) nên
    /// `harts_có_current` có thể LỚN hơn — điều đó không phải vi phạm.
    pub fn boundary_summary(&self) -> (usize, usize, usize) {
        let running = self.running_tcbs();
        let mut harts = 0usize;
        let mut running_harts = 0usize;
        for hart in 0..MAX_HARTS {
            let Some(reference) = self.current[hart] else {
                continue;
            };
            harts += 1;
            if self
                .tcb(reference)
                .is_some_and(|tcb| tcb.state == ThreadState::Running)
            {
                running_harts += 1;
            }
        }
        (running, harts, running_harts)
    }

    /// D18/D19: làm hỏng bất biến một cách có chủ đích để chứng minh
    /// `check_boundary` bắt được (test host, hoặc kernel biến thể
    /// `check-boundary-mutation`; kernel phát hành không biên dịch hàm này).
    #[cfg(any(test, feature = "check-boundary-mutation"))]
    pub fn poison_running_queued(&mut self, hart: usize) -> bool {
        let Some(reference) = self.current_on(hart) else {
            return false;
        };
        let Some(tcb) = self.tcb_mut(reference) else {
            return false;
        };
        tcb.queued = true;
        true
    }

    /// D18/D19: khôi phục trạng thái sau khi đã tiêm lỗi, để biến thể
    /// mutation chỉ in ĐÚNG MỘT `BOUNDARY BAD` thay vì mỗi ranh giới.
    #[cfg(any(test, feature = "check-boundary-mutation"))]
    pub fn clear_running_queued(&mut self, hart: usize) -> bool {
        let Some(reference) = self.current_on(hart) else {
            return false;
        };
        let Some(tcb) = self.tcb_mut(reference) else {
            return false;
        };
        tcb.queued = false;
        true
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
        // Vòng 101 (K57): không ghi vào frame đang có mapping EXEC — đường
        // syscall WRITE mà ghi thẳng PA vào trang RX thì lách W^X của
        // map_frame (giữ đồng nhất: map cấm W+X, copy cũng cấm X).
        if frame.mapped_x != 0 {
            return Err(KError::NoRights);
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
        // Vòng 100 (K13): fault message KHÔNG được gỡ khỏi hàng send — đó là
        // thiết kế: handler khởi động sau vẫn phải nhận được fault đã xếp hàng
        // (test `fault_endpoint_queues_when_handler_is_not_waiting` khẳng định
        // điều này). An toàn vì `send` ở đây là non-call (call=false), nên khi
        // receiver lấy message, `recv` không đổi state của sender — TCB vẫn
        // BlockedFault. Chỉ cần không cho pending cũ làm nó "sống lại" qua
        // đường call, và điều đó không xảy ra với fault.
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

    /// Voluntary exit (mốc 1): task hiện tại rời scheduler, object giữ lại
    /// làm zombie để parent reap qua TcbDestroy. Gỡ khỏi mọi hàng chờ (tái
    /// dùng logic detach nhưng không destroy object). Trả task kế tiếp.
    /// Server đang ôm caller của task exit thì caller được wake lỗi (detach
    /// đã xử lý qua wake_error); client đang BlockedReply vào task exit thì
    /// reply sau trả NoCaller sạch (restart-K36 đã dọn caller ma).
    pub fn exit_current_on(&mut self, hart: usize) -> Option<ObjRef> {
        let slot = self.current.get_mut(hart)?;
        let old = slot.take()?;
        self.remove_from_runq(old);
        // Gỡ khỏi endpoint/send-recv + notification waiters như detach, và
        // trả caller đang trỏ tới task exit để server không treo caller ma.
        for i in 0..MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if let Some(endpoint) = self.endpoint_mut(r) {
                endpoint.send.remove(old);
                endpoint.recv.remove(old);
            }
            if let Some(notification) = self.notification_mut(r) {
                for waiter in notification.waiters.iter_mut() {
                    if *waiter == Some(old) {
                        *waiter = None;
                    }
                }
            }
            if let Some(table) = self.tcb_mut(r) {
                if table.caller.is_some_and(|(client, _)| client == old) {
                    table.caller = None;
                }
            }
        }
        // Client đang BlockedReply chờ task exit: đánh thức lỗi ngay để
        // không treo vô hạn (mốc 1: hết kẹt BlockedReply). Bao gồm cả
        // parent TcbWait qua target.caller. Xóa caller sau wake để zombie
        // không giữ ref stale (restart zombie kiểm tra caller).
        if let Some((client, _)) = self.tcb_mut(old).and_then(|table| table.caller.take()) {
            self.wake_error(client, KError::ObjectDestroyed);
        }
        // (#2a) exit rời sleep/budget cũ (nếu có) — zombie Exited không bao
        // giờ được tick() đánh thức lại (test round103 khẳng định). leave_*
        // check state cũ nên phải snapshot TRƯỚC khi đổi state sang Exited.
        let was_sleeping = self
            .tcb(old)
            .is_some_and(|table| table.state == ThreadState::BlockedSleep);
        let was_budget = self
            .tcb(old)
            .is_some_and(|table| table.state == ThreadState::BlockedBudget);
        if let Some(table) = self.tcb_mut(old) {
            table.pending = None;
            table.resume = None;
            table.resume_error = None;
            table.recv_slot = 0;
            table.caller = None;
            table.state = ThreadState::Exited;
            table.queued = false;
        }
        if was_sleeping {
            self.sleepers_dec();
        } else if was_budget {
            self.budgeters_dec();
        }
        // wake_at luôn xoá (zombie không giữ deadline cũ).
        if let Some(table) = self.tcb_mut(old) {
            table.wake_at = None;
        }
        self.pick_on(hart)
    }

    pub fn tick(&mut self, hart: usize) -> usize {
        if hart < MAX_HARTS {
            self.harts_seen |= 1 << hart;
        }
        if let Some(counter) = self.ticks.get_mut(hart) {
            *counter = counter.wrapping_add(1);
            self.stats.ticks = *counter;
            // (#7) Every hart advances the shared clock with a monotonic
            // max-update instead of only the BSP storing it.  A BSP stalled
            // behind the BKL (e.g. in a 5M-spin shootdown wait) no longer
            // freezes Sleep/GetTime/budget deadlines on all other harts;
            // per-hart `ticks[hart]` remain the source for stats/preemption.
            self.system_ticks
                .fetch_max(*counter, core::sync::atomic::Ordering::AcqRel);
        }
        // (#2a) fast-path: không sleeper/budgeter thì bỏ qua cả hai lần quét
        // 168 object, chỉ còn charge_current_budget O(1). Vẫn gọi charge vì
        // nó O(1) (đi theo current[hart], không quét).
        let now = self.fast_time();
        let slept = if self.sleepers_active == 0 {
            0
        } else {
            self.wake_sleepers(now)
        };
        let budgeted = if self.budgeters_active == 0 {
            0
        } else {
            self.wake_budgeters(now)
        };
        let _ = self.charge_current_budget(hart, now);
        slept + budgeted
    }

    /// Move every deadline-expired `BlockedSleep` TCB to the ready queue.
    /// Returns the number of tasks woken by this call.
    fn wake_sleepers(&mut self, now: u64) -> usize {
        // (#2a) self-heal: test gán state trực tiếp không qua helper có thể
        // làm counter lệch. Nếu quét mà thấy số sleeper thật là 0 thì đồng
        // bộ counter về 0 để tick() fast-path trở lại.
        let mut woke = 0usize;
        let mut seen = 0u32;
        for index in 0..MAX_OBJECTS {
            let reference = ObjRef {
                index: index as u8,
                gen: self.objs[index].gen,
            };
            let expired = match self.objs[index].kind {
                Kind::Tcb(tcb) => {
                    if tcb.state == ThreadState::BlockedSleep {
                        seen = seen.saturating_add(1);
                        tcb.wake_at.is_some_and(|deadline| deadline <= now)
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if !expired {
                continue;
            }
            self.leave_sleep(reference);
            if let Some(tcb) = self.tcb_mut(reference) {
                tcb.state = ThreadState::Ready;
                tcb.resume = Some(Message::empty(0));
                tcb.resume_error = None;
            }
            self.enqueue(reference);
            woke += 1;
        }
        if seen == 0 {
            self.sleepers_active = 0;
        }
        woke
    }

    /// Lock-free system time source.  Every hart advances it with a
    /// monotonic max-update on each timer tick (#7), so all harts may read
    /// it without taking the BKL and no single stalled hart freezes time.
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
            // (#2a) Sleep park thành công — tăng counter. TCB đã ngủ mà Sleep
            // lại (re-park đổi deadline) thì không tăng kép: chỉ tăng khi
            // state trước đó không phải BlockedSleep.
            let already_sleeping = self
                .tcb(tcb)
                .is_some_and(|table| table.state == ThreadState::BlockedSleep);
            let Some(table) = self.tcb_mut(tcb) else {
                return UserAction::Return {
                    value: 0,
                    error: KError::InvalidCap,
                };
            };
            table.state = ThreadState::BlockedSleep;
            table.wake_at = Some(deadline);
            if !already_sleeping {
                self.sleepers_inc();
            }
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
                // Vòng 101 (K73): vé Frame phải chứa quyền DMA yêu cầu —
                // trước đây R-only + IoMap(WRITE) vẫn lọt, tạo mapping
                // DMA-WRITE lên frame chỉ-đọc (FrameMap đã check, IoMap
                // thì chưa).
                if !frame.rights.contains(rights) {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NoRights,
                    };
                }
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
                // Vòng 101 (K74): Iommu-WRITE đơn thuần không được gỡ DMA của
                // người khác — caller phải xuất trình vé Frame cho đúng PA
                // đang map tại IOVA (gương FrameUnmap đòi VSpace-WRITE + vé
                // Frame + khớp pa). Trước đây task B có domain-WRITE gỡ được
                // mapping của task A (DoS/stale-DMA).
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
                let domain_object = match self.iommu(domain.obj) {
                    Some(domain_object) => domain_object,
                    None => {
                        return UserAction::Return {
                            value: 0,
                            error: KError::WrongType,
                        };
                    }
                };
                let owns_mapping = domain_object.mappings.iter().flatten().any(|m| {
                    m.iova == a1 && m.frame == frame.obj && m.pa == frame_object.pa as u64
                });
                if !owns_mapping {
                    return UserAction::Return {
                        value: 0,
                        error: KError::NoRights,
                    };
                }
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
                // (#2b) check trùng O(1) qua map thay vì quét 168 object.
                let duplicate =
                    (number as usize) < 64 && self.irq_by_number[number as usize].is_some();
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
                    cnode: self.table_of(tcb),
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
                // Mốc 1: không destroy task đang là rendezvous-wait của ai
                // (có waiter TcbWait đang chờ) — buộc waiter hoàn tất trước,
                // nếu không waiter treo mà target đã biến mất.
                if self
                    .tcb(target.obj)
                    .and_then(|table| table.caller)
                    .is_some()
                {
                    return UserAction::Return {
                        value: 0,
                        error: KError::WouldBlock,
                    };
                }
                match self.destroy_object(target.obj) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::MemInfo as u64 => {
                // Vòng 154: báo dung lượng **arena lớn nhất** (Untyped gốc của
                // pool) — đây là con số người dùng cần: `value = used | size<<32`.
                // Bản đầu cộng dồn mọi Untyped nên cho ra "free > total" và
                // "604/604" vô nghĩa (đã ghi lại trong log).
                let mut best: Option<(usize, usize)> = None;
                let mut index = 0usize;
                while index < self.objs.len() {
                    let reference = ObjRef {
                        index: index as u8,
                        gen: self.objs[index].gen,
                    };
                    if let Some(untyped) = self.untyped(reference) {
                        if best.map_or(true, |(size, _)| untyped.size > size) {
                            best = Some((untyped.size, untyped.bump));
                        }
                    }
                    index += 1;
                }
                let (size, used) = best.unwrap_or((0, 0));
                let value = (used as u64 & 0xffff_ffff) | ((size as u64 & 0xffff_ffff) << 32);
                UserAction::Return {
                    value,
                    error: KError::Ok,
                }
            }
            n if n == SyscallId::TcbInfo as u64 => {
                // Vòng 152/153: liệt kê TCB đang sống (a0 = chỉ số thứ tự).
                // value = slot | prio<<8 | state<<16 | 4 ký tự tên << 24 | aff<<56.
                let wanted = a0 as usize;
                let mut seen = 0usize;
                let mut index = 0usize;
                while index < self.objs.len() {
                    let reference = ObjRef {
                        index: index as u8,
                        gen: self.objs[index].gen,
                    };
                    if let Some(table) = self.tcb(reference) {
                        if seen == wanted {
                            let state = table.state as u64;
                            let name = table.name.as_bytes();
                            let mut tag = 0u64;
                            let mut k = 0;
                            while k < 4 && k < name.len() {
                                tag |= (name[k] as u64) << (k * 8);
                                k += 1;
                            }
                            let value = (index as u64)
                                | ((table.priority as u64) << 8)
                                | (state << 16)
                                | (tag << 24)
                                | ((table.affinity as u64) << 56);
                            return UserAction::Return {
                                value,
                                error: KError::Ok,
                            };
                        }
                        seen += 1;
                    }
                    index += 1;
                }
                UserAction::Return {
                    value: 0,
                    error: KError::NotFound,
                }
            }
            n if n == SyscallId::ModuleRelease as u64 => UserAction::Return {
                value: self.release_staging() as u64,
                error: KError::Ok,
            },
            n if n == SyscallId::TimerInfo as u64 => UserAction::Return {
                value: self.timer_hz as u64,
                error: KError::Ok,
            },
            n if n == SyscallId::TcbWait as u64 => {
                let target = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::READ,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.wait_tcb(tcb, target.obj) {
                    Ok(true) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
                    Ok(false) => UserAction::Blocked,
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
                    // Vòng 101 (K75): đổi cả không gian địa chỉ phải đòi
                    // WRITE như FrameMap/PageTableMap — READ là quá yếu.
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                let fault_slot = slot_or_invalid!(a5);
                let fault = if fault_slot != 0 {
                    // Vòng 101 (K75): fault_ep trỏ vào slot của NẠN NHÂN
                    // (delivery đọc qua cap_at victim lúc fault). Path cũ
                    // đọc thẳng slot nạn nhân mà không hỏi caller có quyền
                    // gì — attacker có Tcb-WRITE nạn nhân là cắm được kênh
                    // fault từ slot nạn nhân (kể cả vé chết). Giờ yêu cầu
                    // kép: (1) slot nạn nhân phải có vé Endpoint còn sống,
                    // (2) CALLER cũng phải giữ một vé tới cùng endpoint
                    // (chứng minh caller được phép định tuyến fault).
                    let Some(victim_cap) = self.cap_at(SlotAddr {
                        tcb: cap.obj,
                        slot: fault_slot,
                    }) else {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidCap,
                        };
                    };
                    if victim_cap.ty != ObjType::Endpoint || self.get(victim_cap.obj).is_none() {
                        return UserAction::Return {
                            value: 0,
                            error: KError::InvalidCap,
                        };
                    }
                    let _caller_holds_endpoint = self.cspace(tcb).is_some_and(|slots| {
                        slots.iter().flatten().any(|c| {
                            c.ty == ObjType::Endpoint
                                && c.obj == victim_cap.obj
                                && self.get(c.obj).is_some()
                        })
                    });
                    if !_caller_holds_endpoint {
                        return UserAction::Return {
                            value: 0,
                            error: KError::NoRights,
                        };
                    }
                    Some(CapRef {
                        addr: SlotAddr {
                            tcb: cap.obj,
                            slot: fault_slot,
                        },
                        id: victim_cap.id,
                        cnode: self.table_of(cap.obj),
                    })
                } else {
                    None
                };
                match self.configure_tcb(cap.obj, a1, a2, vspace.obj, a4 as u8) {
                    Ok(()) => {
                        if let Err(error) = self.set_fault_endpoint(cap.obj, fault) {
                            return UserAction::Return { value: 0, error };
                        }
                        // D15: configure vừa xoá tham số, nên bước này luôn
                        // dựng context với argc=argv=0; TcbSetArgs bù sau.
                        let (argc, argv) = self
                            .tcb(cap.obj)
                            .map(|t| (t.argc, t.argv))
                            .unwrap_or((0, 0));
                        UserAction::ConfigureTcb {
                            tcb: cap.obj,
                            entry: a1,
                            stack: a2,
                            argc,
                            argv,
                        }
                    }
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            n if n == SyscallId::TcbSetArgs as u64 => {
                // D15 (r246): chỉ TCB đã configure nhưng CHƯA chạy mới nhận
                // tham số; nếu không sẽ ghi đè context của task đang sống.
                let cap = match self.lookup_cap(
                    tcb,
                    slot_or_invalid!(a0),
                    Some(ObjType::Tcb),
                    Rights::WRITE,
                ) {
                    Ok(cap) => cap,
                    Err(error) => return UserAction::Return { value: 0, error },
                };
                match self.set_tcb_args(cap.obj, a1, a2) {
                    Ok(()) => UserAction::SetTcbArgs {
                        tcb: cap.obj,
                        argc: a1,
                        argv: a2,
                    },
                    Err(error) => UserAction::Return { value: 0, error },
                }
            }
            // T27 (r264): uỷ quyền cap sang TCB khác (đường exec cần để con
            // chương trình thừa hưởng endpoint VFS). Caller phải có WRITE trên
            // TCB đích; quyền chuyển bị thu hẹp (không bao giờ có GRANT).
            n if n == SyscallId::TcbGrantCap as u64 => {
                let target_slot = slot_or_invalid!(a0);
                let src_slot = slot_or_invalid!(a1);
                let dst_slot = slot_or_invalid!(a2);
                let target =
                    match self.lookup_cap(tcb, target_slot, Some(ObjType::Tcb), Rights::WRITE) {
                        Ok(cap) => cap,
                        Err(error) => return UserAction::Return { value: 0, error },
                    };
                let mask = Rights((a3 as u8) & Rights::ALL.0);
                match self.grant_cap_to_tcb(tcb, target.obj, src_slot, dst_slot, mask) {
                    Ok(()) => UserAction::Return {
                        value: 0,
                        error: KError::Ok,
                    },
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
                    Ok((entry, stack, argc, argv)) => UserAction::RestartTcb {
                        tcb: cap.obj,
                        entry,
                        stack,
                        argc,
                        argv,
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
                // kernel copy từ địa chỉ chưa map và chết ở EL1. Vòng 101
                // (K72): phát `len` đã kẹp thay vì a3 gốc — runtime clamp
                // lại nên không OOB, nhưng event mang len chưa validate
                // gây lệch hợp đồng (giữ đồng nhất với Read/Write/
                // DebugWrite đều phát len đã kẹp).
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
                    len: len as u64,
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
        // Vòng 101 (K59): create_frame từ chối overlap ngay lúc tạo, nên
        // frame trùng không bao giờ tồn tại để invariant phải bắt.
        let mut alias = Kernel::new();
        assert!(alias.create_frame(0x1000, 4096).is_some());
        assert!(alias.create_frame(0x1000, 4096).is_none());
        assert!(alias.create_frame(0x1800, 4096).is_none());
        assert!(alias.create_frame(0x2000, 4096).is_some());
        assert!(alias.check_invariants().is_ok());

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
    fn get_time_is_monotonic_and_driven_by_bsp_ticks() {
        // Vòng 220: `uptime`/`time` đọc GetTime. Phải TĂNG đơn điệu, không nhảy
        // khi không có tick, và không bị đẩy bởi tick của hart phụ (task migrate
        // giữa các hart mà thời gian nhảy là lỗi rất khó thấy).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let t0 = match kernel.user_syscall(root, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0) {
            UserAction::Return { value, error } => {
                assert_eq!(error, KError::Ok);
                value
            }
            _ => panic!("GetTime phải trả về ngay"),
        };
        // Không tick ⇒ thời gian KHÔNG được đổi.
        let t0b = match kernel.user_syscall(root, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0) {
            UserAction::Return { value, .. } => value,
            _ => panic!("GetTime phải trả về ngay"),
        };
        assert_eq!(t0, t0b, "không tick thì thời gian không đổi");

        for _ in 0..5 {
            kernel.tick(0);
        }
        let t1 = match kernel.user_syscall(root, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0) {
            UserAction::Return { value, .. } => value,
            _ => panic!("GetTime phải trả về ngay"),
        };
        assert_eq!(t1, t0 + 5, "5 tick của BSP phải làm thời gian tăng đúng 5");

        // Tick của hart phụ KHÔNG được đẩy đồng hồ hệ thống.
        kernel.tick(1);
        let t2 = match kernel.user_syscall(root, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0) {
            UserAction::Return { value, .. } => value,
            _ => panic!("GetTime phải trả về ngay"),
        };
        assert_eq!(t2, t1, "tick hart phụ không được làm thời gian nhảy");
        assert!(t2 >= t1, "thời gian phải đơn điệu không giảm");
    }

    #[test]
    fn tcb_info_reports_slot_state_priority_and_name() {
        // Vòng 219: `ps` in `slot/prio/state/aff/name` từ TcbInfo. Nếu syscall
        // báo sai state, người dùng sẽ tưởng hệ thống treo khi nó chỉ đang chờ.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let worker = kernel.create_tcb("wrkr", 7).unwrap();
        kernel.tcb_mut(worker).unwrap().affinity = 2;
        // Index 0 là TCB của CHÍNH root (Kernel::new tạo nó trước) ⇒ worker là 1.
        let UserAction::Return { value, error } =
            kernel.user_syscall(root, SyscallId::TcbInfo as u64, 1, 0, 0, 0, 0, 0)
        else {
            panic!("TcbInfo phải trả về ngay");
        };
        assert_eq!(error, KError::Ok);
        let index = (value & 0xff) as u8;
        assert_eq!(index, worker.index, "slot phải đúng TCB đầu tiên");
        assert_eq!(((value >> 8) & 0xff) as u8, 7, "priority");
        let state_ready = ((value >> 16) & 0xff) as u8;
        let tag = ((value >> 24) & 0xffff_ffff) as u32;
        assert_eq!(tag, u32::from_le_bytes(*b"wrkr"), "4 ký tự tên phải khớp");
        assert_eq!(((value >> 56) & 0xff) as u8, 2, "affinity");

        // Chuyển sang BlockedNotification ⇒ state phải đổi theo.
        let notification = kernel.create_notification().unwrap();
        kernel.wait_as(worker, notification).unwrap();
        let UserAction::Return { value: blocked, .. } =
            kernel.user_syscall(root, SyscallId::TcbInfo as u64, 1, 0, 0, 0, 0, 0)
        else {
            panic!("TcbInfo phải trả về ngay");
        };
        let state_blocked = ((blocked >> 16) & 0xff) as u8;
        assert_ne!(
            state_ready, state_blocked,
            "state trong TcbInfo phải thay đổi khi TCB bị block"
        );
        // Index vượt quá số TCB ⇒ NotFound (ps dừng vòng lặp đúng cách).
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbInfo as u64, 99, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::NotFound
            }
        );
    }

    #[test]
    fn mem_info_reflects_arena_usage_and_grows_with_allocation() {
        // Vòng 218: `free` / `df` của shell đọc MemInfo ⇒ nếu syscall báo sai thì
        // người dùng bị "nói dối" về bộ nhớ. Chưa có test nào cho syscall này.
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x7c0_0000, 16);
        let first = kernel.user_syscall(root, SyscallId::MemInfo as u64, 0, 0, 0, 0, 0, 0);
        let UserAction::Return {
            value: before,
            error,
        } = first
        else {
            panic!("MemInfo phải trả về ngay");
        };
        assert_eq!(error, KError::Ok);
        let used_before = (before & 0xffff_ffff) as usize;
        let size = (before >> 32) as usize;
        assert!(size > 0, "arena phải có dung lượng");
        assert!(used_before <= size, "used không được vượt size");

        // Cấp thêm object từ untyped ⇒ `used` phải TĂNG.
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        assert!(kernel.frame(frame).is_some());
        let UserAction::Return { value: after, .. } =
            kernel.user_syscall(root, SyscallId::MemInfo as u64, 0, 0, 0, 0, 0, 0)
        else {
            panic!("MemInfo phải trả về ngay");
        };
        assert!(
            ((after & 0xffff_ffff) as usize) > used_before,
            "cấp frame phải làm used tăng"
        );
        assert_eq!((after >> 32) as usize, size, "tổng dung lượng không đổi");
    }

    #[test]
    fn timer_info_reports_configured_rate_and_is_stable() {
        // Vòng 218: `uptime` chia cho TimerInfo ⇒ nếu trả 0 hoặc đổi giữa hai
        // lần gọi thì thời gian hiển thị sẽ sai/nhảy.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        kernel.set_timer_hz(128);
        let first = kernel.user_syscall(root, SyscallId::TimerInfo as u64, 0, 0, 0, 0, 0, 0);
        let second = kernel.user_syscall(root, SyscallId::TimerInfo as u64, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            first,
            UserAction::Return {
                value: 128,
                error: KError::Ok
            }
        );
        assert_eq!(first, second, "timer_hz phải ổn định giữa hai lần gọi");
    }

    #[test]
    fn frame_map_distinguishes_norights_from_wx_rejection() {
        // Vòng 211: khoá bài học vòng 210 — khi phép thử W^X trên guest trả
        // `NoRights` (3) thì nó **chưa hề kiểm W^X**; kernel chặn ở bước quyền
        // cap trước. Test này khẳng định THỨ TỰ đó:
        //   1. cap chỉ READ + xin WRITE ⇒ `NoRights`;
        //   2. cap đủ WRITE + xin WRITE|EXEC ⇒ `InvalidArg` (W^X).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let vspace = kernel.create_vspace(0x1000, 0x4000_a000, 4096).unwrap();
        kernel
            .install_cap(root, 2, Cap::new(vspace, ObjType::VSpace, Rights::WRITE))
            .unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        kernel
            .install_cap(root, 3, Cap::new(frame, ObjType::Frame, Rights::READ))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(
                root,
                SyscallId::FrameMap as u64,
                2,
                3,
                0,
                Rights::WRITE.0 as u64,
                0,
                0
            ),
            UserAction::Return {
                value: 0,
                error: KError::NoRights
            },
            "cap chỉ READ mà xin WRITE phải là NoRights (chưa tới W^X)"
        );
        // Nâng quyền cap: giờ xin W|X mới tới được bước kiểm W^X.
        kernel.delete_cap(root, 3).unwrap();
        kernel
            .install_cap(root, 3, Cap::new(frame, ObjType::Frame, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel
                .lookup_cap(root, 3, Some(ObjType::Frame), Rights::NONE)
                .unwrap()
                .rights,
            Rights::ALL,
            "cap frame phải có ALL sau khi cài lại"
        );
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
            },
            // QUAN TRỌNG: W^X cũng trả **NoRights**, GIỐNG hệt trường hợp thiếu
            // quyền cap ở trên ⇒ KHÔNG thể phân biệt hai nguyên nhân bằng mã lỗi.
            // Đây chính là cái bẫy gặp ở vòng 210 khi kiểm W^X trên guest.
            "W|X cũng trả NoRights — cùng mã với thiếu quyền cap (bẫy chẩn đoán)"
        );
        assert!(kernel.check_invariants().is_ok());
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
    fn map_rejects_wx_overlap_and_duplicate_va() {
        // Vòng 174 (bộ nhớ): một mapping không được vừa ghi vừa thi hành
        // (W^X), và không được map hai frame vào cùng một VA.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_vspace_with_page_size(0x1000, 0x4000_0000, 0x4000, 4096)
            .unwrap();
        let frame_a = kernel.create_frame(0x9000, 4096).unwrap();
        let frame_b = kernel.create_frame(0xa000, 4096).unwrap();
        assert_eq!(
            kernel.map_frame(
                space,
                frame_a,
                0x4000_0000,
                Rights::READ | Rights::WRITE | Rights::EXEC
            ),
            Err(KError::NoRights),
            "W^X: RWX trong một mapping phải bị từ chối"
        );
        assert!(kernel
            .map_frame(space, frame_a, 0x4000_0000, Rights::READ | Rights::WRITE)
            .is_ok());
        assert!(
            kernel
                .map_frame(space, frame_b, 0x4000_0000, Rights::READ | Rights::WRITE)
                .is_err(),
            "map đè lên VA đã có phải bị từ chối"
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn unmap_then_remap_at_the_same_va_succeeds() {
        // Vòng 174: sau unmap, VA đó phải dùng lại được (không rò rỉ bảng
        // trang khi map lại cùng vị trí).
        let mut kernel = Kernel::new();
        let space = kernel
            .create_vspace_with_page_size(0x1000, 0x4000_0000, 0x4000, 4096)
            .unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        let first = kernel
            .map_frame(space, frame, 0x4000_1000, Rights::READ)
            .unwrap();
        kernel.unmap_frame(space, first.va).unwrap();
        let again = kernel
            .map_frame(space, frame, first.va, Rights::READ)
            .unwrap();
        assert_eq!(again.va, first.va);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn copy_frame_bytes_rejects_out_of_range_windows() {
        // Vòng 174: ghi vào frame không được vượt biên — đây là chỗ dễ tràn
        // nếu ai đó tính offset bằng số học không kiểm tra.
        let mut kernel = Kernel::new();
        // copy_frame_bytes cần page-table ops để ghi thật.
        kernel.set_page_table_ops(&FLUSH_OPS);
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        assert_eq!(
            kernel.copy_frame_bytes(frame, 4092, &[0u8; 8]),
            Err(KError::InvalidArg)
        );
        assert!(kernel.copy_frame_bytes(frame, 4096 - 8, &[0u8; 8]).is_ok());
        assert_eq!(
            kernel.copy_frame_bytes(frame, usize::MAX, &[0u8; 2]),
            Err(KError::InvalidArg)
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
    fn make_ready_ignores_running_and_suspended_tasks() {
        // Vòng 99 (soi chay K4): make_ready từng set Ready mù quáng nên có thể
        // đặt Ready cho task đang Running (chạy hai lần) hoặc tự gỡ treo task
        // Suspended khi có send tới.
        let mut kernel = Kernel::new();
        let running = kernel.create_tcb("run", 5).unwrap();
        kernel.tcb_mut(running).unwrap().state = ThreadState::Running;
        kernel.make_ready(running);
        assert_eq!(kernel.tcb(running).unwrap().state, ThreadState::Running);
        assert!(!kernel.tcb(running).unwrap().queued);

        let suspended = kernel.create_tcb("susp", 5).unwrap();
        kernel.tcb_mut(suspended).unwrap().state = ThreadState::Suspended;
        kernel.make_ready(suspended);
        assert_eq!(kernel.tcb(suspended).unwrap().state, ThreadState::Suspended);
        assert!(!kernel.tcb(suspended).unwrap().queued);
    }

    #[test]
    fn round70_make_ready_wakes_blocked_budget() {
        // Mốc 2 (vòng 70): make_ready phải đánh thức task BlockedBudget
        // (hết ngân sách) như mọi Blocked* khác — cùng họ lỗi vòng 65.
        // Nếu không send tới task hết budget không đánh thức, message kẹt
        // trong hàng dù receiver đã có việc.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("budgeted", 5).unwrap();
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::BlockedBudget;
        kernel.make_ready(tcb);
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert!(table.queued, "BlockedBudget phải vào runqueue khi wake");
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round71_budget_exhaust_removes_from_runqueue() {
        // Mốc 2 (vòng 71): task hết ngân sách sched_context phải rời
        // runqueue khi chuyển BlockedBudget — nếu không nó vừa Blocked
        // vừa queued, scheduler có thể pick task không còn budget (chạy
        // lố) hoặc kẹt thành viên kép khi wake.
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x850_0000, 8);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        let context = kernel.retype(untyped, ObjType::SchedContext).unwrap();
        kernel.configure_sched_context(context, 1, 100).unwrap();
        let tcb = kernel.create_tcb("spender", 5).unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        kernel
            .configure_tcb(tcb, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.bind_sched_context(tcb, context).unwrap();
        kernel.resume_tcb(tcb).unwrap();
        // Task đang queued sau resume — xác nhận có trong runq.
        assert!(kernel.tcb(tcb).unwrap().queued);
        // Lên sóng: pick rồi charge 1 tick (budget 1 -> cạn ngay).
        assert_eq!(kernel.pick_on(0), Some(tcb));
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(tcb);
        let _ = kernel.tick(0);
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::BlockedBudget);
        assert!(
            !table.queued,
            "hết budget phải rời runqueue (không thành viên kép)"
        );
        // Wake sau đó phải enqueue sạch 1 lần, pick được đúng 1 lần.
        kernel.make_ready(tcb);
        assert!(kernel.tcb(tcb).unwrap().queued);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert_eq!(kernel.pick_on(0), None);
        // Chu kỳ mới: hết budget rồi tick qua deadline -> wake_budgeters
        // đánh thức (Ready + queued), task chạy lại được.
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(tcb);
        // Budget 1 đã cạn ở tick trước; tick tiếp qua deadline period.
        for _ in 0..150 {
            let _ = kernel.tick(0);
        }
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert!(table.queued);
        // 2 pick thành công trước đó (lần đầu + sau make_ready).
        assert_eq!(kernel.scheduler_stats(0).0 + kernel.scheduler_stats(0).1, 2);
        // Lên sóng lại chu kỳ mới rồi check invariants (current phải
        // Running, pick gỡ khỏi runq).
        assert_eq!(kernel.pick_on(0), Some(tcb));
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(tcb);
        assert!(kernel.check_invariants().is_ok());
    }

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
    fn round65_wake_error_queues_blocked_budget() {
        // Mốc 1 (vòng 65): task BlockedBudget (hết ngân sách sched_context)
        // bị wake_error (vd. server nó chờ bị destroy) phải Ready + queued
        // như mọi Blocked* khác — trước đây was_blocked bỏ sót BlockedBudget
        // nên task chỉ nhận resume_error mà treo mãi không ai pick.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("budgeted", 5).unwrap();
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::BlockedBudget;
        kernel.wake_error(tcb, KError::ObjectDestroyed);
        let table = kernel.tcb(tcb).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert!(table.queued, "BlockedBudget phải vào runqueue khi wake");
        assert_eq!(table.resume_error, Some(KError::ObjectDestroyed));
        // Pick được và invariants giữ.
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn wait_on_silent_notification_blocks_until_signalled() {
        // Vòng 183 (lõi IPC): `wait` trên notification chưa có tín hiệu phải
        // BLOCK task, và `signal` phải đánh thức đúng nó với bit đã báo.
        let mut kernel = Kernel::new();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(waiter, ntfn), Ok(None));
        assert_eq!(
            kernel.tcb(waiter).unwrap().state,
            ThreadState::BlockedNotification
        );
        kernel.signal(ntfn, 0b100).unwrap();
        let table = kernel.tcb(waiter).unwrap();
        assert_eq!(
            table.state,
            ThreadState::Ready,
            "signal phải đánh thức waiter"
        );
        let resume = table.resume.expect("phải có message thông báo");
        assert_eq!(resume.label, LABEL_NOTIFICATION);
        assert_eq!(resume.words()[0], 0b100, "bit đã báo phải tới waiter");
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn signal_before_wait_is_not_lost() {
        // Vòng 183: tín hiệu báo TRƯỚC khi ai chờ KHÔNG được mất — bit nằm lại
        // trong word và `wait` sau đó nhận ngay, không block.
        let mut kernel = Kernel::new();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        kernel.signal(ntfn, 0b10).unwrap();
        assert_eq!(kernel.wait_as(waiter, ntfn), Ok(Some(0b10)));
        assert_eq!(
            kernel.tcb(waiter).unwrap().state,
            ThreadState::Inactive,
            "wait có sẵn bit thì KHÔNG được block"
        );
        assert_eq!(kernel.poll(ntfn), Ok(0), "bit đã tiêu thụ");
    }

    #[test]
    fn signal_wakes_every_live_waiter() {
        // Vòng 183 (audit #4 P0): một `signal` phải đánh thức TẤT CẢ waiter còn
        // sống, không chỉ waiter đầu tiên.
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("a", 5).unwrap();
        let b = kernel.create_tcb("b", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(a, ntfn), Ok(None));
        assert_eq!(kernel.wait_as(b, ntfn), Ok(None));
        kernel.signal(ntfn, 0b1).unwrap();
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::Ready);
        assert_eq!(kernel.tcb(b).unwrap().state, ThreadState::Ready);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn cap_transfer_requires_grant_and_strips_the_right() {
        // Vòng 173 (IPC + bảo mật): chuyển capability qua message đòi quyền
        // GRANT ở phía gửi; người nhận KHÔNG được thừa hưởng GRANT (nếu không
        // thì một cap đã cho đi lại có thể cho tiếp — rửa quyền qua IPC).
        let mut kernel = Kernel::new();
        let sender = kernel.create_tcb("sender", 5).unwrap();
        let receiver = kernel.create_tcb("receiver", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let payload = kernel.create_endpoint().unwrap();
        // Không có GRANT: gửi phải bị từ chối ngay.
        kernel
            .install_cap(
                sender,
                1,
                Cap::new(payload, ObjType::Endpoint, Rights::READ),
            )
            .unwrap();
        let msg = Message::new(0x31, &[]).unwrap().with_cap(1);
        assert_eq!(
            kernel.send(sender, endpoint, 0, msg, false),
            Err(KError::NoRights)
        );
        // Có GRANT: gửi được, và bản sao bên nhận mất GRANT.
        kernel
            .install_cap(
                sender,
                2,
                Cap::new(payload, ObjType::Endpoint, Rights::READ | Rights::GRANT),
            )
            .unwrap();
        let msg = Message::new(0x32, &[]).unwrap().with_cap(2);
        assert_eq!(kernel.send(sender, endpoint, 0, msg, false), Ok(false));
        let received = kernel
            .recv(receiver, endpoint, 9)
            .unwrap()
            .expect("message phải ở trong hàng");
        assert_eq!(received.label, 0x32);
        assert_eq!(received.cap, 9, "cap được cài vào slot chỉ định");
        let copied = kernel.lookup_cap(receiver, 9, None, Rights::NONE).unwrap();
        assert_eq!(copied.obj, payload);
        assert!(
            !copied.rights.contains(Rights::GRANT),
            "GRANT không được lan qua IPC"
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn reply_wakes_the_caller_with_the_reply_message() {
        // Vòng 173: `reply` phải đánh thức ĐÚNG client đang BlockedReply và
        // gắn message trả lời vào resume của nó.
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None));
        let call = Message::new(0x40, &[1]).unwrap();
        assert_eq!(kernel.send(client, endpoint, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        let reply = Message::new(0x41, &[2]).unwrap();
        kernel.reply(server, reply).unwrap();
        let table = kernel.tcb(client).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        let resume = table.resume.expect("phải có message trả lời");
        assert_eq!(resume.label, 0x41);
        assert_eq!(resume.words()[0], 2);
        assert_eq!(kernel.tcb(server).unwrap().caller, None);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn send_to_waiting_receiver_hands_the_message_over_directly() {
        // Vòng 172 (IPC): nếu đã có người nhận đang chờ thì message được trao
        // TRỰC TIẾP — người nhận tỉnh dậy với đúng label/badge, không đi qua
        // hàng đợi.
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None));
        let msg = Message::new(0x2222, &[0xaa, 0xbb]).unwrap();
        assert_eq!(kernel.send(client, endpoint, 7, msg, false), Ok(true));
        let table = kernel.tcb(server).unwrap();
        assert_eq!(table.state, ThreadState::Ready, "người nhận phải tỉnh dậy");
        let resume = table.resume.expect("message phải được gắn vào resume");
        assert_eq!(resume.label, 0x2222);
        assert_eq!(resume.badge, 7, "badge của endpoint phải tới người nhận");
        assert_eq!(resume.words()[0], 0xaa);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn send_to_busy_endpoint_queues_fifo() {
        // Vòng 172 (IPC): khi chưa ai chờ, message phải xếp hàng và ra theo
        // đúng thứ tự gửi — FIFO, không đảo.
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 5).unwrap();
        let a = kernel.create_tcb("a", 5).unwrap();
        let b = kernel.create_tcb("b", 5).unwrap();
        let c = kernel.create_tcb("c", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        for (sender, label) in [(a, 0x10u64), (b, 0x20), (c, 0x30)] {
            let msg = Message::new(label, &[]).unwrap();
            // `Ok(false)` = ĐÃ XẾP HÀNG (khác `Ok(true)` = trao trực tiếp).
            assert_eq!(kernel.send(sender, endpoint, 0, msg, false), Ok(false));
        }
        let mut seen = [0u64; 3];
        for slot in seen.iter_mut() {
            let got = kernel
                .recv(server, endpoint, 4)
                .unwrap()
                .expect("phải còn message trong hàng");
            *slot = got.label;
        }
        assert_eq!(seen, [0x10, 0x20, 0x30], "thứ tự phải là FIFO");
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None), "hàng phải rỗng");
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn pick_prefers_higher_priority_and_is_fifo_within_a_level() {
        // Vòng 171 (sched): hàng đợi phải chọn ưu tiên cao nhất, và giữa các
        // TCB cùng ưu tiên thì FIFO theo `queue_seq`.
        let mut kernel = Kernel::new();
        let low = kernel.create_tcb("low", 3).unwrap();
        let high = kernel.create_tcb("high", 9).unwrap();
        let same_a = kernel.create_tcb("same_a", 5).unwrap();
        let same_b = kernel.create_tcb("same_b", 5).unwrap();
        kernel.enqueue(low);
        kernel.enqueue(same_a);
        kernel.enqueue(same_b);
        kernel.enqueue(high);
        assert_eq!(
            kernel.pick(),
            Some(high),
            "ưu tiên cao nhất phải được chọn trước"
        );
        assert_eq!(
            kernel.pick(),
            Some(same_a),
            "cùng ưu tiên thì FIFO theo queue_seq"
        );
        assert_eq!(kernel.pick(), Some(same_b));
        assert_eq!(kernel.pick(), Some(low));
        assert_eq!(kernel.pick(), None, "hàng rỗng phải trả None");
    }

    #[test]
    fn pick_skips_tasks_whose_affinity_excludes_the_hart() {
        // Vòng 171: task ghim hart 1 không bao giờ được chọn trên hart 0, kể cả
        // khi là task duy nhất còn lại — đây là bất biến của affinity.
        let mut kernel = Kernel::new();
        let pinned = kernel.create_tcb("pinned", 7).unwrap();
        kernel.tcb_mut(pinned).unwrap().affinity = 0b10;
        kernel.enqueue(pinned);
        assert_eq!(
            kernel.pick_on(0),
            None,
            "hart 0 không được chạy task ghim hart 1"
        );
        assert_eq!(kernel.pick_on(1), Some(pinned));
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
    fn round69_steal_respects_affinity_and_counts_stats() {
        // Mốc 2 (vòng 69): hart trống được steal task affinity-cho-phép từ
        // hart khác (steal đếm riêng), nhưng không bao giờ cướp task cấm
        // affinity (kẹt đúng hart của nó).
        let mut kernel = Kernel::new();
        let shared = kernel.create_tcb("shared", 5).unwrap();
        kernel.tcb_mut(shared).unwrap().affinity = 0b11;
        kernel.enqueue(shared);
        // shared vào runq hart 0 (ít việc nhất, ngang nhau lấy hart nhỏ).
        // Hart 1 trống: steal shared.
        assert_eq!(kernel.pick_on(1), Some(shared));
        assert_eq!(kernel.scheduler_stats(1), (0, 1));
        assert_eq!(kernel.scheduler_stats(0), (0, 0));
        // Task chỉ affinity hart 1: hart 0 không cướp được.
        let pinned = kernel.create_tcb("pinned", 5).unwrap();
        kernel.tcb_mut(pinned).unwrap().affinity = 0b10;
        kernel.enqueue(pinned);
        assert_eq!(kernel.pick_on(0), None);
        // Hart 1 lấy local (không phải steal).
        assert_eq!(kernel.pick_on(1), Some(pinned));
        assert_eq!(kernel.scheduler_stats(1), (1, 1));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round73_suspend_waiter_keeps_signal_bits() {
        // Mốc 1 (vòng 73): suspend task đang BlockedNotification phải gỡ
        // khỏi waiters (đường suspend thật, không set tay) — signal sau đó
        // không đánh thức task đã treo, bit giữ cho waiter còn lại.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        let other = kernel.create_tcb("other", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(waiter, ntfn), Ok(None));
        assert_eq!(kernel.wait_as(other, ntfn), Ok(None));
        // Suspend waiter qua syscall path (suspend_tcb), không set tay.
        kernel
            .install_cap(root, 1, Cap::new(waiter, ObjType::Tcb, Rights::WRITE))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbSuspend as u64, 1, 0, 0, 0, 0, 0),
            UserAction::TcbSuspended { hart: None }
        );
        assert_eq!(kernel.tcb(waiter).unwrap().state, ThreadState::Suspended);
        // Slot waiter đã sạch, other còn trong hàng.
        let waiters = kernel.notification(ntfn).unwrap().waiters;
        assert!(waiters.iter().all(|slot| *slot != Some(waiter)));
        assert!(waiters.iter().any(|slot| *slot == Some(other)));
        // Signal: other thức, waiter treo không bị động, bit không mất.
        assert_eq!(kernel.signal(ntfn, 0b1), Ok(()));
        assert_eq!(kernel.tcb(other).unwrap().state, ThreadState::Ready);
        assert_eq!(kernel.tcb(waiter).unwrap().state, ThreadState::Suspended);
        assert!(kernel.check_invariants().is_ok());
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
    fn round66_exit_removes_waiter_from_notification() {
        // Mốc 1 (vòng 66): task đang BlockedNotification mà exit phải được
        // gỡ khỏi notification.waiters — nếu không signal sau này chạm slot
        // stale (tái dùng object) hoặc đánh thức zombie. Bit signal giữ
        // nguyên cho waiter còn sống.
        let mut kernel = Kernel::new();
        let dying = kernel.create_tcb("dying", 5).unwrap();
        let survivor = kernel.create_tcb("survivor", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(dying, ntfn), Ok(None));
        assert_eq!(kernel.wait_as(survivor, ntfn), Ok(None));
        // Đặt dying làm current rồi exit.
        kernel.tcb_mut(dying).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(dying);
        kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(dying).unwrap().state, ThreadState::Exited);
        // Slot của dying đã sạch, survivor còn trong hàng.
        let waiters = kernel.notification(ntfn).unwrap().waiters;
        assert!(waiters.iter().all(|slot| *slot != Some(dying)));
        assert!(waiters.iter().any(|slot| *slot == Some(survivor)));
        // Signal đánh thức survivor và không chạm zombie.
        assert_eq!(kernel.signal(ntfn, 0b1), Ok(()));
        assert_eq!(kernel.tcb(survivor).unwrap().state, ThreadState::Ready);
        assert_eq!(kernel.tcb(dying).unwrap().state, ThreadState::Exited);
        assert!(kernel.check_invariants().is_ok());
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
    fn user_range_spanning_unmapped_page_is_rejected() {
        // Vòng 140: một range bắt đầu ở trang CÓ map nhưng vắt sang trang
        // KHÔNG map phải bị từ chối — lỗ hổng bảo mật kinh điển nếu chỉ kiểm
        // trang đầu.
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
        kernel
            .map_frame(space, frame, va, Rights::READ | Rights::WRITE)
            .unwrap();

        let tcb = kernel.create_tcb("range", 5).unwrap();
        kernel.set_tcb_vspace(tcb, space).unwrap();

        // Trang có map: 4 byte cuối trang vẫn hợp lệ.
        assert_eq!(kernel.validate_user_range(tcb, va + 4092, 4, false), Ok(()));
        // Vắt sang trang kế (chưa map) phải bị từ chối.
        assert_eq!(
            kernel.validate_user_range(tcb, va + 4092, 8, false),
            Err(KError::InvalidArg)
        );
        // Cần quyền ghi: mapping RW nên vẫn Ok.
        assert_eq!(kernel.validate_user_range(tcb, va, 8, true), Ok(()));
    }

    #[test]
    fn destroyed_slot_can_be_retyped_again() {
        // Vòng 137: sau khi destroy, slot phải THỰC SỰ trống để retype lại.
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x720_0000, 8);
        let _ = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
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
            },
            "retype lại đúng slot vừa destroy phải thành công"
        );
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
    fn exec_teardown_chain_frees_tcb_vspace_frame() {
        // Vòng 205: khoá HỒI QUY cho chuỗi rò rỉ đã sửa ở vòng 199–200. Thứ tự
        // huỷ đúng như root làm khi dọn task exec: TCB → VSpace → frame. Trước
        // khi sửa, bước 1 trả `InvalidArg` (TCB không nằm trong danh sách loại
        // được huỷ) và bước 2 trả `Busy` (vspace còn mapping) ⇒ cả 3 object kẹt.
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x7a0_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 4);
        kernel.map_frame(vspace, frame, 0, Rights::READ).unwrap();
        kernel.tcb_mut(tcb).unwrap().vspace = Some(vspace);
        assert_eq!(
            kernel.destroy_cap_object(root, 4),
            Ok(()),
            "TCB phải huỷ được qua đường cap"
        );
        assert_eq!(
            kernel.destroy_cap_object(root, 3),
            Ok(()),
            "VSpace có mapping phải tự dọn được"
        );
        assert_eq!(
            kernel.destroy_cap_object(root, 2),
            Ok(()),
            "frame phải huỷ được sau khi VSpace dọn mapping"
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_tears_down_mapped_vspace() {
        // Vòng 200: ĐỔI NGỮ NGHĨA có chủ ý. Trước đây VSpace còn mapping thì
        // `destroy` trả `Busy`, buộc người dùng tự unmap trước. Thực tế root
        // không biết hết VA đã map (code/stack/trang do chương trình tự map) nên
        // VSpace của task exec KHÔNG BAO GIỜ huỷ được ⇒ rò rỉ frame + page table
        // mỗi chu kỳ. Giờ huỷ address space **tự gỡ** mapping của nó (giống mọi
        // microkernel thực dụng); chỉ TCB đang dùng nó mới chặn (kiểm riêng).
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x740_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        kernel.map_frame(vspace, frame, 0, Rights::READ).unwrap();
        assert_eq!(kernel.destroy_cap_object(root, 3), Ok(()));
        // Frame phải huỷ được ngay sau đó — chứng minh counter mapping đã về 0.
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn object_destroy_refuses_vspace_bound_to_live_tcb() {
        // Bất biến còn giữ: VSpace đang được TCB SỐNG dùng thì không huỷ được.
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x745_0000, 16);
        let vspace = retype_at(&mut kernel, root, ObjType::VSpace, 3);
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 4);
        kernel.tcb_mut(tcb).unwrap().vspace = Some(vspace);
        assert_eq!(kernel.destroy_cap_object(root, 3), Err(KError::Busy));
        kernel.tcb_mut(tcb).unwrap().vspace = None;
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
        // Vòng 199: TCB GIỜ huỷ được qua đường cap (trước đây trả `InvalidArg`
        // vĩnh viễn ⇒ TCB exec không bao giờ chết ⇒ rò rỉ cả VSpace lẫn frame).
        let tcb = retype_at(&mut kernel, root, ObjType::Tcb, 2);
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        assert!(kernel.get(tcb).is_none());
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
    fn round67_reclaim_adjacent_to_bump_lowers_watermark() {
        // Mốc 3 (vòng 67): free vùng kề bump phải hạ watermark thay vì
        // chiếm slot free-list — nếu không alloc/free lặp ở đỉnh Untyped
        // làm đầy list 16 slot rồi rò rỉ giả (leaked tăng dù bộ nhớ còn).
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x840_0000, 8);
        let untyped = kernel.cap_at(SlotAddr { tcb: root, slot: 1 }).unwrap().obj;
        // 2 endpoint: offsets 0 và 256, bump = 512.
        let _e0 = retype_at(&mut kernel, root, ObjType::Endpoint, 2);
        let _e1 = retype_at(&mut kernel, root, ObjType::Endpoint, 3);
        assert_eq!(kernel.untyped(untyped).unwrap().bump, 512);
        // Free e1 (offset 256, kề bump 512): bump hạ về 256, không tốn slot.
        assert_eq!(kernel.destroy_cap_object(root, 3), Ok(()));
        let region = kernel.untyped(untyped).unwrap();
        assert_eq!(region.bump, 256);
        assert_eq!(region.free.iter().flatten().count(), 0);
        assert_eq!(region.leaked, 0);
        // Free e0 (offset 0, kề bump 256): bump hạ về 0 — Untyped sạch.
        assert_eq!(kernel.destroy_cap_object(root, 2), Ok(()));
        let region = kernel.untyped(untyped).unwrap();
        assert_eq!(region.bump, 0);
        assert_eq!(region.objects, 0);
        assert_eq!(region.free.iter().flatten().count(), 0);
        // Retype lại từ đầu: tái dùng offset 0 (không rò rỉ).
        let e2 = retype_at(&mut kernel, root, ObjType::Endpoint, 4);
        assert_eq!(kernel.get(e2).unwrap().alloc_offset, 0);
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
    fn maps_two_pages_in_one_fresh_vspace() {
        // Vòng 117: mô phỏng đúng tình huống exec — vspace mới, map trang code
        // trước (tạo bảng), rồi map trang stack thứ hai trong CÙNG bảng L0.
        // Cả hai leaf phải nằm trong bộ nhớ bảng trang giả.
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
        let code = kernel.create_frame(0x4000, 4096).unwrap();
        let stack = kernel.create_frame(0x5000, 4096).unwrap();
        let code_va = 0x4000_1000;
        let stack_va = 0x4000_2000;

        // Trang thứ nhất: tạo chuỗi bảng rồi map.
        assert_eq!(kernel.map_page_table(pt1, space, code_va), Ok(1));
        assert_eq!(kernel.map_page_table(pt2, space, code_va), Ok(2));
        assert_eq!(kernel.map_page_table(pt3, space, code_va), Ok(3));
        kernel
            .map_frame(space, code, code_va, Rights::READ | Rights::EXEC)
            .unwrap();
        // Trang thứ hai: cùng vùng 2 MiB ⇒ dùng lại đúng bảng L0.
        kernel
            .map_frame(space, stack, stack_va, Rights::READ | Rights::WRITE)
            .unwrap();

        // Kiểm tra CẢ PTE thật trong bộ nhớ bảng trang giả: bảng L0 là pt3
        // (PA 0x3000 ⇒ base 1536). Entry 1 = code, entry 2 = stack.
        let mem = FAKE_MEM.lock().unwrap();
        assert_eq!(mem[1536 + 1] & !0xfff, 0x4000, "PTE code phải trỏ 0x4000");
        assert_eq!(mem[1536 + 2] & !0xfff, 0x5000, "PTE stack phải trỏ 0x5000");
        drop(mem);
        let code_map = kernel.mapping_at(space, code_va).expect("code mapping");
        assert_eq!(code_map.pa, 0x4000);
        let stack_map = kernel.mapping_at(space, stack_va).expect("stack mapping");
        assert_eq!(stack_map.pa, 0x5000);
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
    fn round60_tcb_destroy_rejects_current_and_self() {
        // Mốc 1 (vòng 60): TcbDestroy task đang là current trên bất kỳ
        // hart nào, hoặc chính caller, phải IllegalOperation ở mức
        // syscall — kẻ tấn công không tự sát hart mình chạy, cũng không
        // giết task đang chạy trên hart khác (treo hart đó).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let runner = kernel.create_tcb("runner", 5).unwrap();
        kernel
            .configure_tcb(runner, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(runner, ObjType::Tcb, Rights::ALL))
            .unwrap();
        kernel.resume_tcb(runner).unwrap();
        assert_eq!(kernel.pick_on(0), Some(runner));
        kernel.tcb_mut(runner).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(runner);
        // Destroy current qua syscall → cấm, TCB còn nguyên.
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbDestroy as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::IllegalOperation,
            }
        );
        assert!(kernel.get(runner).is_some());
        assert_eq!(kernel.current[0], Some(runner));
        // Caller tự destroy mình qua cap trỏ chính nó → cấm. Chuyển
        // runner sang hart 1 trước (mỗi TCB chỉ current trên một hart).
        kernel.current[0] = None;
        kernel.tcb_mut(runner).unwrap().state = ThreadState::Ready;
        kernel.tcb_mut(runner).unwrap().affinity = 0b11;
        kernel.enqueue(runner);
        assert_eq!(kernel.pick_on(1), Some(runner));
        kernel.tcb_mut(runner).unwrap().state = ThreadState::Running;
        kernel.current[1] = Some(runner);
        kernel
            .install_cap(runner, 7, Cap::new(runner, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(runner, SyscallId::TcbDestroy as u64, 7, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::IllegalOperation,
            }
        );
        assert!(kernel.get(runner).is_some());
        assert!(kernel.check_invariants().is_ok());
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
                cnode: None,
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
    /// 290.4 (r290, đỏ-trước): mỗi lần call phải cấp epoch MỚI.
    ///
    /// Bản cũ `reply_epoch` không bao giờ tăng ⇒ mọi record `caller` đều mang
    /// epoch 0 và guard K49 (`reply_epoch == epoch`) thành `0 == 0`, tức rỗng.
    /// Test này khoá đúng điều đó: hai lần call liên tiếp phải khác epoch.
    #[test]
    /// P0-3 (r290, đỏ-trước) — KỊCH BẢN ĐẦY ĐỦ 5 bước của bản soát:
    ///
    /// 1. root cấp cho A một cap endpoint (parent = cap của root) và một cap
    ///    tới CNode C;
    /// 2. A mint chính cap endpoint đó vào C ⇒ `C.slots[0].parent` = slot của A;
    /// 3. huỷ A ⇒ `parent` phải được **nâng lên tổ tiên còn sống** (cap của
    ///    root), không được thành con trỏ chết;
    /// 4. `revoke` cap gốc của root phải thu hồi được cap trong C;
    /// 5. sau đó object endpoint không còn ai tham chiếu ⇒ destroy được.
    ///
    /// Bản cũ: vòng sửa `parent` chỉ đi qua `cspace_mut` (chỉ với tới CNode
    /// qua TCB đang dùng nó) nên C không bao giờ được sửa ⇒ bước 4 trả 0 và
    /// bước 5 trả `HasDependents` (object bất tử).
    #[test]
    fn orphan_cap_in_a_detached_cnode_is_reparented_and_revoked() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let a = kernel.create_tcb("a", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel.create_untyped(0x6000_0000, 4096).unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();

        // root: slot 1 = endpoint, slot 2 = CNode.
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        // (1) A nhận cap endpoint (parent = root slot 1) và cap CNode.
        assert_eq!(kernel.copy_cap(root, 1, a, 2, Rights::READ, 0), Ok(()));
        assert_eq!(kernel.copy_cap(root, 2, a, 1, Rights::ALL, 0), Ok(()));
        // (2) A mint cap endpoint của mình vào C tại slot 0.
        assert_eq!(
            kernel.user_syscall(
                a,
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
        let minted = kernel.cnode(cnode).unwrap().slots[1].unwrap();
        assert_eq!(minted.parent.unwrap().addr.tcb, a);

        // (3) A chết — C KHÔNG phải cspace của A, nên C chỉ được sửa nếu bộ
        // duyệt đi qua mọi CNode.
        kernel.destroy_object(a).unwrap();
        let after = kernel.cnode(cnode).unwrap().slots[1].unwrap();
        let parent = after.parent.expect("phải được nâng lên tổ tiên còn sống");
        assert_eq!(
            parent.addr.tcb, root,
            "parent phải trỏ về cap gốc của root, không phải TCB đã chết"
        );
        assert!(kernel.resolve_cap_ref(parent).is_some());

        // (4) revoke cap gốc phải thu hồi được cap trong C.
        assert_eq!(
            kernel.revoke_cap(root, 1).unwrap(),
            1,
            "phải thu hồi cap nằm trong CNode đã tháo khỏi TCB"
        );
        assert!(kernel.cnode(cnode).unwrap().slots[1].is_none());

        // (5) object không còn tham chiếu NGOÀI cap gốc ⇒ destroy được.
        let root_cap_id = kernel
            .cap_at(SlotAddr { tcb: root, slot: 1 })
            .expect("cap gốc vẫn còn")
            .id;
        assert_eq!(
            kernel.other_caps_to(endpoint, root_cap_id),
            0,
            "cap trong CNode đã bị thu hồi nên không còn tham chiếu nào khác"
        );
        assert_eq!(kernel.destroy_object(endpoint), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    /// 290.3 (r290, đỏ-trước): `check_invariants` phải bắt được `cap.parent`
    /// trỏ vào bảng ĐÃ CHẾT ở **mọi** nhánh (không chỉ khi `badge != 0`).
    #[test]
    fn check_invariants_rejects_a_parent_pointing_at_a_dead_table() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let dead = kernel.create_tcb("dead", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let mut cap = Cap::new(endpoint, ObjType::Endpoint, Rights::READ);
        cap.badge = 0;
        cap.parent = Some(CapRef {
            addr: SlotAddr { tcb: dead, slot: 3 },
            id: 99,
            cnode: None,
        });
        kernel
            .install_cap(
                root,
                1,
                Cap::new(cnode_dummy(), ObjType::Endpoint, Rights::READ),
            )
            .ok();
        kernel.root_cspace[1] = Some(cap);
        kernel.destroy_object(dead).unwrap();
        assert!(
            kernel.check_invariants().is_err(),
            "parent trỏ bảng đã chết phải bị bắt"
        );
    }

    fn cnode_dummy() -> ObjRef {
        ObjRef { index: 0, gen: 0 }
    }

    fn every_call_issues_a_fresh_reply_epoch() {
        let mut kernel = Kernel::new();
        let server = kernel.create_tcb("server", 1).unwrap();
        let client = kernel.create_tcb("client", 1).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None));
        assert_eq!(
            kernel.send(client, endpoint, 0, Message::new(0x11, &[1]).unwrap(), true),
            Ok(true)
        );
        let (_, first) = kernel.tcb(server).unwrap().caller.unwrap();
        kernel.reply(server, Message::empty(0x12)).unwrap();
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::Ready);
        // Lần gọi thứ hai.
        assert_eq!(kernel.recv(server, endpoint, 4), Ok(None));
        assert_eq!(
            kernel.send(client, endpoint, 0, Message::new(0x13, &[1]).unwrap(), true),
            Ok(true)
        );
        let (_, second) = kernel.tcb(server).unwrap().caller.unwrap();
        assert_ne!(
            first, second,
            "call thứ hai phải mang epoch khác — nếu không guard K49 là 0 == 0"
        );
    }

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
        // 290.4: epoch nằm ở phía CLIENT (cấp mới mỗi lần call), nên "stale"
        // phải là một giá trị KHÁC epoch hiện tại của client.
        let live_epoch = kernel.tcb(server_caller).unwrap().reply_epoch;
        kernel.tcb_mut(server).unwrap().caller = Some((server_caller, live_epoch.wrapping_add(1)));
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
                    cnode: None,
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
                    cnode: None,
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
    fn has_ready_on_ignores_a_queued_tcb_that_is_not_ready() {
        // T33a (red-first, D18): `has_ready_on` từng KHÔNG kiểm `state`, nên
        // một TCB còn trong hàng chờ nhưng đã chuyển sang Running/Blocked vẫn
        // làm nó trả `true`. Hart phụ thấy "có việc" ⇒ thức dậy; nhưng
        // `pick_on` (chỉ nhận `Ready`) trả `None` ⇒ runtime gọi `exit(1)` và
        // hart phụ CHẾT ⇒ boot treo trước `PREEMPT OK` (đo ~1% trên x86_64).
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("worker", 5).unwrap();
        // Cho phép hart 1 nhặt — nếu chỉ affinity hart 0 thì test vô nghĩa.
        kernel.tcb_mut(tcb).unwrap().affinity = 0b11;
        kernel.enqueue(tcb);
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Running;
        assert_eq!(kernel.pick_on(1), None, "pick_on chỉ nhận TCB Ready");
        assert!(
            !kernel.has_ready_on(1),
            "has_ready_on phải đồng ý với pick_on, nếu không hart phụ sẽ chết"
        );
    }

    #[test]
    fn destroy_object_accepts_an_exited_tcb_that_is_still_current() {
        // T32 (red-first, D18): `current` cố ý giữ TCB đã park (K37), nhưng
        // nếu task ĐÃ EXIT mà vẫn là `current` thì object không bao giờ huỷ
        // được ⇒ VSpace kẹt Busy ⇒ vòng exec sau hết slot (soak 100 vòng 62).
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("worker", 5).unwrap();
        kernel.enqueue(tcb);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Exited;
        assert_eq!(kernel.current_on(0), Some(tcb));
        assert_eq!(kernel.destroy_object(tcb), Ok(()));
        assert_eq!(kernel.current_on(0), None, "tàn dư `current` phải được dọn");
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn destroy_object_still_rejects_a_running_current_tcb() {
        // Cùng đường, nhưng task ĐANG chạy thì vẫn phải bị từ chối — nếu
        // không thì ai đó huỷ được task đang thực thi.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("busy", 5).unwrap();
        kernel.enqueue(tcb);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert_eq!(kernel.destroy_object(tcb), Err(KError::IllegalOperation));
        assert_eq!(kernel.current_on(0), Some(tcb));
    }

    #[test]
    fn grant_cap_installs_a_derived_cap_in_the_target_cspace() {
        // T27 (r264, G2 Phần B): uỷ quyền cap — đường mà exec còn thiếu.
        // Caller giữ endpoint ở slot 1; chuyển sang TCB đích ở slot 3.
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(caller, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::RW))
            .unwrap();
        assert_eq!(
            kernel.grant_cap_to_tcb(caller, target, 1, 3, Rights::ALL),
            Ok(())
        );
        let granted = kernel.lookup_cap(target, 3, Some(ObjType::Endpoint), Rights::READ);
        assert!(granted.is_ok(), "cap phải xuất hiện trong cspace của đích");
        // Caller KHÔNG mất cap nguồn (derive, không move).
        assert!(kernel
            .lookup_cap(caller, 1, Some(ObjType::Endpoint), Rights::RW)
            .is_ok());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn grant_cap_strips_grant_right() {
        // Ràng buộc 3: không bao giờ chuyển được GRANT, kể cả khi đòi ALL —
        // nếu không thì quyền có thể bị rửa qua nhiều bước uỷ quyền.
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(
                caller,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::ALL),
            )
            .unwrap();
        assert_eq!(
            kernel.grant_cap_to_tcb(caller, target, 1, 3, Rights::ALL),
            Ok(())
        );
        let granted = kernel.lookup_cap(target, 3, None, Rights::NONE).unwrap();
        assert!(
            !granted.rights.contains(Rights::GRANT),
            "cap được uỷ quyền không được mang GRANT"
        );
        assert_eq!(granted.rights, Rights(0b0111));
    }

    #[test]
    fn grant_cap_rejects_occupied_destination() {
        // Ràng buộc 2: slot đích đã có cap thì trả SlotOccupied, không ghi đè.
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let other = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(caller, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::RW))
            .unwrap();
        kernel
            .install_cap(target, 3, Cap::new(other, ObjType::Endpoint, Rights::RW))
            .unwrap();
        assert_eq!(
            kernel.grant_cap_to_tcb(caller, target, 1, 3, Rights::NONE),
            Err(KError::SlotOccupied)
        );
        // Cap cũ còn nguyên.
        assert!(kernel
            .lookup_cap(target, 3, Some(ObjType::Endpoint), Rights::RW)
            .is_ok());
    }

    #[test]
    fn grant_cap_cannot_hand_out_the_root_untyped() {
        // Ràng buộc 5: cap Untyped gốc là nguồn của mọi object — chuyển nó là
        // trao cả không gian bộ nhớ.
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let untyped = kernel.create_untyped(0x1000_0000, 0x1000).unwrap();
        let mut cap = Cap::new(untyped, ObjType::Untyped, Rights::RW);
        cap.root = true;
        cap.parent = None;
        kernel.install_cap(caller, 1, cap).unwrap();
        assert_eq!(
            kernel.grant_cap_to_tcb(caller, target, 1, 3, Rights::NONE),
            Err(KError::InvalidArg)
        );
    }

    #[test]
    fn grant_cap_keeps_the_source_badge() {
        // Ràng buộc 4: badge là uid của FS server — người nhận không được tự
        // đặt danh tính khác, nên badge phải giữ nguyên từ cap nguồn.
        let mut kernel = Kernel::new();
        let caller = kernel.create_tcb("caller", 5).unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let mut cap = Cap::new(endpoint, ObjType::Endpoint, Rights::RW);
        cap.badge = 0x51de;
        kernel.install_cap(caller, 1, cap).unwrap();
        assert_eq!(
            kernel.grant_cap_to_tcb(caller, target, 1, 3, Rights::NONE),
            Ok(())
        );
        let granted = kernel.lookup_cap(target, 3, None, Rights::NONE).unwrap();
        assert_eq!(granted.badge, 0x51de);
    }

    #[test]
    fn grant_cap_syscall_requires_write_on_the_target_tcb() {
        // Ràng buộc 1: không ai nhét cap vào cspace của task khác mà không có
        // quyền cấu hình TCB đó. Caller chỉ có cap READ trên TCB đích ⇒
        // syscall trả NoRights.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let target = kernel.create_tcb("child", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(endpoint, ObjType::Endpoint, Rights::RW))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(target, ObjType::Tcb, Rights::READ))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(root, SyscallId::TcbGrantCap as u64, 2, 1, 3, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::NoRights,
            }
        );
    }

    #[test]
    fn boundary_check_accepts_consistent_current() {
        // D19 (r259): ở ranh giới trả về user, `current[hart]` phải Running và
        // không còn queued. Đây là trạng thái mà `pick_on` vừa thiết lập.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("user", 5).unwrap();
        kernel.enqueue(tcb);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert_eq!(kernel.check_boundary(0), Ok(()));
        assert_eq!(kernel.current_harts(), 1);
        assert_eq!(kernel.running_tcbs(), 1);
    }

    #[test]
    fn boundary_check_rejects_missing_current() {
        // Mutation: hart ở ranh giới mà không có `current` — quay về user sẽ
        // chạy nhầm context (không hart nào sở hữu TCB đang chạy).
        let kernel = Kernel::new();
        assert_eq!(kernel.check_boundary(0), Err(BoundaryError::NoCurrent));
        assert_eq!(kernel.current_harts(), 0);
        assert_eq!(kernel.running_tcbs(), 0);
    }

    #[test]
    fn boundary_check_rejects_current_not_running() {
        // Mutation: `current` bị treo nhưng hart vẫn tưởng nó đang chạy.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("user", 5).unwrap();
        kernel.enqueue(tcb);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Suspended;
        assert_eq!(
            kernel.check_boundary(0),
            Err(BoundaryError::CurrentNotRunning)
        );
    }

    #[test]
    fn boundary_check_rejects_running_but_queued() {
        // Mutation D18: TCB vừa Running vừa nằm trong ready queue ⇒ có thể
        // được chọn chạy lần thứ hai. `poison_running_queued` tạo đúng trạng
        // thái đó để chứng minh phép kiểm không im lặng.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("user", 5).unwrap();
        kernel.enqueue(tcb);
        assert_eq!(kernel.pick_on(0), Some(tcb));
        assert!(kernel.poison_running_queued(0));
        assert_eq!(
            kernel.check_boundary(0),
            Err(BoundaryError::RunningAndQueued)
        );
    }

    #[test]
    fn boundary_summary_detects_running_tcb_without_a_hart() {
        // D19/T21: ở test-exit, mỗi TCB Running phải thuộc một hart đang chạy
        // user. TCB Running mà không hart nào sở hữu là lỗi "chạy hai lần"
        // phía ngược lại — phép tổng hợp phải lộ ra.
        let mut kernel = Kernel::new();
        let owned = kernel.create_tcb("owned", 5).unwrap();
        kernel.enqueue(owned);
        assert_eq!(kernel.pick_on(0), Some(owned));
        let (running, harts, running_harts) = kernel.boundary_summary();
        assert_eq!((running, harts, running_harts), (1, 1, 1));

        // Mutation: thêm TCB Running nhưng không hart nào sở hữu.
        let stray = kernel.create_tcb("stray", 5).unwrap();
        kernel.tcb_mut(stray).unwrap().state = ThreadState::Running;
        let (running, harts, running_harts) = kernel.boundary_summary();
        assert_eq!((running, harts, running_harts), (2, 1, 1));
        assert!(running != running_harts);
    }

    #[test]
    fn boundary_check_rejects_running_tcb_inside_queue() {
        // Mutation thứ hai: một TCB KHÁC `current` đang Running mà vẫn nằm
        // trong queue — `check_boundary` phải quét cả ready queue, không chỉ
        // nhìn `current`.
        let mut kernel = Kernel::new();
        let first = kernel.create_tcb("first", 5).unwrap();
        kernel.enqueue(first);
        assert_eq!(kernel.pick_on(0), Some(first));
        let second = kernel.create_tcb("second", 5).unwrap();
        kernel.enqueue(second);
        kernel.tcb_mut(second).unwrap().state = ThreadState::Running;
        assert_eq!(
            kernel.check_boundary(0),
            Err(BoundaryError::RunningAndQueued)
        );
        assert_eq!(kernel.running_tcbs(), 2);
        assert_eq!(kernel.current_harts(), 1);
    }

    #[test]
    fn recv_polls_the_bound_notification_before_blocking() {
        // K42 (red-first, D18): `bound_ntfn` từng là trường chết — bind xong
        // `recv` không bao giờ thấy bit. Nay `recv` phải trả event đã chờ thay
        // vì chặn, và tiêu thụ đúng một lần.
        let mut kernel = Kernel::new();
        let task = kernel.create_tcb("driver", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.bind_notification(task, ntfn), Ok(()));
        // Chưa có bit: recv chặn như thường (BlockedRecv).
        assert_eq!(kernel.recv(task, endpoint, 0), Ok(None));
        assert_eq!(kernel.tcb(task).unwrap().state, ThreadState::BlockedRecv);
        // Có ngắt: signal đánh thức task, `recv` phải trả event đã chờ.
        kernel.signal(ntfn, 0b100).unwrap();
        let message = kernel
            .recv(task, endpoint, 0)
            .expect("recv không được lỗi")
            .expect("phải trả event đã chờ thay vì chặn");
        assert_eq!(message.label, LABEL_NOTIFICATION);
        assert_eq!(message.words()[0], 0b100);
        assert_eq!(kernel.poll(ntfn), Ok(0), "bit đã tiêu thụ đúng một lần");
    }

    #[test]
    fn signal_mixed_stale_and_live_waiters_loses_no_bit() {
        // K45 (r260, đóng bằng bằng chứng — D18): audit nghi `signal` xoá word
        // TRƯỚC vòng lặp nên khi hàng có cả waiter chết lẫn waiter sống thì
        // bit bị mất. Dựng đúng ca hỗn hợp và kiểm từng bit.
        let mut kernel = Kernel::new();
        let live = kernel.create_tcb("live", 5).unwrap();
        let stale = kernel.create_tcb("stale", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(live, ntfn), Ok(None));
        assert_eq!(kernel.wait_as(stale, ntfn), Ok(None));
        // waiter "chết": còn trong hàng nhưng không còn BlockedNotification.
        kernel.tcb_mut(stale).unwrap().state = ThreadState::Suspended;
        // Bit cũ đang chờ trong word (ai đó signal trước khi có waiter).
        kernel.notification_mut(ntfn).unwrap().word = 0b0010;
        kernel.signal(ntfn, 0b0001).unwrap();
        let resume = kernel
            .tcb(live)
            .unwrap()
            .resume
            .expect("waiter sống phải được đánh thức");
        assert_eq!(resume.label, LABEL_NOTIFICATION);
        assert_eq!(
            resume.words()[0],
            0b0011,
            "bit cũ 0b0010 + bit mới 0b0001 phải cùng tới waiter sống"
        );
        assert_eq!(
            kernel.tcb(stale).unwrap().state,
            ThreadState::Suspended,
            "waiter đã treo không được đánh thức"
        );
        assert_eq!(kernel.poll(ntfn), Ok(0), "không còn bit nào bị bỏ lại");
    }

    #[test]
    fn poll_clears_irq_pending_so_the_next_edge_is_delivered() {
        // K46 (red-first, D18): `poll` tiêu thụ word nghĩa là driver đã NHẬN
        // ngắt, nên `pending` của IRQ phải được hạ; nếu không, cạnh kế tiếp bị
        // coalesce và người nhận MẤT một wakeup (mất ngắt kinh điển).
        let mut kernel = Kernel::new();
        let ntfn = kernel.create_notification().unwrap();
        let irq = kernel.create_irq(5).unwrap();
        kernel.set_irq_notification(irq, ntfn).unwrap();
        assert!(kernel.irq_raise(5), "cạnh đầu phải được ghi nhận");
        assert_eq!(kernel.poll(ntfn), Ok(1u64 << 5));
        // Trước r260: `pending` còn true ⇒ dòng dưới trả false (mất ngắt).
        assert!(
            kernel.irq_raise(5),
            "sau poll, cạnh mới phải được ghi nhận chứ không bị coalesce"
        );
        assert_eq!(kernel.poll(ntfn), Ok(1u64 << 5));
    }

    #[test]
    fn check_invariants_detects_phantom_harts_loaded_bit() {
        // K60 (red-first, D18): `harts_loaded` có bit cho hart CHƯA TỪNG chạy
        // là mask ma — trước r260 `check_invariants` im lặng, nên mọi test
        // dựa vào nó đều không canh được sổ sách flush.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("owner", 5).unwrap();
        let space = kernel.create_vspace(0x1000, 0x1000, 0x40_0000).unwrap();
        kernel.tcb_mut(tcb).unwrap().vspace = Some(space);
        // Hart 0 nạp ASID hợp lệ (đánh dấu hart đã chạy).
        kernel.note_vspace_hart_loaded(tcb, 0);
        assert_eq!(kernel.vspace_harts_loaded(space), 0b1);
        assert_eq!(kernel.check_invariants(), Ok(()));
        // Tiêm mask ma: hart 3 chưa từng tick/chạy.
        kernel.vspace_mut(space).unwrap().harts_loaded = 0b1001;
        assert_eq!(kernel.check_invariants(), Err(KError::IllegalOperation));
    }

    #[test]
    fn check_invariants_detects_ready_tcb_without_queue_entry() {
        // Vòng 189 (mutation): `check_invariants` phải BẮT được TCB ở trạng
        // thái Ready mà không nằm trong runqueue — lớp lỗi K1/K4. Nếu công cụ
        // chẩn đoán im lặng ở đây thì mọi test khác dựa vào nó đều vô nghĩa.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("orphan", 5).unwrap();
        kernel.tcb_mut(tcb).unwrap().state = ThreadState::Ready;
        assert_eq!(kernel.check_invariants(), Err(KError::IllegalOperation));
    }

    #[test]
    fn check_invariants_detects_garbage_in_runqueue_tail() {
        // Vòng 189: phần tử nằm NGOÀI `runq_len` là rác — phải bị bắt.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("queued", 5).unwrap();
        kernel.enqueue(tcb);
        assert!(kernel.check_invariants().is_ok());
        // Cắt con trỏ độ dài trong khi hàng vẫn còn phần tử.
        kernel.runq_len[0] = 0;
        assert_eq!(kernel.check_invariants(), Err(KError::IllegalOperation));
    }

    #[test]
    fn check_invariants_detects_mapping_without_read_right() {
        // Vòng 189: một mapping KHÔNG có quyền READ là bất biến bị phá (mọi
        // mapping phải đọc được — W^X chỉ cấm W+X, không cấm thiếu R). Đây là
        // loại sai lệch sẽ làm user fault mọi lần truy cập nhưng rất khó thấy
        // khi chỉ nhìn log.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_vspace_with_page_size(0x1000, 0x4000_0000, 0x4000, 4096)
            .unwrap();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        kernel
            .map_frame(space, frame, 0x4000_0000, Rights::READ)
            .unwrap();
        // Chèn thủ công một mapping thứ hai chồng lên cùng VA.
        if let Some(space) = kernel.vspace_mut(space) {
            let victim = space.mappings.iter().position(Option::is_some).unwrap();
            if let Some(mapping) = space.mappings[victim] {
                let mut copy = mapping;
                copy.rights = Rights::WRITE;
                space.mappings[victim] = Some(copy);
            }
        }
        assert_eq!(kernel.check_invariants(), Err(KError::IllegalOperation));
    }

    #[test]
    fn irq_numbers_are_unique_and_reusable_after_destroy() {
        // Vòng 184 (IRQ): mỗi số IRQ chỉ có MỘT object; huỷ rồi thì số đó dùng
        // lại được. Nếu không, driver nạp lại sẽ không lấy lại được ngắt.
        let mut kernel = Kernel::new();
        let first = kernel.create_irq(9).unwrap();
        assert!(
            kernel.create_irq(9).is_none(),
            "không được tạo hai object cho cùng số IRQ"
        );
        kernel.destroy_object(first).unwrap();
        assert!(
            kernel.create_irq(9).is_some(),
            "huỷ IRQ phải trả số về để dùng lại"
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn irq_number_above_63_is_rejected() {
        // Vòng 184 (K47): bit ngắt được ánh xạ `& 63`, nên số >= 64 sẽ alias
        // cùng một bit — phải bị chặn ngay lúc tạo.
        let mut kernel = Kernel::new();
        assert!(kernel.create_irq(63).is_some());
        assert!(kernel.create_irq(64).is_none(), "số 64 phải bị từ chối");
        assert!(kernel.create_irq(u32::MAX).is_none());
    }

    #[test]
    fn irq_raise_without_object_is_ignored() {
        // Vòng 184: nếu không có object IRQ nào cho số đó, `irq_raise` trả
        // false và KHÔNG được làm hỏng trạng thái kernel (đường gọi từ IRQ
        // handler phần cứng có thể tới trước khi user đăng ký).
        let mut kernel = Kernel::new();
        assert!(!kernel.irq_raise(42));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn irq_ack_clears_the_notification_bit() {
        // Vòng 100 (K21): ack chỉ xoá `pending` thì bit cũ nằm lại trong
        // notification; edge kế tiếp dính vào cùng bit -> mất một wakeup.
        let mut kernel = Kernel::new();
        let notification = kernel.create_notification().unwrap();
        let irq = kernel.create_irq(7).unwrap();
        kernel.irq_mut(irq).unwrap().notification = Some(notification);
        // Edge 1: pending bật, bit được đặt.
        assert!(kernel.irq_raise(7));
        // Edge 2 khi chưa ack: bị coalesce (đúng thiết kế).
        assert!(!kernel.irq_raise(7));
        // Ack khi người nhận CHƯA lấy bit: bit cũ phải bị xoá.
        assert_eq!(kernel.irq_ack(irq), Ok(()));
        assert_eq!(kernel.poll(notification), Ok(0));
        // Edge mới sau ack phải tạo lại bit (không bị "dính" bit cũ).
        assert!(kernel.irq_raise(7));
        assert_eq!(kernel.poll(notification), Ok(1 << 7));
    }

    #[test]
    fn round101_resume_rejects_blocked_and_running_targets() {
        // Vòng 101 (K32/K33/K34/K36): resume mù quáng từng ép Ready +
        // enqueue mọi state mà không gỡ hàng chờ (K32), alias current +
        // queued trên Running (K33), resume ngay sau suspend-Running
        // (K34). Giờ chỉ Inactive/Suspended/Ready được resume.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        // BlockedRecv: cài endpoint rồi recv để park.
        let a = kernel.create_tcb("a", 5).unwrap();
        let ep = kernel.create_endpoint().unwrap();
        kernel
            .configure_tcb(a, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(kernel.recv(a, ep, 0), Ok(None));
        assert_eq!(kernel.resume_tcb(a), Err(KError::IllegalOperation));
        // Running: cài current rồi resume phải từ chối (K33).
        let b = kernel.create_tcb("b", 5).unwrap();
        kernel
            .configure_tcb(b, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(b);
        assert_eq!(kernel.pick_on(0), Some(b));
        assert_eq!(kernel.resume_tcb(b), Err(KError::IllegalOperation));
        // Suspended vẫn resume được (đường hợp lệ).
        let c = kernel.create_tcb("c", 5).unwrap();
        kernel
            .configure_tcb(c, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.tcb_mut(c).unwrap().state = ThreadState::Suspended;
        assert_eq!(kernel.resume_tcb(c), Ok(()));
        // BlockedFault cũng resume được (fault handler xử lý xong cho chạy
        // lại — pager flow: server frame_map xong tcb_resume(client)).
        let d = kernel.create_tcb("d", 5).unwrap();
        kernel
            .configure_tcb(d, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.tcb_mut(d).unwrap().state = ThreadState::BlockedFault;
        assert_eq!(kernel.resume_tcb(d), Ok(()));
        assert!(kernel.check_invariants().is_ok());
        let _ = root;
    }

    #[test]
    fn round50_suspend_blocked_reply_is_rejected() {
        // Mốc 1 (vòng 50): suspend client đang BlockedReply phải
        // IllegalOperation — nếu không server ôm caller ma: client bị treo
        // mà `reply` đòi BlockedReply+epoch nên server treo theo, còn
        // client thật thì kẹt Suspended không ai đánh thức.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        kernel
            .configure_tcb(server, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(client, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        let ep = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(server, ep, 0), Ok(None));
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, ep, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        // Suspend client BlockedReply bị cấm thẳng (kẻ tấn công không treo
        // được rendezvous đang bay).
        assert_eq!(
            kernel.suspend_tcb(client, root),
            Err(KError::IllegalOperation)
        );
        // Rendezvous còn nguyên: server reply xong client nhận đúng.
        let reply = Message::new(0x1111, &[1]).unwrap();
        assert_eq!(kernel.reply(server, reply), Ok(()));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::Ready);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_restart_detaches_queues_and_clears_stale_caller() {
        // Vòng 101 (K35/K36): restart từng chỉ remove_from_runq mà không
        // gỡ endpoint.send/recv hay waiters (K35), và restart client
        // BlockedReply để lại server treo caller ma (K36).
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        // K35: BlockedRecv restart xong không còn trong hàng recv.
        let blocked = kernel.create_tcb("blocked", 5).unwrap();
        let waiter = kernel.create_tcb("waiter", 5).unwrap();
        kernel
            .configure_tcb(blocked, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(waiter, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        let ep = kernel.create_endpoint().unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.recv(blocked, ep, 0), Ok(None));
        assert_eq!(kernel.wait_as(blocked, ntfn), Ok(None));
        assert_eq!(
            kernel.restart_tcb(blocked, waiter),
            Ok((0x4000_0000, 0x4000_4000, 0, 0))
        );
        // Hàng recv và waiters đã sạch: recv sau đó park lại từ đầu.
        assert_eq!(kernel.recv(waiter, ep, 0), Ok(None));
        assert_eq!(kernel.poll(ntfn), Ok(0));
        // K36: restart client BlockedReply thì server không treo caller.
        // (Endpoint riêng — phần K35 đã park `waiter` trên `ep` cũ.)
        let ep2 = kernel.create_endpoint().unwrap();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        kernel
            .configure_tcb(server, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(client, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(kernel.recv(server, ep2, 0), Ok(None));
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, ep2, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        // Restart client bị cấm thẳng (client đang BlockedReply).
        assert_eq!(
            kernel.restart_tcb(client, server),
            Err(KError::IllegalOperation)
        );
        // Server reply rồi recv tiếp phải sạch (không kẹt caller cũ).
        let reply = Message::new(0x1111, &[1]).unwrap();
        assert_eq!(kernel.reply(server, reply), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_bind_notification_rejects_alias_dead_and_double() {
        // Vòng 101 (K43/K44/K51): bind N-TCB alias, bind object chết, và
        // nhánh cấm waiter thừa (K51, giờ cho bind khi có waiter).
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("a", 5).unwrap();
        let b = kernel.create_tcb("b", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.bind_notification(a, ntfn), Ok(()));
        // K43: TCB khác bind cùng notification -> AlreadyMapped.
        assert_eq!(
            kernel.bind_notification(b, ntfn),
            Err(KError::AlreadyMapped)
        );
        // K51: waiter tồn tại không chặn bind đầu tiên (đã bind ở trên
        // trước khi có waiter); bind lại chính nó -> AlreadyMapped.
        assert_eq!(kernel.wait_as(a, ntfn), Ok(None));
        assert_eq!(
            kernel.bind_notification(a, ntfn),
            Err(KError::AlreadyMapped)
        );
        // K44: object chết phải lỗi, không Ok ma.
        let dead = kernel.create_tcb("dead", 5).unwrap();
        let ntfn2 = kernel.create_notification().unwrap();
        kernel.destroy_object(dead).unwrap();
        assert!(kernel.bind_notification(dead, ntfn2).is_err());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_irq_rejects_number_above_63() {
        // Vòng 101 (K47): number & 63 alias bit — 7 và 71 chung bit 7.
        assert!(Kernel::new().create_irq(71).is_none());
        assert!(Kernel::new().create_irq(64).is_none());
        assert!(Kernel::new().create_irq(63).is_some());
    }

    #[test]
    fn round101_grant_failure_keeps_fifo_order() {
        // Vòng 101 (K48): grant fail đẩy về đuôi làm mất FIFO — giờ trả về
        // đầu hàng (push_front). Kịch bản: cả 2 sender park KHÔNG grant,
        // receiver pop s1 rồi grant-fail (slot 9 của s1 trống) -> s1 phải
        // về đầu hàng, recv sau vẫn gặp s1 (không bị s2 vượt).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let ep = kernel.create_endpoint().unwrap();
        let s1 = kernel.create_tcb("s1", 5).unwrap();
        let s2 = kernel.create_tcb("s2", 5).unwrap();
        // Hai sender park trước (không có receiver chờ -> vào hàng).
        assert_eq!(kernel.send(s1, ep, 0, Message::default(), false), Ok(false));
        assert_eq!(kernel.send(s2, ep, 0, Message::default(), false), Ok(false));
        // Gài grant vào pending của s1 sau khi đã park (vượt gate GRANT
        // lúc queue — đúng bản chất "fail lúc dequeue" của K48).
        {
            let table = kernel.tcb_mut(s1).unwrap();
            let mut pending = table.pending.take().unwrap();
            pending.can_grant = true;
            pending.msg.cap = 9; // slot trống -> grant fail lúc recv
            table.pending = Some(pending);
        }
        let before = kernel.endpoint(ep).unwrap().send.front();
        assert_eq!(before, Some(s1));
        assert!(kernel.recv(root, ep, 7).is_err());
        let after = kernel.endpoint(ep).unwrap().send.front();
        assert_eq!(after, Some(s1));
        // s2 vẫn ở sau s1 — FIFO giữ nguyên.
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_frame_write_rejects_executable_frame() {
        // Vòng 101 (K57): copy_frame_bytes vào frame đang mapped-X.
        let mut kernel = Kernel::new();
        let frame = kernel.create_frame(0x9000, 4096).unwrap();
        // Giả mapped-X bằng cách set counter (không cần page tables thật).
        kernel.set_frame_mapping_counts(frame, 0, 1);
        assert_eq!(
            kernel.copy_frame_bytes(frame, 0, &[1, 2, 3]),
            Err(KError::NoRights)
        );
        kernel.set_frame_mapping_counts(frame, 0, 0);
        // Không có page_table_ops trên host test -> MissingPageTable là
        // đúng (đã qua gate mapped_x).
        assert_eq!(
            kernel.copy_frame_bytes(frame, 0, &[1, 2, 3]),
            Err(KError::MissingPageTable)
        );
    }

    #[test]
    fn round101_destroy_notification_clears_bound_and_irq_routing() {
        // Vòng 101 (K67/K68): destroy Notification phải gỡ bound_ntfn mọi
        // TCB và irq.notification — nếu không TCB ôm Some(chết) (bind mới
        // mãi AlreadyMapped), IRQ ôm Some(chết) (raise rớt vào hố đen).
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("t", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        let irq = kernel.create_irq(7).unwrap();
        kernel.bind_notification(tcb, ntfn).unwrap();
        kernel.set_irq_notification(irq, ntfn).unwrap();
        kernel.destroy_object(ntfn).unwrap();
        assert_eq!(kernel.tcb(tcb).unwrap().bound_ntfn, None);
        assert_eq!(kernel.irq(irq).unwrap().notification, None);
        // Bind lại notification mới phải Ok (không kẹt AlreadyMapped).
        let ntfn2 = kernel.create_notification().unwrap();
        assert_eq!(kernel.bind_notification(tcb, ntfn2), Ok(()));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_revoke_live_iommu_domain_is_refused() {
        // Vòng 101 (K56): Untyped-revoke không được giết domain còn mapping
        // sống — can_destroy chặn, PA không tái cấp trong khi IOTLB cũ còn
        // trỏ. (Revoke vé Iommu trực tiếp chỉ thu hồi vé, object sống nếu
        // còn vé khác — đó là ngữ nghĩa đúng.)
        let mut kernel = Kernel::new();
        let root = test_root_with_untyped(&mut kernel, 0x740_0000, 16);
        let frame = retype_at(&mut kernel, root, ObjType::Frame, 2);
        let domain = retype_at(&mut kernel, root, ObjType::Iommu, 3);
        kernel
            .iommu_map(domain, frame, IOMMU_IOVA_BASE, Rights::READ | Rights::WRITE)
            .unwrap();
        // Revoke từ vé Untyped (slot 1) phải bị chặn vì domain còn sống.
        assert!(kernel.revoke_cap(root, 1).is_err());
        assert!(kernel
            .iommu(domain)
            .unwrap()
            .find(IOMMU_IOVA_BASE)
            .is_some());
        // Unmap hết rồi revoke phải Ok.
        kernel.iommu_unmap(domain, IOMMU_IOVA_BASE).unwrap();
        assert!(kernel.revoke_cap(root, 1).is_ok());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round101_grant_slot_swap_is_rejected() {
        // Vòng 101 (K49): sender park với grant trỏ slot 2 (vé A). Giữa
        // chừng slot 2 bị đổi sang vé B (vẫn GRANT) — recv phải từ chối
        // (NoRights) thay vì chuyển nhầm vé B cho receiver.
        let mut kernel = Kernel::new();
        let ep = kernel.create_endpoint().unwrap();
        let sender = kernel.create_tcb("sender", 5).unwrap();
        let receiver = kernel.create_tcb("receiver", 5).unwrap();
        let obj_a = kernel.create_endpoint().unwrap();
        let obj_b = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(sender, 2, Cap::new(obj_a, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        // Park với grant (msg.cap = slot 2).
        let mut msg = Message::default();
        msg.cap = 2;
        assert_eq!(kernel.send(sender, ep, 0, msg, false), Ok(false));
        // Đổi slot 2 sang vé B: gỡ A (vé gốc không có con nên gỡ được),
        // cài B (id khác, object khác).
        kernel.delete_cap(sender, 2).unwrap();
        kernel
            .install_cap(sender, 2, Cap::new(obj_b, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        // Receiver lấy message: grant verify snapshot (A) vs hiện tại (B)
        // lệch nhau -> phải lỗi, sender về đầu hàng.
        assert_eq!(kernel.recv(receiver, ep, 7), Err(KError::NoRights));
        let front = kernel.endpoint(ep).unwrap().send.front();
        assert_eq!(front, Some(sender));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round102_exit_current_picks_next_and_zombies() {
        // Mốc 1: Exit chỉ kết thúc task gọi — hart chọn task khác, task cũ
        // thành zombie Exited (reap sau qua destroy_object), không kill máy.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let a = kernel.create_tcb("a", 5).unwrap();
        let b = kernel.create_tcb("b", 5).unwrap();
        kernel
            .configure_tcb(a, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(b, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(a);
        kernel.enqueue(b);
        assert_eq!(kernel.pick_on(0), Some(a));
        // Client block chờ a (server) qua call, rồi a exit: client phải
        // được wake lỗi thay vì treo BlockedReply vô hạn.
        let ep = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(a, ep, 0), Ok(None));
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(b, ep, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(b).unwrap().state, ThreadState::BlockedReply);
        // a đang BlockedRecv (park ở recv) — mô phỏng a đang chạy trên hart
        // bằng cách đặt current trực tiếp (test cùng crate), rồi exit.
        kernel.tcb_mut(a).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(a);
        // a đang giữ caller của b; exit phải wake b lỗi.
        let next = kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::Exited);
        assert!(kernel.check_invariants().is_ok());
        // b được wake lỗi (resume_error), không treo. pick_on cuối
        // exit_current_on đã chọn b chạy tiếp nên b là Running.
        let b_state = kernel.tcb(b).unwrap();
        assert_eq!(b_state.state, ThreadState::Running);
        assert_eq!(b_state.resume_error, Some(KError::ObjectDestroyed));
        assert_eq!(next, Some(b));
        // Zombie reap được qua destroy_object.
        kernel.destroy_object(a).unwrap();
        assert!(kernel.tcb(a).is_none());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round102_exited_rejects_resume_and_suspend() {
        // Task Exited không resume/suspend được nữa, nhưng restart được
        // (tái sinh như từ Inactive).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("t", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(t);
        assert_eq!(kernel.pick_on(0), Some(t));
        kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Exited);
        assert_eq!(kernel.resume_tcb(t), Err(KError::IllegalOperation));
        assert_eq!(kernel.suspend_tcb(t, root), Err(KError::IllegalOperation));
        assert_eq!(
            kernel.restart_tcb(t, root),
            Ok((0x4000_0000, 0x4000_4000, 0, 0))
        );
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Ready);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn r251_pick_on_skips_non_ready_entries() {
        // K40 (nửa sửa được): `best_in_queue`/đường steal trước đây chọn TCB
        // theo priority mà KHÔNG xét trạng thái, nên một entry rác mang trạng
        // thái Suspended vẫn được cấp CPU. Nay chỉ `Ready` mới được chọn.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("t", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(t);
        // Dựng đúng trạng thái rác: có mặt trong runqueue nhưng Suspended.
        kernel.tcb_mut(t).unwrap().state = ThreadState::Suspended;
        assert_eq!(kernel.pick_on(0), None);
        // Trả về Ready thì chạy lại được bình thường.
        kernel.tcb_mut(t).unwrap().state = ThreadState::Ready;
        assert_eq!(kernel.pick_on(0), Some(t));
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Running);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn r251_running_without_current_is_a_valid_state() {
        // Vì sao K40 KHÔNG thể thành bất biến "Running ⇒ phải là current":
        // kernel hợp lệ có TCB Running mà không hart nào giữ `current`.
        // Đo được: thêm bất biến đó làm `selftest_passes`,
        // `per_hart_runqueue_serves_hart2_and_hart3`,
        // `round69_steal_respects_affinity_and_counts_stats` đỏ ngay.
        // Test này khoá lại SỰ THẬT ĐÃ ĐO để lần sau không ai thêm lại.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("t", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(t);
        assert_eq!(kernel.pick_on(0), Some(t));
        // Bỏ `current` nhưng giữ Running — trạng thái này được CHẤP NHẬN.
        kernel.current[0] = None;
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Running);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn r246_set_tcb_args_validates_bounds_and_lifetime() {
        // D15: argv chỉ nhận TCB đã configure nhưng CHƯA chạy, argc <= 8,
        // argv canh 8 B, và argc != 0 phải có argv.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("t", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(kernel.set_tcb_args(t, 0, 0), Ok(()));
        assert_eq!(
            kernel.set_tcb_args(t, 9, 0x4000_3000),
            Err(KError::InvalidArg)
        );
        assert_eq!(kernel.set_tcb_args(t, 2, 0), Err(KError::InvalidArg));
        assert_eq!(
            kernel.set_tcb_args(t, 2, 0x4000_3004),
            Err(KError::InvalidArg)
        );
        assert_eq!(kernel.set_tcb_args(t, 2, 0x4000_3000), Ok(()));
        assert_eq!(kernel.tcb(t).unwrap().argc, 2);
        assert_eq!(kernel.tcb(t).unwrap().argv, 0x4000_3000);
        // Configure lại = chương trình mới ⇒ tham số bị xoá.
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(kernel.tcb(t).unwrap().argc, 0);
        assert_eq!(kernel.tcb(t).unwrap().argv, 0);
        // Task đã chạy thì từ chối (không ghi đè context sống).
        assert_eq!(kernel.set_tcb_args(t, 1, 0x4000_3000), Ok(()));
        kernel.enqueue(t);
        assert_eq!(kernel.pick_on(0), Some(t));
        assert_eq!(
            kernel.set_tcb_args(t, 1, 0x4000_3000),
            Err(KError::IllegalOperation)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_tcb_wait_rendezvous() {
        // Mốc 1: parent TcbWait con — con sống thì park, con exit thì
        // parent được wake lỗi (ObjectDestroyed) và có thể reap.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let parent = kernel.create_tcb("parent", 5).unwrap();
        let child = kernel.create_tcb("child", 5).unwrap();
        kernel
            .configure_tcb(parent, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(child, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        // Con đã exit: wait xong ngay.
        kernel.enqueue(child);
        assert_eq!(kernel.pick_on(0), Some(child));
        kernel.exit_current_on(0);
        assert_eq!(kernel.wait_tcb(parent, child), Ok(true));
        // Con sống: wait park parent.
        let child2 = kernel.create_tcb("child2", 5).unwrap();
        kernel
            .configure_tcb(child2, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.tcb_mut(child2).unwrap().state = ThreadState::Suspended;
        kernel.resume_tcb(child2).unwrap();
        // Đặt parent làm current trực tiếp (scheduler có thể chọn child2
        // trước vì resume sớm hơn — không quan trọng với kịch bản wait).
        kernel.tcb_mut(parent).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(parent);
        assert_eq!(kernel.wait_tcb(parent, child2), Ok(false));
        assert_eq!(kernel.tcb(parent).unwrap().state, ThreadState::BlockedReply);
        // Waiter thứ hai bị từ chối (một waiter tại một thời điểm).
        let other = kernel.create_tcb("other", 5).unwrap();
        kernel
            .configure_tcb(other, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(kernel.wait_tcb(other, child2), Err(KError::WouldBlock));
        // Destroy con đang có waiter bị chặn — waiter phải xong trước.
        kernel
            .install_cap(parent, 1, Cap::new(child2, ObjType::Tcb, Rights::ALL))
            .unwrap();
        assert_eq!(
            kernel.user_syscall(parent, SyscallId::TcbDestroy as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::WouldBlock
            }
        );
        // Con exit: parent được wake lỗi.
        kernel.tcb_mut(child2).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(child2);
        kernel.exit_current_on(0);
        assert_eq!(
            kernel.tcb(parent).unwrap().resume_error,
            Some(KError::ObjectDestroyed)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round53_nested_wait_and_inactive_target_rejected() {
        // Mốc 1 (vòng 53): waiter đang BlockedReply (đã wait/call ai đó)
        // mà wait tiếp phải WouldBlock — nếu không một waiter ôm hai
        // rendezvous, exit một bên không biết đánh thức đường nào.
        // Wait target Inactive phải IllegalOperation (task chưa từng chạy).
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let a = kernel.create_tcb("a", 5).unwrap();
        let b = kernel.create_tcb("b", 5).unwrap();
        let c = kernel.create_tcb("c", 5).unwrap();
        for (t, entry) in [(a, 0x4000_0000u64), (b, 0x4000_1000), (c, 0x4000_2000)] {
            kernel
                .configure_tcb(t, entry, 0x4000_4000, space, 5)
                .unwrap();
        }
        // a wait b: park thành công (resume đã enqueue, không enqueue
        // tay hai lần).
        kernel.resume_tcb(b).unwrap();
        kernel.tcb_mut(a).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(a);
        assert_eq!(kernel.wait_tcb(a, b), Ok(false));
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::BlockedReply);
        // a đang BlockedReply mà wait c tiếp → WouldBlock, c không bị
        // gán caller (không ôm rendezvous thứ hai). c phải Ready trước để
        // target-check qua được và lộ đúng nhánh waiter-check.
        kernel.resume_tcb(c).unwrap();
        assert_eq!(kernel.wait_tcb(a, c), Err(KError::WouldBlock));
        assert!(kernel.tcb(c).unwrap().caller.is_none());
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::BlockedReply);
        // Wait target Inactive (chưa resume bao giờ) → IllegalOperation.
        let fresh = kernel.create_tcb("fresh", 5).unwrap();
        kernel
            .configure_tcb(fresh, 0x4000_3000, 0x4000_4000, space, 5)
            .unwrap();
        let watcher = kernel.create_tcb("watcher", 5).unwrap();
        kernel
            .configure_tcb(watcher, 0x4000_3100, 0x4000_4000, space, 5)
            .unwrap();
        assert_eq!(
            kernel.wait_tcb(watcher, fresh),
            Err(KError::IllegalOperation)
        );
        // Wait park waiter đang Running nhưng giữ current (runtime tiếp
        // tục schedule từ đó) — invariants đòi current là Running nên
        // chuyển hart sang task khác trước khi check, như scheduler thật.
        assert_eq!(kernel.pick_on(0), Some(b));
        kernel.tcb_mut(b).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(b);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round55_destroy_busy_server_wakes_client_clean() {
        // Mốc 1 (vòng 55): destroy server đang giữ caller (client call chờ
        // reply) — client phải wake lỗi ObjectDestroyed + Ready, không treo
        // BlockedReply với caller ma; server biến mất khỏi mọi hàng chờ.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        kernel
            .configure_tcb(server, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel
            .configure_tcb(client, 0x4000_1000, 0x4000_4000, space, 5)
            .unwrap();
        let ep = kernel.create_endpoint().unwrap();
        assert_eq!(kernel.recv(server, ep, 0), Ok(None));
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, ep, 0, call, true), Ok(true));
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
        // Destroy server đang BlockedRecv giữ caller=client.
        assert_eq!(kernel.destroy_object(server), Ok(()));
        // Client wake lỗi + Ready, caller đã gỡ.
        let table = kernel.tcb(client).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert_eq!(table.resume_error, Some(KError::ObjectDestroyed));
        assert!(table.caller.is_none());
        // Server không còn trong hàng recv; client có thể recv lại sạch.
        assert_eq!(kernel.recv(client, ep, 0), Ok(None));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round62_wait_self_is_rejected() {
        // Mốc 1 (vòng 62): TcbWait tự chờ chính mình phải IllegalOperation
        // — nếu không task tự gán caller=(mình) + park BlockedReply, tự
        // deadlock không ai wake được (exit cũng chỉ wake caller đã chết).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("lonely", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.resume_tcb(t).unwrap();
        assert_eq!(kernel.pick_on(0), Some(t));
        // Mức kernel lẫn mức syscall đều cấm.
        assert_eq!(kernel.wait_tcb(t, t), Err(KError::IllegalOperation));
        kernel
            .install_cap(t, 1, Cap::new(t, ObjType::Tcb, Rights::READ))
            .unwrap();
        kernel.tcb_mut(t).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(t);
        assert_eq!(
            kernel.user_syscall(t, SyscallId::TcbWait as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::IllegalOperation,
            }
        );
        // Task còn nguyên, không dính caller tự thân.
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Running);
        assert!(kernel.tcb(t).unwrap().caller.is_none());
        let _ = root;
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round64_restart_current_is_rejected() {
        // Mốc 1 (vòng 64): restart task đang là current trên bất kỳ hart
        // nào phải IllegalOperation — nếu không hart đó quay lại user với
        // context cũ trong khi TCB đã bị reset về Ready (alias current,
        // cùng họ K33 của resume).
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("runner", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.resume_tcb(t).unwrap();
        assert_eq!(kernel.pick_on(0), Some(t));
        kernel.tcb_mut(t).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(t);
        // Restart current qua mức kernel lẫn syscall đều cấm.
        assert_eq!(kernel.restart_tcb(t, root), Err(KError::IllegalOperation));
        kernel
            .install_cap(root, 1, Cap::new(t, ObjType::Tcb, Rights::WRITE))
            .unwrap();
        // Restart trả UserAction::RestartTcb khi thành công; ở đây phải
        // là Return + IllegalOperation.
        assert!(matches!(
            kernel.user_syscall(root, SyscallId::TcbRestart as u64, 1, 0, 0, 0, 0, 0),
            UserAction::Return {
                error: KError::IllegalOperation,
                ..
            }
        ));
        // Task còn nguyên Running, current giữ nguyên.
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Running);
        assert_eq!(kernel.current[0], Some(t));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_reply_carries_grant_cap() {
        // Mốc 5: server reply mang cap GRANT cho client qua recv_slot.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let ep = kernel.create_endpoint().unwrap();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        let obj = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(root, 1, Cap::new(ep, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(server, 2, Cap::new(obj, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        assert_eq!(kernel.recv(server, ep, 7), Ok(None));
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, ep, 0, call, true), Ok(true));
        kernel.tcb_mut(client).unwrap().recv_slot = 5;
        let mut reply = Message::default();
        reply.cap = 2;
        assert_eq!(kernel.reply(server, reply), Ok(()));
        let got = kernel
            .cap_at(SlotAddr {
                tcb: client,
                slot: 5,
            })
            .expect("grant installed");
        assert_eq!(got.obj, obj);
        assert!(!got.rights.contains(Rights::GRANT));
        assert!(kernel.check_invariants().is_ok());
        // Reply vé không GRANT thì từ chối, caller đã take (server giữ
        // nguyên, client vẫn BlockedReply chờ reply khác).
        let obj2 = kernel.create_endpoint().unwrap();
        kernel
            .install_cap(
                server,
                3,
                Cap::new(obj2, ObjType::Endpoint, Rights::READ | Rights::WRITE),
            )
            .unwrap();
        assert_eq!(kernel.recv(server, ep, 7), Ok(None));
        assert_eq!(kernel.send(client, ep, 0, call, true), Ok(true));
        let mut bad = Message::default();
        bad.cap = 3;
        assert_eq!(kernel.reply(server, bad), Err(KError::NoRights));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_exit_while_sleeping_never_revives() {
        // Mốc 1: task BlockedSleep exit thì zombie Exited không bị
        // wake_sleepers/wake_budgeters đánh thức lại (state đã Exited,
        // wake_at đã xóa). Tương tự với BlockedBudget.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let sleeper = kernel.create_tcb("sleeper", 5).unwrap();
        kernel
            .configure_tcb(sleeper, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        let other = kernel.create_tcb("other", 5).unwrap();
        kernel
            .configure_tcb(other, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(sleeper);
        kernel.enqueue(other);
        assert_eq!(kernel.pick_on(0), Some(sleeper));
        // Park ngủ qua syscall Sleep (timeout tương đối 100 ticks; cần có
        // task khác ready nếu không Sleep trả WouldBlock).
        assert_eq!(
            kernel.user_syscall(sleeper, SyscallId::Sleep as u64, 100, 0, 0, 0, 0, 0),
            UserAction::Sleep
        );
        assert_eq!(
            kernel.tcb(sleeper).unwrap().state,
            ThreadState::BlockedSleep
        );
        // Đặt làm current rồi exit.
        kernel.tcb_mut(sleeper).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(sleeper);
        kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(sleeper).unwrap().state, ThreadState::Exited);
        // Quay thời gian qua deadline: zombie không sống lại.
        let _ = kernel.tick(0);
        for _ in 0..10 {
            let _ = kernel.tick(0);
        }
        assert_eq!(kernel.tcb(sleeper).unwrap().state, ThreadState::Exited);
        assert!(kernel.check_invariants().is_ok());
        // Zombie giữ sched_context bound vẫn consistent, reap giải phóng.
        kernel.destroy_object(sleeper).unwrap();
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round58_destroy_sleeping_tcb_never_revives() {
        // Mốc 1 (vòng 58): destroy TCB đang BlockedSleep — object free
        // luôn, wake_sleepers sau này không thể đánh thức (gen đã tăng,
        // không còn TCB ở slot đó); không rò rỉ vào runqueue.
        let mut kernel = Kernel::new();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let sleeper = kernel.create_tcb("sleeper", 5).unwrap();
        kernel
            .configure_tcb(sleeper, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        let other = kernel.create_tcb("other", 5).unwrap();
        kernel
            .configure_tcb(other, 0x4000_1000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.enqueue(sleeper);
        kernel.enqueue(other);
        assert_eq!(kernel.pick_on(0), Some(sleeper));
        assert_eq!(
            kernel.user_syscall(sleeper, SyscallId::Sleep as u64, 100, 0, 0, 0, 0, 0),
            UserAction::Sleep
        );
        assert_eq!(
            kernel.tcb(sleeper).unwrap().state,
            ThreadState::BlockedSleep
        );
        // Destroy thẳng sleeper đang ngủ (không phải current): pick
        // other ra khỏi runq rồi mới đánh dấu Running (tránh vừa
        // Running vừa queued — luật K1).
        assert_eq!(kernel.pick_on(0), Some(other));
        kernel.tcb_mut(other).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(other);
        let gen_before = sleeper.gen;
        assert_eq!(kernel.destroy_object(sleeper), Ok(()));
        // Quay thời gian qua deadline: slot cũ đã free, không revive.
        for _ in 0..200 {
            let _ = kernel.tick(0);
        }
        assert!(kernel.tcb(sleeper).is_none());
        // Retype slot đó cho TCB mới: gen tăng, state sạch Inactive.
        let fresh = kernel.create_tcb("fresh", 5).unwrap();
        assert!(fresh.gen != gen_before);
        assert_eq!(kernel.tcb(fresh).unwrap().state, ThreadState::Inactive);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_destroy_tcb_cleans_all_binds_and_queues() {
        // Mốc 1/2: destroy TCB đang park BlockedRecv + bound notification +
        // ôm caller của client + giữ sched_context bound — mọi phía đều
        // được dọn (detach_tcb), không còn ref treo, invariant sạch.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let ep = kernel.create_endpoint().unwrap();
        let ntfn = kernel.create_notification().unwrap();
        let server = kernel.create_tcb("server", 5).unwrap();
        let client = kernel.create_tcb("client", 5).unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let context = kernel.retype(untyped, ObjType::SchedContext).unwrap();
        kernel.configure_sched_context(context, 100, 100).unwrap();
        kernel.bind_sched_context(server, context).unwrap();
        // Server park recv + bound notification.
        kernel
            .install_cap(root, 1, Cap::new(ep, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        assert_eq!(kernel.recv(server, ep, 7), Ok(None));
        kernel
            .install_cap(root, 2, Cap::new(ntfn, ObjType::Notification, Rights::ALL))
            .unwrap();
        kernel.bind_notification(server, ntfn).unwrap();
        // Client call vào server (server ôm caller).
        let call = Message::new(0x1234, &[7]).unwrap();
        assert_eq!(kernel.send(client, ep, 0, call, true), Ok(true));
        assert!(kernel.tcb(server).unwrap().caller.is_some());
        // Destroy server: client được wake lỗi, notification unbound,
        // sched_context unbound, hàng recv sạch.
        kernel.destroy_object(server).unwrap();
        assert!(kernel.tcb(server).is_none());
        let client_table = kernel.tcb(client).unwrap();
        assert_eq!(client_table.state, ThreadState::Ready);
        assert_eq!(client_table.resume_error, Some(KError::ObjectDestroyed));
        assert!(kernel.sched_context(context).unwrap().bound.is_none());
        assert!(kernel
            .notification(ntfn)
            .unwrap()
            .waiters
            .iter()
            .all(|slot| slot.is_none()));
        assert!(kernel.endpoint(ep).unwrap().recv.is_empty());
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_restart_from_exited_rebirths_clean() {
        // Mốc 1: restart zombie Exited tái sinh sạch — Ready + enqueue,
        // pending/resume/fds/wake_at reset, entry/stack giữ, bound_ntfn giữ
        // (thuộc tính task), sched_context giữ bind.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("t", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        let ntfn = kernel.create_notification().unwrap();
        let untyped = kernel
            .create_untyped_with_limit(0x1000_0000, 4096, 4096, 8)
            .unwrap();
        let context = kernel.retype(untyped, ObjType::SchedContext).unwrap();
        kernel.configure_sched_context(context, 100, 100).unwrap();
        kernel.bind_sched_context(t, context).unwrap();
        kernel.bind_notification(t, ntfn).unwrap();
        kernel.enqueue(t);
        assert_eq!(kernel.pick_on(0), Some(t));
        kernel.tcb_mut(t).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(t);
        // Bẩn hóa đời cũ: pending + resume_error + wake_at + fd giả.
        kernel.tcb_mut(t).unwrap().pending = Some(PendingSend {
            badge: 0,
            msg: Message::empty(0),
            is_call: false,
            can_grant: false,
            grant_key: 0,
        });
        kernel.tcb_mut(t).unwrap().resume_error = Some(KError::Ok);
        kernel.tcb_mut(t).unwrap().wake_at = Some(999);
        kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Exited);
        assert!(kernel.tcb(t).unwrap().pending.is_none());
        // Restart tái sinh.
        let (entry, stack, _argc, _argv) = kernel.restart_tcb(t, root).unwrap();
        assert_eq!((entry, stack), (0x4000_0000, 0x4000_4000));
        let table = kernel.tcb(t).unwrap();
        assert_eq!(table.state, ThreadState::Ready);
        assert!(table.queued);
        assert_eq!(table.resume, None);
        assert_eq!(table.resume_error, None);
        assert_eq!(table.wake_at, None);
        assert_eq!(table.bound_ntfn, Some(ntfn));
        assert_eq!(table.sched_context, Some(context));
        assert!(kernel.check_invariants().is_ok());
        // Task tái sinh chạy được.
        assert_eq!(kernel.pick_on(0), Some(t));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round42_restart_from_ready_rebirths_runnable() {
        // Mốc 1 (vòng 42): `run` dồn dập restart TCB khi nó còn Ready
        // (chưa kịp chạy sau resume) — phải Ready + queued + chạy được,
        // không IllegalOperation, invariants giữ.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("exec", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.resume_tcb(t).unwrap();
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Ready);
        // Restart khi còn Ready (chưa pick lần nào).
        let (entry, stack, _argc, _argv) = kernel.restart_tcb(t, root).unwrap();
        assert_eq!((entry, stack), (0x4000_0000, 0x4000_4000));
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Ready);
        // Tái sinh chạy được ngay.
        assert_eq!(kernel.pick_on(0), Some(t));
        assert!(kernel.check_invariants().is_ok());
        // Exit rồi restart lại (đời zombie) vẫn sạch.
        kernel.exit_current_on(0);
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Exited);
        kernel.restart_tcb(t, root).unwrap();
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Ready);
        assert_eq!(kernel.pick_on(0), Some(t));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round44_restart_running_is_rejected_and_current_untouched() {
        // Mốc 1 (vòng 44): restart TCB đang Running phải IllegalOperation
        // (kẻ tấn công không thể cướp context hart khác), `current` giữ
        // nguyên, invariants giữ.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let space = kernel
            .create_user_vspace(0x1000, 0x4000_a000, 4096)
            .unwrap();
        let t = kernel.create_tcb("runner", 5).unwrap();
        kernel
            .configure_tcb(t, 0x4000_0000, 0x4000_4000, space, 5)
            .unwrap();
        kernel.resume_tcb(t).unwrap();
        assert_eq!(kernel.pick_on(0), Some(t));
        kernel.tcb_mut(t).unwrap().state = ThreadState::Running;
        kernel.current[0] = Some(t);
        assert_eq!(kernel.restart_tcb(t, root), Err(KError::IllegalOperation));
        assert_eq!(kernel.current[0], Some(t));
        assert_eq!(kernel.tcb(t).unwrap().state, ThreadState::Running);
        // Restart chính caller cũng bị cấm (tự sát qua restart).
        assert_eq!(
            kernel.restart_tcb(root, root),
            Err(KError::IllegalOperation)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round103_failed_syscalls_return_zero_value() {
        // Mốc 7: syscall lỗi không được rò rỉ dữ liệu kernel qua `value`
        // (userspace nhận cả error lẫn value). Quét các họ syscall với
        // cap/slot sai: mọi lỗi phải đi kèm value = 0.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let bad_slot = 200u64; // vượt cspace
        let cases: &[(SyscallId, [u64; 6])] = &[
            (SyscallId::Send, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::Call, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::Recv, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::Reply, [0, 0, 0, 0, 0, 0]),
            (SyscallId::Wait, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::TcbResume, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::TcbSuspend, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::TcbDestroy, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::TcbWait, [bad_slot, 0, 0, 0, 0, 0]),
            (SyscallId::CapCopy, [bad_slot, 1, 1, 0, 0, 0]),
            (SyscallId::FrameMap, [bad_slot, bad_slot, 0, 0, 0, 0]),
            (SyscallId::BlockRead, [0, bad_slot, 0, 0, 0, 0]),
            (SyscallId::Open, [bad_slot, 0, 0, 0, 0, 0]),
        ];
        for (id, args) in cases {
            let action = kernel.user_syscall(
                root, *id as u64, args[0], args[1], args[2], args[3], args[4], args[5],
            );
            match action {
                UserAction::Return { value, error } => {
                    assert!(
                        error == KError::Ok || value == 0,
                        "{id:?}: error {error:?} leaks value {value:#x}"
                    );
                }
                // Blocked/Sleep/...: không trả value, miễn kiểm.
                _ => {}
            }
        }
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn round104_revoke_group_reclaims_tree_keeps_sibling() {
        // Mốc 1/2 (vòng 21): destroy-cả-nhóm qua revoke Untyped — nhóm A
        // (TCB + Frame con cháu) bị thu hồi cả cây, nhóm B (Untyped anh em
        // từ cùng cha) còn nguyên, invariant sạch.
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let parent = kernel
            .create_untyped_with_limit(0x900_0000, 0x20000, 4096, 64)
            .unwrap();
        kernel
            .install_cap(root, 1, Cap::new(parent, ObjType::Untyped, Rights::ALL))
            .unwrap();
        // Nhóm A: child untyped + TCB + Frame trong đó.
        let child_a = kernel
            .retype_untyped_child_as(parent, 0x8000, 16, None)
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(child_a, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let tcb_a = kernel.retype(child_a, ObjType::Tcb).unwrap();
        let frame_a = kernel.retype(child_a, ObjType::Frame).unwrap();
        // Nhóm B: child untyped anh em + TCB riêng.
        let child_b = kernel
            .retype_untyped_child_as(parent, 0x8000, 16, None)
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(child_b, ObjType::Untyped, Rights::ALL))
            .unwrap();
        let tcb_b = kernel.retype(child_b, ObjType::Tcb).unwrap();
        // Revoke nhóm A qua vé child_a (trả về số CAP bị xóa, không phải
        // số object — TCB/Frame tạo trực tiếp không có cap trỏ tới).
        let _ = kernel.revoke_cap(root, 2).unwrap();
        assert!(kernel.tcb(tcb_a).is_none());
        assert!(kernel.get(frame_a).is_none());
        // Nhóm B còn nguyên.
        assert!(kernel.tcb(tcb_b).is_some());
        assert!(kernel.untyped(child_b).is_some());
        assert!(kernel.check_invariants().is_ok());
        // Dọn nhóm B để không rò.
        kernel.revoke_cap(root, 3).unwrap();
        assert!(kernel.tcb(tcb_b).is_none());
        assert!(kernel.check_invariants().is_ok());
        let _ = frame_a;
    }

    #[test]
    fn round104_validate_user_range_rejects_wrap_and_empty() {
        // Mốc 7: validate_user_range phải từ chối wrap-around địa chỉ
        // (u64::MAX - N + M) và len = 0, kể cả khi mapping bên dưới có vẻ
        // hợp lệ — nếu không userspace đọc/ghi được ngoài vspace.
        let mut kernel = Kernel::new();
        let tcb = kernel.create_tcb("t", 5).unwrap();
        // Không vspace -> InvalidArg.
        assert_eq!(
            kernel.validate_user_range(tcb, 0x1000, 64, false),
            Err(KError::InvalidArg)
        );
        assert_eq!(
            kernel.validate_user_range(tcb, 0, 0, false),
            Err(KError::InvalidArg)
        );
        // Wrap-around: va gần MAX + len tràn.
        assert_eq!(
            kernel.validate_user_range(tcb, u64::MAX - 10, 64, false),
            Err(KError::InvalidArg)
        );
        assert_eq!(
            kernel.validate_user_range(tcb, u64::MAX, 1, true),
            Err(KError::InvalidArg)
        );
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn set_cap_badge_cannot_be_cleared_or_changed() {
        // Vòng 100 (K16): cho badge -> 0 rồi mint lại badge khác là đường rửa
        // danh tính (CapMint chỉ chặn mint từ nguồn đã có badge).
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
        assert_eq!(kernel.set_cap_badge(root, 1, 7), Ok(()));
        assert_eq!(kernel.set_cap_badge(root, 1, 0), Err(KError::NoRights));
        assert_eq!(kernel.set_cap_badge(root, 1, 9), Err(KError::NoRights));
        let cap = kernel
            .lookup_cap(root, 1, Some(ObjType::Endpoint), Rights::READ)
            .unwrap();
        assert_eq!(cap.badge, 7);
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
        // (#7) Shared clock advances by max-update from every hart:
        // BSP reached 2, AP reached 3, so system time is 3 on both harts.
        assert_eq!(av, 3);
        assert_eq!(bv, 3);
        kernel.tick(1);
        let b2 = kernel.user_syscall_on(b, 1, SyscallId::GetTime as u64, 0, 0, 0, 0, 0, 0);
        let UserAction::Return { value: bv2, .. } = b2 else {
            panic!("GetTime did not return");
        };
        // (#7) Every hart advances the shared clock by monotonic max-update,
        // so AP ticks DO advance system time — a stalled BSP no longer
        // freezes Sleep/GetTime/budget deadlines on other harts.
        // AP counter runs 3->4 here, so time becomes 4.
        assert_eq!(bv2, 4, "AP ticks advance system time by max-update");
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
                stack: 0x9000,
                argc: 0,
                argv: 0
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

    /// (#7) A sleeper must wake from an AP-only tick stream: `system_ticks`
    /// advances by monotonic max-update from every hart, so a BSP stalled
    /// behind the BKL no longer freezes Sleep deadlines on other harts.
    #[test]
    fn sleep_wakes_from_ap_only_ticks() {
        let mut kernel = Kernel::new();
        let sleeper = kernel.create_tcb("sleeper", 5).unwrap();
        let idle = kernel.create_tcb("idle", 1).unwrap();
        // Both TCBs may run on hart 1 so `Sleep` on hart 1 finds a ready
        // task there (`has_ready_on(1)`) and the AP tick stream can drive
        // the deadline without any BSP tick after the baseline.
        kernel.tcb_mut(sleeper).unwrap().affinity = 0b11;
        kernel.tcb_mut(idle).unwrap().affinity = 0b11;
        kernel.enqueue(idle);
        kernel.enqueue(sleeper);
        // One BSP tick sets the baseline (time = 1), then the BSP stalls.
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(
            kernel.user_syscall_on(sleeper, 1, SyscallId::Sleep as u64, 3, 0, 0, 0, 0, 0),
            UserAction::Sleep
        );
        assert_eq!(kernel.tcb(sleeper).unwrap().wake_at, Some(4));
        // Only hart 1 ticks from here on; the sleeper must still wake.
        // Baseline time is 1, deadline is 4: hart-1's private counter runs
        // 0->1->2->3->4, so the 4th AP tick reaches now=4 and wakes.
        assert_eq!(kernel.tick(1), 0);
        assert_eq!(kernel.tick(1), 0);
        assert_eq!(kernel.tick(1), 0);
        assert_eq!(kernel.tick(1), 1);
        assert_eq!(kernel.tcb(sleeper).unwrap().state, ThreadState::Ready);
    }

    #[test]
    fn issue2a_tick_idle_fast_path_without_scanning() {
        // (#2a) tick với 0 sleeper/budgeter: early-return 0, counter giữ 0,
        // không quét (sleepers_active/budgeters_active đều 0).
        let mut kernel = Kernel::new();
        assert_eq!(kernel.sleepers_active, 0);
        assert_eq!(kernel.budgeters_active, 0);
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.tick(1), 0);
        assert_eq!(kernel.sleepers_active, 0);
        assert_eq!(kernel.budgeters_active, 0);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue2a_sleep_counter_tracks_park_and_wake() {
        // (#2a) counter tăng khi Sleep park, giảm khi wake hết deadline.
        // Không đụng current[] để invariant current/Running luôn giữ.
        let mut kernel = Kernel::new();
        let sleeper = kernel.create_tcb("sleeper", 5).unwrap();
        let idle = kernel.create_tcb("idle", 1).unwrap();
        kernel.enqueue(idle);
        kernel.enqueue(sleeper);
        assert_eq!(kernel.tick(0), 0); // time = 1
        assert_eq!(kernel.sleepers_active, 0);
        assert_eq!(
            kernel.user_syscall_on(sleeper, 0, SyscallId::Sleep as u64, 2, 0, 0, 0, 0, 0),
            UserAction::Sleep
        );
        assert_eq!(kernel.sleepers_active, 1);
        assert_eq!(kernel.tick(0), 0); // time = 2, chưa tới deadline 3
        assert_eq!(kernel.sleepers_active, 1);
        assert_eq!(kernel.tick(0), 1); // time = 3, wake
        assert_eq!(kernel.sleepers_active, 0);
        assert_eq!(kernel.tcb(sleeper).unwrap().state, ThreadState::Ready);
        // tick sau khi wake vẫn fast-path.
        assert_eq!(kernel.tick(0), 0);
        assert_eq!(kernel.sleepers_active, 0);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue2a_budget_counter_tracks_exhaust_and_replenish() {
        // (#2a) counter tăng khi cạn budget, giảm khi wake đúng kỳ.
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
        kernel.configure_sched_context(context, 1, 3).unwrap();
        kernel.bind_sched_context(target, context).unwrap();
        assert_eq!(kernel.budgeters_active, 0);
        kernel.current[0] = Some(target);
        kernel.tcb_mut(target).unwrap().state = ThreadState::Running;
        assert_eq!(kernel.tick(0), 0); // cạn budget ngay (budget 1)
        assert_eq!(
            kernel.tcb(target).unwrap().state,
            ThreadState::BlockedBudget
        );
        assert_eq!(kernel.budgeters_active, 1);
        // Quay qua deadline period (period 3, deadline = period_start+3):
        // tick trả 1 khi wake, counter về 0.
        let mut woke = 0;
        for _ in 0..10 {
            woke += kernel.tick(0);
            if woke > 0 {
                break;
            }
        }
        assert_eq!(woke, 1);
        assert_eq!(kernel.budgeters_active, 0);
        assert_eq!(kernel.tcb(target).unwrap().state, ThreadState::Ready);
        kernel.current[0] = None;
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue2b_irq_map_roundtrip_create_raise_destroy() {
        // (#2b) create -> raise (qua map) -> destroy (dọn map) -> raise rớt.
        let mut kernel = Kernel::new();
        let number = 7u32;
        assert!(!kernel.irq_raise(number)); // chưa có object
        let irq = kernel.create_irq(number).unwrap();
        assert_eq!(kernel.irq_by_number[number as usize], Some(irq));
        assert!(kernel.irq_raise(number)); // edge mới qua map
        assert!(!kernel.irq_raise(number)); // coalesce: đang pending
        kernel.irq_ack(irq).unwrap();
        assert!(kernel.irq_raise(number)); // ack xong raise lại được
        kernel.destroy_object(irq).unwrap();
        assert_eq!(kernel.irq_by_number[number as usize], None);
        assert!(!kernel.irq_raise(number)); // object chết, raise rớt
                                            // number tái dùng được sau destroy.
        let irq2 = kernel.create_irq(number).unwrap();
        assert_eq!(kernel.irq_by_number[number as usize], Some(irq2));
        assert!(kernel.irq_raise(number));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue2b_irq_map_rejects_duplicate_and_big_number() {
        // (#2b) trùng number và number >= 64 đều từ chối, map không bẩn.
        let mut kernel = Kernel::new();
        let irq = kernel.create_irq(9).unwrap();
        assert!(kernel.create_irq(9).is_none());
        assert!(kernel.create_irq(64).is_none());
        assert!(kernel.create_irq(u32::MAX).is_none());
        assert_eq!(kernel.irq_by_number[9], Some(irq));
        assert!(!kernel.irq_raise(64));
        assert!(!kernel.irq_raise(71));
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue3b_tcb_count_and_slots_are_o1_cached() {
        // (#3b) counter/mask đồng bộ với create/destroy, slot tái dùng.
        let mut kernel = Kernel::new();
        let base = kernel.live_tcb_count; // root đã có từ reset
        assert!(base >= 1);
        let a = kernel.create_tcb("a", 1).unwrap();
        let b = kernel.create_tcb("b", 1).unwrap();
        assert_eq!(kernel.live_tcb_count, base + 2);
        assert_eq!(kernel.tcb_count(), base + 2);
        let slot_a = kernel.context_slot(a).unwrap();
        let _slot_b = kernel.context_slot(b).unwrap();
        assert_ne!(kernel.context_slot(a), kernel.context_slot(b));
        kernel.destroy_object(a).unwrap();
        assert_eq!(kernel.live_tcb_count, base + 1);
        let c = kernel.create_tcb("c", 1).unwrap();
        // slot của a được tái dùng (mask O(1) tìm đúng slot trống).
        assert_eq!(kernel.context_slot(c), Some(slot_a));
        assert_eq!(kernel.live_tcb_count, base + 2);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue12a_queue_remove_keeps_fifo_and_reports() {
        // (#12a) remove trả bool, giữ FIFO survivors, xoá mọi occurrence.
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("qa", 1).unwrap();
        let b = kernel.create_tcb("qb", 1).unwrap();
        let c = kernel.create_tcb("qc", 1).unwrap();
        let ep = kernel.create_endpoint().unwrap();
        // Dựng hàng send [a, b, c] qua endpoint (dùng push trực tiếp).
        {
            let endpoint = kernel.endpoint_mut(ep).unwrap();
            assert!(endpoint.send.push(a));
            assert!(endpoint.send.push(b));
            assert!(endpoint.send.push(c));
        }
        // Xoá giữa: còn [a, c], FIFO giữ nguyên.
        {
            let endpoint = kernel.endpoint_mut(ep).unwrap();
            assert!(endpoint.send.remove(b));
            assert_eq!(endpoint.send.pop(), Some(a));
            assert_eq!(endpoint.send.pop(), Some(c));
            assert_eq!(endpoint.send.pop(), None);
        }
        // Xoá phần tử không có: trả false, hàng nguyên vẹn.
        {
            let endpoint = kernel.endpoint_mut(ep).unwrap();
            assert!(endpoint.send.push(a));
            assert!(!endpoint.send.remove(b));
            assert_eq!(endpoint.send.pop(), Some(a));
        }
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    fn issue12_signal_one_wakes_exactly_one_waiter() {
        // (#12, lib.rs): signal_one chỉ đánh thức 1 waiter (FIFO), không herd.
        let mut kernel = Kernel::new();
        let a = kernel.create_tcb("wa", 5).unwrap();
        let b = kernel.create_tcb("wb", 5).unwrap();
        let ntfn = kernel.create_notification().unwrap();
        assert_eq!(kernel.wait_as(a, ntfn), Ok(None));
        assert_eq!(kernel.wait_as(b, ntfn), Ok(None));
        assert_eq!(kernel.signal_one(ntfn, 0b1), Ok(true));
        // a (slot đầu) được wake, b vẫn chờ.
        assert_eq!(kernel.tcb(a).unwrap().state, ThreadState::Ready);
        assert_eq!(
            kernel.tcb(b).unwrap().state,
            ThreadState::BlockedNotification
        );
        assert_eq!(kernel.signal_one(ntfn, 0b10), Ok(true));
        assert_eq!(kernel.tcb(b).unwrap().state, ThreadState::Ready);
        // Không ai chờ: bit giữ lại, trả false.
        assert_eq!(kernel.signal_one(ntfn, 0b100), Ok(false));
        assert_eq!(kernel.notification(ntfn).unwrap().word, 0b100);
        assert!(kernel.check_invariants().is_ok());
    }

    #[test]
    /// P0-3 (r290, đỏ-trước): `cap.parent` trỏ vào slot của một TCB sắp chết
    /// phải được SỬA khi TCB đó bị huỷ — kể cả khi TCB đang dùng CNode.
    ///
    /// Bản cũ `return` sớm trong `reparent_tcb_caps` khi thấy TCB dùng CNode,
    /// nên `cap.parent` trong CNode giữ nguyên con trỏ chết ⇒ `revoke` không
    /// phân giải được ⇒ object thành bất tử (`HasDependents` mãi mãi).
    #[test]
    fn parent_pointing_into_a_dying_tcb_is_repaired_inside_a_cnode() {
        let mut kernel = Kernel::new();
        let dying = kernel.create_tcb("dying", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel.create_untyped(0x5000_0000, 4096).unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        // `dying` dùng CNode làm cspace.
        kernel.tcb_mut(dying).unwrap().cnode = Some(cnode);

        // CNode slot 0 = "tổ tiên" (cap gốc); slot 1 = cap phái sinh trỏ parent
        // vào slot 0. Cả hai nằm trong CNode sẽ bị tháo khi `dying` chết.
        let mut ancestor = Cap::new(endpoint, ObjType::Endpoint, Rights::ALL);
        ancestor.id = 10;
        kernel.cnode_mut(cnode).unwrap().slots[0] = Some(ancestor);
        let mut child = Cap::new(endpoint, ObjType::Endpoint, Rights::READ);
        child.id = 11;
        child.parent = Some(CapRef {
            addr: SlotAddr {
                tcb: dying,
                slot: 0,
            },
            id: 10,
            cnode: Some(cnode),
        });
        kernel.cnode_mut(cnode).unwrap().slots[1] = Some(child);

        kernel.destroy_object(dying).unwrap();

        let after =
            kernel.cnode(cnode).unwrap().slots[1].expect("cap phái sinh vẫn còn trong CNode");
        match after.parent {
            None => {}
            Some(parent) => assert!(
                kernel.resolve_cap_ref(parent).is_some(),
                "parent còn trỏ bảng đã chết — revoke sẽ không tới được (P0-3)"
            ),
        }
    }

    /// 290.1 (r290): bộ duyệt bảng chìa dùng chung phải đi qua ĐÚNG tổng dung
    /// lượng các bảng đang tồn tại — kể cả CNode đã tháo khỏi TCB (bốn bộ duyệt
    /// cũ bỏ sót chính chỗ này).
    #[test]
    fn for_each_cap_visits_every_slot_of_every_live_table() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let plain = kernel.create_tcb("plain", 5).unwrap();
        let with_cnode = kernel.create_tcb("with-cnode", 5).unwrap();
        let scratch = kernel.create_tcb("scratch", 5).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();
        let untyped = kernel.create_untyped(0x3000_0000, 4096).unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        let untyped2 = kernel.create_untyped(0x4000_0000, 4096).unwrap();
        let detached = kernel.retype(untyped2, ObjType::CNode).unwrap();

        // Root: cap CNode (1) + cap TCB (2,3,4) để gọi CNodeInstall.
        kernel
            .install_cap(root, 1, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(with_cnode, ObjType::Tcb, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(detached, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 4, Cap::new(scratch, ObjType::Tcb, Rights::ALL))
            .unwrap();
        // TCB thường: cap nằm trong bảng NHÚNG của nó.
        kernel
            .install_cap(
                plain,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::READ),
            )
            .unwrap();
        // `with_cnode` dùng CNode ⇒ cap cài qua nó nằm trong CNode.
        kernel.tcb_mut(with_cnode).unwrap().cnode = Some(cnode);
        kernel
            .install_cap(
                with_cnode,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::READ),
            )
            .unwrap();
        // `scratch` dùng CNode khác, cài cap rồi THÁO CNode khỏi TCB.
        kernel.tcb_mut(scratch).unwrap().cnode = Some(detached);
        kernel
            .install_cap(
                scratch,
                1,
                Cap::new(endpoint, ObjType::Endpoint, Rights::READ),
            )
            .unwrap();
        assert!(kernel.tcb(scratch).unwrap().cnode == Some(detached));
        kernel.tcb_mut(scratch).unwrap().cnode = None;

        let mut seen = 0usize;
        let mut tables: [Option<TableId>; 8] = [None; 8];
        let mut table_count = 0usize;
        kernel.for_each_cap(|table, _slot, _cap| {
            seen += 1;
            if !tables[..table_count].iter().flatten().any(|t| *t == table) {
                tables[table_count] = Some(table);
                table_count += 1;
            }
        });
        assert_eq!(
            seen, 7,
            "root 4 cap + plain 1 + CNode gắn 1 + CNode ĐÃ THÁO 1 = 7"
        );
        assert_eq!(table_count, 4, "root, TCB nhúng, CNode gắn, CNode đã tháo");
        assert!(
            (0..table_count)
                .filter_map(|i| tables[i])
                .any(|t| matches!(t, TableId::CNode(_))),
            "CNode phải xuất hiện trong bộ duyệt"
        );
        // root + bảng nhúng của `plain` + bảng nhúng (rỗng) của `scratch` sau
        // khi tháo CNode + 2 CNode = 4 bảng cỡ TASK.
        assert_eq!(
            kernel.cap_slot_capacity(),
            MAX_CAPS_ROOT + 4 * MAX_CAPS_TASK,
            "root + 2 bảng nhúng + 2 CNode"
        );
    }

    /// P0-3 (r290, đỏ-trước): cap phái sinh nằm trong một CNode **đã tháo**
    /// khỏi TCB phải bị `revoke` thu hồi được, nếu không object thành bất tử.
    ///
    /// Kịch bản 5 bước của bản soát:
    /// 1. mint một cap endpoint vào CNode C từ slot 2 của TCB A;
    /// 2. gắn C làm cspace của A rồi huỷ A (C bị tháo khỏi A);
    /// 3. revoke cap gốc ở TCB còn sống phải thu hồi cap phái sinh đó;
    /// 4. sau đó `destroy_cap_object` trên object phải THÀNH CÔNG;
    /// 5. (không có bước 5.)
    ///
    /// Bản cũ: bước 3 trả `Ok(0)` (không phân giải được parent qua CNode mồ
    /// côi) ⇒ bước 4 trả `HasDependents` ⇒ object sống mãi.
    #[test]
    fn revoked_cap_inside_a_detached_cnode_still_collects_and_destroys() {
        let mut kernel = Kernel::new();
        let root = kernel.root().unwrap();
        let holder = kernel.create_tcb("holder", 5).unwrap();
        let untyped = kernel.create_untyped(0x2000_0000, 4096).unwrap();
        let cnode = kernel.retype(untyped, ObjType::CNode).unwrap();
        let endpoint = kernel.create_endpoint().unwrap();

        // Root giữ: cap CNode (1), cap endpoint gốc (2), cap TCB `holder` (3).
        kernel
            .install_cap(root, 1, Cap::new(cnode, ObjType::CNode, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 2, Cap::new(endpoint, ObjType::Endpoint, Rights::ALL))
            .unwrap();
        kernel
            .install_cap(root, 3, Cap::new(holder, ObjType::Tcb, Rights::ALL))
            .unwrap();

        // (1) mint cap endpoint vào C.slots[1], parent = root slot 2.
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
        // (2) gắn C làm cspace của `holder`, rồi huỷ `holder`.
        assert_eq!(
            kernel.user_syscall(root, SyscallId::CNodeInstall as u64, 3, 1, 0, 0, 0, 0),
            UserAction::Return {
                value: 0,
                error: KError::Ok
            }
        );
        assert!(kernel.tcb(holder).unwrap().cnode == Some(cnode));
        assert_eq!(kernel.destroy_object(holder), Ok(()));
        // C đã tháo khỏi TCB nhưng cap phái sinh vẫn nằm trong C.
        assert!(kernel.cnode(cnode).unwrap().slots[1].is_some());

        // (3) revoke cap gốc: phải thu hồi được cap phái sinh trong C.
        let collected = kernel.revoke_cap(root, 2).unwrap();
        assert_eq!(
            collected, 1,
            "phải thu hồi được cap phái sinh trong CNode đã tháo"
        );
        assert!(kernel.cnode(cnode).unwrap().slots[1].is_none());

        // (4) object không còn ai tham chiếu ⇒ huỷ được. Dùng `other_caps_to`
        // làm phép kiểm "không còn tham chiếu" (đúng điều kiện mà
        // `destroy_cap_object` dùng để trả HasDependents).
        let root_cap = kernel.cap_at(SlotAddr { tcb: root, slot: 2 }).unwrap();
        assert_eq!(
            kernel.other_caps_to(endpoint, root_cap.id),
            0,
            "cap phái sinh trong CNode đã bị thu hồi nên không còn tham chiếu"
        );
        assert_eq!(kernel.destroy_object(endpoint), Ok(()));
        assert!(kernel.check_invariants().is_ok());
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

    #[test]
    fn per_hart_runqueue_serves_hart2_and_hart3() {
        // Mốc 5 (vòng 31): scheduler generic theo MAX_HARTS — task affinity
        // hart 2/3 được enqueue đúng hàng, pick_on/steal hoạt động, hart
        // không tồn tại (>= MAX_HARTS) trả None.
        let mut kernel = Kernel::new();
        let g = kernel.create_tcb("g", 1).unwrap();
        let h = kernel.create_tcb("h", 1).unwrap();
        let floater = kernel.create_tcb("floater", 1).unwrap();
        kernel.tcb_mut(g).unwrap().affinity = 0b0100;
        kernel.tcb_mut(h).unwrap().affinity = 0b1000;
        kernel.tcb_mut(floater).unwrap().affinity = 0b1111;
        kernel.enqueue(g);
        kernel.enqueue(h);
        kernel.enqueue(floater);
        assert_eq!(kernel.runq_depth(2), 1);
        assert_eq!(kernel.runq_depth(3), 1);
        assert_eq!(kernel.pick_on(2), Some(g));
        assert_eq!(kernel.pick_on(3), Some(h));
        // Hart 2 rảnh, steal task floater (affinity gồm bit 2).
        assert_eq!(kernel.pick_on(2), Some(floater));
        assert_eq!(kernel.scheduler_stats(2), (1, 1));
        assert_eq!(kernel.pick_on(99), None);
        assert!(kernel.check_invariants().is_ok());
    }
    #[test]
    fn reply_with_bad_cap_slot_restores_caller() {
        // r242 (T9): hồi quy cho fix P0 "reply-orphan". Trước fix, `reply` gọi
        // `caller.take()` TRƯỚC khi tra cap nguồn; nếu bước tra cap lỗi thì
        // hàm return sớm và `caller` đã mất — client kẹt `BlockedReply` vĩnh
        // viễn, mọi lần reply sau đều `NoCaller`. Test này đỏ trên code cũ.
        let mut kernel = Kernel::new();
        let client = kernel.create_tcb("client", 0b01).unwrap();
        let server = kernel.create_tcb("server", 0b01).unwrap();
        let epoch = kernel.tcb(client).unwrap().reply_epoch;
        kernel.tcb_mut(server).unwrap().caller = Some((client, epoch));
        kernel.tcb_mut(client).unwrap().state = ThreadState::BlockedReply;

        let mut bad = Message::default();
        bad.cap = 99; // slot không tồn tại trong cspace của server
        assert_eq!(kernel.reply(server, bad), Err(KError::InvalidCap));
        assert_eq!(
            kernel.tcb(server).unwrap().caller.map(|(c, _)| c),
            Some(client),
            "caller phải được khôi phục sau lỗi tra cap"
        );
        assert_eq!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);

        // Reply lại không kèm cap phải thành công và đánh thức client.
        assert_eq!(kernel.reply(server, Message::default()), Ok(()));
        assert_ne!(kernel.tcb(client).unwrap().state, ThreadState::BlockedReply);
    }
}
