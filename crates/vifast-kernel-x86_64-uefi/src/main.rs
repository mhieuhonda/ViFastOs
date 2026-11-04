#![no_std]
#![no_main]

use core::ffi::c_void;
use core::panic::PanicInfo;
use vifast_arch::Arch;
use vifast_platform_x86_64::X86Platform;
use vifast_runtime::{boot, PlatformInfo, UserImage};

core::arch::global_asm!(include_str!("user.S"));

extern "C" {
    static user_program_start: u8;
    static user_program_end: u8;
    fn user_thread_a();
    fn user_thread_b();
    fn user_root();
}

fn user_image() -> UserImage {
    let start = core::ptr::addr_of!(user_program_start) as usize;
    let end = core::ptr::addr_of!(user_program_end) as usize;
    let entry_root = user_root as *const () as usize - start;
    let entry_a = user_thread_a as *const () as usize - start;
    let entry_b = user_thread_b as *const () as usize - start;
    UserImage {
        code: unsafe { core::slice::from_raw_parts(start as *const u8, end - start) },
        entry_a,
        entry_b,
        entry_root,
    }
}

#[no_mangle]
pub extern "efiapi" fn efi_main(_image_handle: *mut c_void, system_table: *mut c_void) -> usize {
    boot::<X86Platform>(
        PlatformInfo {
            system_table: system_table as usize,
            memory_base: 0,
            memory_size: 0,
            ..PlatformInfo::default()
        },
        user_image(),
    )
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    X86Platform::write_str("\r\nPANIC\r\n");
    X86Platform::exit(1)
}
