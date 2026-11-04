//! Rust server application loaded by the root task.
//!
//! The server is also the first user-space VFS service: `/motd` and
//! `/scratch` are ordinary request/reply operations over the same endpoint
//! used by the IPC-buffer demonstration.  The kernel ramfs remains the
//! bootstrap/test fallback; normal client file I/O no longer needs it.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::IPC_BUF_SERVER_VA;
use vifast_abi::vfs;
use vifast_abi::KError;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const SERVER: &[u8] = b"Server\n";
const VFS_READY: &[u8] = b"VFS SERVER OK\n";
const IPC_BYTES: usize = 11;
const FILE_CAP: usize = 64;
const MOTD: &[u8] = b"ViFastOS ramfs\n";
const FS_MAGIC: &[u8; 4] = b"VF01";
const FS_SECTOR: u64 = 0;

#[derive(Clone, Copy)]
struct File {
    name: [u8; vfs::MAX_NAME],
    name_len: u8,
    len: usize,
    data: [u8; FILE_CAP],
}

const EMPTY_FILE: File = File {
    name: [0; vfs::MAX_NAME],
    name_len: 0,
    len: 0,
    data: [0; FILE_CAP],
};

static mut FILES: [File; vfs::MAX_FILES] = [EMPTY_FILE; vfs::MAX_FILES];
static mut FILE_COUNT: usize = 0;
static mut DISK_OK: bool = false;
static mut PERSISTED: bool = false;

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

fn set_name(file: &mut File, name: &[u8]) {
    let len = core::cmp::min(name.len(), vfs::MAX_NAME);
    file.name[..len].copy_from_slice(&name[..len]);
    file.name_len = len as u8;
}

unsafe fn init_files() -> &'static mut [File; vfs::MAX_FILES] {
    let files = &mut *core::ptr::addr_of_mut!(FILES);
    files[0] = EMPTY_FILE;
    set_name(&mut files[0], b"/motd");
    files[0].len = MOTD.len();
    files[0].data[..MOTD.len()].copy_from_slice(MOTD);

    files[1] = EMPTY_FILE;
    set_name(&mut files[1], b"/scratch");
    *core::ptr::addr_of_mut!(FILE_COUNT) = 2;
    load_scratch(files);
    files
}

unsafe fn load_scratch(files: &mut [File; vfs::MAX_FILES]) {
    let mut sector = [0u8; 512];
    let result = vifast_user::block_read(FS_SECTOR, sector.as_mut_ptr() as u64);
    if result.error != KError::Ok {
        return;
    }
    DISK_OK = true;
    if &sector[..4] == FS_MAGIC {
        PERSISTED = true;
        let len = u32::from_le_bytes([sector[4], sector[5], sector[6], sector[7]]) as usize;
        let len = core::cmp::min(len, FILE_CAP);
        files[1].data[..len].copy_from_slice(&sector[8..8 + len]);
        files[1].len = len;
    } else {
        persist_scratch(files);
    }
}

unsafe fn persist_scratch(files: &[File; vfs::MAX_FILES]) {
    if !DISK_OK {
        return;
    }
    let mut sector = [0u8; 512];
    sector[..4].copy_from_slice(FS_MAGIC);
    sector[4..8].copy_from_slice(&(files[1].len as u32).to_le_bytes());
    let len = core::cmp::min(files[1].len, FILE_CAP);
    sector[8..8 + len].copy_from_slice(&files[1].data[..len]);
    let _ = vifast_user::block_write(FS_SECTOR, sector.as_ptr() as u64);
}

unsafe fn files() -> &'static mut [File; vfs::MAX_FILES] {
    &mut *core::ptr::addr_of_mut!(FILES)
}

fn find_file(files: &[File], name: &[u8]) -> Option<usize> {
    files.iter().position(|file| {
        let len = file.name_len as usize;
        len == name.len() && file.name[..len] == *name
    })
}

fn open_request(data: [u64; 4]) -> (u64, u64, u64, u64) {
    let name = data[0].to_le_bytes();
    let name_len = data[1] as usize;
    let flags = data[2];
    if name_len == 0 || name_len > vfs::MAX_NAME {
        return (vfs::ERR, 0, 0, 0);
    }
    let name = &name[..name_len];
    unsafe {
        let files = files();
        if let Some(fd) = find_file(files, name) {
            return (vfs::OK, fd as u64, files[fd].len as u64, 0);
        }
        if flags & vifast_abi::O_CREATE == 0 {
            return (vfs::ERR, 0, 0, 0);
        }
        let count = *core::ptr::addr_of!(FILE_COUNT);
        if count >= vfs::MAX_FILES {
            return (vfs::ERR, 0, 0, 0);
        }
        files[count] = EMPTY_FILE;
        set_name(&mut files[count], name);
        *core::ptr::addr_of_mut!(FILE_COUNT) = count + 1;
        (vfs::OK, count as u64, 0, 0)
    }
}

fn read_request(data: [u64; 4]) -> (u64, u64, u64, u64) {
    let fd = data[0] as usize;
    let offset = data[1] as usize;
    let len = (data[2] as usize).min(vfs::MAX_IO);
    unsafe {
        let files = files();
        if fd >= *core::ptr::addr_of!(FILE_COUNT) || offset > files[fd].len {
            return (vfs::ERR, 0, 0, 0);
        }
        let available = files[fd].len - offset;
        let count = core::cmp::min(len, available);
        let mut words = [0u64; 3];
        let mut bytes = [0u8; vfs::MAX_IO];
        bytes[..count].copy_from_slice(&files[fd].data[offset..offset + count]);
        let mut i = 0;
        while i < 3 {
            let start = i * 8;
            let mut chunk = [0u8; 8];
            let n = core::cmp::min(8, count.saturating_sub(start));
            chunk[..n].copy_from_slice(&bytes[start..start + n]);
            words[i] = u64::from_le_bytes(chunk);
            i += 1;
        }
        (count as u64, words[0], words[1], words[2])
    }
}

fn stat_request(data: [u64; 4]) -> (u64, u64, u64, u64) {
    let fd = data[0] as usize;
    unsafe {
        if fd >= *core::ptr::addr_of!(FILE_COUNT) {
            return (vfs::ERR, 0, 0, 0);
        }
        (vfs::OK, files()[fd].len as u64, 0, 0)
    }
}

fn write_request(data: [u64; 4]) -> (u64, u64, u64, u64) {
    let fd = data[0] as usize;
    let offset = data[1] as usize;
    let len = (data[2] as usize).min(vfs::MAX_IO);
    unsafe {
        let files = files();
        if fd >= *core::ptr::addr_of!(FILE_COUNT) || offset > FILE_CAP || len > FILE_CAP - offset {
            return (vfs::ERR, 0, 0, 0);
        }
        let source = core::slice::from_raw_parts(IPC_BUF_SERVER_VA as *const u8, len);
        files[fd].data[offset..offset + len].copy_from_slice(source);
        if offset + len > files[fd].len {
            files[fd].len = offset + len;
        }
        if fd == 1 {
            persist_scratch(files);
        }
        (len as u64, 0, 0, 0)
    }
}

fn handle(request: &vifast_user::MessageResult) -> (u64, u64, u64, u64) {
    if request.error != KError::Ok {
        return (vfs::ERR, 0, 0, 0);
    }
    match request.label {
        0x1234 => {
            let _ = vifast_user::debug_write(IPC_BUF_SERVER_VA as u64, IPC_BYTES as u64);
            let _ = vifast_user::signal(4);
            (0x55, 42, 0, 0)
        }
        vfs::OPEN => open_request(request.data),
        vfs::READ => read_request(request.data),
        vfs::WRITE => write_request(request.data),
        vfs::STAT => stat_request(request.data),
        vfs::CLOSE => {
            let fd = request.data[0] as usize;
            if fd < unsafe { *core::ptr::addr_of!(FILE_COUNT) } {
                (vfs::OK, 0, 0, 0)
            } else {
                (vfs::ERR, 0, 0, 0)
            }
        }
        _ => (vfs::ERR, 0, 0, 0),
    }
}

#[no_mangle]
pub extern "C" fn server_main() -> ! {
    write(SERVER);
    unsafe { init_files() };
    if unsafe { DISK_OK } {
        write(b"FS DISK OK\n");
        if unsafe { PERSISTED } {
            write(b"FS PERSIST OK\n");
        }
    } else {
        write(b"FS DISK RAM\n");
    }
    write(VFS_READY);
    // First receive has no pending caller; its reply arguments are ignored.
    let mut request = unsafe { vifast_user::reply_recv_message(1, 4, 0, 0, 0, 0) };
    loop {
        let (label, d0, d1, d2) = handle(&request);
        request = unsafe { vifast_user::reply_recv_message(1, 4, label, d0, d1, d2) };
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write(b"SERVER PANIC\n");
    loop {
        vifast_user::yield_now();
    }
}
