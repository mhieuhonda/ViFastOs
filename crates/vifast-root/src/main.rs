//! Rust root task image.
//!
//! The bootstrap actions mirror the old `user_root` assembly.  The image is
//! linked as an ELF at `ROOT_ELF_BASE`; the runtime loads PT_LOAD segments
//! before entering `_start`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{
    CLIENT_STACK_TOP, CLIENT_TCB_SLOT, ENDPOINT_SLOT, FAULT1_STACK_TOP, FAULT1_TCB_SLOT,
    FAULT2_STACK_TOP, FAULT2_TCB_SLOT, FAULT3_STACK_TOP, FAULT3_TCB_SLOT, META_UNTYPED_SLOT,
    NOTIFICATION_SLOT, ROOT_NOTIFICATION_SLOT, ROOT_VSPACE_SLOT, SERVER_STACK_TOP, SERVER_TCB_SLOT,
    SERVER_VSPACE_SLOT, USER_ENTRY_CLIENT, USER_ENTRY_FAULT1, USER_ENTRY_FAULT2, USER_ENTRY_FAULT3,
    USER_ENTRY_SERVER,
};
use vifast_abi::{KError, ObjType, Rights};
use vifast_user::SyscallResult;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const ROOT_RUST: &[u8] = b"ROOT RUST\n";
const ROOT_PANIC: &[u8] = b"ROOT PANIC\n";

fn check(result: SyscallResult) {
    if result.error != KError::Ok {
        panic!("root syscall failed: {:?}", result.error);
    }
}

fn write(bytes: &[u8]) {
    check(vifast_user::debug_write(
        bytes.as_ptr() as u64,
        bytes.len() as u64,
    ));
}

fn retype(ty: ObjType, slot: u8) {
    check(vifast_user::retype(
        META_UNTYPED_SLOT as u8,
        ty as u64,
        slot,
    ));
}

fn copy(src: u8, tcb: u8, dst: u8, rights: Rights) {
    check(vifast_user::cap_copy(src, tcb, dst, rights.0));
}

#[no_mangle]
pub extern "C" fn root_main() -> ! {
    write(ROOT_RUST);

    retype(ObjType::Endpoint, ENDPOINT_SLOT as u8);
    retype(ObjType::Tcb, SERVER_TCB_SLOT as u8);
    retype(ObjType::Tcb, CLIENT_TCB_SLOT as u8);
    retype(ObjType::Notification, NOTIFICATION_SLOT as u8);
    retype(ObjType::Notification, ROOT_NOTIFICATION_SLOT as u8);
    retype(ObjType::Tcb, FAULT1_TCB_SLOT as u8);
    retype(ObjType::Tcb, FAULT2_TCB_SLOT as u8);
    retype(ObjType::Tcb, FAULT3_TCB_SLOT as u8);

    copy(ENDPOINT_SLOT as u8, SERVER_TCB_SLOT as u8, 1, Rights::READ);
    copy(ENDPOINT_SLOT as u8, CLIENT_TCB_SLOT as u8, 1, Rights::WRITE);
    copy(
        ROOT_VSPACE_SLOT as u8,
        CLIENT_TCB_SLOT as u8,
        2,
        Rights::WRITE,
    );
    copy(
        vifast_abi::user_layout::DATA_UNTYPED_SLOT as u8,
        CLIENT_TCB_SLOT as u8,
        3,
        Rights::WRITE,
    );
    copy(
        NOTIFICATION_SLOT as u8,
        CLIENT_TCB_SLOT as u8,
        5,
        Rights::READ | Rights::WRITE | Rights::GRANT,
    );

    check(vifast_user::tcb_configure(
        SERVER_TCB_SLOT as u8,
        USER_ENTRY_SERVER as u64,
        SERVER_STACK_TOP as u64,
        SERVER_VSPACE_SLOT as u8,
        5,
    ));
    check(vifast_user::tcb_configure(
        CLIENT_TCB_SLOT as u8,
        USER_ENTRY_CLIENT as u64,
        CLIENT_STACK_TOP as u64,
        ROOT_VSPACE_SLOT as u8,
        5,
    ));
    check(vifast_user::tcb_resume(SERVER_TCB_SLOT as u8));
    check(vifast_user::tcb_resume(CLIENT_TCB_SLOT as u8));

    check(vifast_user::tcb_configure(
        FAULT1_TCB_SLOT as u8,
        USER_ENTRY_FAULT1 as u64,
        FAULT1_STACK_TOP as u64,
        ROOT_VSPACE_SLOT as u8,
        6,
    ));
    check(vifast_user::tcb_resume(FAULT1_TCB_SLOT as u8));
    check(vifast_user::tcb_configure(
        FAULT2_TCB_SLOT as u8,
        USER_ENTRY_FAULT2 as u64,
        FAULT2_STACK_TOP as u64,
        ROOT_VSPACE_SLOT as u8,
        6,
    ));
    check(vifast_user::tcb_resume(FAULT2_TCB_SLOT as u8));
    check(vifast_user::tcb_configure(
        FAULT3_TCB_SLOT as u8,
        USER_ENTRY_FAULT3 as u64,
        FAULT3_STACK_TOP as u64,
        ROOT_VSPACE_SLOT as u8,
        6,
    ));
    check(vifast_user::tcb_resume(FAULT3_TCB_SLOT as u8));

    loop {
        let _ = vifast_user::wait(ROOT_NOTIFICATION_SLOT as u8);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = vifast_user::debug_write(ROOT_PANIC.as_ptr() as u64, ROOT_PANIC.len() as u64);
    vifast_user::exit()
}
