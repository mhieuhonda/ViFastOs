//! Rust client application loaded by the root task.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{DATA_VA, IPC_BUF_CLIENT_VA};
use vifast_abi::vfs;
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
const VFS_OK: &[u8] = b"VFS OK\n";
const VFS_BAD: &[u8] = b"VFS BAD\n";
const VFS_STAT_OK: &[u8] = b"VFS STAT OK\n";
const PERCPU_OK: &[u8] = b"PERCPU CLOBBER OK\n";
const PERCPU_BAD: &[u8] = b"PERCPU CLOBBER BAD\n";

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

unsafe fn copy_to(va: u64, bytes: &[u8]) {
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), va as *mut u8, bytes.len());
}

fn pack_name(name: &[u8]) -> (u64, u64) {
    let len = core::cmp::min(name.len(), vfs::MAX_NAME);
    let mut bytes = [0u8; vfs::MAX_NAME];
    bytes[..len].copy_from_slice(&name[..len]);
    (u64::from_le_bytes(bytes), len as u64)
}

fn percpu_clobber_ok() -> bool {
    #[cfg(target_arch = "riscv64")]
    {
        let saved: u64;
        let after: u64;
        let sentinel = 0xdead_beef_dead_beefu64;
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) saved);
            core::arch::asm!("mv tp, {}", in(reg) sentinel);
        }
        let result = vifast_user::get_time();
        unsafe {
            core::arch::asm!("mv {}, tp", out(reg) after);
            core::arch::asm!("mv tp, {}", in(reg) saved);
        }
        result.error == KError::Ok && after == sentinel
    }
    #[cfg(target_arch = "aarch64")]
    {
        let saved: u64;
        let after: u64;
        let sentinel = 0xdead_beef_dead_beefu64;
        unsafe {
            core::arch::asm!("mrs {}, tpidr_el0", out(reg) saved);
            core::arch::asm!("msr tpidr_el0, {}", in(reg) sentinel);
        }
        let result = vifast_user::get_time();
        unsafe {
            core::arch::asm!("mrs {}, tpidr_el0", out(reg) after);
            core::arch::asm!("msr tpidr_el0, {}", in(reg) saved);
        }
        result.error == KError::Ok && after == sentinel
    }
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!("mov ax, 0", out("ax") _, options(nostack, preserves_flags));
            core::arch::asm!("mov gs, ax", in("ax") 0u16, options(nostack, preserves_flags));
        }
        let result = vifast_user::get_time();
        let selector: u16;
        unsafe {
            core::arch::asm!("mov {0:x}, gs", out(reg) selector, options(nostack, preserves_flags));
        }
        result.error == KError::Ok && selector == 0
    }
}

fn vfs_open(name: &[u8], flags: u64) -> Option<(u64, u64)> {
    let (packed, len) = pack_name(name);
    let reply = unsafe { vifast_user::call_message(1, vfs::OPEN, packed, len, flags) };
    if reply.error == KError::Ok && reply.label == vfs::OK {
        Some((reply.data[0], reply.data[1]))
    } else {
        None
    }
}

fn vfs_read(fd: u64, offset: u64, len: usize, out: &mut [u8; vfs::MAX_IO]) -> Option<usize> {
    let reply = unsafe { vifast_user::call_message(1, vfs::READ, fd, offset, len as u64) };
    if reply.error != KError::Ok || reply.label == vfs::ERR {
        return None;
    }
    let count = core::cmp::min(reply.label as usize, out.len());
    for index in 0..count {
        let word = reply.data[index / 8];
        out[index] = (word >> ((index % 8) * 8)) as u8;
    }
    Some(count)
}

fn vfs_write(fd: u64, offset: u64, bytes: &[u8]) -> Option<usize> {
    let len = core::cmp::min(bytes.len(), vfs::MAX_IO);
    unsafe {
        let dst = IPC_BUF_CLIENT_VA as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
    }
    let reply = unsafe { vifast_user::call_message(1, vfs::WRITE, fd, offset, len as u64) };
    if reply.error == KError::Ok && reply.label != vfs::ERR {
        Some(reply.label as usize)
    } else {
        None
    }
}

fn vfs_stat(fd: u64) -> Option<u64> {
    let reply = unsafe { vifast_user::call_message(1, vfs::STAT, fd, 0, 0) };
    if reply.error == KError::Ok && reply.label == vfs::OK {
        Some(reply.data[0])
    } else {
        None
    }
}

fn vfs_close(fd: u64) -> Option<()> {
    let reply = unsafe { vifast_user::call_message(1, vfs::CLOSE, fd, 0, 0) };
    if reply.error == KError::Ok && reply.label == vfs::OK {
        Some(())
    } else {
        None
    }
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
    let first = vifast_user::call_with_cap(1, 0x1234, 0, 0, 5, 5);
    let first_poll = vifast_user::poll(5);
    write(if first_poll.error == KError::Ok {
        CAP_OK
    } else {
        CAP_BAD
    });
    write(if first.error == KError::Ok {
        REPLY_OK
    } else {
        REPLY_BAD
    });
    for _ in 0..4 {
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

    let mut file_buf = [0u8; vfs::MAX_IO];
    let mut vfs_ok = true;
    if let Some((fd, _)) = vfs_open(b"/motd", 0) {
        vfs_ok &= vfs_read(fd, 0, vfs::MAX_IO, &mut file_buf).is_some_and(|n| {
            let _ = vifast_user::debug_write(file_buf.as_ptr() as u64, n as u64);
            n != 0
        });
        let _ = vfs_close(fd);
    } else {
        vfs_ok = false;
    }
    if let Some((fd, _)) = vfs_open(b"/scratch", 6) {
        vfs_ok &= vfs_write(fd, 0, WRITE_TEXT).is_some();
        let _ = vfs_close(fd);
        if let Some((fd, _)) = vfs_open(b"/scratch", 0) {
            vfs_ok &= vfs_read(fd, 0, vfs::MAX_IO, &mut file_buf).is_some_and(|n| {
                let _ = vifast_user::debug_write(file_buf.as_ptr() as u64, n as u64);
                n == WRITE_TEXT.len()
            });
            vfs_ok &= vfs_stat(fd) == Some(WRITE_TEXT.len() as u64);
            let _ = vfs_close(fd);
        } else {
            vfs_ok = false;
        }
    } else {
        vfs_ok = false;
    }
    write(if vfs_ok { VFS_OK } else { VFS_BAD });
    write(if vfs_ok { VFS_STAT_OK } else { VFS_BAD });
    write(if percpu_clobber_ok() {
        PERCPU_OK
    } else {
        PERCPU_BAD
    });

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
