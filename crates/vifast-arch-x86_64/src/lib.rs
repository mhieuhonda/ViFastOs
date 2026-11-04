//! x86_64 architecture port.
//!
//! Target: QEMU PC machine booted through the multiboot protocol.  The 32-bit
//! entry code in the binary crate enables PAE/long mode and jumps here.  The
//! minimal port uses COM1 for early output; IDT/APIC/SMP come next.

#![no_std]

use vifast_arch::Arch;

pub struct X86_64;

const COM1: u16 = 0x3F8;
const COM1_LSR: u16 = 0x3FD;
const LSR_THRE: u8 = 0x20;
/// QEMU `isa-debug-exit` port used by the test harness.
const DEBUG_EXIT_PORT: u16 = 0x501;

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value);
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value);
    value
}

impl Arch for X86_64 {
    const NAME: &'static str = "x86_64";

    fn init() {
        unsafe {
            outb(COM1 + 1, 0x00); // disable interrupts
            outb(COM1 + 3, 0x80); // DLAB
            outb(COM1 + 0, 0x03); // divisor low: 38400 baud
            outb(COM1 + 1, 0x00); // divisor high
            outb(COM1 + 3, 0x03); // 8N1
            outb(COM1 + 2, 0xC7); // FIFO
            outb(COM1 + 4, 0x0B); // IRQs, RTS/DSR
        }
    }

    fn putc(byte: u8) {
        unsafe {
            while inb(COM1_LSR) & LSR_THRE == 0 {
                core::hint::spin_loop();
            }
            outb(COM1, byte);
        }
    }

    fn now() -> u64 {
        let lo: u32;
        let hi: u32;
        unsafe {
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi);
        }
        ((hi as u64) << 32) | lo as u64
    }

    fn irq_disable() {
        unsafe {
            core::arch::asm!("cli");
        }
    }

    fn irq_enable() {
        unsafe {
            core::arch::asm!("sti");
        }
    }

    fn send_ipi(_hart: usize) {
        // LAPIC ICR will be added with SMP support.
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }

    fn exit(code: u8) -> ! {
        // With `-device isa-debug-exit`, writing (code << 1) | 1 terminates
        // QEMU.  On real hardware this is simply an unused I/O port, so fall
        // back to halting.
        unsafe {
            outb(DEBUG_EXIT_PORT, (code << 1) | 1);
        }
        Self::halt()
    }
}
