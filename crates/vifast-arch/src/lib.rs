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

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

pub mod bkl;
pub mod panic;

/// Small single-producer/single-consumer byte queue used by UART RX IRQ
/// handlers.  It is lock-free and safe to read from the syscall path while an
/// interrupt handler pushes bytes.
pub struct RxQueue {
    bytes: [AtomicU8; 64],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl RxQueue {
    pub const fn new() -> Self {
        Self {
            bytes: [const { AtomicU8::new(0) }; 64],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, byte: u8) {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let next = (head + 1) % self.bytes.len();
        if next == tail {
            return;
        }
        self.bytes[head].store(byte, Ordering::Relaxed);
        self.head.store(next, Ordering::Release);
    }

    pub fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let byte = self.bytes[tail].load(Ordering::Relaxed);
        self.tail
            .store((tail + 1) % self.bytes.len(), Ordering::Release);
        Some(byte)
    }
}

impl Default for RxQueue {
    fn default() -> Self {
        Self::new()
    }
}

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

    /// Push a byte observed by a UART RX interrupt handler.  The default
    /// implementation drops it for ports without interrupt-driven console.
    fn stash_rx(_byte: u8) {}

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

#[cfg(test)]
mod rx_queue_tests {
    use super::RxQueue;

    #[test]
    fn rx_queue_is_fifo_and_drops_when_full() {
        let queue = RxQueue::new();
        for byte in 0..63u8 {
            queue.push(byte);
        }
        assert_eq!(queue.pop(), Some(0));
        queue.push(200);
        for byte in 1..63u8 {
            assert_eq!(queue.pop(), Some(byte));
        }
        assert_eq!(queue.pop(), Some(200));
        assert_eq!(queue.pop(), None);
    }
}
