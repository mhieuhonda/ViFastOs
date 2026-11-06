//! Minimal VirtIO-MMIO block driver used by the AArch64 QEMU port.
//!
//! One split virtqueue and one 512-byte request are enough for the reference
//! filesystem.  Queue memory is identity-mapped static memory.

#![no_std]

use core::sync::atomic::{fence, Ordering};

pub const SECTOR_SIZE: usize = 512;
/// VirtIO device IDs (subset relevant to QEMU virt).
pub const DEVICE_NET: u32 = 1;
pub const DEVICE_BLOCK: u32 = 2;
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

/// Maximum Ethernet frame QEMU user-net delivers (1514) plus headroom.
pub const NET_BUF_SIZE: usize = 2048;
/// VirtIO-net header: flags(1) + gso_type(1) + hdr_len(2) + gso_size(2) +
/// csum_start(2) + csum_offset(2) + num_buffers(2).
pub const NET_HDR_SIZE: usize = 12;

/// Dedicated split-queue for VirtIO-net (vòng 45). Separate static from the
/// block `QUEUE` so net bring-up never disturbs the disk path; both live in
/// `.bss` after `KERNEL_STORAGE`/BKL (nm-confirmed 4 KiB block QUEUE at
/// 0x401ee000 with room to spare), not inside `Kernel`, so the 8-byte
/// AArch64 static gap is untouched.
#[repr(C, align(4096))]
struct NetQueue {
    desc: [Descriptor; QUEUE_SIZE],
    avail: Available,
    used: Used,
    hdr: [u8; NET_HDR_SIZE],
    buf: [u8; NET_BUF_SIZE],
}

static mut NET_QUEUE: NetQueue = NetQueue {
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
    hdr: [0; NET_HDR_SIZE],
    buf: [0; NET_BUF_SIZE],
};

/// Address and size of the dedicated net queue (for platform logging and
/// future TX/RX wiring). Read-only w.r.t. device state.
pub fn net_queue_info() -> (u64, usize) {
    let base = core::ptr::addr_of!(NET_QUEUE) as u64;
    (base, core::mem::size_of::<NetQueue>())
}

static mut NET_READY: bool = false;
static mut NET_BASE: u64 = 0;

/// Negotiate a VirtIO-net device at `base`: reset, ACK+DRIVER, read features,
/// accept the MAC/STATUS/VERSION_1 subset we need for a 1-queue-pair MVP,
/// attach queue 1 (TX) to the dedicated `NET_QUEUE` static, then DRIVER_OK.
/// Queue 0 (RX) is left for the next round. Returns the accepted feature word.
/// Mirrors `init_at` (block) but never touches the block queue or globals.
pub fn net_init_tx(base: u64) -> Option<u64> {
    fn read_at(base: u64, offset: u64) -> u32 {
        unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
    }
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    fn write64_at(base: u64, offset: u64, value: u64) {
        write_at(base, offset, value as u32);
        write_at(base, offset + 4, (value >> 32) as u32);
    }
    let magic = read_at(base, 0);
    let version = read_at(base, 4);
    let device = read_at(base, 8);
    if magic != 0x7472_6976 || version != 2 || device != DEVICE_NET {
        return None;
    }
    // We only need MAC + STATUS for the MVP; accept that subset.
    const WANT: u64 = (1 << 5) | (1 << 16) | (1 << 32);
    write_at(base, 0x70, 0);
    write_at(base, 0x70, 1 | 2);
    let offered = read_at(base, 0x10) as u64 | ((read_at(base, 0x14) as u64) << 32);
    let accepted = offered & WANT;
    write_at(base, 0x20, accepted as u32);
    write_at(base, 0x24, (accepted >> 32) as u32);
    write_at(base, 0x70, 1 | 2 | 8);
    if read_at(base, 0x70) & 8 == 0 {
        return None;
    }
    // Attach TX queue (index 1) to the dedicated static.
    let queue = core::ptr::addr_of_mut!(NET_QUEUE);
    unsafe {
        (*queue).avail.idx = 0;
        (*queue).used.idx = 0;
        (*queue).hdr = [0; NET_HDR_SIZE];
    }
    write_at(base, 0x30, 1);
    write_at(base, 0x38, QUEUE_SIZE as u32);
    let qbase = queue as u64;
    write64_at(
        base,
        0x80,
        qbase + core::mem::offset_of!(NetQueue, desc) as u64,
    );
    write64_at(
        base,
        0x90,
        qbase + core::mem::offset_of!(NetQueue, avail) as u64,
    );
    write64_at(
        base,
        0xa0,
        qbase + core::mem::offset_of!(NetQueue, used) as u64,
    );
    write_at(base, 0x44, 1);
    write_at(base, 0x70, 1 | 2 | 8 | 4);
    if read_at(base, 0x70) & 4 == 0 {
        return None;
    }
    unsafe {
        NET_READY = true;
        NET_BASE = base;
    }
    Some(accepted)
}

pub fn net_ready() -> bool {
    unsafe { NET_READY }
}

static mut BASE: u64 = 0;
static mut READY: bool = false;
static mut LAST_BASE: u64 = 0;
static mut LAST_MAGIC: u32 = 0;
static mut LAST_VERSION: u32 = 0;
static mut LAST_DEVICE: u32 = 0;
static mut LEGACY_SEEN: bool = false;

fn write32(offset: u64, value: u32) {
    unsafe { core::ptr::write_volatile((BASE + offset) as *mut u32, value) }
}

fn queue_ptr() -> *mut Queue {
    core::ptr::addr_of_mut!(QUEUE)
}

fn read_probe(candidate: u64) -> (u32, u32, u32) {
    // Probe with the candidate address directly before publishing BASE.  This
    // keeps the first MMIO read independent of the mutable global and makes a
    // bad slot fail before any device state is touched.
    let magic = unsafe { core::ptr::read_volatile(candidate as *const u32) };
    let version = unsafe { core::ptr::read_volatile((candidate + 4) as *const u32) };
    let device = unsafe { core::ptr::read_volatile((candidate + 8) as *const u32) };
    unsafe {
        BASE = candidate;
        LAST_BASE = candidate;
        LAST_MAGIC = magic;
        LAST_VERSION = version;
        LAST_DEVICE = device;
    }
    (magic, version, device)
}

pub fn probe_info() -> (u64, u32, u32, u32) {
    unsafe { (LAST_BASE, LAST_MAGIC, LAST_VERSION, LAST_DEVICE) }
}

/// Read the 6-byte MAC from a VirtIO-net config space (offset 0x100).
/// Returns None when the slot is not a net device.
pub fn net_mac(base: u64) -> Option<[u8; 6]> {
    let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
    let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
    if magic != 0x7472_6976 || device != DEVICE_NET {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((base + 0x100 + i as u64) as *const u8) };
    }
    Some(mac)
}

/// Read a VirtIO-net device's offered feature bits (DeviceFeatures, 0x10/0x14).
/// Read-only: safe to call during the pre-init probe (vòng 39-44 showed only
/// post-claim state writes can hang QEMU). Returns None for non-net slots.
pub fn net_features(base: u64) -> Option<u64> {
    let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
    let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
    if magic != 0x7472_6976 || device != DEVICE_NET {
        return None;
    }
    let lo = unsafe { core::ptr::read_volatile((base + 0x10) as *const u32) };
    let hi = unsafe { core::ptr::read_volatile((base + 0x14) as *const u32) };
    Some(lo as u64 | ((hi as u64) << 32))
}

fn init_at(candidate: u64) -> bool {
    fn read_at(base: u64, offset: u64) -> u32 {
        unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
    }
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    fn write64_at(base: u64, offset: u64, value: u64) {
        write_at(base, offset, value as u32);
        write_at(base, offset + 4, (value >> 32) as u32);
    }

    let (magic, version, device) = read_probe(candidate);
    if magic != 0x7472_6976 || version != 2 || device != 2 {
        return false;
    }
    write_at(candidate, 0x70, 0);
    write_at(candidate, 0x70, 1 | 2);
    write_at(candidate, 0x20, 0);
    write_at(candidate, 0x24, 0);
    write_at(candidate, 0x70, 1 | 2 | 8);
    if read_at(candidate, 0x70) & 8 == 0 {
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
    write_at(candidate, 0x30, 0);
    write_at(candidate, 0x38, QUEUE_SIZE as u32);
    let base = queue as u64;
    write64_at(
        candidate,
        0x80,
        base + core::mem::offset_of!(Queue, desc) as u64,
    );
    write64_at(
        candidate,
        0x90,
        base + core::mem::offset_of!(Queue, avail) as u64,
    );
    write64_at(
        candidate,
        0xa0,
        base + core::mem::offset_of!(Queue, used) as u64,
    );
    write_at(candidate, 0x44, 1);
    write_at(candidate, 0x70, 1 | 2 | 8 | 4);
    unsafe { READY = true };
    true
}

/// Scan a VirtIO-MMIO window and report every present device slot.
/// The callback receives `(base, device_id)`; slot 0 is the block device
/// used by `init`. Device IDs: 1 = net, 2 = block, 4 = entropy, 9 = console.
pub fn scan_devices(start: u64, end: u64, stride: u64, mut visit: impl FnMut(u64, u32)) {
    let mut base = start;
    while base < end {
        let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
        let version = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
        let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
        if magic == 0x7472_6976 && (version == 1 || version == 2) && device != 0 {
            visit(base, device);
        }
        base += stride;
    }
}

/// Scan a VirtIO-MMIO window.  The caller supplies the window and stride so
/// RISC-V can use its own slot range while AArch64 keeps the existing one.
pub fn init_range(start: u64, end: u64, stride: u64) -> bool {
    unsafe {
        READY = false;
        LEGACY_SEEN = false;
        BASE = 0;
    }
    let mut base = start;
    while base < end {
        let (magic, version, device) = read_probe(base);
        if magic == 0x7472_6976 && device == 2 {
            if version == 1 {
                unsafe { LEGACY_SEEN = true };
            } else if version == 2 && init_at(base) {
                return true;
            }
        }
        base += stride;
    }
    false
}

/// Scan the AArch64 QEMU virtio-mmio window (stride 0x200, 32 slots).
pub fn init() -> bool {
    init_range(0x0a00_0000, 0x0a00_4000, 0x200)
}

pub fn initialized() -> bool {
    unsafe { READY }
}

pub fn legacy_seen() -> bool {
    unsafe { LEGACY_SEEN }
}

/// DMA range covered by the static queue.  It must be inside the platform's
/// kernel-accessible identity range.
pub fn dma_range() -> (u64, u64) {
    let base = core::ptr::addr_of!(QUEUE) as u64;
    (base, base + core::mem::size_of::<Queue>() as u64)
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
        let slot = core::ptr::read_volatile(&q.avail.idx) as usize % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        fence(Ordering::SeqCst);
        let next_avail = core::ptr::read_volatile(&q.avail.idx).wrapping_add(1);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        fence(Ordering::SeqCst);
        write32(0x50, 0);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 20_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        fence(Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target
            || core::ptr::read_volatile(&q.status) != 0
        {
            return false;
        }
        if !write {
            buffer.copy_from_slice(&q.data);
        }
    }
    true
}

fn request_with_retry(write: bool, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    for _ in 0..3 {
        if request(write, sector, buffer) {
            return true;
        }
        // QEMU's virtio-mmio device can need a fresh queue after an early
        // probe; reinitialise the same slot before retrying.
        let base = unsafe { BASE };
        if base != 0 && !init_at(base) {
            break;
        }
    }
    false
}

pub fn read_sector(sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    request_with_retry(false, sector, buffer)
}
pub fn write_sector(sector: u64, buffer: &[u8; SECTOR_SIZE]) -> bool {
    let mut local = [0u8; SECTOR_SIZE];
    local.copy_from_slice(buffer);
    request_with_retry(true, sector, &mut local)
}

fn request_flush() -> bool {
    if unsafe { !READY || BASE == 0 } {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        let q = &mut *queue;
        // VIRTIO_BLK_T_FLUSH has no data payload; header + status are enough.
        q.header = [4, 0, 0, 0];
        q.status = 0xff;
        let base = q as *const Queue as u64;
        q.desc[0] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, header) as u64,
            len: 16,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, status) as u64,
            len: 1,
            flags: DESC_WRITE,
            next: 0,
        };
        let slot = core::ptr::read_volatile(&q.avail.idx) as usize % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        fence(Ordering::SeqCst);
        let next_avail = core::ptr::read_volatile(&q.avail.idx).wrapping_add(1);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        fence(Ordering::SeqCst);
        write32(0x50, 0);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 20_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        fence(Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target
            || core::ptr::read_volatile(&q.status) != 0
        {
            return false;
        }
    }
    true
}

pub fn flush() -> bool {
    for _ in 0..3 {
        if request_flush() {
            return true;
        }
        let base = unsafe { BASE };
        if base != 0 && !init_at(base) {
            break;
        }
    }
    false
}
