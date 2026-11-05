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

#[repr(C)]
struct ConfigEntry {
    vendor_guid: Guid,
    vendor_table: *mut c_void,
}

const ACPI_20_GUID: Guid = Guid {
    data1: 0x8868_e871,
    data2: 0xe4f1,
    data3: 0x11d3,
    data4: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};

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

static ROOT_ELF: &[u8] = include_bytes!(env!("VIFAST_ROOT_ELF"));
static SERVER_ELF: &[u8] = include_bytes!(env!("VIFAST_SERVER_ELF"));
static CLIENT_ELF: &[u8] = include_bytes!(env!("VIFAST_CLIENT_ELF"));

core::arch::global_asm!(
    include_str!("user.S"),
    DATA_VA = const vifast_abi::user_layout::DATA_VA,
    ROOT_ELF_STACK_BASE = const vifast_abi::user_layout::ROOT_ELF_STACK_BASE,
    IPC_BUF_SERVER_VA = const vifast_abi::user_layout::IPC_BUF_SERVER_VA,
    IPC_BUF_CLIENT_VA = const vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
    SERVER_STACK_BASE = const vifast_abi::user_layout::SERVER_STACK_BASE,
);

extern "C" {
    static user_program_start: u8;
    static user_program_end: u8;
    fn user_thread_a();
    fn user_thread_b();
    fn user_entry_server();
    fn user_entry_client();
    fn user_entry_fault1();
    fn user_entry_fault2();
    fn user_entry_fault3();
    fn user_entry_fault4();
}

fn user_image() -> UserImage {
    let start = core::ptr::addr_of!(user_program_start) as usize;
    let end = core::ptr::addr_of!(user_program_end) as usize;
    let entry_a = user_thread_a as *const () as usize - start;
    let entry_b = user_thread_b as *const () as usize - start;
    assert_eq!(
        user_entry_server as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_SERVER - vifast_abi::user_layout::CODE_VA
    );
    assert_eq!(
        user_entry_client as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_CLIENT - vifast_abi::user_layout::CODE_VA
    );
    assert_eq!(
        user_entry_fault1 as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_FAULT1 - vifast_abi::user_layout::CODE_VA
    );
    assert_eq!(
        user_entry_fault2 as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_FAULT2 - vifast_abi::user_layout::CODE_VA
    );
    assert_eq!(
        user_entry_fault3 as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_FAULT3 - vifast_abi::user_layout::CODE_VA
    );
    assert_eq!(
        user_entry_fault4 as *const () as usize - start,
        vifast_abi::user_layout::USER_ENTRY_FAULT4 - vifast_abi::user_layout::CODE_VA
    );
    UserImage {
        code: unsafe { core::slice::from_raw_parts(start as *const u8, end - start) },
        entry_a,
        entry_b,
        root_elf: ROOT_ELF,
        server_elf: SERVER_ELF,
        client_elf: CLIENT_ELF,
    }
}

unsafe fn find_acpi_rsdp(system_table: *mut c_void) -> u64 {
    if system_table.is_null() {
        return 0;
    }
    let count = *((system_table as *const u8).add(104) as *const usize);
    let entries = *((system_table as *const u8).add(112) as *const *const ConfigEntry);
    if entries.is_null() || count > 1024 {
        return 0;
    }
    for index in 0..count {
        let entry = &*entries.add(index);
        let guid = &entry.vendor_guid;
        if guid.data1 == ACPI_20_GUID.data1
            && guid.data2 == ACPI_20_GUID.data2
            && guid.data3 == ACPI_20_GUID.data3
            && guid.data4 == ACPI_20_GUID.data4
        {
            return entry.vendor_table as u64;
        }
    }
    0
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
            acpi_rsdp: unsafe { find_acpi_rsdp(system_table) },
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
