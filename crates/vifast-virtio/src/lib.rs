//! Minimal VirtIO-MMIO block driver used by the AArch64 QEMU port.
//!
//! One split virtqueue and one 512-byte request are enough for the reference
//! filesystem.  Queue memory is identity-mapped static memory.

#![no_std]

use core::sync::atomic::{fence, Ordering};

pub const SECTOR_SIZE: usize = 512;
const QUEUE_SIZE: usize = 8;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(4))]
struct Available {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE],
    event: u16,
}

#[repr(C, align(4))]
#[derive(Clone, Copy)]
struct UsedElem {
    id: u32,
    len: u32,
}

#[repr(C, align(4))]
struct Used {
    flags: u16,
    idx: u16,
    ring: [UsedElem; QUEUE_SIZE],
    event: u16,
}

#[repr(C, align(4096))]
struct Queue {
    desc: [Descriptor; QUEUE_SIZE],
    avail: Available,
    used: Used,
    header: [u32; 4],
    data: [u8; SECTOR_SIZE],
    status: u8,
    pad: [u8; 7],
}

const EMPTY_DESC: Descriptor = Descriptor {
    addr: 0,
    len: 0,
    flags: 0,
    next: 0,
};

static mut QUEUE: Queue = Queue {
    desc: [EMPTY_DESC; QUEUE_SIZE],
    avail: Available {
        flags: 0,
        idx: 0,
        ring: [0; QUEUE_SIZE],
        event: 0,
    },
    used: Used {
        flags: 0,
        idx: 0,
        ring: [UsedElem { id: 0, len: 0 }; QUEUE_SIZE],
        event: 0,
    },
    header: [0; 4],
    data: [0; SECTOR_SIZE],
    status: 0,
    pad: [0; 7],
};

static mut BASE: u64 = 0;
static mut READY: bool = false;
static mut LAST_BASE: u64 = 0;
static mut LAST_MAGIC: u32 = 0;
static mut LAST_VERSION: u32 = 0;
static mut LAST_DEVICE: u32 = 0;

fn read32(offset: u64) -> u32 {
    unsafe { core::ptr::read_volatile((BASE + offset) as *const u32) }
}

fn write32(offset: u64, value: u32) {
    unsafe { core::ptr::write_volatile((BASE + offset) as *mut u32, value) }
}

fn write64(offset: u64, value: u64) {
    write32(offset, value as u32);
    write32(offset + 4, (value >> 32) as u32);
}

fn queue_ptr() -> *mut Queue {
    core::ptr::addr_of_mut!(QUEUE)
}

fn probe(candidate: u64) -> bool {
    unsafe { BASE = candidate };
    let magic = read32(0);
    let version = read32(4);
    let device = read32(8);
    unsafe {
        LAST_BASE = candidate;
        LAST_MAGIC = magic;
        LAST_VERSION = version;
        LAST_DEVICE = device;
    }
    magic == 0x7472_6976 && version >= 2 && device == 2
}

pub fn probe_info() -> (u64, u32, u32, u32) {
    unsafe { (LAST_BASE, LAST_MAGIC, LAST_VERSION, LAST_DEVICE) }
}

fn init_at(candidate: u64) -> bool {
    if !probe(candidate) {
        return false;
    }
    write32(0x70, 0);
    write32(0x70, 1 | 2);
    write32(0x20, 0);
    write32(0x24, 0);
    write32(0x70, 1 | 2 | 8);
    if read32(0x70) & 8 == 0 {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        (*queue).avail.idx = 0;
        (*queue).used.idx = 0;
        (*queue).header = [0; 4];
        (*queue).data = [0; SECTOR_SIZE];
        (*queue).status = 0xff;
    }
    write32(0x30, 0);
    write32(0x38, QUEUE_SIZE as u32);
    let base = queue as u64;
    write64(0x80, base + core::mem::offset_of!(Queue, desc) as u64);
    write64(0x90, base + core::mem::offset_of!(Queue, avail) as u64);
    write64(0xa0, base + core::mem::offset_of!(Queue, used) as u64);
    write32(0x44, 1);
    write32(0x70, 1 | 2 | 8 | 4);
    unsafe { READY = true };
    true
}

/// Scan the AArch64 QEMU virtio-mmio window (stride 0x200, 32 slots).
pub fn init() -> bool {
    let mut base = 0x0a00_0000u64;
    while base < 0x0a00_4000 {
        if init_at(base) {
            return true;
        }
        base += 0x200;
    }
    unsafe { READY = false };
    false
}

pub fn initialized() -> bool {
    unsafe { READY }
}

fn request(write: bool, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    if unsafe { !READY || BASE == 0 } {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        let q = &mut *queue;
        q.header = [
            if write { 1 } else { 0 },
            0,
            sector as u32,
            (sector >> 32) as u32,
        ];
        if write {
            q.data.copy_from_slice(buffer);
        }
        q.status = 0xff;
        let base = q as *const Queue as u64;
        q.desc[0] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, header) as u64,
            len: 16,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, data) as u64,
            len: SECTOR_SIZE as u32,
            flags: DESC_NEXT | if write { 0 } else { DESC_WRITE },
            next: 2,
        };
        q.desc[2] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, status) as u64,
            len: 1,
            flags: DESC_WRITE,
            next: 0,
        };
        let slot = q.avail.idx as usize % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        fence(Ordering::SeqCst);
        q.avail.idx = q.avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);
        write32(0x50, 0);
        let target = q.used.idx.wrapping_add(1);
        let mut spins = 0u64;
        while q.used.idx != target && spins < 20_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        fence(Ordering::SeqCst);
        if q.used.idx != target || q.status != 0 {
            return false;
        }
        if !write {
            buffer.copy_from_slice(&q.data);
        }
    }
    true
}

pub fn read_sector(sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    request(false, sector, buffer)
}
pub fn write_sector(sector: u64, buffer: &[u8; SECTOR_SIZE]) -> bool {
    let mut local = [0u8; SECTOR_SIZE];
    local.copy_from_slice(buffer);
    request(true, sector, &mut local)
}
