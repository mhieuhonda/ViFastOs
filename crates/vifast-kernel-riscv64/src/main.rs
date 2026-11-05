#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_arch::Arch;
use vifast_platform_riscv64::RiscvPlatform;
use vifast_runtime::{boot, PlatformInfo, UserImage};

static ROOT_ELF: &[u8] = include_bytes!(env!("VIFAST_ROOT_ELF"));
static SERVER_ELF: &[u8] = include_bytes!(env!("VIFAST_SERVER_ELF"));
static CLIENT_ELF: &[u8] = include_bytes!(env!("VIFAST_CLIENT_ELF"));

#[cfg(feature = "test-boot-handoff")]
core::arch::global_asm!(include_str!("boot_handoff.S"));
#[cfg(not(feature = "test-boot-handoff"))]
core::arch::global_asm!(include_str!("boot.S"));
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

#[no_mangle]
pub extern "C" fn rust_main(hartid: usize, dtb: usize) -> ! {
    boot::<RiscvPlatform>(
        PlatformInfo {
            hartid,
            dtb: dtb as u64,
            memory_base: 0x8000_0000,
            memory_size: 0x800_0000,
            ..PlatformInfo::default()
        },
        user_image(),
    )
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    RiscvPlatform::write_str("\r\nPANIC\r\n");
    RiscvPlatform::exit(1)
}
