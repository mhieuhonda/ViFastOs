//! Host-side FS v2 image utility used by the persistence tests.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::ExitCode;

use vifast_fs::{format, BlockDev, Fs, FsError, BLOCK_SIZE};

struct FileDev {
    file: File,
    blocks: u32,
}

impl FileDev {
    fn open(path: &str, write: bool) -> Result<Self, String> {
        let file = if write {
            OpenOptions::new().read(true).write(true).open(path)
        } else {
            File::open(path)
        }
        .map_err(|e| format!("open {path}: {e}"))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        Ok(Self {
            file,
            blocks: (len / BLOCK_SIZE as u64) as u32,
        })
    }
}

impl BlockDev for FileDev {
    fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba >= self.blocks {
            return Err(FsError::Invalid);
        }
        self.file
            .seek(SeekFrom::Start(lba as u64 * BLOCK_SIZE as u64))
            .map_err(|_| FsError::Io)?;
        self.file.read_exact(out).map_err(|_| FsError::Io)
    }
    fn write(&mut self, lba: u32, data: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba >= self.blocks {
            return Err(FsError::Invalid);
        }
        self.file
            .seek(SeekFrom::Start(lba as u64 * BLOCK_SIZE as u64))
            .map_err(|_| FsError::Io)?;
        self.file.write_all(data).map_err(|_| FsError::Io)
    }
    fn block_count(&self) -> u32 {
        self.blocks
    }
}

fn usage() -> ! {
    eprintln!("usage: vifast-fsutil mkfs <disk> | fsck <disk> | put <disk> <path> <text> | put-file <disk> <path> <host-file> | mkdir <disk> <path> | rm <disk> <path> | mv <disk> <old> <new> | stat <disk> <path> | truncate <disk> <path> <size> | link <disk> <old> <new> | symlink <disk> <target> <path> | readlink <disk> <path> | get <disk> <path> | ls <disk> <path> | inject-journal <disk> --valid|--torn");
    std::process::exit(2)
}

fn ensure_parent(fs: &mut Fs<FileDev>, path: &str) -> Result<(), String> {
    let bytes = path.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] != b'/' {
        end -= 1;
    }
    if end <= 1 {
        return Ok(());
    }
    let parent = &path[..end - 1];
    if fs.lookup(parent.as_bytes(), 0).is_ok() {
        return Ok(());
    }
    ensure_parent(fs, parent)?;
    fs.mkdir(parent.as_bytes(), 0)
        .map_err(|e| format!("mkdir {parent}: {e:?}"))?;
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        usage();
    }
    let result: Result<(), String> = (|| {
        let command = args[1].as_str();
        let disk = args[2].as_str();
        match command {
            "mkfs" => {
                if Path::new(disk).exists() {
                    let _ = std::fs::remove_file(disk);
                }
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(disk)
                    .map_err(|e| e.to_string())?;
                file.set_len(vifast_fs::TOTAL_BLOCKS as u64 * BLOCK_SIZE as u64)
                    .map_err(|e| e.to_string())?;
                let mut dev = FileDev::open(disk, true)?;
                format(&mut dev).map_err(|e| format!("{e:?}"))
            }
            "fsck" => {
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                fs.fsck().map_err(|e| format!("fsck: {e:?}"))
            }
            "put" => {
                if args.len() != 5 {
                    usage();
                }
                let path = args[3].as_str();
                let data = args[4].as_bytes();
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                ensure_parent(&mut fs, path)?;
                let ino = match fs.lookup(path.as_bytes(), 0) {
                    Ok(ino) => ino,
                    Err(FsError::NotFound) => fs
                        .create(path.as_bytes(), 0, vifast_fs::INODE_FILE)
                        .map_err(|e| format!("create: {e:?}"))?,
                    Err(e) => return Err(format!("lookup: {e:?}")),
                };
                fs.truncate(ino, 0, 0)
                    .map_err(|e| format!("truncate: {e:?}"))?;
                fs.write(ino, 0, data, 0)
                    .map(|_| ())
                    .map_err(|e| format!("write: {e:?}"))
            }
            "mkdir" => {
                if args.len() != 4 {
                    usage();
                }
                let path = args[3].as_str();
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                ensure_parent(&mut fs, path)?;
                fs.mkdir(path.as_bytes(), 0)
                    .map(|_| ())
                    .map_err(|e| format!("mkdir: {e:?}"))
            }
            "rm" => {
                if args.len() != 4 {
                    usage();
                }
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                fs.unlink(args[3].as_bytes(), 0)
                    .map_err(|e| format!("rm: {e:?}"))
            }
            "mv" => {
                if args.len() != 5 {
                    usage();
                }
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                fs.rename(args[3].as_bytes(), args[4].as_bytes(), 0)
                    .map_err(|e| format!("mv: {e:?}"))
            }
            "stat" => {
                if args.len() != 4 {
                    usage();
                }
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let ino = fs
                    .lookup(args[3].as_bytes(), 0)
                    .map_err(|e| format!("lookup: {e:?}"))?;
                let info = fs.info(ino).map_err(|e| format!("{e:?}"))?;
                println!(
                    "ino={} kind={} size={} mode={} uid={} nlink={}",
                    ino, info.kind, info.size, info.mode, info.uid, info.nlink
                );
                Ok(())
            }
            "truncate" => {
                if args.len() != 5 {
                    usage();
                }
                let size = args[4]
                    .parse::<usize>()
                    .map_err(|_| "truncate: invalid size".to_string())?;
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let ino = fs
                    .lookup(args[3].as_bytes(), 0)
                    .map_err(|e| format!("lookup: {e:?}"))?;
                fs.truncate(ino, size, 0)
                    .map_err(|e| format!("truncate: {e:?}"))
            }
            "link" => {
                if args.len() != 5 {
                    usage();
                }
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                fs.link(args[3].as_bytes(), args[4].as_bytes(), 0)
                    .map_err(|e| format!("link: {e:?}"))?;
                Ok(())
            }
            "put-file" => {
                if args.len() != 5 {
                    usage();
                }
                let path = args[3].as_str();
                let data = std::fs::read(args[4].as_str()).map_err(|e| e.to_string())?;
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                ensure_parent(&mut fs, path)?;
                let ino = match fs.lookup(path.as_bytes(), 0) {
                    Ok(ino) => ino,
                    Err(FsError::NotFound) => fs
                        .create(path.as_bytes(), 0, vifast_fs::INODE_FILE)
                        .map_err(|e| format!("create: {e:?}"))?,
                    Err(e) => return Err(format!("lookup: {e:?}")),
                };
                fs.truncate(ino, 0, 0)
                    .map_err(|e| format!("truncate: {e:?}"))?;
                fs.write(ino, 0, &data, 0)
                    .map(|_| ())
                    .map_err(|e| format!("write: {e:?}"))
            }
            "symlink" => {
                if args.len() != 5 {
                    usage();
                }
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                ensure_parent(&mut fs, args[4].as_str())?;
                fs.symlink(args[3].as_bytes(), args[4].as_bytes(), 0)
                    .map(|_| ())
                    .map_err(|e| format!("symlink: {e:?}"))
            }
            "readlink" => {
                if args.len() != 4 {
                    usage();
                }
                let path = args[3].as_str();
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let (target, len) = fs
                    .readlink(path.as_bytes(), 0)
                    .map_err(|e| format!("readlink: {e:?}"))?;
                std::io::stdout()
                    .write_all(&target[..len])
                    .map_err(|e| e.to_string())
            }
            "get" => {
                if args.len() != 4 {
                    usage();
                }
                let path = args[3].as_str();
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let ino = fs
                    .lookup(path.as_bytes(), 0)
                    .map_err(|e| format!("lookup: {e:?}"))?;
                let size = fs.info(ino).map_err(|e| format!("{e:?}"))?.size as usize;
                let mut out = vec![0u8; size];
                fs.read(ino, 0, &mut out, 0)
                    .map_err(|e| format!("read: {e:?}"))?;
                std::io::stdout().write_all(&out).map_err(|e| e.to_string())
            }
            "ls" => {
                if args.len() != 4 {
                    usage();
                }
                let path = args[3].as_str();
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let ino = fs
                    .lookup(path.as_bytes(), 0)
                    .map_err(|e| format!("lookup: {e:?}"))?;
                let _ = fs.info(ino).map_err(|e| format!("{e:?}"))?;
                let mut index = 0usize;
                while let Some((child, name, len)) = fs
                    .read_dir_entry(ino, index)
                    .map_err(|e| format!("{e:?}"))?
                {
                    println!("{} {}", child, String::from_utf8_lossy(&name[..len]));
                    index += 1;
                }
                Ok(())
            }
            "inject-journal" => {
                if args.len() != 4 {
                    usage();
                }
                let torn = match args[3].as_str() {
                    "--valid" => false,
                    "--torn" => true,
                    _ => usage(),
                };
                let dev = FileDev::open(disk, true)?;
                let mut fs = Fs::mount(dev).map_err(|e| format!("mount: {e:?}"))?;
                let _ino = match fs.lookup(b"/j", 0) {
                    Ok(ino) => ino,
                    Err(FsError::NotFound) => {
                        ensure_parent(&mut fs, "/j")?;
                        fs.create(b"/j", 0, vifast_fs::INODE_FILE)
                            .map_err(|e| format!("{e:?}"))?
                    }
                    Err(e) => return Err(format!("lookup: {e:?}")),
                };
                fs.inject_pending_write(b"/j", b"journal-new\n", 0, torn)
                    .map_err(|e| format!("{e:?}"))
            }
            _ => usage(),
        }
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
