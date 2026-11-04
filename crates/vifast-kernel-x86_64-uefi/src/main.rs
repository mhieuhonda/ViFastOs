#![no_std]
#![no_main]

use core::ffi::c_void;
use core::panic::PanicInfo;
use vifast_arch::Arch;
use vifast_platform_x86_64::X86Platform;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct LoadedImage {
    revision: u32,
    _pad0: u32,
    parent_handle: *mut c_void,
    system_table: *mut c_void,
    device_handle: *mut c_void,
    file_path: *mut c_void,
    reserved: *mut c_void,
    load_options_size: u32,
    _pad1: u32,
    load_options: *mut c_void,
    image_base: *mut c_void,
    image_size: u64,
}

const LOADED_IMAGE_GUID: Guid = Guid {
    data1: 0x5b1b31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

type HandleProtocol =
    unsafe extern "efiapi" fn(*mut c_void, *const Guid, *mut *mut c_void) -> usize;

unsafe fn check_loaded_image(image_handle: *mut c_void, system_table: *mut c_void) -> bool {
    if image_handle.is_null() || system_table.is_null() {
        return false;
    }
    // EFI_SYSTEM_TABLE.BootServices is at offset 96.
    let boot_services = *((system_table as *const u8).add(96) as *const *mut c_void);
    if boot_services.is_null() {
        return false;
    }
    // EFI_BOOT_SERVICES.HandleProtocol is at offset 152.
    let handle_protocol = *((boot_services as *const u8).add(152) as *const HandleProtocol);
    let mut interface: *mut c_void = core::ptr::null_mut();
    let status = handle_protocol(
        image_handle,
        &LOADED_IMAGE_GUID,
        &mut interface as *mut *mut c_void,
    );
    if status != 0 || interface.is_null() {
        return false;
    }
    let image = &*(interface as *const LoadedImage);
    let base = image.image_base as u64;
    let end = base.saturating_add(image.image_size);
    base < 0x4000_0000 && end <= 0x4000_0000
}
use vifast_runtime::{boot, PlatformInfo, UserImage};

core::arch::global_asm!(
    include_str!("user.S"),
    DATA_VA = const vifast_abi::user_layout::DATA_VA,
    SERVER_STACK_BASE = const vifast_abi::user_layout::SERVER_STACK_BASE,
    SERVER_STACK_TOP = const vifast_abi::user_layout::SERVER_STACK_TOP,
    CLIENT_STACK_TOP = const vifast_abi::user_layout::CLIENT_STACK_TOP,
    FAULT1_STACK_TOP = const vifast_abi::user_layout::FAULT1_STACK_TOP,
    FAULT2_STACK_TOP = const vifast_abi::user_layout::FAULT2_STACK_TOP,
    FAULT3_STACK_TOP = const vifast_abi::user_layout::FAULT3_STACK_TOP,
    SERVER_VSPACE_SLOT = const vifast_abi::user_layout::SERVER_VSPACE_SLOT,
);

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
pub extern "efiapi" fn efi_main(image_handle: *mut c_void, system_table: *mut c_void) -> usize {
    if !unsafe { check_loaded_image(image_handle, system_table) } {
        <X86Platform as Arch>::write_str("\r\nX86 IMAGE OUTSIDE 0..1 GiB\r\n");
        X86Platform::exit(1);
    }
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
