//! Rust server application loaded by the root task.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::IPC_BUF_SERVER_VA;
use vifast_abi::KError;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const SERVER: &[u8] = b"Server\n";
const IPC_BYTES: usize = 11;

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

#[no_mangle]
pub extern "C" fn server_main() -> ! {
    write(SERVER);
    loop {
        let result = vifast_user::reply_recv(1, 4, 0x55, 42, 0, 0);
        if result.error == KError::Ok {
            let _ = vifast_user::debug_write(IPC_BUF_SERVER_VA as u64, IPC_BYTES as u64);
            let _ = vifast_user::signal(4);
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"SERVER PANIC\n");
    loop {
        vifast_user::yield_now();
    }
}
