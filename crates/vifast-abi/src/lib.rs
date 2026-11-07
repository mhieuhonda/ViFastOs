//! ViFastOs wire ABI.
//!
//! This crate is deliberately free of `std`, `alloc`, architecture-specific
//! types and Rust-only layout assumptions.  Every type here has a stable
//! `#[repr(C)]` or integer representation so the same definitions can be used
//! by the kernel, the architecture ports and user space.
//!
//! Syscall *numbers* and error values are architecture independent.  The
//! mapping between a syscall and CPU registers lives in the per-architecture
//! crates, not here.

#![no_std]
#![forbid(unsafe_code)]

pub const MSG_MAX_WORDS: usize = 8;
pub const IPC_BUFFER_WORDS: usize = 32;
/// Số byte tối đa một syscall copy giữa user và kernel (vòng 80).
pub const USER_COPY_MAX: usize = 256;
pub const DEFAULT_PAGE_SIZE: u64 = 4096;

/// Shared user-address layout for all architectures.  The high base keeps
/// user mappings away from kernel identity maps and low MMIO.
pub mod pager {
    //! Wire layout for the general pager service (protocol v2).  The kernel
    //! only relays faults; mapping, eviction and file I/O stay in user space.

    pub const LABEL_FAULT: u64 = 0x5047_0001;
    pub const LABEL_MAP: u64 = 0x5047_0002;
    pub const LABEL_UNMAP: u64 = 0x5047_0003;
    pub const LABEL_EVICT: u64 = 0x5047_0004;
    pub const LABEL_REPLY: u64 = 0x5047_0005;

    pub const MAX_CLIENTS: usize = 4;
    pub const POOL_FRAMES: usize = 8;
    pub const BACKING_ENTRIES: usize = 16;
    pub const PAGE_BYTES: usize = 4096;
    /// File backing demand-load từ VFS (vòng 71): trang 4 KiB với giá trị
    /// nhận dạng ở đầu và cuối để kiểm tra nội dung nạp đúng.
    pub const DEMAND_FILE: &[u8] = b"/page.bin";
    pub const DEMAND_VALUE: u64 = 0x4445_4d41_4e44_0004;
    pub const DEMAND_TAIL: u64 = 0x4445_4d41_4e44_ffff;
    pub const DEMAND_BYTES: usize = PAGE_BYTES;

    /// Private VA window reserved for pager clients, above the bootstrap
    /// user image and below the file-backed window of the next round.
    pub const CLIENT_VA_BASE: u64 = super::user_layout::CODE_VA as u64 + 0x200_0000;
    pub const CLIENT_VA_STRIDE: u64 = 0x10_0000;
    pub const CLIENT_VA_END: u64 = CLIENT_VA_BASE + MAX_CLIENTS as u64 * CLIENT_VA_STRIDE;

    pub const fn valid_label(label: u64) -> bool {
        matches!(
            label,
            LABEL_FAULT | LABEL_MAP | LABEL_UNMAP | LABEL_EVICT | LABEL_REPLY
        )
    }

    pub const fn client_va(slot: usize) -> Option<u64> {
        if slot >= MAX_CLIENTS {
            return None;
        }
        Some(CLIENT_VA_BASE + slot as u64 * CLIENT_VA_STRIDE)
    }

    /// Fault relay: one page fault for `tcb` at `va`.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Fault {
        pub va: u64,
        pub cause: u64,
        pub endpoint_slot: u64,
        pub tcb: u64,
    }

    impl Fault {
        pub const fn encode(self) -> [u64; 4] {
            [self.va, self.cause, self.endpoint_slot, self.tcb]
        }

        pub const fn decode(words: [u64; 4]) -> Option<Self> {
            if words[0] % PAGE_BYTES as u64 != 0 {
                return None;
            }
            Some(Self {
                va: words[0],
                cause: words[1],
                endpoint_slot: words[2],
                tcb: words[3],
            })
        }
    }

    /// Root asks the pager to install a file-backed mapping.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Map {
        pub vspace_slot: u64,
        pub inode: u16,
        pub file_offset: u64,
        pub len: u32,
    }

    impl Map {
        pub const fn encode(self) -> [u64; 4] {
            [
                self.vspace_slot,
                self.inode as u64,
                self.file_offset,
                self.len as u64,
            ]
        }

        pub const fn decode(words: [u64; 4]) -> Option<Self> {
            let inode = words[1];
            let len = words[3];
            if inode > u16::MAX as u64 || len > u32::MAX as u64 || len == 0 {
                return None;
            }
            Some(Self {
                vspace_slot: words[0],
                inode: inode as u16,
                file_offset: words[2],
                len: len as u32,
            })
        }
    }

    const _: () = assert!(MAX_CLIENTS <= 8);
    const _: () = assert!(POOL_FRAMES >= 4);
    const _: () = assert!(CLIENT_VA_BASE % PAGE_BYTES as u64 == 0);
    const _: () = assert!(CLIENT_VA_STRIDE % PAGE_BYTES as u64 == 0);
    const _: () = assert!(CLIENT_VA_BASE > super::user_layout::CODE_VA as u64 + 0x100_0000);
    const _: () = assert!(CLIENT_VA_END <= super::user_layout::USER_VA_END as u64);
    const _: () = assert!(LABEL_FAULT != LABEL_MAP);
    const _: () = assert!(LABEL_REPLY != LABEL_EVICT);

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn labels_are_distinct_and_valid() {
            let labels = [
                LABEL_FAULT,
                LABEL_MAP,
                LABEL_UNMAP,
                LABEL_EVICT,
                LABEL_REPLY,
            ];
            for (index, label) in labels.iter().enumerate() {
                assert!(valid_label(*label));
                for other in labels.iter().skip(index + 1) {
                    assert_ne!(label, other);
                }
            }
            assert!(!valid_label(0));
        }

        #[test]
        fn client_va_windows_are_aligned_and_disjoint() {
            let mut last_end = CLIENT_VA_BASE;
            for slot in 0..MAX_CLIENTS {
                let va = client_va(slot).unwrap();
                assert_eq!(va % PAGE_BYTES as u64, 0);
                assert!(va >= last_end);
                last_end = va + CLIENT_VA_STRIDE;
            }
            assert_eq!(last_end, CLIENT_VA_END);
            assert_eq!(client_va(MAX_CLIENTS), None);
        }

        #[test]
        fn fault_roundtrip_rejects_unaligned_va() {
            let fault = Fault {
                va: CLIENT_VA_BASE + 0x2000,
                cause: 15,
                endpoint_slot: 7,
                tcb: 42,
            };
            assert_eq!(Fault::decode(fault.encode()), Some(fault));
            let mut words = fault.encode();
            words[0] += 1;
            assert_eq!(Fault::decode(words), None);
        }

        #[test]
        fn map_roundtrip_checks_ranges() {
            let map = Map {
                vspace_slot: 9,
                inode: 12,
                file_offset: 4096,
                len: 8192,
            };
            assert_eq!(Map::decode(map.encode()), Some(map));
            let mut words = map.encode();
            words[1] = u16::MAX as u64 + 1;
            assert_eq!(Map::decode(words), None);
            words = map.encode();
            words[3] = 0;
            assert_eq!(Map::decode(words), None);
        }
    }
}

pub mod user_layout {
    pub const CODE_VA: usize = 0x0000_0010_0000_0000;
    pub const DATA_VA: usize = CODE_VA + 0xa000;
    pub const STACK_BASE: usize = CODE_VA + 0x1000;
    pub const STACK_LEN: usize = 0x8000;

    /// Per-thread stack tops used by the bootstrap user image.  The layout is
    /// deliberately disjoint: the server page at +0x2000 is not mapped in the
    /// root VSpace so the isolation test can fault on it.
    pub const SERVER_STACK_BASE: usize = CODE_VA + 0x2000;
    pub const SERVER_STACK_TOP: usize = CODE_VA + 0x5000;
    pub const CLIENT_STACK_BASE: usize = CODE_VA + 0x8000;
    pub const CLIENT_STACK_TOP: usize = CODE_VA + 0x9000;
    pub const FAULT1_STACK_BASE: usize = CODE_VA + 0x5000;
    pub const FAULT1_STACK_TOP: usize = CODE_VA + 0x6000;
    pub const FAULT2_STACK_BASE: usize = CODE_VA + 0x6000;
    pub const FAULT2_STACK_TOP: usize = CODE_VA + 0x7000;
    pub const FAULT3_STACK_BASE: usize = CODE_VA + 0x7000;
    pub const FAULT3_STACK_TOP: usize = CODE_VA + 0x8000;
    pub const FAULT4_STACK_BASE: usize = CODE_VA + 0x9000;
    pub const FAULT4_STACK_TOP: usize = CODE_VA + 0xa000;
    /// Dedicated IPC-buffer pages, private to each task VSpace.  The kernel
    /// copies IPC_BUFFER_WORDS between these pages on every endpoint send.
    pub const IPC_BUF_SERVER_VA: usize = CODE_VA + 0xb000;
    pub const IPC_BUF_CLIENT_VA: usize = CODE_VA + 0xc000;
    /// Root-VSpace stacks used by the RISC-V SMP worker demo.
    pub const WORKER_A_STACK_BASE: usize = CODE_VA + 0xd000;
    pub const WORKER_A_STACK_TOP: usize = CODE_VA + 0xe000;
    pub const WORKER_B_STACK_BASE: usize = CODE_VA + 0xe000;
    pub const WORKER_B_STACK_TOP: usize = CODE_VA + 0xf000;
    pub const WORKER_C_STACK_BASE: usize = CODE_VA + 0x10000;
    pub const WORKER_C_STACK_TOP: usize = CODE_VA + 0x11000;
    pub const WORKER_D_STACK_BASE: usize = CODE_VA + 0x11000;
    pub const WORKER_D_STACK_TOP: usize = CODE_VA + 0x12000;
    pub const WORKER_E_STACK_BASE: usize = CODE_VA + 0x12000;
    pub const WORKER_E_STACK_TOP: usize = CODE_VA + 0x13000;
    /// BSP idle task stack.  The idle task only runs when no other hart-0
    /// TCB is runnable; a separate page keeps root's Rust stack untouched.
    pub const IDLE_STACK_BASE: usize = CODE_VA + 0x14000;
    pub const IDLE_STACK_TOP: usize = CODE_VA + 0x15000;
    /// Stack for the end-to-end CNode demo task.
    pub const CNODE_STACK_BASE: usize = CODE_VA + 0x16000;
    pub const CNODE_STACK_TOP: usize = CODE_VA + 0x17000;
    /// Stack for the demand-paging/fault-resume demo task.
    pub const PAGER_STACK_BASE: usize = CODE_VA + 0x18000;
    pub const PAGER_STACK_TOP: usize = CODE_VA + 0x19000;
    /// Separate stack for the user-space pager service.
    pub const PAGER_SERVER_STACK_BASE: usize = CODE_VA + 0x1a000;
    pub const PAGER_SERVER_STACK_TOP: usize = CODE_VA + 0x1b000;
    /// Second pager client proves the service handles more than one faulting
    /// VSpace/TCB.
    pub const PAGER2_STACK_BASE: usize = CODE_VA + 0x1c000;
    pub const PAGER2_STACK_TOP: usize = CODE_VA + 0x1d000;
    /// Third pager client: proves the service scales past two clients on the
    /// same fault endpoint (vòng 37, cần MAX_CAPS_ROOT=128 ở vòng 36).
    pub const PAGER3_STACK_BASE: usize = CODE_VA + 0x1e000;
    pub const PAGER3_STACK_TOP: usize = CODE_VA + 0x1f000;
    /// Trang IPC buffer riêng của pager server khi nó gọi VFS (vòng 71).
    pub const PAGER_IPC_BUF_VA: usize = CODE_VA + 0x1f000;
    const _: () = assert!(WORKER_A_STACK_BASE > DATA_VA);
    const _: () = assert!(WORKER_A_STACK_TOP <= WORKER_B_STACK_BASE);
    const _: () = assert!(WORKER_B_STACK_TOP <= WORKER_C_STACK_BASE);
    const _: () = assert!(WORKER_C_STACK_TOP <= WORKER_D_STACK_BASE);
    const _: () = assert!(WORKER_D_STACK_TOP <= WORKER_E_STACK_BASE);
    const _: () = assert!(WORKER_E_STACK_TOP <= IDLE_STACK_BASE);
    const _: () = assert!(IDLE_STACK_TOP <= CNODE_STACK_BASE);
    const _: () = assert!(CNODE_STACK_TOP <= PAGER_STACK_BASE);
    const _: () = assert!(PAGER_STACK_TOP <= PAGER_SERVER_STACK_BASE);
    const _: () = assert!(PAGER_SERVER_STACK_TOP <= PAGER2_STACK_BASE);
    const _: () = assert!(PAGER2_STACK_TOP <= PAGER3_STACK_BASE);
    const _: () = assert!(PAGER3_STACK_TOP <= PAGER_IPC_BUF_VA);

    /// Root Rust ELF lives in the next 2 MiB user window, after the bootstrap
    /// user.S code/stack window.  Its stack has four mapped pages and one
    /// unmapped guard page immediately below.
    pub const ROOT_ELF_BASE: usize = CODE_VA + 0x20_0000;
    pub const ROOT_ELF_STACK_GUARD: usize = ROOT_ELF_BASE + 0x10_0000;
    pub const ROOT_ELF_STACK_BASE: usize = ROOT_ELF_STACK_GUARD + 0x1000;
    pub const ROOT_ELF_STACK_TOP: usize = ROOT_ELF_STACK_BASE + 0x4000;
    /// Read-only BootInfo page passed to the root task in its first argument.
    /// It is above the root ELF stack and outside the ELF parse window.
    pub const ROOT_BOOTINFO_VA: usize = ROOT_ELF_STACK_TOP + 0x1000;
    const _: () = assert!(ROOT_BOOTINFO_VA >= ROOT_ELF_STACK_TOP);
    const _: () = assert!(ROOT_BOOTINFO_VA >= ROOT_ELF_STACK_GUARD);
    /// Read-only staging window where the runtime maps boot-module ELF bytes
    /// for the root loader.  The two modules are separated by 64 KiB.
    pub const ROOT_MODULE_SERVER_VA: usize = ROOT_BOOTINFO_VA + 0x1000;
    pub const ROOT_MODULE_CLIENT_VA: usize = ROOT_MODULE_SERVER_VA + 0x1_0000;
    pub const ROOT_MODULE_END: usize = ROOT_MODULE_CLIENT_VA + 0x1_0000;
    const _: () = assert!(ROOT_MODULE_SERVER_VA >= ROOT_BOOTINFO_VA);
    const _: () = assert!(ROOT_MODULE_CLIENT_VA >= ROOT_MODULE_SERVER_VA);

    /// Fixed jump-table entries at the start of the bootstrap user image.
    pub const USER_ENTRY_SERVER: usize = CODE_VA;
    pub const USER_ENTRY_CLIENT: usize = CODE_VA + 0x10;
    pub const USER_ENTRY_FAULT1: usize = CODE_VA + 0x20;
    pub const USER_ENTRY_FAULT2: usize = CODE_VA + 0x30;
    pub const USER_ENTRY_FAULT3: usize = CODE_VA + 0x40;
    pub const USER_ENTRY_FAULT4: usize = CODE_VA + 0x50;

    /// Bootstrap capability slots.  BootInfo will replace these constants.
    pub const ROOT_VSPACE_SLOT: usize = 1;
    pub const META_UNTYPED_SLOT: usize = 2;
    pub const DATA_UNTYPED_SLOT: usize = 3;
    pub const SERVER_VSPACE_SLOT: usize = 12;
    pub const BLOCK_DEVICE_SLOT: usize = 13;
    pub const ENDPOINT_SLOT: usize = 4;
    pub const SERVER_TCB_SLOT: usize = 5;
    pub const CLIENT_TCB_SLOT: usize = 6;
    pub const NOTIFICATION_SLOT: usize = 7;
    pub const ROOT_NOTIFICATION_SLOT: usize = 8;
    pub const FAULT1_TCB_SLOT: usize = 9;
    pub const FAULT2_TCB_SLOT: usize = 10;
    pub const FAULT3_TCB_SLOT: usize = 11;
    /// Client cspace slots used for the IRQ demos/notifications.
    pub const CLIENT_IRQ_SLOT: u8 = 8;
    pub const CLIENT_IRQ_NOTIFICATION_SLOT: u8 = 9;
    pub const CLIENT_UART_IRQ_SLOT: u8 = 10;
    pub const CLIENT_UART_NOTIFICATION_SLOT: u8 = 11;
    /// Maximum user virtual window reserved for code/data/stack/IPC buffers.
    pub const WINDOW_BASE: usize = CODE_VA;
    pub const WINDOW_SIZE: usize = 0x20_0000;
    /// Full user virtual-address range accepted by the MMU layer.
    ///
    /// This is deliberately not the full lower half of the address space: it
    /// starts above every kernel identity mapping and leaves room for page
    /// tables below it.  The initial boot window above is only a small part of
    /// this range and will disappear once code/stack are ordinary mappings.
    pub const USER_VA_START: usize = CODE_VA;
    pub const USER_VA_END: usize = CODE_VA + 0x10_0000_0000;
}
/// Magic value for the read-only boot information page (`VFBOOT01` LE).
pub const BOOTINFO_MAGIC: u64 = u64::from_le_bytes(*b"VFBOOT01");
pub const BOOTINFO_VERSION: u32 = 3;

/// Fixed layout passed to the root task as its first argument.  All fields are
/// integers so the ABI has no Rust layout surprises; `free_slot_start` is the
/// first slot the root may allocate and `free_slot_end` is exclusive.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub size: u32,
    pub root_vspace: u16,
    pub meta_untyped: u16,
    pub data_untyped: u16,
    pub server_vspace: u16,
    pub user_code_first: u16,
    pub user_code_count: u16,
    pub free_slot_start: u16,
    pub free_slot_end: u16,
    /// Slot in the root CNode holding the block-device capability.
    pub block_device: u16,
    pub server_elf_va: u64,
    pub server_elf_len: u64,
    pub client_elf_va: u64,
    pub client_elf_len: u64,
    /// Bitmask of harts that can run normal user tasks (bit 0 always set).
    pub sched_harts: u32,
}
const _: () = assert!(core::mem::size_of::<BootInfo>() <= 4096);

/// Maximum bytes copied through a single user I/O syscall.
pub const USER_IO_MAX: usize = 256;
/// Fixed untyped-memory accounting sizes.  These make object creation
/// deterministic even while the reference model still stores metadata in the
/// kernel object table.
pub const OBJ_ENDPOINT_BYTES: usize = 256;
pub const OBJ_NOTIFICATION_BYTES: usize = 128;
pub const OBJ_TCB_BYTES: usize = 4096;
pub const OBJ_SCHED_CONTEXT_BYTES: usize = 64;
pub const OBJ_METADATA_ALIGN: usize = 16;
pub const OBJ_IOMMU_BYTES: usize = 1024;
pub const OBJ_BLOCK_DEVICE_BYTES: usize = 64;
pub const OBJ_IRQ_BYTES: usize = 64;
pub const OBJ_CNODE_BYTES: usize = 4096;
/// Logical IRQ numbers visible to user space.  Ports map these to their
/// interrupt-controller lines and keep the controller details private.
pub const IRQ_TIMER: u32 = 0;
pub const IRQ_UART: u32 = 1;
/// Base of the software IOMMU IOVA window used by the portable model.
pub const IOMMU_IOVA_BASE: u64 = 0x1_0000_0000;
pub const LABEL_FAULT: u64 = 0xFFFF_0001;
pub const LABEL_NOTIFICATION: u64 = 0xFFFF_0002;
/// Sentinel used instead of `Option<u64>` in the wire message.
pub const CAP_NONE: u64 = u64::MAX;

/// Endpoint badge identity rules (D6).  Badge 0 has no identity;
/// the single superuser badge is 0xFFFF.
pub const BADGE_NONE: u64 = 0;
pub const BADGE_SUPERUSER: u64 = 0xFFFF;
/// Badge cấp cho pager server khi gọi VFS (vòng 71).
pub const BADGE_PAGER: u64 = 3;

/// File open flags for the tiny ramfs.
pub const O_RDONLY: u64 = 0;
pub const O_WRONLY: u64 = 1;
pub const O_RDWR: u64 = 2;
pub const O_CREATE: u64 = 4;
pub const O_TRUNC: u64 = 8;
pub const O_APPEND: u64 = 16;
/// Fail instead of opening an existing file created with `O_CREATE`.
pub const O_EXCL: u64 = 32;
/// Require the opened path to be a directory.
pub const O_DIRECTORY: u64 = 64;

/// Minimal user-space VFS request/reply protocol carried by the ordinary IPC
/// message label and three data words.  Reads and writes are deliberately
/// bounded to three words so a reply fits in one `ReplyRecv`.
pub mod vfs {
    pub const OPEN: u64 = 0x2001;
    pub const READ: u64 = 0x2002;
    pub const WRITE: u64 = 0x2003;
    pub const CLOSE: u64 = 0x2004;
    pub const STAT: u64 = 0x2005;
    pub const MKDIR: u64 = 0x2006;
    pub const READDIR: u64 = 0x2007;
    pub const UNLINK: u64 = 0x2008;
    pub const RENAME: u64 = 0x2009;
    pub const TRUNCATE: u64 = 0x200A;
    pub const FSYNC: u64 = 0x200B;
    pub const FSCK: u64 = 0x200C;
    /// Identity probe: server returns the kernel-stamped badge in data[0].
    pub const BADGE_PROBE: u64 = 0x200D;
    /// Hard link: old path in buffer[0], new path in buffer[MAX_PATH].
    pub const LINK: u64 = 0x200E;
    /// Symlink: target in buffer[0], link path in buffer[MAX_PATH].
    pub const SYMLINK: u64 = 0x200F;
    /// Read symlink target through the IPC buffer.
    pub const READLINK: u64 = 0x2010;
    /// Stat by path: data[0] = path length in the IPC buffer.
    pub const STAT_PATH: u64 = 0x2011;

    pub const ENOENT: u64 = 1;
    pub const EEXIST: u64 = 2;
    pub const EACCES: u64 = 3;
    pub const ENOSPC: u64 = 4;
    pub const ENOTDIR: u64 = 5;
    pub const EISDIR: u64 = 6;
    pub const ENOTEMPTY: u64 = 7;
    pub const EINVAL: u64 = 8;
    pub const ENAMETOOLONG: u64 = 9;
    pub const EBADF: u64 = 10;
    pub const EIO: u64 = 11;

    /// Successful open/close reply label.  Read/write replies use the byte
    /// count as the label so zero-length operations remain distinguishable.
    pub const OK: u64 = 1;
    /// Reply label used for malformed requests, missing files and invalid fds.
    pub const ERR: u64 = u64::MAX;
    pub const MAX_NAME: usize = 8;
    pub const MAX_COMPONENT: usize = 28;
    pub const MAX_IO: usize = 128;
    pub const MAX_FILES: usize = 16;
    pub const MAX_PATH: usize = 128;
    pub const MAX_FD: usize = 8;
}

/// Kernel error codes.  These values are part of the ABI and must never be
/// renumbered.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KError {
    #[default]
    Ok = 0,
    InvalidCap = 1,
    WrongType = 2,
    NoRights = 3,
    SlotOccupied = 4,
    OutOfMemory = 5,
    InvalidArg = 6,
    Alignment = 7,
    AlreadyMapped = 8,
    NotMapped = 9,
    NoCaller = 10,
    ObjectDestroyed = 11,
    IrqInUse = 12,
    PrioTooHigh = 13,
    Cancelled = 14,
    MissingPageTable = 15,
    IllegalOperation = 16,
    NotImplemented = 17,
    WouldBlock = 18,
    NotFound = 19,
    /// The object is still referenced by a mapping, queue, binding or TCB.
    Busy = 20,
    /// Other capabilities still point at the object (revoke them first).
    HasDependents = 21,
}

impl KError {
    pub const fn from_u32(value: u32) -> KError {
        match value {
            0 => KError::Ok,
            1 => KError::InvalidCap,
            2 => KError::WrongType,
            3 => KError::NoRights,
            4 => KError::SlotOccupied,
            5 => KError::OutOfMemory,
            6 => KError::InvalidArg,
            7 => KError::Alignment,
            8 => KError::AlreadyMapped,
            9 => KError::NotMapped,
            10 => KError::NoCaller,
            11 => KError::ObjectDestroyed,
            12 => KError::IrqInUse,
            13 => KError::PrioTooHigh,
            14 => KError::Cancelled,
            15 => KError::MissingPageTable,
            16 => KError::IllegalOperation,
            17 => KError::NotImplemented,
            18 => KError::WouldBlock,
            19 => KError::NotFound,
            20 => KError::Busy,
            21 => KError::HasDependents,
            _ => KError::InvalidArg,
        }
    }

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
            KError::NotImplemented => "NOT_IMPLEMENTED",
            KError::WouldBlock => "WOULD_BLOCK",
            KError::NotFound => "NOT_FOUND",
            KError::Busy => "BUSY",
            KError::HasDependents => "HAS_DEPENDENTS",
        }
    }
}

pub type KResult<T> = Result<T, KError>;

/// Capability rights.  The numeric values are part of the ABI.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rights(pub u8);

impl Rights {
    pub const NONE: Rights = Rights(0);
    pub const READ: Rights = Rights(1);
    pub const WRITE: Rights = Rights(2);
    pub const EXEC: Rights = Rights(4);
    pub const GRANT: Rights = Rights(8);
    pub const ALL: Rights = Rights(15);
    pub const RW: Rights = Rights(3);

    pub const fn contains(self, other: Rights) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn intersects(self, other: Rights) -> bool {
        (self.0 & other.0) != 0
    }

    pub const fn without(self, other: Rights) -> Rights {
        Rights(self.0 & !other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitOr for Rights {
    type Output = Rights;
    fn bitor(self, rhs: Rights) -> Rights {
        Rights(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for Rights {
    type Output = Rights;
    fn bitand(self, rhs: Rights) -> Rights {
        Rights(self.0 & rhs.0)
    }
}

impl core::ops::BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Rights) {
        self.0 |= rhs.0;
    }
}

/// Kernel object kinds.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjType {
    Untyped = 0,
    Tcb = 1,
    Endpoint = 2,
    Notification = 3,
    CNode = 4,
    Frame = 5,
    VSpace = 6,
    PageTable = 7,
    SchedContext = 8,
    Iommu = 9,
    BlockDevice = 10,
    Irq = 11,
}

/// Thread states are exposed to the ABI for introspection and fault reports.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThreadState {
    #[default]
    Inactive = 0,
    Ready = 1,
    Running = 2,
    BlockedSend = 3,
    BlockedRecv = 4,
    BlockedReply = 5,
    BlockedNotification = 6,
    Faulted = 7,
    BlockedFault = 8,
    Suspended = 9,
    BlockedSleep = 10,
    /// Budget exhausted; the scheduler wakes the TCB when its period ends.
    BlockedBudget = 11,
}

impl ThreadState {
    pub const fn is_blocked(self) -> bool {
        matches!(
            self,
            ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
                | ThreadState::BlockedSleep
                | ThreadState::BlockedBudget
        )
    }
}

#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultKind {
    VmFault = 1,
    UserException = 2,
    Exited = 3,
    BadSyscall = 4,
}

/// Fixed-size IPC message.  No Rust `Option`, `String`, `Vec` or heap is used
/// on the wire.  `len` is validated by the kernel and clamped by `words()`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Message {
    pub label: u64,
    pub len: u8,
    pub _pad: [u8; 7],
    /// Slot number of a transferred capability, or `CAP_NONE`.
    pub cap: u64,
    /// Sender badge captured from the endpoint capability.  The receiver
    /// gets it only in the dedicated badge register (a7/x7/r10), never in
    /// the message value word.
    pub badge: u64,
    pub data: [u64; MSG_MAX_WORDS],
}

impl Default for Message {
    fn default() -> Self {
        Self {
            label: 0,
            len: 0,
            _pad: [0; 7],
            cap: CAP_NONE,
            badge: 0,
            data: [0; MSG_MAX_WORDS],
        }
    }
}

impl Message {
    pub const fn empty(label: u64) -> Message {
        Message {
            label,
            len: 0,
            _pad: [0; 7],
            cap: CAP_NONE,
            badge: 0,
            data: [0; MSG_MAX_WORDS],
        }
    }

    pub fn new(label: u64, words: &[u64]) -> KResult<Message> {
        if words.len() > MSG_MAX_WORDS {
            return Err(KError::InvalidArg);
        }
        let mut data = [0u64; MSG_MAX_WORDS];
        let mut i = 0;
        while i < words.len() {
            data[i] = words[i];
            i += 1;
        }
        Ok(Message {
            label,
            len: words.len() as u8,
            _pad: [0; 7],
            cap: CAP_NONE,
            badge: 0,
            data,
        })
    }

    pub const fn with_cap(mut self, slot: u64) -> Message {
        self.cap = slot;
        self
    }

    pub fn words(&self) -> &[u64] {
        let n = if (self.len as usize) > MSG_MAX_WORDS {
            MSG_MAX_WORDS
        } else {
            self.len as usize
        };
        &self.data[..n]
    }

    pub fn cap_slot(&self) -> Option<u64> {
        if self.cap == CAP_NONE {
            None
        } else {
            Some(self.cap)
        }
    }
}

/// Syscall numbers.  They are stable across architectures; the register
/// mapping is provided by `vifast-arch-*` and `vifast-user-*`.
#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyscallId {
    Send = 0,
    Call = 1,
    Recv = 2,
    Reply = 3,
    ReplyRecv = 4,
    Signal = 5,
    Wait = 6,
    Poll = 7,
    Yield = 8,
    Retype = 9,
    CapCopy = 10,
    CapMint = 11,
    CNodeMint = 12,
    CapDelete = 13,
    CapRevoke = 14,
    TcbConfigure = 15,
    TcbResume = 16,
    TcbSuspend = 17,
    TcbRestart = 18,
    TcbSetPriority = 19,
    TcbSetAffinity = 20,
    TcbBindNotification = 21,
    SchedContextConfigure = 22,
    SchedContextBind = 23,
    PageTableMap = 24,
    FrameMap = 25,
    FrameUnmap = 26,
    IoMap = 27,
    IoUnmap = 28,
    IrqGet = 29,
    IrqSetNotification = 30,
    IrqAck = 31,
    DebugWrite = 32,
    GetTime = 33,
    Exit = 34,
    Open = 35,
    Read = 36,
    Write = 37,
    Close = 38,
    Stat = 39,
    Seek = 40,
    TcbSetIpcBuffer = 41,
    FrameWrite = 42,
    BlockRead = 43,
    BlockWrite = 44,
    BlockFlush = 45,
    Sleep = 46,
    ConsoleRead = 47,
    /// Install or remove a flat CNode as a TCB's capability space.
    CNodeInstall = 48,
    /// Destroy a non-current TCB after revoking/reparenting its cspace caps.
    TcbDestroy = 49,
    /// Destroy a retyped object through its root capability.
    ObjectDestroy = 50,
}

/// Dispatch status for every ABI number.  `true` means the portable kernel
/// has a real handler; `false` means the number is reserved/placeholder and
/// reaches the `NotImplemented` fallback.  `gen-docs.py` reads this table.
pub const SYSCALL_STATUS: &[(SyscallId, bool)] = &[
    (SyscallId::Send, true),
    (SyscallId::Call, true),
    (SyscallId::Recv, true),
    (SyscallId::Reply, true),
    (SyscallId::ReplyRecv, true),
    (SyscallId::Signal, true),
    (SyscallId::Wait, true),
    (SyscallId::Poll, true),
    (SyscallId::Yield, true),
    (SyscallId::Retype, true),
    (SyscallId::CapCopy, true),
    (SyscallId::CapMint, true),
    (SyscallId::CNodeMint, true),
    (SyscallId::CapDelete, true),
    (SyscallId::CapRevoke, true),
    (SyscallId::TcbConfigure, true),
    (SyscallId::TcbResume, true),
    (SyscallId::TcbSuspend, true),
    (SyscallId::TcbRestart, true),
    (SyscallId::TcbSetPriority, true),
    (SyscallId::TcbSetAffinity, true),
    (SyscallId::TcbBindNotification, true),
    (SyscallId::SchedContextConfigure, true),
    (SyscallId::SchedContextBind, true),
    (SyscallId::PageTableMap, true),
    (SyscallId::FrameMap, true),
    (SyscallId::FrameUnmap, true),
    (SyscallId::IoMap, true),
    (SyscallId::IoUnmap, true),
    (SyscallId::IrqGet, true),
    (SyscallId::IrqSetNotification, true),
    (SyscallId::IrqAck, true),
    (SyscallId::DebugWrite, true),
    (SyscallId::GetTime, true),
    (SyscallId::Exit, true),
    (SyscallId::Open, true),
    (SyscallId::Read, true),
    (SyscallId::Write, true),
    (SyscallId::Close, true),
    (SyscallId::Stat, true),
    (SyscallId::Seek, true),
    (SyscallId::TcbSetIpcBuffer, true),
    (SyscallId::FrameWrite, true),
    (SyscallId::BlockRead, true),
    (SyscallId::BlockWrite, true),
    (SyscallId::BlockFlush, true),
    (SyscallId::Sleep, true),
    (SyscallId::ConsoleRead, true),
    (SyscallId::CNodeInstall, true),
    (SyscallId::TcbDestroy, true),
    (SyscallId::ObjectDestroy, true),
];

/// Generic register arguments for a syscall.  Architecture ports decode
/// `SyscallId` + these words; the kernel validates them.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SyscallArgs {
    pub number: u64,
    pub a0: u64,
    pub a1: u64,
    pub a2: u64,
    pub a3: u64,
    pub a4: u64,
    pub a5: u64,
}

impl SyscallArgs {
    pub const fn new(number: u64) -> Self {
        Self {
            number,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
        }
    }
}
