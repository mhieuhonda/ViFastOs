//! Tiny fixed-capacity ramfs and per-TCB file descriptors.
//!
//! This is deliberately a small VFS reference: no heap, no block device, no
//! path walker beyond a flat root directory.  It gives user mode a real
//! open/read/write/close surface while keeping the portable core allocation
//! free.

use vifast_abi::*;

use crate::object::*;
use crate::Kernel;

pub const MAX_FILES: usize = 8;
pub const MAX_FILE_NAME: usize = 24;
pub const MAX_FILE_SIZE: usize = 512;
pub const MAX_PATH: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct RamFile {
    pub name: [u8; MAX_FILE_NAME],
    pub name_len: u8,
    pub data: [u8; MAX_FILE_SIZE],
    pub len: usize,
    pub used: bool,
    pub writable: bool,
}

impl Default for RamFile {
    fn default() -> Self {
        RamFile {
            name: [0; MAX_FILE_NAME],
            name_len: 0,
            data: [0; MAX_FILE_SIZE],
            len: 0,
            used: false,
            writable: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RamFs {
    files: [RamFile; MAX_FILES],
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl RamFs {
    pub const fn new() -> Self {
        RamFs {
            files: [RamFile {
                name: [0; MAX_FILE_NAME],
                name_len: 0,
                data: [0; MAX_FILE_SIZE],
                len: 0,
                used: false,
                writable: true,
            }; MAX_FILES],
        }
    }

    /// Create the deterministic files used by the bring-up user program.
    pub fn format(&mut self) {
        for file in self.files.iter_mut() {
            *file = RamFile::default();
        }
        let _ = self.create(b"/motd", b"ViFastOs ramfs\n");
        let _ = self.create(b"/hello", b"hello from ViFastOs\n");
    }

    pub fn create(&mut self, name: &[u8], data: &[u8]) -> KResult<usize> {
        if name.is_empty() || name.len() > MAX_FILE_NAME {
            return Err(KError::InvalidArg);
        }
        if data.len() > MAX_FILE_SIZE {
            return Err(KError::InvalidArg);
        }
        for (i, file) in self.files.iter_mut().enumerate() {
            if !file.used {
                file.name[..name.len()].copy_from_slice(name);
                file.name_len = name.len() as u8;
                file.data[..data.len()].copy_from_slice(data);
                file.len = data.len();
                file.used = true;
                return Ok(i);
            }
        }
        Err(KError::OutOfMemory)
    }

    pub fn find(&self, name: &[u8]) -> Option<usize> {
        if name.is_empty() || name.len() > MAX_FILE_NAME {
            return None;
        }
        self.files.iter().position(|file| {
            file.used && file.name_len as usize == name.len() && file.name[..name.len()] == *name
        })
    }

    pub fn file(&self, index: usize) -> Option<&RamFile> {
        self.files.get(index).filter(|file| file.used)
    }

    pub fn file_mut(&mut self, index: usize) -> Option<&mut RamFile> {
        self.files.get_mut(index).filter(|file| file.used)
    }

    pub fn read(&self, index: usize, offset: usize, out: &mut [u8]) -> KResult<usize> {
        let file = self.file(index).ok_or(KError::NotFound)?;
        if offset >= file.len {
            return Ok(0);
        }
        let count = out.len().min(file.len - offset);
        out[..count].copy_from_slice(&file.data[offset..offset + count]);
        Ok(count)
    }

    pub fn write(&mut self, index: usize, offset: usize, input: &[u8]) -> KResult<usize> {
        let file = self.file_mut(index).ok_or(KError::NotFound)?;
        if !file.writable {
            return Err(KError::NoRights);
        }
        if offset >= MAX_FILE_SIZE {
            return Err(KError::OutOfMemory);
        }
        let count = input.len().min(MAX_FILE_SIZE - offset);
        file.data[offset..offset + count].copy_from_slice(&input[..count]);
        if offset + count > file.len {
            file.len = offset + count;
        }
        Ok(count)
    }
}

impl Kernel {
    pub(crate) fn init_fs(&mut self) {
        self.fs.format();
    }

    pub fn open_path(&mut self, tcb: ObjRef, path: &[u8], flags: u64) -> KResult<u64> {
        let access = flags & 0x3;
        if flags & !(O_RDWR | O_CREATE) != 0 || access > O_RDWR {
            return Err(KError::InvalidArg);
        }
        let file = match self.fs.find(path) {
            Some(file) => file,
            None if flags & O_CREATE != 0 => self.fs.create(path, &[])?,
            None => return Err(KError::NotFound),
        };
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        for (fd, slot) in table.fds.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(Fd {
                    file: file as u8,
                    offset: 0,
                    flags: access as u8,
                });
                return Ok(fd as u64);
            }
        }
        Err(KError::OutOfMemory)
    }

    pub fn read_fd(&mut self, tcb: ObjRef, fd: u64, out: &mut [u8]) -> KResult<usize> {
        let fd = usize::try_from(fd).map_err(|_| KError::InvalidArg)?;
        let (file, offset, flags) = {
            let table = self.tcb(tcb).ok_or(KError::InvalidCap)?;
            let entry = table
                .fds
                .get(fd)
                .and_then(|slot| *slot)
                .ok_or(KError::InvalidArg)?;
            (entry.file as usize, entry.offset, entry.flags)
        };
        if (flags as u64) == O_WRONLY {
            return Err(KError::NoRights);
        }
        let count = self.fs.read(file, offset, out)?;
        if let Some(entry) = self
            .tcb_mut(tcb)
            .and_then(|table| table.fds.get_mut(fd))
            .and_then(|slot| slot.as_mut())
        {
            entry.offset = entry.offset.saturating_add(count);
        }
        Ok(count)
    }

    pub fn write_fd(&mut self, tcb: ObjRef, fd: u64, input: &[u8]) -> KResult<usize> {
        let fd = usize::try_from(fd).map_err(|_| KError::InvalidArg)?;
        let (file, offset, flags) = {
            let table = self.tcb(tcb).ok_or(KError::InvalidCap)?;
            let entry = table
                .fds
                .get(fd)
                .and_then(|slot| *slot)
                .ok_or(KError::InvalidArg)?;
            (entry.file as usize, entry.offset, entry.flags)
        };
        if (flags as u64) == O_RDONLY {
            return Err(KError::NoRights);
        }
        let count = self.fs.write(file, offset, input)?;
        if let Some(entry) = self
            .tcb_mut(tcb)
            .and_then(|table| table.fds.get_mut(fd))
            .and_then(|slot| slot.as_mut())
        {
            entry.offset = entry.offset.saturating_add(count);
        }
        Ok(count)
    }

    pub fn seek_fd(&mut self, tcb: ObjRef, fd: u64, offset: u64) -> KResult<usize> {
        let fd = usize::try_from(fd).map_err(|_| KError::InvalidArg)?;
        let offset = usize::try_from(offset).map_err(|_| KError::InvalidArg)?;
        let file = {
            let table = self.tcb(tcb).ok_or(KError::InvalidCap)?;
            let entry = table
                .fds
                .get(fd)
                .and_then(|slot| *slot)
                .ok_or(KError::InvalidArg)?;
            entry.file as usize
        };
        let len = self.fs.file(file).ok_or(KError::NotFound)?.len;
        if offset > len {
            return Err(KError::InvalidArg);
        }
        if let Some(entry) = self
            .tcb_mut(tcb)
            .and_then(|table| table.fds.get_mut(fd))
            .and_then(|slot| slot.as_mut())
        {
            entry.offset = offset;
        }
        Ok(offset)
    }

    pub fn stat_fd(&self, tcb: ObjRef, fd: u64) -> KResult<usize> {
        let fd = usize::try_from(fd).map_err(|_| KError::InvalidArg)?;
        let file = {
            let table = self.tcb(tcb).ok_or(KError::InvalidCap)?;
            let entry = table
                .fds
                .get(fd)
                .and_then(|slot| *slot)
                .ok_or(KError::InvalidArg)?;
            entry.file as usize
        };
        self.fs
            .file(file)
            .map(|file| file.len)
            .ok_or(KError::NotFound)
    }

    pub fn close_fd(&mut self, tcb: ObjRef, fd: u64) -> KResult<()> {
        let fd = usize::try_from(fd).map_err(|_| KError::InvalidArg)?;
        let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
        let slot = table.fds.get_mut(fd).ok_or(KError::InvalidArg)?;
        if slot.take().is_none() {
            return Err(KError::InvalidArg);
        }
        Ok(())
    }
}
