//! Chương trình ngoài (D10): các lệnh chỉ ĐỌC thông tin, chạy từ FS qua
//! exec-from-FS thay vì nằm trong shell builtin.
//!
//! Cùng một crate được build 4 lần với `VIFAST_BIN_MODE` khác nhau
//! (`uname`, `uptime`, `free`, `df`) để tránh phải truyền argv qua ELF (cơ chế
//! hiện tại của exec là patch byte vào offset cố định của ảnh demo, không dùng
//! được cho chương trình Rust thật).

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const MODE: &str = match option_env!("VIFAST_BIN_MODE") {
    Some(value) => value,
    None => "uname",
};

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

/// `uname`: định danh hệ thống (vòng 141, chuyển ra chương trình ngoài ở r237).
fn run_uname() {
    write(b"ViFastOs ");
    write(arch_name());
    write(b" bootinfo-v3\n");
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

#[no_mangle]
pub extern "C" fn bin_main() -> ! {
    // Marker tự kiểm chứng: mỗi chương trình ngoài in đúng một dòng
    // `BIN OK <tên>` khi được exec, để cổng biết nó THẬT SỰ chạy từ FS.
    write(b"BIN OK ");
    write(MODE.as_bytes());
    write(b"\n");
    if MODE == "uname" {
        run_uname();
    } else if MODE == "uptime" {
        run_uptime();
    } else if MODE == "free" {
        run_free();
    }
    vifast_user::exit();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"BIN PANIC\n");
    vifast_user::exit();
}
