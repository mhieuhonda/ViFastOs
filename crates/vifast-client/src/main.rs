//! Rust client application loaded by the root task.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{DATA_VA, IPC_BUF_CLIENT_VA};
use vifast_abi::{KError, ObjType, Rights};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const CLIENT: &[u8] = b"Client\n";
const MAPPED: &[u8] = b"Mapped\n";
const ZERO_OK: &[u8] = b"Zero OK\n";
const ZERO_BAD: &[u8] = b"Zero BAD\n";
const CAP_OK: &[u8] = b"Cap OK\n";
const CAP_BAD: &[u8] = b"Cap BAD\n";
const REPLY_OK: &[u8] = b"Reply OK\n";
const REPLY_BAD: &[u8] = b"Reply BAD\n";
const WRITE_TEXT: &[u8] = b"ViFastOS write\n";
const IPC_TEXT: &[u8] = b"IPC BUF OK\n";

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

unsafe fn copy_to(va: u64, bytes: &[u8]) {
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), va as *mut u8, bytes.len());
}

#[no_mangle]
pub extern "C" fn client_main() -> ! {
    let _ = vifast_user::retype(3, ObjType::Frame as u64, 4);
    let first_mapping =
        vifast_user::frame_map(2, 4, DATA_VA as u64, (Rights::READ | Rights::WRITE).0);
    if first_mapping.error == KError::Ok {
        let mapped_va = first_mapping.value;
        unsafe { copy_to(mapped_va, MAPPED) };
        let _ = vifast_user::debug_write(mapped_va, MAPPED.len() as u64);
    }

    let motd = vifast_user::open(b"/motd".as_ptr() as u64, 5, 0);
    if motd.error == KError::Ok {
        let n = vifast_user::read(motd.value, DATA_VA as u64, 64);
        if n.error == KError::Ok {
            let _ = vifast_user::debug_write(DATA_VA as u64, n.value);
        }
        let _ = vifast_user::close(motd.value);
    }
    let scratch = vifast_user::open(b"/scratch".as_ptr() as u64, 8, 6);
    if scratch.error == KError::Ok {
        let _ = vifast_user::write(
            scratch.value,
            WRITE_TEXT.as_ptr() as u64,
            WRITE_TEXT.len() as u64,
        );
        let _ = vifast_user::close(scratch.value);
        let read_back = vifast_user::open(b"/scratch".as_ptr() as u64, 8, 0);
        if read_back.error == KError::Ok {
            let n = vifast_user::read(read_back.value, DATA_VA as u64, 64);
            if n.error == KError::Ok {
                let _ = vifast_user::debug_write(DATA_VA as u64, n.value);
            }
            let _ = vifast_user::close(read_back.value);
        }
    }

    unsafe { core::ptr::write_volatile(DATA_VA as *mut u64, 0xdead_beef_dead_beef) };
    let _ = vifast_user::frame_unmap(2, DATA_VA as u64, 4);
    let _ = vifast_user::cap_revoke(3);
    let _ = vifast_user::retype(3, ObjType::Frame as u64, 4);
    let mapped = vifast_user::frame_map(2, 4, DATA_VA as u64, (Rights::READ | Rights::WRITE).0);
    if mapped.error == KError::Ok {
        let value = unsafe { core::ptr::read_volatile(DATA_VA as *const u64) };
        write(if value == 0 { ZERO_OK } else { ZERO_BAD });
    } else {
        write(ZERO_BAD);
    }
    let _ = vifast_user::frame_unmap(2, DATA_VA as u64, 4);
    write(CLIENT);

    unsafe { copy_to(IPC_BUF_CLIENT_VA as u64, IPC_TEXT) };
    for _ in 0..5 {
        let reply = vifast_user::call(1, 0x1234, 0, 0, 5);
        let polled = vifast_user::poll(5);
        write(if polled.error == KError::Ok {
            CAP_OK
        } else {
            CAP_BAD
        });
        write(if reply.error == KError::Ok {
            REPLY_OK
        } else {
            REPLY_BAD
        });
    }
    loop {
        vifast_user::yield_now();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"CLIENT PANIC\n");
    loop {
        vifast_user::yield_now();
    }
}
