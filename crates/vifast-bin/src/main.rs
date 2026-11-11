//! Chương trình ngoài (D10, D14): MỘT ảnh ELF multicall `/bin/vf` trên đĩa,
//! các tên lệnh là HARD LINK tới cùng ảnh đó (`/bin/uname`, `/bin/uptime`,
//! `/bin/free`, `/bin/args`). Mode được chọn theo `argv[0]` — nhờ ABI argv
//! của D15 (r246): kernel đặt `a0 = argc`, `a1 = argv`, tham số nằm trên
//! trang stack của exec.
//!
//! Trước r246 crate này phải build 3 ảnh riêng vì exec chỉ biết "vá byte" vào
//! offset cố định của ảnh demo, không truyền được argv cho chương trình Rust.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

fn write_decimal(mut value: u64) {
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

fn arch_name() -> &'static [u8] {
    if cfg!(target_arch = "riscv64") {
        b"riscv64"
    } else if cfg!(target_arch = "aarch64") {
        b"aarch64"
    } else {
        b"x86_64"
    }
}

/// Đọc một chuỗi NUL-terminated do kernel/root đặt trên trang stack.
/// Giới hạn 256 B để một con trỏ hỏng không làm treo vòng quét.
unsafe fn c_str(ptr: *const u8) -> &'static [u8] {
    if ptr.is_null() {
        return b"";
    }
    let mut len = 0usize;
    while len < 256 && core::ptr::read_volatile(ptr.add(len)) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

fn basename(path: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut i = 0usize;
    while i < path.len() {
        if path[i] == b'/' {
            start = i + 1;
        }
        i += 1;
    }
    &path[start..]
}

fn run_uname() {
    write(b"ViFastOs ");
    write(arch_name());
    write(b" bootinfo-v3\n");
}

/// `df`: dung lượng FS qua VFS endpoint ở slot 1 — applet ĐẦU TIÊN cần cap,
/// và là lý do tồn tại của việc uỷ quyền cap cho chương trình con (T27/G2).
/// Chương trình con exec từ FS nhận đúng cap đó (badge 1) từ root.
fn run_df() {
    // STATFS không mang dữ liệu qua IPC buffer (mọi tham số nằm trong message
    // words) nên applet chỉ cần cap, không cần `TcbSetIpcBuffer`.
    let reply = unsafe { vifast_user::call_message(1, vifast_abi::vfs::STATFS, 0, 0, 0) };
    if reply.error == vifast_abi::KError::Ok && reply.label == vifast_abi::vfs::OK {
        let used = reply.data[0];
        let total = reply.data[1].max(1);
        write_decimal(used);
        write(b"/");
        write_decimal(total);
        write(b" blocks\n");
    } else {
        write(b"df: error\n");
    }
}

/// `free`: dung lượng Untyped còn trống (chardev `MemInfo`, không cần cap).
fn run_free() {
    let info = vifast_user::mem_info();
    if info.error == vifast_abi::KError::Ok {
        let used_kib = (info.value & 0xffff_ffff) / 1024;
        let total_kib = (info.value >> 32) / 1024;
        write_decimal(total_kib.saturating_sub(used_kib));
        write(b"/");
        write_decimal(total_kib);
        write(b" KiB free\n");
    } else {
        write(b"free: error\n");
    }
}

/// `uptime`: thời gian chạy tính từ `GetTime`, in dạng `NmSSs` như shell cũ.
fn run_uptime() {
    let ticks = vifast_user::get_time().value;
    // Timer 100 Hz (mọi arch trong cấu hình hiện tại) → 1 giây = 100 tick.
    let seconds = ticks / 100;
    write(b"uptime ");
    write_decimal(seconds / 60);
    write(b"m");
    let rest = seconds % 60;
    if rest < 10 {
        write(b"0");
    }
    write_decimal(rest);
    write(b"s\n");
}

/// `ps`: liệt kê TCB đang sống qua `TcbInfo` (syscall toàn cục, không cần
/// cap) — chuyển ra chương trình ngoài ở r249 vì nó không phải lệnh có trạng
/// thái, đúng D10.
fn run_ps() {
    let mut index = 0u64;
    loop {
        let info = vifast_user::tcb_info(index);
        if info.error != vifast_abi::KError::Ok {
            break;
        }
        write_decimal(info.value & 0xff);
        write(b" ");
        write_decimal((info.value >> 8) & 0xff);
        write(b" ");
        write_decimal((info.value >> 16) & 0xff);
        write(b" aff=");
        write_decimal((info.value >> 56) & 0xff);
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
}

/// `jobs`: đếm TCB đang Ready/Running qua `TcbInfo` (cùng nguồn với `ps`).
fn run_jobs() {
    let mut index = 0u64;
    let mut running = 0u64;
    loop {
        let info = vifast_user::tcb_info(index);
        if info.error != vifast_abi::KError::Ok {
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
    write_decimal(running);
    write(b" runnable\n");
}

/// `args`: in các tham số nhận được. Đây là chương trình chứng minh ABI argv:
/// chuỗi in ra KHÔNG có trong ảnh trên đĩa, chỉ đến từ `argv` của exec.
fn run_args(argc: usize, argv: *const *const u8) {
    write(b"argv:");
    let mut i = 1usize;
    while i < argc && i < vifast_abi::user_layout::EXEC_ARGS_MAX {
        let ptr = unsafe { core::ptr::read_volatile(argv.add(i)) };
        write(b" ");
        write(unsafe { c_str(ptr) });
        i += 1;
    }
    write(b"\n");
}

/// r295 (bước 0): in `BUILD <name>=<id> feats=<feats>` một lần lúc khởi động.
/// Id do `scripts/build.sh` truyền qua môi trường; `unknown` = build tay.
fn write_build(name: &[u8]) {
    let mut line = [0u8; 96];
    let count = vifast_abi::build::line(name, &mut line);
    write(&line[..count]);
}

#[no_mangle]
pub extern "C" fn bin_main(argc: usize, argv: *const *const u8) -> ! {
    write_build(b"bin");
    // D14: chọn mode theo basename(argv[0]) — cùng một ảnh, nhiều tên lệnh.
    let mut argv0: &[u8] = b"";
    if argc > 0 && !argv.is_null() {
        argv0 = unsafe { c_str(core::ptr::read_volatile(argv)) };
    }
    let base = basename(argv0);
    if base == b"uname" {
        write(b"BIN OK uname\n");
        run_uname();
    } else if base == b"uptime" {
        write(b"BIN OK uptime\n");
        run_uptime();
    } else if base == b"free" {
        write(b"BIN OK free\n");
        run_free();
    } else if base == b"ps" {
        write(b"BIN OK ps\n");
        run_ps();
    } else if base == b"jobs" {
        write(b"BIN OK jobs\n");
        run_jobs();
    } else if base == b"args" {
        write(b"BIN OK args\n");
        run_args(argc, argv);
    } else if base == b"df" {
        write(b"BIN OK df\n");
        run_df();
    } else {
        // Không nhánh nào in "BIN BAD": marker đó bị cổng cấm tuyệt đối.
        write(b"vf: unknown applet '");
        write(base);
        write(b"'\n");
        write(b"vf: usage: uname | uptime | free | ps | jobs | args | df\n");
        if argc > 1 {
            write(b"vf argv:");
            let mut i = 1usize;
            while i < argc && i < vifast_abi::user_layout::EXEC_ARGS_MAX {
                let ptr = unsafe { core::ptr::read_volatile(argv.add(i)) };
                write(b" ");
                write(unsafe { c_str(ptr) });
                i += 1;
            }
            write(b"\n");
        }
    }
    vifast_user::exit();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"BIN PANIC\n");
    vifast_user::exit();
}
