#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_arch::{Arch, BootInfo};
use vifast_platform_x86_64::X86_64;

core::arch::global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    vifast_kernel::boot::<X86_64>(BootInfo {
        hartid: 0,
        dtb: 0,
        memory_base: 0,
        memory_size: 128 * 1024 * 1024,
    })
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    X86_64::write_str("\r\nPANIC\r\n");
    X86_64::exit(1)
}
