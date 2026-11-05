//! Architecture abstraction layer for ViFastOs.
//!
//! The portable kernel only sees this trait.  Everything that requires
//! privileged instructions, architectural registers or board knowledge lives
//! in `vifast-arch-*` crates.
//!
//! A port must provide:
//! * early console output (so the kernel can report progress before a full
//!   driver model exists),
//! * a monotonic clock,
//! * interrupt masking / ack primitives,
//! * an IPI primitive,
//! * a machine shutdown primitive for tests.
//!
//! This crate is deliberately only the low-level primitive layer.  Trap
//! frames, page tables, timers, interrupt controllers and SMP bring-up live in
//! `vifast-platform-*` crates, which implement the higher-level
//! `vifast_runtime::Platform` trait.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod bkl;
pub mod panic;

/// Small per-CPU array bound used by the platform ports.  The secondary harts
/// are still maintenance-only, but trap stacks and CPU state are already
/// indexed by hart so a later scheduler can adopt them without another layout
/// change.
pub const MAX_HARTS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuState {
    pub hart_id: usize,
    pub current_tcb: usize,
    pub trap_stack_top: u64,
    /// User stack saved by the RISC-V trap entry before switching to the
    /// kernel trap stack.  Unused on other architectures.
    pub user_sp: u64,
    pub active_root: u64,
    pub active_asid: u16,
}

impl CpuState {
    pub const fn empty() -> Self {
        Self {
            hart_id: 0,
            current_tcb: 0,
            trap_stack_top: 0,
            user_sp: 0,
            active_root: 0,
            active_asid: 0,
        }
    }
}

/// Information passed from the early boot code to the portable kernel.
#[derive(Clone, Copy, Debug, Default)]
pub struct BootInfo {
    /// Hardware thread / hart / CPU number that entered the kernel.
    pub hartid: usize,
    /// Physical address of the device tree, if the platform has one.
    pub dtb: u64,
    /// First usable physical RAM address.
    pub memory_base: u64,
    /// Size of usable physical RAM.
    pub memory_size: u64,
}

/// The contract between the portable kernel and one architecture port.
pub trait Arch {
    /// Short name used in boot logs, e.g. `"riscv64"`.
    const NAME: &'static str;

    /// Base page size of the architecture.  All three current targets use 4
    /// KiB, but ARM supports 16/64 KiB granules and x86 supports large pages.
    /// Keep this in the architecture layer; never hard-code it in the portable
    /// kernel.
    const PAGE_SIZE: usize = 4096;
    const PAGE_SHIFT: u32 = 12;
    /// Upper bound of the user half of the virtual address space.
    const USER_VA_LIMIT: u64 = 1 << 38;

    /// Early CPU/console initialisation.  Must be idempotent enough to be
    /// called once from the common entry point.
    fn init();

    /// Write one byte to the early debug console.
    fn putc(byte: u8);

    /// Poll one byte from the early debug console without blocking.
    fn getc() -> Option<u8> {
        None
    }

    /// Monotonic timestamp.  The unit is architecture specific; the portable
    /// scheduler only needs monotonicity for now.
    fn now() -> u64;

    /// Mask all maskable interrupts on the current CPU.
    fn irq_disable();

    /// Enable maskable interrupts on the current CPU.
    fn irq_enable();

    /// Send an inter-processor interrupt to another CPU.
    fn send_ipi(hart: usize);

    /// Stop the current CPU forever.
    fn halt() -> !;

    /// Shut down the machine (used by the QEMU test harness).  If the
    /// platform cannot power off, implementations may fall back to `halt`.
    fn exit(code: u8) -> !;

    /// Write a string to the early console.
    fn write_str(text: &str) {
        for byte in text.bytes() {
            Self::putc(byte);
        }
    }
}
