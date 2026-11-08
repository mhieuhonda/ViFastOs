//! Rust client application loaded by the root task.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::pager::{LABEL_KILL, LABEL_LOOP, LABEL_RUN};
use vifast_abi::user_layout::{
    CLIENT_CMD_SLOT, CLIENT_IRQ_NOTIFICATION_SLOT, CLIENT_IRQ_SLOT, CLIENT_UART_IRQ_SLOT,
    CLIENT_UART_NOTIFICATION_SLOT, DATA_VA, IPC_BUF_CLIENT_VA,
};
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
const WRITE_TEXT: &[u8] = b"ViFastOs write\n";
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

/// Copy `path\0args\0` để root tách được cả hai (vòng 102).
fn copy_path_args(path: &[u8], args: &[u8]) -> usize {
    let plen = core::cmp::min(path.len(), vfs::MAX_PATH - 1);
    let alen = core::cmp::min(args.len(), 31);
    unsafe {
        let base = IPC_BUF_CLIENT_VA as *mut u8;
        core::ptr::copy_nonoverlapping(path.as_ptr(), base, plen);
        core::ptr::write_volatile(base.add(plen), 0);
        core::ptr::copy_nonoverlapping(args.as_ptr(), base.add(plen + 1), alen);
        core::ptr::write_volatile(base.add(plen + 1 + alen), 0);
    }
    plen
}

fn copy_path(path: &[u8]) -> usize {
    let len = core::cmp::min(path.len(), vfs::MAX_PATH - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(path.as_ptr(), IPC_BUF_CLIENT_VA as *mut u8, len);
        // NUL-terminate để root đọc path từ IPC buffer (vòng 46).
        core::ptr::write_volatile((IPC_BUF_CLIENT_VA as *mut u8).add(len), 0);
    }
    len
}

fn vfs_reply(reply: vifast_user::MessageResult) -> Result<vifast_user::MessageResult, u64> {
    if reply.error != KError::Ok {
        return Err(vfs::EIO);
    }
    if reply.label == vfs::ERR {
        return Err(reply.data[0]);
    }
    Ok(reply)
}

fn vfs_open_via(slot: u8, name: &[u8], flags: u64) -> Option<(u64, u64)> {
    let len = copy_path(name);
    let reply = unsafe { vifast_user::call_message(slot, vfs::OPEN, flags, len as u64, 0) };
    let reply = vfs_reply(reply).ok()?;
    if reply.label == vfs::OK {
        Some((reply.data[0], reply.data[1]))
    } else {
        None
    }
}

fn vfs_open(name: &[u8], flags: u64) -> Option<(u64, u64)> {
    vfs_open_via(1, name, flags)
}

fn vfs_open_error(name: &[u8], flags: u64) -> u64 {
    let len = copy_path(name);
    let reply = unsafe { vifast_user::call_message(1, vfs::OPEN, flags, len as u64, 0) };
    match vfs_reply(reply) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

fn vfs_read_via(
    slot: u8,
    fd: u64,
    offset: u64,
    len: usize,
    out: &mut [u8; vfs::MAX_IO],
) -> Option<usize> {
    let reply = unsafe { vifast_user::call_message(slot, vfs::READ, fd, offset, len as u64) };
    let reply = vfs_reply(reply).ok()?;
    let count = core::cmp::min(reply.label as usize, out.len());
    unsafe {
        core::ptr::copy_nonoverlapping(IPC_BUF_CLIENT_VA as *const u8, out.as_mut_ptr(), count);
    }
    Some(count)
}

fn vfs_read(fd: u64, offset: u64, len: usize, out: &mut [u8; vfs::MAX_IO]) -> Option<usize> {
    vfs_read_via(1, fd, offset, len, out)
}

fn vfs_write_via(slot: u8, fd: u64, offset: u64, bytes: &[u8]) -> Option<usize> {
    let len = core::cmp::min(bytes.len(), vfs::MAX_IO);
    unsafe {
        let dst = IPC_BUF_CLIENT_VA as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, len);
    }
    let reply = unsafe { vifast_user::call_message(slot, vfs::WRITE, fd, offset, len as u64) };
    let reply = vfs_reply(reply).ok()?;
    Some(reply.label as usize)
}

fn vfs_write(fd: u64, offset: u64, bytes: &[u8]) -> Option<usize> {
    vfs_write_via(1, fd, offset, bytes)
}

fn vfs_stat_path(name: &[u8]) -> Option<(u64, u64, u64)> {
    let len = copy_path(name);
    let reply = unsafe { vifast_user::call_message(1, vfs::STAT_PATH, len as u64, 0, 0) };
    let reply = vfs_reply(reply).ok()?;
    if reply.label == vfs::OK {
        Some((reply.data[0], reply.data[1], reply.data[2]))
    } else {
        None
    }
}

fn vfs_stat(fd: u64) -> Option<u64> {
    let reply = unsafe { vifast_user::call_message(1, vfs::STAT, fd, 0, 0) };
    let reply = vfs_reply(reply).ok()?;
    if reply.label == vfs::OK {
        Some(reply.data[0])
    } else {
        None
    }
}

fn vfs_close(fd: u64) -> Option<()> {
    let reply = unsafe { vifast_user::call_message(1, vfs::CLOSE, fd, 0, 0) };
    vfs_reply(reply).ok().map(|_| ())
}

fn vfs_fsync(fd: u64) -> Option<()> {
    let reply = unsafe { vifast_user::call_message(1, vfs::FSYNC, fd, 0, 0) };
    vfs_reply(reply).ok().map(|_| ())
}

fn vfs_mkdir_via(slot: u8, path: &[u8]) -> Result<u64, u64> {
    let len = copy_path(path);
    let reply = unsafe { vifast_user::call_message(slot, vfs::MKDIR, len as u64, 0, 0) };
    vfs_reply(reply).map(|reply| reply.data[0])
}

fn vfs_mkdir(path: &[u8]) -> Result<u64, u64> {
    vfs_mkdir_via(1, path)
}

fn vfs_unlink_via(slot: u8, path: &[u8]) -> Result<(), u64> {
    let len = copy_path(path);
    let reply = unsafe { vifast_user::call_message(slot, vfs::UNLINK, len as u64, 0, 0) };
    vfs_reply(reply).map(|_| ())
}

fn vfs_unlink(path: &[u8]) -> Result<(), u64> {
    vfs_unlink_via(1, path)
}

fn vfs_symlink(target: &[u8], path: &[u8]) -> Result<u64, u64> {
    let target_len = core::cmp::min(target.len(), vfs::MAX_PATH);
    let path_len = core::cmp::min(path.len(), vfs::MAX_PATH);
    unsafe {
        core::ptr::copy_nonoverlapping(target.as_ptr(), IPC_BUF_CLIENT_VA as *mut u8, target_len);
        core::ptr::copy_nonoverlapping(
            path.as_ptr(),
            (IPC_BUF_CLIENT_VA + vfs::MAX_PATH) as *mut u8,
            path_len,
        );
    }
    let reply = unsafe {
        vifast_user::call_message(1, vfs::SYMLINK, target_len as u64, path_len as u64, 0)
    };
    vfs_reply(reply).map(|reply| reply.data[0])
}

fn vfs_readlink(path: &[u8], out: &mut [u8; vfs::MAX_PATH]) -> Option<usize> {
    let len = copy_path(path);
    let reply = unsafe { vifast_user::call_message(1, vfs::READLINK, len as u64, 0, 0) };
    let reply = vfs_reply(reply).ok()?;
    let count = core::cmp::min(reply.label as usize, out.len());
    unsafe {
        core::ptr::copy_nonoverlapping(IPC_BUF_CLIENT_VA as *const u8, out.as_mut_ptr(), count);
    }
    Some(count)
}

fn vfs_link(old: &[u8], new: &[u8]) -> Result<(), u64> {
    let old_len = copy_path(old);
    let new_len = core::cmp::min(new.len(), vfs::MAX_PATH);
    unsafe {
        core::ptr::copy_nonoverlapping(
            new.as_ptr(),
            (IPC_BUF_CLIENT_VA + vfs::MAX_PATH) as *mut u8,
            new_len,
        );
    }
    let reply =
        unsafe { vifast_user::call_message(1, vfs::LINK, old_len as u64, new_len as u64, 0) };
    vfs_reply(reply).map(|_| ())
}

fn vfs_rename(old: &[u8], new: &[u8]) -> Result<(), u64> {
    let old_len = copy_path(old);
    let new_len = core::cmp::min(new.len(), vfs::MAX_PATH);
    unsafe {
        core::ptr::copy_nonoverlapping(
            new.as_ptr(),
            (IPC_BUF_CLIENT_VA + vfs::MAX_PATH) as *mut u8,
            new_len,
        );
    }
    let reply =
        unsafe { vifast_user::call_message(1, vfs::RENAME, old_len as u64, new_len as u64, 0) };
    vfs_reply(reply).map(|_| ())
}

fn vfs_truncate(fd: u64, size: u64) -> Result<(), u64> {
    let reply = unsafe { vifast_user::call_message(1, vfs::TRUNCATE, fd, size, 0) };
    vfs_reply(reply).map(|_| ())
}

fn vfs_readdir(fd: u64, index: u64) -> Result<Option<(u64, [u8; 28], usize)>, u64> {
    let reply = unsafe { vifast_user::call_message(1, vfs::READDIR, fd, index, 0) };
    match vfs_reply(reply) {
        Ok(reply) => {
            if reply.label != vfs::OK {
                return Ok(None);
            }
            // Vòng 83 (audit #2): server trả về độ dài; phải kẹp theo buffer
            // đích trước khi copy, nếu không một reply sai làm ghi quá biên.
            let len = core::cmp::min(reply.data[1] as usize, 28);
            let mut name = [0u8; 28];
            unsafe {
                core::ptr::copy_nonoverlapping(
                    IPC_BUF_CLIENT_VA as *const u8,
                    name.as_mut_ptr(),
                    len,
                );
            }
            Ok(Some((reply.data[0], name, len)))
        }
        Err(vfs::ENOENT) => Ok(None),
        Err(code) => Err(code),
    }
}

fn vfs_fsck() -> bool {
    let reply = unsafe { vifast_user::call_message(1, vfs::FSCK, 0, 0, 0) };
    vfs_reply(reply).is_ok()
}

fn write_u64(mut value: u64) {
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    write(&digits[at..]);
}

/// Vòng 161: nhớ lệnh trước đó cho `!!` và `history` (một ô 32 byte).
/// Dùng con trỏ thô để không tạo tham chiếu tới `static mut`.
static mut LAST_CMD: [u8; 32] = [0; 32];

fn last_cmd_at(index: usize) -> u8 {
    unsafe { *core::ptr::addr_of!(LAST_CMD).cast::<u8>().add(index) }
}

fn remember_last(line: &[u8]) {
    let n = core::cmp::min(line.len(), 32);
    unsafe {
        let dst = core::ptr::addr_of_mut!(LAST_CMD).cast::<u8>();
        core::ptr::copy_nonoverlapping(line.as_ptr(), dst, n);
        let mut i = n;
        while i < 32 {
            *dst.add(i) = 0;
            i += 1;
        }
    }
}

fn last_cmd_into(buf: &mut [u8; 32]) -> usize {
    let mut i = 0;
    while i < 32 {
        let byte = last_cmd_at(i);
        if byte == 0 {
            return i;
        }
        buf[i] = byte;
        i += 1;
    }
    32
}

fn shell_command(line: &[u8]) {
    if line.is_empty() {
        return;
    }
    // `exec <path> [args]` là bí danh của `runfs` — dịch tiền tố rồi xử lý
    // lại, để hai lệnh chia sẻ đúng một đường mã.
    if let Some(rest) = line.strip_prefix(b"exec ") {
        let mut buf = [0u8; 64];
        buf[..6].copy_from_slice(b"runfs ");
        let n = core::cmp::min(rest.len(), 57);
        buf[6..6 + n].copy_from_slice(&rest[..n]);
        shell_command(&buf[..6 + n]);
        return;
    }
    if line == b"history" {
        let mut buf = [0u8; 32];
        let len = last_cmd_into(&mut buf);
        write(b"last: ");
        if len == 0 {
            write(b"(empty)\n");
        } else {
            write(&buf[..len]);
            write(b"\n");
        }
        return;
    }
    if line == b"!!" {
        let mut owned = [0u8; 32];
        let len = last_cmd_into(&mut owned);
        if len == 0 {
            write(b"(empty)\n");
            return;
        }
        write(b"!!: ");
        write(&owned[..len]);
        write(b"\n");
        shell_command(&owned[..len]);
        return;
    }
    if line == b"help" {
        write(b"help history !! jobs kill free ps uname uptime time echo ls [path] cat <path> mkdir <path> rm <path>\n");
        write(b"mv <old> <new> ln <old> <new> stat <path> sync <path>\n");
        write(b"truncate <path> <size> df fsck run [path]\n");
        write(b"runfs <path> [args...] exec <path> [args...]\n");
    } else if line == b"uname" {
        // Vòng 141: định danh hệ thống — arch + phiên bản BootInfo.
        write(b"ViFastOs ");
        write(if cfg!(target_arch = "riscv64") {
            b"riscv64"
        } else if cfg!(target_arch = "aarch64") {
            b"aarch64"
        } else {
            b"x86_64"
        });
        write(b" bootinfo-v3\n");
    } else if line == b"free" {
        // Vòng 154: dung lượng Untyped còn trống (KiB).
        let info = vifast_user::mem_info();
        if info.error == KError::Ok {
            let used_kib = (info.value & 0xffff_ffff) / 1024;
            let total_kib = (info.value >> 32) / 1024;
            let free_kib = total_kib.saturating_sub(used_kib);
            write_u64(free_kib);
            write(b"/");
            write_u64(total_kib);
            write(b" KiB free\n");
        } else {
            write(b"free: error\n");
        }
    } else if line == b"jobs" {
        // Vòng 170: đếm TCB đang chạy/ready qua TcbInfo (xấp xỉ "job đang sống").
        let mut index = 0u64;
        let mut running = 0u64;
        loop {
            let info = vifast_user::tcb_info(index);
            if info.error != KError::Ok {
                break;
            }
            let state = (info.value >> 16) & 0xff;
            if state == 1 || state == 2 {
                running += 1;
            }
            index += 1;
            if index > 64 {
                break;
            }
        }
        write(b"jobs: ");
        write_u64(running);
        write(b" runnable\n");
    } else if line == b"kill" {
        // Vòng 169: dừng chương trình exec đang chạy (root giữ cap TCB).
        let r = vifast_user::send(CLIENT_CMD_SLOT, LABEL_KILL, 0, 0, 0);
        write(if r.error == KError::Ok {
            b"kill sent\n"
        } else {
            b"kill failed\n"
        });
    } else if line == b"ps" {
        // Vòng 152: liệt kê TCB đang sống: "<slot> <prio> <state>".
        let mut index = 0u64;
        loop {
            let info = vifast_user::tcb_info(index);
            if info.error != KError::Ok {
                break;
            }
            write_u64(info.value & 0xff);
            write(b" ");
            write_u64((info.value >> 8) & 0xff);
            write(b" ");
            write_u64((info.value >> 16) & 0xff);
            write(b" aff=");
            write_u64((info.value >> 56) & 0xff);
            write(b" ");
            let mut k = 0u32;
            while k < 4 {
                let byte = ((info.value >> (24 + k * 8)) & 0xff) as u8;
                if byte != 0 {
                    write(&[byte]);
                }
                k += 1;
            }
            write(b"\n");
            index += 1;
            if index > 64 {
                break;
            }
        }
    } else if line == b"uptime" {
        // Vòng 156: in cả giây (dùng tick-rate do platform công bố) và tick.
        let ticks = vifast_user::get_time().value;
        let hz = vifast_user::timer_hz().value.max(1);
        write_u64(ticks / hz);
        write(b" s (");
        write_u64(ticks);
        write(b" ticks @ ");
        write_u64(hz);
        write(b" Hz)\n");
    } else if line == b"time" {
        write_u64(vifast_user::get_time().value);
        write(b"\n");
    } else if let Some(text) = line.strip_prefix(b"echo ") {
        write(text);
        write(b"\n");
    } else if line == b"ls" || line.starts_with(b"ls ") {
        let path = if line.len() > 3 { &line[3..] } else { b"/" };
        if let Some((fd, _)) = vfs_open(path, 0) {
            let mut index = 0u64;
            // Mốc 8 (vòng 25): hậu tố loại — `/` cho dir, `@` cho symlink
            // (qua STAT_PATH từng entry; INODE_DIR=2, INODE_SYMLINK=3).
            let mut full = [0u8; 256];
            while let Ok(Some((_, name, len))) = vfs_readdir(fd, index) {
                let mut flen = 0usize;
                if path != b"/" {
                    flen = path.len().min(200);
                    full[..flen].copy_from_slice(&path[..flen]);
                    if flen < full.len() {
                        full[flen] = b'/';
                        flen += 1;
                    }
                } else if flen < full.len() {
                    full[flen] = b'/';
                    flen += 1;
                }
                let take = len.min(full.len() - flen);
                full[flen..flen + take].copy_from_slice(&name[..take]);
                flen += take;
                let suffix: &[u8] = match vfs_stat_path(&full[..flen]) {
                    Some((_, _, 2)) => b"/",
                    Some((_, _, 3)) => b"@",
                    _ => b"",
                };
                write(&name[..len]);
                write(suffix);
                write(b"\n");
                index += 1;
            }
            let _ = vfs_close(fd);
        } else {
            write(b"ls: not found\n");
        }
    } else if let Some(path) = line.strip_prefix(b"cat ") {
        if let Some((fd, _)) = vfs_open(path, 0) {
            let mut offset = 0u64;
            loop {
                let mut buf = [0u8; vfs::MAX_IO];
                match vfs_read(fd, offset, vfs::MAX_IO, &mut buf) {
                    Some(count) if count != 0 => {
                        write(&buf[..count]);
                        offset += count as u64;
                    }
                    _ => break,
                }
            }
            let _ = vfs_close(fd);
        } else {
            write(b"cat: not found\n");
        }
    } else if let Some(path) = line.strip_prefix(b"mkdir ") {
        write(if vfs_mkdir(path).is_ok() {
            b"ok\n"
        } else {
            b"mkdir failed\n"
        });
    } else if let Some(path) = line.strip_prefix(b"rm ") {
        write(if vfs_unlink(path).is_ok() {
            b"ok\n"
        } else {
            b"rm failed\n"
        });
    } else if let Some(args) = line.strip_prefix(b"mv ") {
        if let Some((old, new)) = split_two(args) {
            write(if vfs_rename(old, new).is_ok() {
                b"ok\n"
            } else {
                b"mv failed\n"
            });
        } else {
            write(b"usage: mv <old> <new>\n");
        }
    } else if let Some(args) = line.strip_prefix(b"ln ") {
        if let Some((old, new)) = split_two(args) {
            write(if vfs_link(old, new).is_ok() {
                b"ok\n"
            } else {
                b"ln failed\n"
            });
        } else {
            write(b"usage: ln <old> <new>\n");
        }
    } else if let Some(path) = line.strip_prefix(b"stat ") {
        // Vòng 150: in cả LOẠI (file/dir/link), không chỉ kích thước.
        if let Some((size, _, kind)) = vfs_stat_path(path) {
            write_u64(size);
            write(match kind {
                2 => b" bytes dir\n",
                3 => b" bytes link\n",
                _ => b" bytes file\n",
            });
        } else {
            write(b"stat: not found\n");
        }
    } else if let Some(path) = line.strip_prefix(b"sync ") {
        if let Some((fd, _)) = vfs_open(path, 0) {
            write(if vfs_fsync(fd).is_some() {
                b"ok\n"
            } else {
                b"sync failed\n"
            });
            let _ = vfs_close(fd);
        } else {
            write(b"sync: not found\n");
        }
    } else if let Some(args) = line.strip_prefix(b"truncate ") {
        if let Some((path, size)) = split_two(args) {
            if let (Some(size), Some((fd, _))) = (parse_u64(size), vfs_open(path, 2)) {
                write(if vfs_truncate(fd, size).is_ok() {
                    b"ok\n"
                } else {
                    b"truncate failed\n"
                });
                let _ = vfs_close(fd);
            } else {
                write(b"truncate failed\n");
            }
        } else {
            write(b"usage: truncate <path> <size>\n");
        }
    } else if line == b"loop" || line.starts_with(b"loop ") {
        // Vòng 201: `loop <n>` — nhờ ROOT lặp spawn/destroy n lần (root in
        // LOOP OK/BAD). Client không lặp để giữ mã nhỏ (ngân sách 7 trang).
        let rest: &[u8] = if line.len() > 5 { &line[5..] } else { b"" };
        let mut n: u64 = 0;
        for byte in rest {
            if !byte.is_ascii_digit() {
                break;
            }
            n = n.saturating_mul(10).saturating_add((byte - b'0') as u64);
        }
        if n == 0 {
            write(b"usage: loop <n>\n");
        } else {
            let r = vifast_user::send(CLIENT_CMD_SLOT, LABEL_LOOP, n, 0, 0);
            write(if r.error == KError::Ok {
                b"loop sent\n"
            } else {
                b"loop failed\n"
            });
        }
    } else if line == b"runfs" || line.starts_with(b"runfs ") {
        // Mốc 9 (vòng 89): exec-thật từ FS, KHÔNG kiểm tra file tồn tại
        // trước (root tự tạo /prog.elf từ ảnh ELF dựng sẵn rồi nạp từ
        // filesystem). Dùng để kiểm chứng đường exec-from-FS.
        // "runfs " dài 6 byte → phần còn lại là "<path> [args...]".
        let rest: &[u8] = if line.len() > 6 {
            &line[6..]
        } else {
            b"/prog.elf"
        };
        let mut split = rest.len();
        let mut k = 0;
        while k < rest.len() {
            if rest[k] == b' ' {
                split = k;
                break;
            }
            k += 1;
        }
        let path = &rest[..split];
        let args: &[u8] = if split < rest.len() {
            &rest[split + 1..]
        } else {
            b""
        };
        copy_path_args(path, args);
        let r = vifast_user::send(CLIENT_CMD_SLOT, LABEL_RUN, 0, 0, 0);
        write(if r.error == KError::Ok {
            b"runfs sent\n"
        } else {
            b"runfs failed\n"
        });
    } else if line == b"run" || line.starts_with(b"run ") {
        // Mốc 9 (vòng 33/38/46): yêu cầu root exec qua cap lệnh.
        // Fire-and-forget: root in RUN OK/SKIP; shell không block chờ reply
        // nên không treo nếu root bận (root recv fault_endpoint chung).
        // Path copy vào IPC buffer của client — kernel copy sang root khi
        // send, root tự STAT qua VFS endpoint gốc (vòng 46).
        let arg = if line.len() > 3 { &line[4..] } else { b"" };
        let mut ready = true;
        if !arg.is_empty() {
            match vfs_open(arg, 0) {
                Some((fd, _)) => {
                    let _ = vfs_close(fd);
                    copy_path(arg);
                }
                None => {
                    write(b"run: not found\n");
                    ready = false;
                }
            }
        }
        if ready {
            // Run trần: xóa byte đầu IPC buffer để root biết không có path
            // (tránh đọc rác của lệnh trước còn sót — vòng 46).
            if arg.is_empty() {
                copy_path(b"");
            }
            let r = vifast_user::send(CLIENT_CMD_SLOT, LABEL_RUN, 0, 0, 0);
            write(if r.error == KError::Ok {
                b"run sent\n"
            } else {
                b"run failed\n"
            });
        }
    } else if line == b"df" {
        // Vòng 139: dung lượng FS (vùng data): "used/total" theo block.
        let reply = unsafe { vifast_user::call_message(1, vfs::STATFS, 0, 0, 0) };
        if reply.error == KError::Ok && reply.label == vfs::OK {
            write_u64(reply.data[0]);
            write(b"/");
            write_u64(reply.data[1]);
            write(b"\n");
        } else {
            write(b"df: error\n");
        }
    } else if line == b"fsck" {
        write(if vfs_fsck() {
            b"ok\n"
        } else {
            b"fsck failed\n"
        });
    } else {
        write(b"unknown command; try help\n");
    }
}

fn split_two(args: &[u8]) -> Option<(&[u8], &[u8])> {
    let split = args.iter().position(|byte| *byte == b' ')?;
    let left = &args[..split];
    let right = &args[split + 1..];
    if left.is_empty() || right.is_empty() {
        None
    } else {
        Some((left, right))
    }
}

fn parse_u64(text: &[u8]) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for byte in text {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((*byte - b'0') as u64)?;
    }
    Some(value)
}

fn shell() -> ! {
    // Mốc 8 (vòng 9): dòng lệnh 256B + lịch sử 4 lệnh + Ctrl-C/L/P/N.
    // (Giới hạn 4 lệnh vì client stack chỉ 4 KiB; static 2 KiB trong .bss
    // làm client ELF to ra và treo timer phase — nguyên nhân chưa rõ, đã
    // bisect xác nhận. TODO mốc 2: cấp buffer lịch sử từ Untyped riêng.)
    const LINE_MAX: usize = 256;
    const HIST_MAX: usize = 4;
    let mut line = [0u8; LINE_MAX];
    let mut len = 0usize;
    let mut hist = [[0u8; LINE_MAX]; HIST_MAX];
    let mut hist_len = [0usize; HIST_MAX];
    let mut hist_count = 0usize;
    let mut hist_pos: Option<usize> = None;
    // Vòng 61 (pha 4 input): marker chứng minh input task (UART console_read
    // + IRQ notify) đã vào vòng lặp — harness khóa exact-one cả 3 arch.
    write(b"SHELL READY\n");
    write(b"shell> ");
    loop {
        let input = vifast_user::console_read();
        if input.error == KError::WouldBlock {
            let notified = vifast_user::wait(CLIENT_UART_NOTIFICATION_SLOT);
            if notified.error == KError::Ok {
                let _ = vifast_user::irq_ack(CLIENT_UART_IRQ_SLOT);
            } else {
                let _ = vifast_user::yield_now();
            }
            continue;
        }
        if input.error == KError::Ok {
            // Vòng 94 (audit #7): ack ngay sau khi lấy được byte. Trước đây chỉ
            // ack trong nhánh WouldBlock nên nếu byte đọc được ngay thì pending
            // IRQ kẹt và phím sau bị coalesce.
            let _ = vifast_user::irq_ack(CLIENT_UART_IRQ_SLOT);
        }
        if input.error != KError::Ok {
            let _ = vifast_user::yield_now();
            continue;
        }
        let byte = input.value as u8;
        if byte == b'\r' || byte == b'\n' {
            write(b"\n");
            if len != 0 {
                if hist_count < HIST_MAX {
                    hist_count += 1;
                } else {
                    hist.copy_within(1.., 0);
                    hist_len.copy_within(1.., 0);
                    hist_count = HIST_MAX;
                }
                let slot = hist_count - 1;
                hist[slot][..len].copy_from_slice(&line[..len]);
                hist_len[slot] = len;
            }
            hist_pos = None;
            if &line[..len] != b"!!" && &line[..len] != b"history" {
                remember_last(&line[..len]);
            }
            shell_command(&line[..len]);
            len = 0;
            write(b"shell> ");
        } else if byte == 8 || byte == 127 {
            if len != 0 {
                len -= 1;
                write(b"\x08 \x08");
            }
            hist_pos = None;
        } else if byte == 3 {
            while len != 0 {
                len -= 1;
                write(b"\x08 \x08");
            }
            hist_pos = None;
            write(b"^C\nshell> ");
        } else if byte == 12 {
            write(b"\x1b[2J\x1b[Hshell> ");
            write(&line[..len]);
            hist_pos = None;
        } else if byte == 16 || byte == 14 {
            if hist_count != 0 {
                let next = match hist_pos {
                    None if byte == 16 => Some(hist_count - 1),
                    Some(pos) if byte == 16 && pos > 0 => Some(pos - 1),
                    Some(pos) if byte == 14 && pos + 1 < hist_count => Some(pos + 1),
                    _ => hist_pos,
                };
                if next != hist_pos {
                    while len != 0 {
                        len -= 1;
                        write(b"\x08 \x08");
                    }
                    if let Some(pos) = next {
                        let hlen = hist_len[pos];
                        line[..hlen].copy_from_slice(&hist[pos][..hlen]);
                        len = hlen;
                        write(&line[..len]);
                    }
                    hist_pos = next;
                }
            }
        } else if byte >= 32 && len < line.len() {
            line[len] = byte;
            len += 1;
            write(&[byte]);
            hist_pos = None;
        }
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
    let before_sleep = vifast_user::get_time();
    let slept = vifast_user::sleep(3);
    let after_sleep = vifast_user::get_time();
    write(
        if slept.error == KError::Ok && after_sleep.value >= before_sleep.value.saturating_add(3) {
            b"SLEEP OK\n"
        } else {
            b"SLEEP BAD\n"
        },
    );
    let denied = vifast_user::block_write(0, 0, IPC_BUF_CLIENT_VA as u64);
    write(if denied.error == KError::InvalidCap {
        b"BLOCK DENIED OK\n"
    } else {
        b"BLOCK DENIED BAD\n"
    });

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
    let mut scratch: &[u8] = b"/scratch";
    if let Some((fd, size)) = vfs_open(b"/scratch", 0) {
        let _ = vfs_close(fd);
        if size != 0 {
            scratch = b"/scratch2";
        }
    }
    if let Some((fd, _)) = vfs_open(scratch, 6) {
        vfs_ok &= vfs_write(fd, 0, WRITE_TEXT).is_some();
        let _ = vfs_close(fd);
        if let Some((fd, _)) = vfs_open(scratch, 0) {
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

    let mut v2 = true;
    v2 &= vfs_mkdir(b"/d").is_ok();
    let fd = match vfs_open(b"/d/a", vifast_abi::O_CREATE | vifast_abi::O_RDWR) {
        Some((fd, _)) => fd,
        None => {
            v2 = false;
            0
        }
    };
    let mut pattern = [0u8; 600];
    let mut index = 0usize;
    while index < pattern.len() {
        pattern[index] = (index as u8) ^ 0x5a;
        index += 1;
    }
    let mut written = 0usize;
    while written < pattern.len() {
        let chunk = core::cmp::min(vfs::MAX_IO, pattern.len() - written);
        let Some(count) = vfs_write(fd, written as u64, &pattern[written..written + chunk]) else {
            v2 = false;
            break;
        };
        if count != chunk {
            v2 = false;
            break;
        }
        written += count;
    }
    v2 &= written == pattern.len();
    v2 &= vfs_close(fd).is_some();

    if let Some((fd, _)) = vfs_open(b"/d/a", 0) {
        let mut readback = [0u8; 600];
        let mut done = 0usize;
        while done < readback.len() {
            let mut chunk = [0u8; vfs::MAX_IO];
            let want = core::cmp::min(vfs::MAX_IO, readback.len() - done);
            let Some(count) = vfs_read(fd, done as u64, want, &mut chunk) else {
                v2 = false;
                break;
            };
            if count == 0 {
                break;
            }
            readback[done..done + count].copy_from_slice(&chunk[..count]);
            done += count;
        }
        v2 &= done == pattern.len() && readback == pattern;
        v2 &= vfs_close(fd).is_some();
    } else {
        v2 = false;
    }

    let mut found_d = false;
    if let Some((root_fd, _)) = vfs_open(b"/", 0) {
        let mut entry = 0u64;
        while let Ok(Some((_, name, len))) = vfs_readdir(root_fd, entry) {
            if len == 1 && name[0] == b'd' {
                found_d = true;
            }
            entry += 1;
        }
        v2 &= vfs_close(root_fd).is_some();
    } else {
        v2 = false;
    }
    v2 &= found_d;
    let mut found_a = false;
    if let Some((dir_fd, _)) = vfs_open(b"/d", 0) {
        let mut entry = 0u64;
        while let Ok(Some((_, name, len))) = vfs_readdir(dir_fd, entry) {
            if len == 1 && name[0] == b'a' {
                found_a = true;
            }
            entry += 1;
        }
        v2 &= vfs_close(dir_fd).is_some();
    } else {
        v2 = false;
    }
    v2 &= found_a;
    v2 &= vfs_rename(b"/d/a", b"/d/b").is_ok();
    v2 &= vfs_open_error(b"/d/a", 0) == vfs::ENOENT;
    if let Some((fd, _)) = vfs_open(b"/d/b", 0) {
        v2 &= vfs_stat(fd) == Some(600);
        v2 &= vfs_truncate(fd, 100).is_ok();
        v2 &= vfs_stat(fd) == Some(100);
        v2 &= vfs_close(fd).is_some();
    } else {
        v2 = false;
    }
    v2 &= vfs_unlink(b"/d/b").is_ok();
    v2 &= vfs_open_error(b"/d/b", 0) == vfs::ENOENT;
    v2 &= vfs_mkdir(b"/d/x").is_ok();
    v2 &= vfs_unlink(b"/d").is_err(); // ENOTEMPTY
    v2 &= vfs_unlink(b"/d/x").is_ok();
    v2 &= vfs_unlink(b"/d").is_ok();
    v2 &= vfs_mkdir(b"/d").is_ok();
    v2 &= vfs_mkdir(b"/d").is_err(); // EEXIST
    v2 &= vfs_unlink(b"/d").is_ok();
    v2 &= vfs_open_error(b"/does-not-exist", 0) == vfs::ENOENT;
    let fsck = vfs_fsck();
    write(if v2 && fsck {
        b"FS V2 OK\n"
    } else {
        b"FS V2 BAD\n"
    });

    let mut perm = vfs_mkdir_via(1, b"/priv").is_ok();
    if !perm {
        let _ = vifast_user::debug_write(b"PERM MKDIR BAD\n".as_ptr() as u64, 15);
    }
    let file = match vfs_open_via(1, b"/priv/f", vifast_abi::O_CREATE | vifast_abi::O_RDWR) {
        Some((fd, _)) => {
            perm &= vfs_write_via(1, fd, 0, b"perm").is_some();
            let _ = vfs_close(fd);
            true
        }
        None => false,
    };
    perm &= file;
    if !file {
        let _ = vifast_user::debug_write(b"PERM CREATE BAD\n".as_ptr() as u64, 16);
    }
    if let Some((fd, _)) = vfs_open_via(6, b"/priv/f", 0) {
        let mut buf = [0u8; vfs::MAX_IO];
        let read_ok = vfs_read_via(6, fd, 0, 4, &mut buf) == Some(4) && &buf[..4] == b"perm";
        perm &= read_ok;
        if !read_ok {
            let _ = vifast_user::debug_write(b"PERM READ BAD\n".as_ptr() as u64, 14);
        }
        let write_denied = vfs_write_via(6, fd, 0, b"nope").is_none();
        perm &= write_denied;
        if !write_denied {
            let _ = vifast_user::debug_write(b"PERM WRITE BAD\n".as_ptr() as u64, 15);
        }
        let _ = vfs_close(fd);
    } else {
        perm = false;
    }
    let unlink_denied = matches!(vfs_unlink_via(6, b"/priv"), Err(vfs::EACCES));
    perm &= unlink_denied;
    if !unlink_denied {
        let _ = vifast_user::debug_write(b"PERM UNLINK BAD\n".as_ptr() as u64, 16);
    }
    let mut badge_ok = vifast_user::cap_mint(6, 7, 8, vifast_abi::Rights::WRITE.0, 0).error
        == KError::IllegalOperation;
    let probe = unsafe { vifast_user::call_message(6, vfs::BADGE_PROBE, 0, 0xFFFF, 0) };
    badge_ok &= probe.error == KError::Ok && probe.data[0] == 2;
    if let Some((fd, _)) = vfs_open_via(6, b"/priv/f", 0) {
        badge_ok &= vfs_write_via(6, fd, 0, b"spoof").is_none();
        let _ = vfs_close(fd);
    } else {
        badge_ok = false;
    }
    let _ = vfs_unlink(b"/priv/f");
    let _ = vfs_unlink(b"/priv");
    write(if perm {
        b"FS PERM OK\n"
    } else {
        b"FS PERM BAD\n"
    });
    write(if badge_ok {
        b"BADGE SPOOF DENIED OK\n"
    } else {
        b"BADGE SPOOF BAD\n"
    });
    let deleted = vifast_user::cap_delete(6);
    let denied_after_delete = vfs_open_via(6, b"/priv/f", 0).is_none();
    write(if deleted.error == KError::Ok && denied_after_delete {
        b"CAP DELETE OK\n"
    } else {
        b"CAP DELETE BAD\n"
    });
    let mut large_ok = true;
    if let Some((fd, _)) = vfs_open_via(
        1,
        b"/large",
        vifast_abi::O_CREATE | vifast_abi::O_RDWR | vifast_abi::O_TRUNC,
    ) {
        const LARGE_BYTES: usize = 20 * 1024;
        const CHUNKS: usize = LARGE_BYTES / vfs::MAX_IO;
        let mut chunk = [0u8; vfs::MAX_IO];
        for block in 0..CHUNKS {
            for (index, byte) in chunk.iter_mut().enumerate() {
                *byte = (block * 17 + index * 7 + 3) as u8;
            }
            large_ok &= vfs_write_via(1, fd, (block * vfs::MAX_IO) as u64, &chunk).is_some();
        }
        large_ok &= vfs_fsync(fd).is_some();
        let _ = vfs_close(fd);
        if let Some((fd, _)) = vfs_open_via(1, b"/large", 0) {
            let mut out = [0u8; vfs::MAX_IO];
            large_ok &= vfs_stat(fd) == Some(LARGE_BYTES as u64);
            for block in 0..CHUNKS {
                if vfs_read_via(1, fd, (block * vfs::MAX_IO) as u64, vfs::MAX_IO, &mut out)
                    != Some(vfs::MAX_IO)
                {
                    large_ok = false;
                    break;
                }
                for (index, byte) in out.iter().enumerate() {
                    if *byte != (block * 17 + index * 7 + 3) as u8 {
                        large_ok = false;
                    }
                }
            }
            let _ = vfs_close(fd);
        } else {
            large_ok = false;
        }
        let _ = vfs_unlink(b"/large");
        large_ok &= vfs_fsck();
    } else {
        large_ok = false;
    }
    write(if large_ok {
        b"FS LARGE OK\n"
    } else {
        b"FS LARGE BAD\n"
    });

    let mut append_ok = true;
    if let Some((fd, _)) = vfs_open_via(
        1,
        b"/append",
        vifast_abi::O_CREATE | vifast_abi::O_RDWR | vifast_abi::O_TRUNC | vifast_abi::O_APPEND,
    ) {
        append_ok &= vfs_write_via(1, fd, 0, b"a").is_some();
        append_ok &= vfs_write_via(1, fd, 0, b"b").is_some();
        append_ok &= vfs_stat(fd) == Some(2);
        let _ = vfs_close(fd);
        if let Some((fd, _)) = vfs_open_via(1, b"/append", 0) {
            let mut out = [0u8; vfs::MAX_IO];
            append_ok &= vfs_read_via(1, fd, 0, 2, &mut out) == Some(2) && &out[..2] == b"ab";
            let _ = vfs_close(fd);
        } else {
            append_ok = false;
        }
        append_ok &= vfs_unlink(b"/append").is_ok();
    } else {
        append_ok = false;
    }
    write(if append_ok {
        b"FS APPEND OK\n"
    } else {
        b"FS APPEND BAD\n"
    });

    let _ = vfs_unlink(b"/excl");
    let mut open_flags_ok = true;
    if let Some((fd, _)) = vfs_open(
        b"/excl",
        vifast_abi::O_CREATE | vifast_abi::O_EXCL | vifast_abi::O_RDWR,
    ) {
        open_flags_ok &= vfs_close(fd).is_some();
    } else {
        open_flags_ok = false;
    }
    open_flags_ok &= vfs_open_error(
        b"/excl",
        vifast_abi::O_CREATE | vifast_abi::O_EXCL | vifast_abi::O_RDWR,
    ) == vfs::EEXIST;
    open_flags_ok &= vfs_open_error(b"/excl", vifast_abi::O_DIRECTORY) == vfs::ENOTDIR;
    open_flags_ok &=
        vfs_open_error(b"/dirflag", vifast_abi::O_CREATE | vifast_abi::O_DIRECTORY) == vfs::EINVAL;
    if let Some((fd, _)) = vfs_open(b"/", vifast_abi::O_DIRECTORY) {
        open_flags_ok &= vfs_close(fd).is_some();
    } else {
        open_flags_ok = false;
    }
    open_flags_ok &= vfs_unlink(b"/excl").is_ok();
    write(if open_flags_ok {
        b"FS OPEN FLAGS OK\n"
    } else {
        b"FS OPEN FLAGS BAD\n"
    });

    let mut stat_path_ok =
        vfs_stat_path(b"/motd").is_some_and(|(size, _, kind)| size > 0 && kind == 1);
    stat_path_ok &= vfs_stat_path(b"/").is_some_and(|(_, _, kind)| kind == 2);
    stat_path_ok &= vfs_stat_path(b"/no-such-stat").is_none();
    write(if stat_path_ok {
        b"FS STAT PATH OK\n"
    } else {
        b"FS STAT PATH BAD\n"
    });

    let _ = vfs_unlink(b"/hard_a");
    let _ = vfs_unlink(b"/hard_b");
    let mut hard_ok = true;
    if let Some((fd, _)) = vfs_open_via(
        1,
        b"/hard_a",
        vifast_abi::O_CREATE | vifast_abi::O_RDWR | vifast_abi::O_TRUNC,
    ) {
        let mut chunk = [0u8; vfs::MAX_IO];
        for block in 0..4usize {
            for (index, byte) in chunk.iter_mut().enumerate() {
                *byte = (0xa0u8).wrapping_add((block * 19 + index * 3) as u8);
            }
            hard_ok &= vfs_write_via(1, fd, (block * vfs::MAX_IO) as u64, &chunk).is_some();
        }
        hard_ok &= vfs_fsync(fd).is_some();
        let _ = vfs_close(fd);
        hard_ok &= vfs_link(b"/hard_a", b"/hard_b").is_ok();
        if let Some((fd, _)) = vfs_open_via(1, b"/hard_b", 0) {
            let mut out = [0u8; vfs::MAX_IO];
            hard_ok &= vfs_stat(fd) == Some((4 * vfs::MAX_IO) as u64);
            for block in 0..4usize {
                if vfs_read_via(1, fd, (block * vfs::MAX_IO) as u64, vfs::MAX_IO, &mut out)
                    != Some(vfs::MAX_IO)
                {
                    hard_ok = false;
                    break;
                }
                for (index, byte) in out.iter().enumerate() {
                    if *byte != (0xa0u8).wrapping_add((block * 19 + index * 3) as u8) {
                        hard_ok = false;
                    }
                }
            }
            let _ = vfs_close(fd);
        } else {
            hard_ok = false;
        }
        hard_ok &= vfs_unlink(b"/hard_a").is_ok();
        if let Some((fd, _)) = vfs_open_via(1, b"/hard_b", 0) {
            hard_ok &= vfs_stat(fd) == Some((4 * vfs::MAX_IO) as u64);
            let _ = vfs_close(fd);
        } else {
            hard_ok = false;
        }
        hard_ok &= vfs_unlink(b"/hard_b").is_ok();
        hard_ok &= vfs_fsck();
    } else {
        hard_ok = false;
    }
    write(if hard_ok {
        b"HARD LINK OK\n"
    } else {
        b"HARD LINK BAD\n"
    });
    let _ = vfs_unlink(b"/sym_link");
    let _ = vfs_unlink(b"/sym_target");
    let mut sym_ok = true;
    if let Some((fd, _)) = vfs_open_via(
        1,
        b"/sym_target",
        vifast_abi::O_CREATE | vifast_abi::O_RDWR | vifast_abi::O_TRUNC,
    ) {
        let mut chunk = [0u8; vfs::MAX_IO];
        for block in 0..2usize {
            for (index, byte) in chunk.iter_mut().enumerate() {
                *byte = (0x30u8).wrapping_add((block * 23 + index * 5) as u8);
            }
            sym_ok &= vfs_write_via(1, fd, (block * vfs::MAX_IO) as u64, &chunk).is_some();
        }
        sym_ok &= vfs_fsync(fd).is_some();
        let _ = vfs_close(fd);
        if vfs_symlink(b"/sym_target", b"/sym_link").is_err() {
            sym_ok = false;
        }
        let mut target = [0u8; vfs::MAX_PATH];
        let target_len = vfs_readlink(b"/sym_link", &mut target);
        if target_len != Some(b"/sym_target".len())
            || !target_len.is_some_and(|len| &target[..len] == b"/sym_target")
        {
            sym_ok = false;
        }
        if let Some((fd, _)) = vfs_open_via(1, b"/sym_link", 0) {
            let mut out = [0u8; vfs::MAX_IO];
            for block in 0..2usize {
                if vfs_read_via(1, fd, (block * vfs::MAX_IO) as u64, vfs::MAX_IO, &mut out)
                    != Some(vfs::MAX_IO)
                {
                    sym_ok = false;
                    break;
                }
                for (index, byte) in out.iter().enumerate() {
                    if *byte != (0x30u8).wrapping_add((block * 23 + index * 5) as u8) {
                        sym_ok = false;
                    }
                }
            }
            let _ = vfs_close(fd);
        } else {
            sym_ok = false;
        }
        if vfs_unlink(b"/sym_link").is_err() {
            sym_ok = false;
        }
        if let Some((fd, _)) = vfs_open_via(1, b"/sym_target", 0) {
            let _ = vfs_close(fd);
        } else {
            sym_ok = false;
        }
        sym_ok &= vfs_unlink(b"/sym_target").is_ok();
        if !vfs_fsck() {
            sym_ok = false;
        }
    } else {
        sym_ok = false;
    }
    write(if sym_ok {
        b"SYMLINK OK\n"
    } else {
        b"SYMLINK BAD\n"
    });

    let irq = vifast_user::wait(CLIENT_IRQ_NOTIFICATION_SLOT);
    let irq_ok = irq.error == KError::Ok
        && irq.value & 1 != 0
        && vifast_user::irq_ack(CLIENT_IRQ_SLOT).error == KError::Ok;
    write(if irq_ok { b"IRQ OK\n" } else { b"IRQ BAD\n" });

    shell()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"CLIENT PANIC\n");
    loop {
        vifast_user::yield_now();
    }
}
