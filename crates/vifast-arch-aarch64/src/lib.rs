//! AArch64 architecture port.
//!
//! Target: QEMU `virt` (PL011 UART at 0x0900_0000).  The port starts at EL1
//! or drops from EL2 to EL1 before entering Rust.  MMU, GIC and the generic
//! timer interrupt path are the next layer.

#![no_std]

use vifast_arch::{Arch, RxQueue};

pub struct Aarch64;

static RX_QUEUE: RxQueue = RxQueue::new();

const PL011_BASE: usize = 0x0900_0000;
const PL011_DR: usize = 0x00;
const PL011_FR: usize = 0x18;
const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;

impl Arch for Aarch64 {
    const NAME: &'static str = "aarch64";

    fn init() {
        // Mask interrupts until the interrupt controller and vector table are
        // ready.  The user-mode bring-up only uses synchronous SVC traps.
        unsafe {
            core::arch::asm!("msr daifset, #0xf");
        }
        // Enable FP/SIMD at EL1 only (vòng 87, audit #3 P1-2). Trước đây để
        // FPEN=0b11, tức EL0 cũng dùng được FP/SIMD; kernel không lưu q0-q31
        // khi switch task nên user sau đọc được thanh ghi còn sót của user
        // trước (rò rỉ liên tiến trình). EL1 vẫn cần FP/SIMD vì compiler dùng
        // cho bulk memory. User build softfloat và ELF client/server không có
        // lệnh FP/SIMD nào (đã kiểm bằng llvm-objdump), nên chặn EL0 là an
        // toàn; nếu sau này có ELF dùng FP, nó sẽ trap rõ ràng thay vì rò rỉ.
        unsafe {
            let mut cpacr: u64;
            core::arch::asm!("mrs {}, cpacr_el1", out(reg) cpacr);
            cpacr &= !(3 << 20);
            cpacr |= 1 << 20;
            core::arch::asm!("msr cpacr_el1, {}", in(reg) cpacr);
            core::arch::asm!("isb");
        }
    }

    fn putc(byte: u8) {
        unsafe {
            let fr = (PL011_BASE + PL011_FR) as *const u32;
            while core::ptr::read_volatile(fr) & FR_TXFF != 0 {
                core::hint::spin_loop();
            }
            core::ptr::write_volatile((PL011_BASE + PL011_DR) as *mut u32, byte as u32);
        }
    }

    fn getc() -> Option<u8> {
        if let Some(byte) = RX_QUEUE.pop() {
            return Some(byte);
        }
        unsafe {
            let fr = (PL011_BASE + PL011_FR) as *const u32;
            if core::ptr::read_volatile(fr) & FR_RXFE != 0 {
                return None;
            }
            Some(core::ptr::read_volatile((PL011_BASE + PL011_DR) as *const u32) as u8)
        }
    }

    fn stash_rx(byte: u8) {
        RX_QUEUE.push(byte);
    }

    fn now() -> u64 {
        let value: u64;
        unsafe {
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) value);
        }
        value
    }

    fn irq_disable() {
        unsafe {
            core::arch::asm!("msr daifset, #2");
        }
    }

    fn irq_enable() {
        unsafe {
            core::arch::asm!("msr daifclr, #2");
        }
    }

    fn send_ipi(_hart: usize) {
        // GIC SGI will be added with SMP support.
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("wfi");
            }
        }
    }

    fn exit(code: u8) -> ! {
        // PSCI SYSTEM_OFF.
        let _ = code;
        unsafe {
            core::arch::asm!("hvc #0", in("x0") 0x8400_0008u64);
        }
        Self::halt()
    }
}
