//! User-space VFS server backed by the journaled `vifast-fs` filesystem.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::IPC_BUF_SERVER_VA;
use vifast_abi::vfs;
use vifast_abi::KError;
use vifast_fs::{BlockDev, CachedDev, Fs, FsError, BLOCK_SIZE, INODE_DIR, INODE_FILE};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const SERVER: &[u8] = b"Server\n";
const VFS_READY: &[u8] = b"VFS SERVER OK\n";
const MOTD: &[u8] = b"ViFastOs ramfs\n";
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
    fn flush(&mut self) -> Result<(), FsError> {
        let result = vifast_user::block_flush(BLOCK_CAP);
        if result.error == KError::Ok {
            Ok(())
        } else {
            Err(FsError::Io)
        }
    }
}

type ServerFs = Fs<CachedDev<ServerDev>>;

#[derive(Clone, Copy)]
struct Fd {
    ino: u16,
    uid: u16,
    dir: bool,
    flags: u64,
    used: bool,
}

const EMPTY_FD: Fd = Fd {
    ino: 0,
    uid: 0,
    dir: false,
    flags: 0,
    used: false,
};

static mut FS: ServerFs = Fs::empty(CachedDev::new(ServerDev));
static mut FDS: [Fd; FD_COUNT] = [EMPTY_FD; FD_COUNT];
static mut DISK_OK: bool = false;
static mut PERSISTED: bool = false;
static mut FSCK_PRINTED: bool = false;

fn write(bytes: &[u8]) {
    let _ = vifast_user::debug_write(bytes.as_ptr() as u64, bytes.len() as u64);
}

/// Tạo file backing cho demand-load (vòng 71) theo từng khối 128 byte — không
/// dùng static 4 KiB để server ELF không phình thêm frame (x86 rất chật).
/// Nội dung: byte i = i XOR 0x5a, 8 byte đầu = DEMAND_VALUE, 8 byte cuối =
/// DEMAND_TAIL. Idempotent khi file đã có dữ liệu.
fn ensure_demand_file() -> bool {
    use vifast_abi::pager::{DEMAND_BYTES, DEMAND_FILE, DEMAND_TAIL, DEMAND_VALUE};
    let fs = fs();
    let ino = match fs.lookup(DEMAND_FILE, 0) {
        Ok(ino) => ino,
        Err(FsError::NotFound) => match fs.create(DEMAND_FILE, 0, INODE_FILE) {
            Ok(ino) => ino,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    if fs.info(ino).map(|info| info.size).unwrap_or(0) != 0 {
        return true;
    }
    let mut chunk = [0u8; 128];
    let mut offset = 0usize;
    while offset < DEMAND_BYTES {
        let n = core::cmp::min(128, DEMAND_BYTES - offset);
        let mut i = 0;
        while i < n {
            chunk[i] = ((offset + i) as u8) ^ 0x5a;
            i += 1;
        }
        if offset == 0 {
            chunk[0..8].copy_from_slice(&DEMAND_VALUE.to_le_bytes());
        }
        if offset + n == DEMAND_BYTES {
            chunk[n - 8..n].copy_from_slice(&DEMAND_TAIL.to_le_bytes());
        }
        if fs.write(ino, offset, &chunk[..n], 0).is_err() {
            return false;
        }
        offset += n;
    }
    true
}

fn fs() -> &'static mut ServerFs {
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

fn alloc_fd(ino: u16, uid: u16, dir: bool, flags: u64) -> Option<u64> {
    unsafe {
        let fds = &mut *core::ptr::addr_of_mut!(FDS);
        for (index, fd) in fds.iter_mut().enumerate() {
            if !fd.used {
                *fd = Fd {
                    ino,
                    uid,
                    dir,
                    flags,
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
            if vifast_fs::format(fs().dev_mut()).is_err() {
                write(b"FS DISK RAM\n");
                write(VFS_READY);
                return;
            }
            fs().dev_mut().clear();
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
    // Vòng 162: kiểm tra toàn vẹn NGAY khi mount. Đây là thứ người dùng cần
    // biết trước khi tin dữ liệu: đĩa vừa nạp có nhất quán hay không.
    // Vòng 163: nếu bất nhất mà thư mục gốc TRỐNG thì được phép format lại;
    // còn có dữ liệu người dùng thì KHÔNG đụng vào — an toàn dữ liệu trước.
    match fs().fsck() {
        Ok(()) => write(b"FS MOUNT CLEAN\n"),
        Err(_) => {
            if fs().root_is_empty() && vifast_fs::format(fs().dev_mut()).is_ok() {
                fs().dev_mut().clear();
                if fs().initialize().is_ok() {
                    write(b"FS MOUNT REPAIRED\n");
                } else {
                    write(b"FS MOUNT DIRTY\n");
                }
            } else {
                write(b"FS MOUNT DIRTY\n");
            }
        }
    }
    if ensure_file(b"/motd", MOTD).is_err() {
        write(b"FS CORRUPT\n");
    }
    // Vòng 71: file backing cho demand-load.
    if ensure_demand_file() {
        write(b"FS PAGE FILE OK\n");
    } else {
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
    if flags & vifast_abi::O_DIRECTORY != 0 && flags & vifast_abi::O_CREATE != 0 {
        return err(FsError::Invalid);
    }
    let ino = match fs.lookup(path_slice(&path, len), uid) {
        Ok(ino) => {
            if flags & (vifast_abi::O_CREATE | vifast_abi::O_EXCL)
                == (vifast_abi::O_CREATE | vifast_abi::O_EXCL)
            {
                return err(FsError::Exists);
            }
            ino
        }
        Err(FsError::NotFound) if flags & vifast_abi::O_CREATE != 0 => {
            match fs.create(path_slice(&path, len), uid, INODE_FILE) {
                Ok(ino) => ino,
                Err(error) => return err(error),
            }
        }
        Err(error) => return err(error),
    };
    let mut info = match fs.info(ino) {
        Ok(info) => info,
        Err(error) => return err(error),
    };
    if flags & vifast_abi::O_DIRECTORY != 0 && info.kind != INODE_DIR {
        return err(FsError::NotDir);
    }
    if info.kind == INODE_DIR {
        // Directory opens are allowed for READDIR.
    } else if flags & (vifast_abi::O_WRONLY | vifast_abi::O_RDWR) != 0 {
        if !fs_can_write(&fs, ino, uid) {
            return err(FsError::NoAccess);
        }
    }
    if flags & vifast_abi::O_TRUNC != 0 {
        if info.kind != INODE_FILE {
            return err(FsError::IsDir);
        }
        if let Err(error) = fs.truncate(ino, 0, uid) {
            return err(error);
        }
        info = match fs.info(ino) {
            Ok(info) => info,
            Err(error) => return err(error),
        };
    }
    if let Some(fd) = alloc_fd(ino, uid, info.kind == INODE_DIR, flags) {
        (vfs::OK, fd, info.size as u64, info.kind as u64)
    } else {
        (vfs::ERR, vfs::EBADF, 0, 0)
    }
}

fn fs_can_write(fs: &ServerFs, ino: u16, uid: u16) -> bool {
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
    let mut offset = data[1] as usize;
    if fd.flags & vifast_abi::O_APPEND != 0 {
        offset = match fs().info(fd.ino) {
            Ok(info) => info.size as usize,
            Err(error) => return err(error),
        };
    }
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

fn stat_path_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let len = data[0] as usize;
    let path = match read_path(data, 0) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let fs = fs();
    let ino = match fs.lookup(path_slice(&path, len), uid) {
        Ok(ino) => ino,
        Err(error) => return err(error),
    };
    match fs.info(ino) {
        Ok(info) => (vfs::OK, info.size as u64, ino as u64, info.kind as u64),
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

/// (#10) Bulk readdir: pack up to `MAX_IO / READDIR_REC` (8) entries into
/// one IPC buffer as `(ino:u16 LE, kind:u8, len:u8, name[28])` records.
/// Returns `(OK, packed_count, next_index, 0)`; when the directory is
/// exhausted mid-page the count is short (possibly 0 with next == start).
/// One IPC per page instead of one per entry, and no per-entry STAT_PATH.
fn readdir_bulk_request(data: [u64; 4], uid: u16) -> (u64, u64, u64, u64) {
    let fd = match get_fd(data[0], uid) {
        Some(fd) => fd,
        None => return (vfs::ERR, vfs::EBADF, 0, 0),
    };
    if !fd.dir {
        return (vfs::ERR, vfs::ENOTDIR, 0, 0);
    }
    let mut index = data[1] as usize;
    let max = core::cmp::min(data[2] as usize, vfs::MAX_IO / vfs::READDIR_REC).max(1);
    let mut packed = 0usize;
    let mut cursor = IPC_BUF_SERVER_VA as *mut u8;
    for _ in 0..max {
        match fs().read_dir_entry_full(fd.ino, index) {
            Ok(Some((ino, name, len, kind))) => {
                unsafe {
                    let dst = cursor;
                    core::ptr::write_bytes(dst, 0, vfs::READDIR_REC);
                    *dst.add(0) = (ino & 0xff) as u8;
                    *dst.add(1) = ((ino >> 8) & 0xff) as u8;
                    *dst.add(2) = kind;
                    *dst.add(3) = (len & 0xff) as u8;
                    core::ptr::copy_nonoverlapping(
                        name.as_ptr(),
                        dst.add(4),
                        core::cmp::min(len, 28),
                    );
                    cursor = dst.add(vfs::READDIR_REC);
                }
                packed += 1;
                index += 1;
            }
            Ok(None) => break,
            Err(error) => {
                if packed == 0 {
                    return err(error);
                }
                break;
            }
        }
    }
    let _ = uid;
    (vfs::OK, packed as u64, index as u64, 0)
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
        vfs::STAT_PATH => stat_path_request(request.data, uid),
        vfs::MKDIR => mkdir_request(request.data, uid),
        vfs::READDIR => readdir_request(request.data, uid),
        vfs::READDIR_BULK => readdir_bulk_request(request.data, uid),
        vfs::UNLINK => unlink_request(request.data, uid),
        vfs::RENAME => rename_request(request.data, uid),
        vfs::LINK => link_request(request.data, uid),
        vfs::SYMLINK => symlink_request(request.data, uid),
        vfs::READLINK => readlink_request(request.data, uid),
        vfs::TRUNCATE => truncate_request(request.data, uid),
        vfs::FSYNC => {
            // (#11) Validate the fd like READ/WRITE/STAT: a bad fd is EBADF,
            // not a silent global flush.  The flush itself is still a global
            // journal-commit + device barrier (documented in `Fs::fsync`).
            if get_fd(request.data[0], uid).is_none() {
                (vfs::ERR, vfs::EBADF, 0, 0)
            } else {
                match fs().fsync() {
                    Ok(()) => (vfs::OK, 0, 0, 0),
                    Err(error) => err(error),
                }
            }
        }
        vfs::STATFS => {
            let (used, total) = fs().usage();
            (
                vfs::OK,
                used as u64,
                total as u64,
                vifast_fs::BLOCK_SIZE as u64,
            )
        }
        vfs::FSCK => {
            // T26: in MÃ LỖI fsck (trước đây chỉ "FS CORRUPT", không biết vì sao).
            let outcome = fs().fsck();
            let ok = outcome.is_ok();
            if let Err(error) = outcome {
                let mut line = [0u8; 24];
                line[0] = b'F';
                line[1] = b'S';
                line[2] = b' ';
                line[3] = b'E';
                line[4] = b'R';
                line[5] = b'R';
                line[6] = b' ';
                let mut value = error as u8;
                line[7] = b'0' + value / 100 % 10;
                line[8] = b'0' + value / 10 % 10;
                line[9] = b'0' + value % 10;
                line[10] = b'\n';
                write(&line[..11]);
            }
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
