//! RISC-V 64-bit architecture port.
//!
//! Target: QEMU `virt` with OpenSBI.  The kernel is loaded at 0x8020_0000 and
//! uses the legacy 16550 UART at 0x1000_0000.  Timer, PLIC and full trap
//! handling are intentionally the next layer above this minimal port.

#![no_std]

use vifast_arch::Arch;

pub struct Riscv64;

const UART_BASE: usize = 0x1000_0000;
const UART_THR: usize = 0;
const UART_LSR: usize = 5;
const LSR_DR: u8 = 0x01;
const LSR_THRE: u8 = 0x20;

impl Arch for Riscv64 {
    const NAME: &'static str = "riscv64";

    fn init() {
        // The QEMU 16550 is usable without a full baud-rate setup.  Configure
        // 8N1 anyway so the same code is closer to real hardware.
        unsafe {
            let lcr = (UART_BASE + 3) as *mut u8;
            core::ptr::write_volatile(lcr, 0x03);
        }
    }

    fn putc(byte: u8) {
        unsafe {
            let lsr = (UART_BASE + UART_LSR) as *const u8;
            while core::ptr::read_volatile(lsr) & LSR_THRE == 0 {
                core::hint::spin_loop();
            }
            core::ptr::write_volatile((UART_BASE + UART_THR) as *mut u8, byte);
        }
    }

    fn getc() -> Option<u8> {
        unsafe {
            let lsr = (UART_BASE + UART_LSR) as *const u8;
            if core::ptr::read_volatile(lsr) & LSR_DR == 0 {
                return None;
            }
            Some(core::ptr::read_volatile(
                (UART_BASE + UART_THR) as *const u8,
            ))
        }
    }

    fn now() -> u64 {
        let value: u64;
        unsafe {
            core::arch::asm!("csrr {}, time", out(reg) value);
        }
        value
    }

    fn irq_disable() {
        unsafe {
            core::arch::asm!("csrrc zero, sstatus, {}", in(reg) 2u64);
        }
    }

    fn irq_enable() {
        unsafe {
            core::arch::asm!("csrs sstatus, {}", in(reg) 2u64);
        }
    }

    fn send_ipi(hart: usize) {
        let mask = 1usize << hart;
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a7") 0x7350_49usize,
                in("a6") 0usize,
                in("a0") mask,
                in("a1") 0usize,
            );
        }
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("wfi");
            }
        }
    }

    fn exit(code: u8) -> ! {
        // SBI SRST extension: a7 = "SRST", a6 = 0, a0 = type (0 = shutdown).
        let _ = code;
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a7") 0x5352_5354usize,
                in("a6") 0usize,
                in("a0") 0usize,
                in("a1") 0usize,
            );
        }
        Self::halt()
    }
}
