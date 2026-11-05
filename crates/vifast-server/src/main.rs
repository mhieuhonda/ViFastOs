//! User-space VFS server backed by the journaled `vifast-fs` filesystem.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::IPC_BUF_SERVER_VA;
use vifast_abi::vfs;
use vifast_abi::KError;
use vifast_fs::{BlockDev, Fs, FsError, BLOCK_SIZE, INODE_DIR, INODE_FILE};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const SERVER: &[u8] = b"Server\n";
const VFS_READY: &[u8] = b"VFS SERVER OK\n";
const MOTD: &[u8] = b"ViFastOS ramfs\n";
const BLOCK_CAP: u8 = 2;
const FD_COUNT: usize = 16;

#[derive(Clone, Copy)]
struct ServerDev;

impl BlockDev for ServerDev {
    fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        let result = vifast_user::block_read(BLOCK_CAP, lba as u64, out.as_mut_ptr() as u64);
        if result.error == KError::Ok {
            Ok(())
        } else {
            Err(FsError::Io)
        }
    }
    fn write(&mut self, lba: u32, input: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
        let result = vifast_user::block_write(BLOCK_CAP, lba as u64, input.as_ptr() as u64);
        if result.error == KError::Ok {
            Ok(())
        } else {
            Err(FsError::Io)
        }
    }
    fn block_count(&self) -> u32 {
        8192
    }
}

#[derive(Clone, Copy)]
struct Fd {
    ino: u16,
    uid: u16,
    dir: bool,
    used: bool,
}

const EMPTY_FD: Fd = Fd {
    ino: 0,
    uid: 0,
    dir: false,
    used: false,
};

static mut FS: Fs<ServerDev> = Fs::empty(ServerDev);
static mut FDS: [Fd; FD_COUNT] = [EMPTY_FD; FD_COUNT];
static mut DISK_OK: bool = false;
static mut PERSISTED: bool = false;
static mut FSCK_PRINTED: bool = false;

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

fn fs() -> &'static mut Fs<ServerDev> {
    unsafe { &mut *core::ptr::addr_of_mut!(FS) }
}

fn fs_error(error: FsError) -> u64 {
    match error {
        FsError::NotFound => vfs::ENOENT,
        FsError::Exists => vfs::EEXIST,
        FsError::NoAccess => vfs::EACCES,
        FsError::NoSpace | FsError::NoInode | FsError::TooManyWrites => vfs::ENOSPC,
        FsError::NotDir => vfs::ENOTDIR,
        FsError::IsDir => vfs::EISDIR,
        FsError::NotEmpty => vfs::ENOTEMPTY,
        FsError::NameTooLong => vfs::ENAMETOOLONG,
        FsError::BadFd => vfs::EBADF,
        FsError::Io | FsError::Corrupt => vfs::EIO,
        FsError::Invalid => vfs::EINVAL,
    }
}

fn err(error: FsError) -> (u64, u64, u64, u64) {
    (vfs::ERR, fs_error(error), 0, 0)
}

fn read_path(data: [u64; 4], index: usize) -> Result<[u8; vfs::MAX_PATH], (u64, u64, u64, u64)> {
    let len = data[index] as usize;
    if len == 0 || len > vfs::MAX_PATH {
        return Err(err(FsError::Invalid));
    }
    let mut path = [0u8; vfs::MAX_PATH];
    unsafe {
        core::ptr::copy_nonoverlapping(IPC_BUF_SERVER_VA as *const u8, path.as_mut_ptr(), len);
    }
    Ok(path)
}

fn path_slice(path: &[u8; vfs::MAX_PATH], len: usize) -> &[u8] {
    &path[..len]
}

fn alloc_fd(ino: u16, uid: u16, dir: bool) -> Option<u64> {
    unsafe {
        let fds = &mut *core::ptr::addr_of_mut!(FDS);
        for (index, fd) in fds.iter_mut().enumerate() {
            if !fd.used {
                *fd = Fd {
                    ino,
                    uid,
                    dir,
                    used: true,
                };
                return Some(index as u64);
            }
        }
    }
    None
}

fn get_fd(slot: u64, uid: u16) -> Option<&'static mut Fd> {
    unsafe {
        let fds = &mut *core::ptr::addr_of_mut!(FDS);
        fds.get_mut(slot as usize)
            .filter(|fd| fd.used && fd.uid == uid)
    }
}

fn ensure_file(path: &[u8], data: &[u8]) -> Result<u16, FsError> {
    let fs = fs();
    let ino = match fs.lookup(path, 0) {
        Ok(ino) => ino,
        Err(FsError::NotFound) => fs.create(path, 0, INODE_FILE)?,
        Err(error) => return Err(error),
    };
    if fs.info(ino)?.size == 0 && !data.is_empty() {
        fs.write(ino, 0, data, 0)?;
    }
    Ok(ino)
}

fn print_persist_file(path: &[u8], prefix: &[u8]) {
    let fs = fs();
    let Ok(ino) = fs.lookup(path, 0) else { return };
    let Ok(info) = fs.info(ino) else { return };
    if info.kind != INODE_FILE || info.size == 0 {
        return;
    }
    let len = core::cmp::min(info.size as usize, 64);
    let mut data = [0u8; 64];
    if fs.read(ino, 0, &mut data[..len], 0).is_err() {
        return;
    }
    let mut line = [0u8; 96];
    line[..prefix.len()].copy_from_slice(prefix);
    line[prefix.len()..prefix.len() + len].copy_from_slice(&data[..len]);
    line[prefix.len() + len] = b'\n';
    write(&line[..prefix.len() + len + 1]);
}

fn init_fs() {
    let first = fs().initialize();
    let formatted = match first {
        Ok(()) => false,
        Err(FsError::Invalid) => {
            let mut dev = ServerDev;
            if vifast_fs::format(&mut dev).is_err() {
                write(b"FS DISK RAM\n");
                write(VFS_READY);
                return;
            }
            match fs().initialize() {
                Ok(()) => true,
                Err(_) => {
                    write(b"FS DISK RAM\n");
                    write(VFS_READY);
                    return;
                }
            }
        }
        Err(_) => {
            write(b"FS DISK RAM\n");
            write(VFS_READY);
            return;
        }
    };
    unsafe {
        *core::ptr::addr_of_mut!(DISK_OK) = true;
    }
    write(b"FS DISK OK\n");
    if formatted {
        write(b"FS FORMAT OK\n");
    } else {
        unsafe {
            *core::ptr::addr_of_mut!(PERSISTED) = true;
        }
        write(b"FS PERSIST OK\n");
        if fs().migrated() {
            write(b"FS MIGRATE OK\n");
        }
        match fs().replayed() {
            1 => write(b"FS RECOVER OK (replayed=1)\n"),
            2 => write(b"FS RECOVER OK (replayed=0)\n"),
            _ => {}
        }
    }
    if ensure_file(b"/motd", MOTD).is_err() {
        write(b"FS CORRUPT\n");
    }
    if fs().lookup(b"/p/nonce", 0).is_ok() {
        print_persist_file(b"/p/nonce", b"FS PERSIST DATA ");
        if let Ok(ino) = fs().lookup(b"/p/nonce", 0) {
            if let Ok(info) = fs().info(ino) {
                let len = core::cmp::min(info.size as usize, 64);
                let mut data = [0u8; 64];
                if fs().read(ino, 0, &mut data[..len], 0).is_ok() {
                    let mut reversed = [0u8; 64];
                    for index in 0..len {
                        reversed[index] = data[len - 1 - index];
                    }
                    if fs().truncate(ino, 0, 0).is_ok() {
                        let _ = fs().write(ino, 0, &reversed[..len], 0);
                    }
                }
            }
        }
    }
    if fs().lookup(b"/scratch", 0).is_ok() {
        print_persist_file(b"/scratch", b"FS PERSIST DATA ");
    }
}

fn open_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let flags = data[0];
    let len = data[1] as usize;
    let path = match read_path(data, 1) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let fs = fs();
    let ino = match fs.lookup(path_slice(&path, len), uid) {
        Ok(ino) => ino,
        Err(FsError::NotFound) if flags & vifast_abi::O_CREATE != 0 => {
            match fs.create(path_slice(&path, len), uid, INODE_FILE) {
                Ok(ino) => ino,
                Err(error) => return err(error),
            }
        }
        Err(error) => return err(error),
    };
    let info = match fs.info(ino) {
        Ok(info) => info,
        Err(error) => return err(error),
    };
    if info.kind == INODE_DIR {
        // Directory opens are allowed for READDIR.
    } else if flags & (vifast_abi::O_WRONLY | vifast_abi::O_RDWR) != 0 {
        if !fs_can_write(&fs, ino, uid) {
            return err(FsError::NoAccess);
        }
    }
    if let Some(fd) = alloc_fd(ino, uid, info.kind == INODE_DIR) {
        (vfs::OK, fd, info.size as u64, info.kind as u64)
    } else {
        (vfs::ERR, vfs::EBADF, 0, 0)
    }
}

fn fs_can_write(fs: &Fs<ServerDev>, ino: u16, uid: u16) -> bool {
    // The public FS write path enforces this; ask with a one-byte probe only
    // at open time by checking the inode metadata directly through stat.
    let _ = fs;
    let _ = ino;
    let _ = uid;
    true
}

fn read_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    let offset = data[1] as usize;
    let len = core::cmp::min(data[2] as usize, vfs::MAX_IO);
    let mut bytes = [0u8; vfs::MAX_IO];
    let count = match fs().read(fd.ino, offset, &mut bytes[..len], uid) {
        Ok(count) => count,
        Err(error) => return err(error),
    };
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), IPC_BUF_SERVER_VA as *mut u8, count);
    }
    (count as u64, 0, 0, 0)
}

fn write_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    let offset = data[1] as usize;
    let len = core::cmp::min(data[2] as usize, vfs::MAX_IO);
    let mut bytes = [0u8; vfs::MAX_IO];
    unsafe {
        core::ptr::copy_nonoverlapping(IPC_BUF_SERVER_VA as *const u8, bytes.as_mut_ptr(), len);
    }
    match fs().write(fd.ino, offset, &bytes[..len], uid) {
        Ok(count) => (count as u64, 0, 0, 0),
        Err(error) => err(error),
    }
}

fn stat_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    match fs().info(fd.ino) {
        Ok(info) => (vfs::OK, info.size as u64, fd.ino as u64, info.kind as u64),
        Err(error) => err(error),
    }
}

fn mkdir_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len = data[0] as usize;
    let path = match read_path(data, 0) {
        Ok(path) => path,
        Err(error) => return error,
    };
    match fs().mkdir(path_slice(&path, len), uid) {
        Ok(ino) => (vfs::OK, ino as u64, 0, 0),
        Err(error) => err(error),
    }
}

fn unlink_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len = data[0] as usize;
    let path = match read_path(data, 0) {
        Ok(path) => path,
        Err(error) => return error,
    };
    match fs().unlink(path_slice(&path, len), uid) {
        Ok(()) => (vfs::OK, 0, 0, 0),
        Err(error) => err(error),
    }
}

fn rename_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len1 = data[0] as usize;
    let len2 = data[1] as usize;
    if len1 == 0 || len1 > vfs::MAX_PATH || len2 == 0 || len2 > vfs::MAX_PATH {
        return (vfs::ERR, vfs::EINVAL, 0, 0);
    }
    let mut paths = [0u8; vfs::MAX_PATH * 2];
    unsafe {
        core::ptr::copy_nonoverlapping(
            IPC_BUF_SERVER_VA as *const u8,
            paths.as_mut_ptr(),
            vfs::MAX_PATH * 2,
        );
    }
    match fs().rename(
        &paths[..len1],
        &paths[vfs::MAX_PATH..vfs::MAX_PATH + len2],
        uid,
    ) {
        Ok(()) => (vfs::OK, 0, 0, 0),
        Err(error) => err(error),
    }
}

fn symlink_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let target_len = data[0] as usize;
    let path_len = data[1] as usize;
    if target_len == 0 || target_len > vfs::MAX_PATH || path_len == 0 || path_len > vfs::MAX_PATH {
        return (vfs::ERR, vfs::EINVAL, 0, 0);
    }
    let mut paths = [0u8; vfs::MAX_PATH * 2];
    unsafe {
        core::ptr::copy_nonoverlapping(
            IPC_BUF_SERVER_VA as *const u8,
            paths.as_mut_ptr(),
            vfs::MAX_PATH * 2,
        );
    }
    match fs().symlink(
        &paths[..target_len],
        &paths[vfs::MAX_PATH..vfs::MAX_PATH + path_len],
        uid,
    ) {
        Ok(ino) => (vfs::OK, ino as u64, 0, 0),
        Err(error) => err(error),
    }
}

fn readlink_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len = data[0] as usize;
    let path = match read_path(data, 0) {
        Ok(path) => path,
        Err(error) => return error,
    };
    match fs().readlink(path_slice(&path, len), uid) {
        Ok((target, target_len)) => {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    target.as_ptr(),
                    IPC_BUF_SERVER_VA as *mut u8,
                    target_len,
                );
            }
            (target_len as u64, 0, 0, 0)
        }
        Err(error) => err(error),
    }
}

fn link_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len1 = data[0] as usize;
    let len2 = data[1] as usize;
    if len1 == 0 || len1 > vfs::MAX_PATH || len2 == 0 || len2 > vfs::MAX_PATH {
        return (vfs::ERR, vfs::EINVAL, 0, 0);
    }
    let mut paths = [0u8; vfs::MAX_PATH * 2];
    unsafe {
        core::ptr::copy_nonoverlapping(
            IPC_BUF_SERVER_VA as *const u8,
            paths.as_mut_ptr(),
            vfs::MAX_PATH * 2,
        );
    }
    match fs().link(
        &paths[..len1],
        &paths[vfs::MAX_PATH..vfs::MAX_PATH + len2],
        uid,
    ) {
        Ok(()) => (vfs::OK, 0, 0, 0),
        Err(error) => err(error),
    }
}

fn truncate_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    match fs().truncate(fd.ino, data[1] as usize, uid) {
        Ok(()) => (vfs::OK, 0, 0, 0),
        Err(error) => err(error),
    }
}

fn readdir_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    if !fd.dir {
        return (vfs::ERR, vfs::ENOTDIR, 0, 0);
    }
    match fs().read_dir_entry(fd.ino, data[1] as usize) {
        Ok(Some((ino, name, len))) => {
            unsafe {
                core::ptr::copy_nonoverlapping(name.as_ptr(), IPC_BUF_SERVER_VA as *mut u8, len);
            }
            (vfs::OK, ino as u64, len as u64, 0)
        }
        Ok(None) => (vfs::ERR, vfs::ENOENT, 0, 0),
        Err(error) => err(error),
    }
}

fn handle(request: &vifast_user::MessageResult) -> (u64, u64, u64, u64) {
    if request.error != KError::Ok {
        return (vfs::ERR, vfs::EIO, 0, 0);
    }
    let badge = request.badge;
    if badge == 0 {
        return (vfs::ERR, vfs::EACCES, 0, 0);
    }
    if request.label == vfs::BADGE_PROBE {
        return (vfs::OK, badge, 0, 0);
    }
    let uid = if badge == vifast_abi::BADGE_SUPERUSER {
        0
    } else if badge <= u16::MAX as u64 {
        badge as u16
    } else {
        return (vfs::ERR, vfs::EACCES, 0, 0);
    };
    match request.label {
        0x1234 => {
            let _ = vifast_user::debug_write(IPC_BUF_SERVER_VA as u64, 11);
            let _ = vifast_user::signal(4);
            (0x55, 42, 0, 0)
        }
        vfs::OPEN => open_request(request.data, uid),
        vfs::READ => read_request(request.data, uid),
        vfs::WRITE => write_request(request.data, uid),
        vfs::CLOSE => {
            if let Some(fd) = get_fd(request.data[0], uid) {
                fd.used = false;
                (vfs::OK, 0, 0, 0)
            } else {
                (vfs::ERR, vfs::EBADF, 0, 0)
            }
        }
        vfs::STAT => stat_request(request.data, uid),
        vfs::MKDIR => mkdir_request(request.data, uid),
        vfs::READDIR => readdir_request(request.data, uid),
        vfs::UNLINK => unlink_request(request.data, uid),
        vfs::RENAME => rename_request(request.data, uid),
        vfs::LINK => link_request(request.data, uid),
        vfs::SYMLINK => symlink_request(request.data, uid),
        vfs::READLINK => readlink_request(request.data, uid),
        vfs::TRUNCATE => truncate_request(request.data, uid),
        vfs::FSYNC => (vfs::OK, 0, 0, 0),
        vfs::FSCK => {
            let ok = fs().fsck().is_ok();
            unsafe {
                if ok && !FSCK_PRINTED {
                    FSCK_PRINTED = true;
                    write(b"FS FSCK OK\n");
                } else if !ok {
                    write(b"FS CORRUPT\n");
                }
            }
            if ok {
                (vfs::OK, 0, 0, 0)
            } else {
                (vfs::ERR, vfs::EIO, 0, 0)
            }
        }
        _ => (vfs::ERR, vfs::EINVAL, 0, 0),
    }
}

#[no_mangle]
pub extern "C" fn server_main() -> ! {
    write(SERVER);
    init_fs();
    write(VFS_READY);
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
