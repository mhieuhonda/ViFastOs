//! ViFastOs FS v3: a fixed-size, journaled filesystem for user-space VFS.
//!
//! The filesystem is deliberately small: no allocation, 64 inodes, and one
//! 16-block write-ahead journal.  New `VF04` images use 10 direct blocks plus
//! one single-indirect and one double-indirect block per file (up to 8.4 MiB
//! logically, bounded by the 4 MiB image).  Legacy `VF03` images still mount
//! with their original 11-direct + single-indirect geometry (69 KiB files).
//! A large write is split into per-block atomic transactions rather than
//! claiming whole-file atomicity.
#![no_std]
#![forbid(unsafe_code)]

pub const BLOCK_SIZE: usize = 512;
pub const TOTAL_BLOCKS: u32 = 8192;
pub const BITMAP_START: u32 = 2;
pub const BITMAP_BLOCKS: u32 = 2;
pub const INODE_START: u32 = 4;
pub const INODE_BLOCKS: u32 = 8;
pub const JOURNAL_START: u32 = 12;
pub const JOURNAL_BLOCKS: u32 = 18;
pub const JOURNAL_DATA_START: u32 = 13;
pub const JOURNAL_COMMIT_LBA: u32 = 29;
pub const DATA_START: u32 = 30;
pub const MAX_INODES: usize = 64;
pub const INODE_SIZE: usize = 64;
pub const MAX_BLOCKS_PER_INODE: usize = 12;
/// Legacy `VF03` geometry: 11 direct blocks + one single-indirect block.
pub const MAX_DIRECT_BLOCKS: usize = 11;
pub const INDIRECT_ENTRIES: usize = BLOCK_SIZE / 4;
pub const MAX_FILE_BLOCKS_V3: usize = MAX_DIRECT_BLOCKS + INDIRECT_ENTRIES;
pub const MAX_FILE_SIZE_V3: usize = MAX_FILE_BLOCKS_V3 * BLOCK_SIZE;
/// `VF04` geometry: 10 direct + one single-indirect + one double-indirect.
pub const V4_DIRECT_BLOCKS: usize = 10;
pub const V4_SINGLE_INDIRECT_INDEX: usize = V4_DIRECT_BLOCKS;
pub const V4_DOUBLE_INDIRECT_INDEX: usize = V4_DIRECT_BLOCKS + 1;
pub const MAX_FILE_BLOCKS: usize =
    V4_DIRECT_BLOCKS + INDIRECT_ENTRIES + INDIRECT_ENTRIES * INDIRECT_ENTRIES;
pub const MAX_FILE_SIZE: usize = MAX_FILE_BLOCKS * BLOCK_SIZE;
pub const MAX_DIR_BLOCKS: usize = MAX_DIRECT_BLOCKS;
pub const DIR_ENTRY_SIZE: usize = 32;
pub const DIR_ENTRIES_PER_BLOCK: usize = 16;
pub const MAX_NAME: usize = 28;
pub const MAX_PATH: usize = 128;
pub const ROOT_INO: u16 = 1;
pub const INODE_FREE: u8 = 0;
pub const INODE_FILE: u8 = 1;
pub const INODE_DIR: u8 = 2;
pub const INODE_SYMLINK: u8 = 3;
pub const MAX_SYMLINK_DEPTH: usize = 8;

/// Format-time geometry.  `total_blocks == 0` uses the device capacity.
#[derive(Clone, Copy, Debug)]
pub struct FormatOptions {
    pub total_blocks: u32,
    pub inode_limit: u16,
    pub dir_block_limit: u8,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            total_blocks: 0,
            inode_limit: MAX_INODES as u16,
            dir_block_limit: MAX_DIR_BLOCKS as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SuperInfo {
    version: u8,
    inode_limit: usize,
    dir_block_limit: usize,
    direct_blocks: usize,
    max_file_blocks: usize,
    max_file_size: usize,
    /// P1-1 (r290): số block của FS theo SUPERBLOCK, không theo thiết bị.
    /// Đĩa lớn hơn vùng đã format là hợp lệ; dùng `dev.block_count()` ở đây
    /// từng làm `load_metadata` quét quá biên bitmap (panic out of bounds).
    total: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    Io,
    Invalid,
    NoSpace,
    NoInode,
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotEmpty,
    NoAccess,
    NameTooLong,
    BadFd,
    TooManyWrites,
    Corrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Inode {
    pub kind: u8,
    pub mode: u8,
    pub uid: u16,
    pub nlink: u16,
    pub size: u32,
    pub generation: u32,
    pub blocks: [u32; MAX_BLOCKS_PER_INODE],
}

#[derive(Clone, Copy, Debug)]
pub struct InodeInfo {
    pub kind: u8,
    pub mode: u8,
    pub uid: u16,
    pub nlink: u16,
    pub size: u32,
}

pub trait BlockDev {
    fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError>;
    fn write(&mut self, lba: u32, data: &[u8; BLOCK_SIZE]) -> Result<(), FsError>;
    fn block_count(&self) -> u32 {
        TOTAL_BLOCKS
    }
    /// Flush the device's volatile write cache.  Backends without a cache can
    /// keep the default no-op; real block drivers should override it.
    fn flush(&mut self) -> Result<(), FsError> {
        Ok(())
    }

    /// Đọc **bỏ qua cache** (T26: đèn phân biệt "cache khác đĩa" với "đĩa thật
    /// sự cũ"). Thiết bị không có cache giữ mặc định = `read`.
    fn read_uncached(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        self.read(lba, out)
    }

    /// r296 (DIRSNAP): `(có trong cache, đang bẩn)`. Mặc định "không có cache";
    /// `CachedDev` ghi đè. Chỉ dùng cho chẩn đoán.
    fn slot_state(&self, lba: u32) -> (bool, bool) {
        let _ = lba;
        (false, false)
    }

    /// r296: `(evict_lost, evict_dirty)`. Mặc định 0 cho thiết bị không cache.
    fn evict_stats(&self) -> (u64, u64) {
        (0, 0)
    }
}

/// Number of 512-byte blocks kept in the write-through cache.
///
/// Eight slots cost about 4 KiB and fit comfortably in the VFS server's
/// static storage.  Keeping the cache small also makes eviction predictable:
/// the wrapper uses first-free, then round-robin replacement.
///
/// NOTE (#9a REVERTED): raising this to 32 broke x86 QEMU deterministically
/// (`PROG SIZE BAD` / `FS V2 BAD` via ATA `EIO`): with 32 slots, `flush()`
/// pushes many dirty blocks back-to-back and QEMU's ATA emulation drops the
/// burst (`ata_wait_bsy` timeout).  Virtio ports (riscv64/aarch64) tolerated
/// it, but correctness on all three ports comes first.  Revisit with a
/// larger cache only alongside an ATA driver that paces burst writes.
pub const CACHE_SLOTS: usize = 8;

/// T26: ba số đo của lần `read_dir_entry_full` trả `Ok(None)` gần nhất:
/// `(index, số entry đếm được qua cache, số entry đếm được qua đường RAW)`.
/// Lệch nhau ⇒ ảnh cache khác ảnh đĩa (đèn quyết định), không phải "hết thật".
static DIR_PROBE: [core::sync::atomic::AtomicU32; 3] = [
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
];

/// T26: hình dạng thư mục tại lần `Ok(None)` gần nhất:
/// `(size, blocks_used, số entry khối 0, số entry khối 1)`.
static DIR_SHAPE: [core::sync::atomic::AtomicU32; 4] = [
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
];

/// Số lần `commit()` ghi xong mà đọc lại (bỏ cache) thấy KHÁC ảnh journal.
static COMMIT_LOST: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Tổng số target đã kiểm (mẫu số của đèn trên).
static COMMIT_VERIFIED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// T26: hình dạng inode của lần `lookup_child` TÌM THẤY gần nhất:
/// `(size, blocks_used, blocks[0], ino)`. So với lần `read_dir_entry_full` trả
/// `Ok(None)` để biết hai hàm có thấy cùng một phiên bản inode không.
static LOOK_HIT: [core::sync::atomic::AtomicU32; 4] = [
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
];

/// T26: `ino` của 8 slot đầu trong block 0 thư mục, tại lần `Ok(None)` gần nhất.
static SLOT_INOS: [core::sync::atomic::AtomicU32; 8] = [
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
    core::sync::atomic::AtomicU32::new(0),
];

pub fn slot_inos() -> [u32; 8] {
    [
        SLOT_INOS[0].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[1].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[2].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[3].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[4].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[5].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[6].load(core::sync::atomic::Ordering::Relaxed),
        SLOT_INOS[7].load(core::sync::atomic::Ordering::Relaxed),
    ]
}

pub fn look_hit() -> (u32, u32, u32, u32) {
    (
        LOOK_HIT[0].load(core::sync::atomic::Ordering::Relaxed),
        LOOK_HIT[1].load(core::sync::atomic::Ordering::Relaxed),
        LOOK_HIT[2].load(core::sync::atomic::Ordering::Relaxed),
        LOOK_HIT[3].load(core::sync::atomic::Ordering::Relaxed),
    )
}

pub fn commit_lost() -> (u32, u32) {
    (
        COMMIT_LOST.load(core::sync::atomic::Ordering::Relaxed),
        COMMIT_VERIFIED.load(core::sync::atomic::Ordering::Relaxed),
    )
}

pub fn dir_shape() -> (u32, u32, u32, u32) {
    (
        DIR_SHAPE[0].load(core::sync::atomic::Ordering::Relaxed),
        DIR_SHAPE[1].load(core::sync::atomic::Ordering::Relaxed),
        DIR_SHAPE[2].load(core::sync::atomic::Ordering::Relaxed),
        DIR_SHAPE[3].load(core::sync::atomic::Ordering::Relaxed),
    )
}

pub fn dir_probe() -> (u32, u32, u32) {
    (
        DIR_PROBE[0].load(core::sync::atomic::Ordering::Relaxed),
        DIR_PROBE[1].load(core::sync::atomic::Ordering::Relaxed),
        DIR_PROBE[2].load(core::sync::atomic::Ordering::Relaxed),
    )
}

#[derive(Clone, Copy)]
struct CacheSlot {
    lba: u32,
    data: [u8; BLOCK_SIZE],
    dirty: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
}

/// A small block cache with write-back for file data and write-through for
/// metadata/journal blocks.
///
/// Journal data, journal headers and commit records live below `DATA_START`
/// and are written through immediately, preserving write-ahead ordering.
/// File data blocks are cached dirty; `commit()` flushes them before clearing
/// the journal commit record, and `Fs::fsync()` flushes them explicitly.
pub struct CachedDev<D> {
    inner: D,
    slots: [Option<CacheSlot>; CACHE_SLOTS],
    next: usize,
    hits: u64,
    misses: u64,
    writes: u64,
    /// Số lần "ghi xong mà đọc lại thấy khác" — bằng chứng trực tiếp của việc
    /// mất lệnh ghi (T26).
    lost_writes: u64,
    /// r296: số lần ĐẨY MỘT BLOCK BẨN RA KHỎI CACHE (eviction) xong đọc lại thì
    /// thấy ảnh CŨ. Đây là đường ghi duy nhất trước đây KHÔNG được kiểm: `flush()`
    /// có kiểm, còn `insert()` thì không. Giả thuyết T26 mới nhất: eviction ghi
    /// xuống thiết bị nhưng lần đọc sau (cache miss) vẫn thấy ảnh trước đó.
    evict_lost: u64,
    /// r296: tổng số lần evict một block bẩn (mẫu số của `evict_lost`).
    evict_dirty: u64,
}

impl<D: BlockDev> CachedDev<D> {
    pub const fn new(inner: D) -> Self {
        Self {
            inner,
            slots: [None; CACHE_SLOTS],
            next: 0,
            hits: 0,
            misses: 0,
            writes: 0,
            lost_writes: 0,
            evict_lost: 0,
            evict_dirty: 0,
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            writes: self.writes,
        }
    }

    /// Số lần ghi-xong-mà-đọc-lại-khác (T26). Chỉ dùng cho chẩn đoán.
    pub fn lost_writes(&self) -> u64 {
        self.lost_writes
    }

    /// r296: `(evict_lost, evict_dirty)` — số lần đẩy block bẩn ra khỏi cache
    /// mà đọc lại thấy ảnh cũ / tổng số lần đẩy block bẩn.
    pub fn evict_stats(&self) -> (u64, u64) {
        (self.evict_lost, self.evict_dirty)
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    /// r296 (DIRSNAP): bản ghi này có đang nằm trong cache không, và có bẩn
    /// (chưa xuống đĩa) không. Chỉ để chẩn đoán.
    pub fn slot_state(&self, lba: u32) -> (bool, bool) {
        for slot in self.slots.iter().flatten() {
            if slot.lba == lba {
                return (true, slot.dirty);
            }
        }
        (false, false)
    }

    /// Drop all cached blocks.  Used after formatting a device outside the
    /// cache or after an external writer changes the disk.
    pub fn clear(&mut self) {
        self.slots = [None; CACHE_SLOTS];
        self.next = 0;
    }

    fn insert(&mut self, lba: u32, data: &[u8; BLOCK_SIZE], dirty: bool) -> Result<(), FsError> {
        for slot in self.slots.iter_mut().flatten() {
            if slot.lba == lba {
                slot.data.copy_from_slice(data);
                slot.dirty = dirty;
                return Ok(());
            }
        }
        let index = self
            .slots
            .iter()
            .position(|slot| slot.is_none())
            .unwrap_or_else(|| {
                let index = self.next;
                self.next = (self.next + 1) % CACHE_SLOTS;
                index
            });
        if let Some(old) = self.slots[index] {
            if old.dirty {
                self.inner.write(old.lba, &old.data)?;
                self.evict_dirty = self.evict_dirty.wrapping_add(1);
                // r296: ĐỌC LẠI ngay sau khi đẩy ra — đường ghi này trước đây
                // không có nhân chứng nào.
                let mut verify = [0u8; BLOCK_SIZE];
                if self.inner.read(old.lba, &mut verify).is_ok() && verify != old.data {
                    self.evict_lost = self.evict_lost.wrapping_add(1);
                }
            }
        }
        self.slots[index] = Some(CacheSlot {
            lba,
            data: *data,
            dirty,
        });
        Ok(())
    }
}

impl<D: BlockDev> BlockDev for CachedDev<D> {
    fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        for slot in self.slots.iter().flatten() {
            if slot.lba == lba {
                out.copy_from_slice(&slot.data);
                self.hits = self.hits.wrapping_add(1);
                return Ok(());
            }
        }
        let mut data = [0u8; BLOCK_SIZE];
        self.inner.read(lba, &mut data)?;
        out.copy_from_slice(&data);
        self.insert(lba, &data, false)?;
        self.misses = self.misses.wrapping_add(1);
        Ok(())
    }

    fn write(&mut self, lba: u32, data: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba < DATA_START {
            self.inner.write(lba, data)?;
            self.insert(lba, data, false)?;
        } else {
            self.insert(lba, data, true)?;
        }
        self.writes = self.writes.wrapping_add(1);
        Ok(())
    }

    fn block_count(&self) -> u32 {
        self.inner.block_count()
    }

    /// Đi thẳng xuống thiết bị, **không** qua slot cache.
    fn read_uncached(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        self.inner.read(lba, out)
    }

    fn slot_state(&self, lba: u32) -> (bool, bool) {
        for slot in self.slots.iter().flatten() {
            if slot.lba == lba {
                return (true, slot.dirty);
            }
        }
        (false, false)
    }

    fn evict_stats(&self) -> (u64, u64) {
        (self.evict_lost, self.evict_dirty)
    }

    fn flush(&mut self) -> Result<(), FsError> {
        // T26 (đèn quyết định): ghi xong thì ĐỌC LẠI và so. Nếu lệnh ghi bị
        // thiết bị bỏ im lặng (giả thuyết "burst flush bị QEMU đánh rơi" ghi
        // trong chú thích `CACHE_SLOTS` ở đầu tệp), thì đọc lại sẽ thấy ảnh
        // cũ — đây là cách chứng minh trực tiếp thay vì suy từ status.
        for slot in self.slots.iter_mut().flatten() {
            if slot.dirty {
                self.inner.write(slot.lba, &slot.data)?;
                slot.dirty = false;
                let mut verify = [0u8; BLOCK_SIZE];
                if self.inner.read(slot.lba, &mut verify).is_ok() && verify != slot.data {
                    self.lost_writes = self.lost_writes.wrapping_add(1);
                }
            }
        }
        self.inner.flush()
    }
}

fn get_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}
fn get_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}
fn put_u16(data: &mut [u8], off: usize, value: u16) {
    data[off..off + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(data: &mut [u8], off: usize, value: u32) {
    data[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

/// Precomputed CRC-32 (IEEE, polynomial `0xedb88320`) lookup table.
///
/// Generated at compile time by `build_crc32_table()` so the source carries
/// no 256 hand-typed constants that could hide a typo.  `crc32()` and
/// `crc32_continue()` consult this table instead of shifting one bit at a
/// time (~8 table lookups per byte vs 64 bit-steps).
const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0u32;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub const CRC32_TABLE: [u32; 256] = build_crc32_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        let index = ((crc ^ (*byte as u32)) & 0xff) as usize;
        crc = CRC32_TABLE[index] ^ (crc >> 8);
    }
    !crc
}

fn inode_encode(inode: &Inode, out: &mut [u8]) {
    for byte in out.iter_mut() {
        *byte = 0;
    }
    out[0] = inode.kind;
    out[1] = inode.mode;
    put_u16(out, 2, inode.uid);
    put_u16(out, 4, inode.nlink);
    put_u32(out, 8, inode.size);
    put_u32(out, 12, inode.generation);
    for (index, block) in inode.blocks.iter().enumerate() {
        put_u32(out, 16 + index * 4, *block);
    }
}
fn inode_decode(input: &[u8]) -> Inode {
    let mut inode = Inode::default();
    inode.kind = input[0];
    inode.mode = input[1];
    inode.uid = get_u16(input, 2);
    inode.nlink = get_u16(input, 4);
    inode.size = get_u32(input, 8);
    inode.generation = get_u32(input, 12);
    for index in 0..MAX_BLOCKS_PER_INODE {
        inode.blocks[index] = get_u32(input, 16 + index * 4);
    }
    inode
}

fn encode_super(
    total_blocks: u32,
    root: u16,
    inode_limit: u16,
    dir_block_limit: u8,
) -> [u8; BLOCK_SIZE] {
    let mut block = [0u8; BLOCK_SIZE];
    block[..4].copy_from_slice(b"VF04");
    put_u32(&mut block, 4, 4);
    put_u32(&mut block, 8, BLOCK_SIZE as u32);
    put_u32(&mut block, 12, total_blocks);
    put_u32(&mut block, 16, inode_limit as u32);
    put_u32(&mut block, 20, BITMAP_START);
    put_u32(&mut block, 24, BITMAP_BLOCKS);
    put_u32(&mut block, 28, INODE_START);
    put_u32(&mut block, 32, INODE_BLOCKS);
    put_u32(&mut block, 36, JOURNAL_START);
    put_u32(&mut block, 40, JOURNAL_BLOCKS);
    put_u32(&mut block, 44, DATA_START);
    put_u32(&mut block, 48, root as u32);
    put_u32(&mut block, 52, dir_block_limit as u32);
    let crc = crc32(&block[..56]);
    put_u32(&mut block, 56, crc);
    block
}

fn validate_super(block: &[u8; BLOCK_SIZE], capacity: u32) -> Result<SuperInfo, FsError> {
    let (version, direct_blocks, max_file_blocks, max_dir_blocks, size_limit) =
        if &block[..4] == b"VF04" && get_u32(block, 4) == 4 {
            (
                4u8,
                V4_DIRECT_BLOCKS,
                MAX_FILE_BLOCKS,
                MAX_DIR_BLOCKS,
                MAX_FILE_SIZE,
            )
        } else if &block[..4] == b"VF03" && get_u32(block, 4) == 3 {
            (
                3u8,
                MAX_DIRECT_BLOCKS,
                MAX_FILE_BLOCKS_V3,
                MAX_DIR_BLOCKS,
                MAX_FILE_SIZE_V3,
            )
        } else if &block[..4] == b"VF02" && get_u32(block, 4) == 2 {
            (
                2u8,
                MAX_BLOCKS_PER_INODE,
                MAX_BLOCKS_PER_INODE,
                MAX_BLOCKS_PER_INODE,
                MAX_BLOCKS_PER_INODE * BLOCK_SIZE,
            )
        } else {
            return Err(FsError::Invalid);
        };
    if get_u32(block, 8) != BLOCK_SIZE as u32 {
        return Err(FsError::Invalid);
    }
    let total = get_u32(block, 12);
    if total < DATA_START + 2 || total > TOTAL_BLOCKS || total > capacity {
        return Err(FsError::Invalid);
    }
    let inode_limit = get_u32(block, 16) as usize;
    let dir_block_limit = get_u32(block, 52) as usize;
    if inode_limit == 0
        || inode_limit > MAX_INODES
        || dir_block_limit == 0
        || dir_block_limit > max_dir_blocks
        || get_u32(block, 20) != BITMAP_START
        || get_u32(block, 24) != BITMAP_BLOCKS
        || get_u32(block, 28) != INODE_START
        || get_u32(block, 32) != INODE_BLOCKS
        || get_u32(block, 36) != JOURNAL_START
        || get_u32(block, 40) != JOURNAL_BLOCKS
        || get_u32(block, 44) != DATA_START
        || get_u32(block, 48) != ROOT_INO as u32
    {
        return Err(FsError::Invalid);
    }
    let mut copy = *block;
    let expected = get_u32(block, 56);
    put_u32(&mut copy, 56, 0);
    if crc32(&copy[..56]) != expected {
        return Err(FsError::Corrupt);
    }
    Ok(SuperInfo {
        version,
        inode_limit,
        dir_block_limit,
        direct_blocks,
        max_file_blocks,
        max_file_size: size_limit,
        total,
    })
}

fn zero_block() -> [u8; BLOCK_SIZE] {
    [0u8; BLOCK_SIZE]
}

/// Format a block device with an empty root directory.  Formatting is not
/// journaled (there is no old filesystem to preserve); every call is
/// deterministic and writes all metadata.
pub fn format<D: BlockDev>(dev: &mut D) -> Result<(), FsError> {
    format_with_options(dev, FormatOptions::default())
}

pub fn format_with_options<D: BlockDev>(
    dev: &mut D,
    options: FormatOptions,
) -> Result<(), FsError> {
    if dev.block_count() < DATA_START + 2 {
        return Err(FsError::NoSpace);
    }
    if options.inode_limit == 0 || options.inode_limit as usize > MAX_INODES {
        return Err(FsError::Invalid);
    }
    if options.dir_block_limit == 0 || options.dir_block_limit as usize > MAX_DIR_BLOCKS {
        return Err(FsError::Invalid);
    }
    let capacity = dev.block_count();
    let total = if options.total_blocks == 0 {
        capacity.min(TOTAL_BLOCKS)
    } else {
        options.total_blocks.min(capacity).min(TOTAL_BLOCKS)
    };
    if total < DATA_START + 2 {
        return Err(FsError::NoSpace);
    }
    let mut zero = zero_block();
    dev.write(0, &zero)?;
    let superblock = encode_super(
        total,
        ROOT_INO,
        options.inode_limit,
        options.dir_block_limit,
    );
    dev.write(1, &superblock)?;
    zero = zero_block();
    for lba in BITMAP_START..BITMAP_START + BITMAP_BLOCKS {
        dev.write(lba, &zero)?;
    }
    let mut inode_block = zero_block();
    let root = Inode {
        kind: INODE_DIR,
        mode: 0b0111,
        uid: 1,
        nlink: 1,
        size: 0,
        generation: 1,
        blocks: [0; MAX_BLOCKS_PER_INODE],
    };
    inode_encode(&root, &mut inode_block[..INODE_SIZE]);
    dev.write(INODE_START, &inode_block)?;
    zero = zero_block();
    for lba in INODE_START + 1..INODE_START + INODE_BLOCKS {
        dev.write(lba, &zero)?;
    }
    for lba in JOURNAL_START..JOURNAL_START + JOURNAL_BLOCKS {
        dev.write(lba, &zero)?;
    }
    Ok(())
}

/// Maximum blocks staged in one journal transaction.  The on-disk journal
/// holds `JOURNAL_BLOCKS - 2` data blocks and the commit header reserves
/// exactly this many target slots (`header[16..80]`).
pub const MAX_TX_WRITES: usize = 16;

/// r296 (DIRSNAP): ảnh chụp ba đường đọc của một block thư mục.
#[derive(Clone, Copy, Default)]
pub struct DirSnap {
    pub blocks: u32,
    pub blk0: u32,
    pub isize: u32,
    pub index: u32,
    pub cached: bool,
    pub dirty: bool,
    pub n_cache: u32,
    pub n_raw: u32,
    pub n_jrnl: u32,
    pub lookup: u8,
    pub lookup_ino: u16,
    pub tx: u32,
    /// Tên của từng slot trong block 0 (để chứng minh `d` có/không có mặt).
    pub names: [[u8; MAX_NAME]; DIR_ENTRIES_PER_BLOCK],
    pub name_len: [u8; DIR_ENTRIES_PER_BLOCK],
    pub inos: [u16; DIR_ENTRIES_PER_BLOCK],
}

/// Đếm entry hợp lệ (ino != 0) trong một block thư mục.
fn count_dir_entries(block: &[u8; BLOCK_SIZE]) -> u32 {
    let mut count = 0u32;
    for slot in 0..DIR_ENTRIES_PER_BLOCK {
        let off = slot * DIR_ENTRY_SIZE;
        if get_u16(block, off) != 0 {
            count += 1;
        }
    }
    count
}

pub struct Fs<D: BlockDev> {
    dev: D,
    inodes: [Inode; MAX_INODES],
    bitmap: [u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
    journal: [u8; (JOURNAL_BLOCKS as usize - 2) * BLOCK_SIZE],
    targets: [u32; MAX_TX_WRITES],
    tx_count: usize,
    seq: u32,
    replay_status: u8,
    migrated: bool,
    inode_limit: usize,
    dir_block_limit: usize,
    version: u8,
    direct_blocks: usize,
    max_file_blocks: usize,
    max_file_size: usize,
    /// Running count of allocated blocks in `DATA_START..block_count`.
    /// Maintained by `mark_used`/`mark_free` (bit flips only) and rebuilt by
    /// `load_metadata`, so `usage()` is O(1) instead of scanning the bitmap.
    used_blocks: u32,
    /// P1-1 (r290): số block của FS theo superblock — biên THẬT của bitmap và
    /// của vùng cấp phát. Thiết bị có thể lớn hơn (đĩa gắn thêm) nhưng FS
    /// không được quét/ghi ra ngoài `capacity`.
    capacity: u32,
    /// Hint where the next `alloc_block()` scan starts.  Freed blocks below
    /// the cursor pull it back, so steady-state allocation is O(1) instead
    /// of re-scanning from `DATA_START` every time.
    next_free: u32,
}

const EMPTY_INODE: Inode = Inode {
    kind: INODE_FREE,
    mode: 0,
    uid: 0,
    nlink: 0,
    size: 0,
    generation: 0,
    blocks: [0; MAX_BLOCKS_PER_INODE],
};

impl<D: BlockDev> Fs<D> {
    pub const fn empty(dev: D) -> Self {
        Self {
            dev,
            inodes: [EMPTY_INODE; MAX_INODES],
            bitmap: [0u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
            journal: [0u8; (JOURNAL_BLOCKS as usize - 2) * BLOCK_SIZE],
            targets: [0; MAX_TX_WRITES],
            tx_count: 0,
            seq: 1,
            replay_status: 0,
            migrated: false,
            inode_limit: MAX_INODES,
            dir_block_limit: MAX_DIR_BLOCKS,
            version: 4,
            direct_blocks: V4_DIRECT_BLOCKS,
            max_file_blocks: MAX_FILE_BLOCKS,
            max_file_size: MAX_FILE_SIZE,
            used_blocks: 0,
            capacity: 0,
            next_free: DATA_START,
        }
    }

    /// Initialize a preallocated filesystem object in place.  Server uses
    /// this so the 13 KiB FS state lives in `.bss`, not on its one-page stack.
    pub fn initialize(&mut self) -> Result<(), FsError> {
        self.inodes = [EMPTY_INODE; MAX_INODES];
        self.bitmap = [0u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE];
        self.journal = [0u8; (JOURNAL_BLOCKS as usize - 2) * BLOCK_SIZE];
        self.targets = [0; MAX_TX_WRITES];
        self.tx_count = 0;
        self.seq = 1;
        self.replay_status = 0;
        self.migrated = false;
        self.inode_limit = MAX_INODES;
        self.dir_block_limit = MAX_DIR_BLOCKS;
        self.version = 4;
        self.direct_blocks = V4_DIRECT_BLOCKS;
        self.max_file_blocks = MAX_FILE_BLOCKS;
        self.max_file_size = MAX_FILE_SIZE;
        self.used_blocks = 0;
        self.next_free = DATA_START;
        let capacity = self.dev.block_count();
        self.capacity = capacity;
        let mut superblock = zero_block();
        self.dev.read(1, &mut superblock)?;
        let mut legacy = zero_block();
        let mut is_legacy = false;
        if validate_super(&superblock, capacity).is_err() {
            self.dev.read(0, &mut legacy)?;
            if &legacy[..4] != b"VF01" {
                return Err(FsError::Invalid);
            }
            is_legacy = true;
            format(&mut self.dev)?;
            self.dev.read(1, &mut superblock)?;
        }
        let info = validate_super(&superblock, capacity)?;
        self.capacity = info.total;
        self.inode_limit = info.inode_limit;
        self.dir_block_limit = info.dir_block_limit;
        self.version = info.version;
        self.direct_blocks = info.direct_blocks;
        self.max_file_blocks = info.max_file_blocks;
        self.max_file_size = info.max_file_size;
        self.load_metadata()?;
        self.replay()?;
        if self.replay_status != 0 {
            self.load_metadata()?;
        }
        if is_legacy {
            let len = get_u32(&legacy, 4) as usize;
            let len = core::cmp::min(len, 64);
            self.migrated = true;
            let ino = self.create(b"/scratch", 0, INODE_FILE)?;
            self.write(ino, 0, &legacy[8..8 + len], 0)?;
            let zero = zero_block();
            self.dev.write(0, &zero)?;
        }
        Ok(())
    }

    pub fn mount(dev: D) -> Result<Self, FsError> {
        let mut fs = Self::empty(dev);
        fs.initialize()?;
        Ok(fs)
    }

    pub fn migrated(&self) -> bool {
        self.migrated
    }

    pub fn replayed(&self) -> u8 {
        self.replay_status
    }

    /// Access the underlying device for cache statistics, explicit format, or
    /// future flush operations.
    pub fn dev(&self) -> &D {
        &self.dev
    }

    pub fn dev_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    fn fsck_mark_block(
        &self,
        block: u32,
        block_used: &mut [u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
    ) -> Result<(), FsError> {
        // P1-1/291.6: biên là FS (capacity), không phải dung lượng thiết bị.
        if block < DATA_START || block >= self.capacity {
            return Err(FsError::Corrupt);
        }
        let byte = block as usize / 8;
        let bit = 1u8 << (block as usize % 8);
        if block_used[byte] & bit != 0 {
            return Err(FsError::Corrupt);
        }
        block_used[byte] |= bit;
        Ok(())
    }

    fn fsck_file_tables(
        &mut self,
        inode: &Inode,
        used_blocks: usize,
        block_used: &mut [u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
    ) -> Result<(), FsError> {
        let direct = self.direct_blocks;
        let single = inode.blocks[direct];
        if self.version < 4 {
            if used_blocks > direct {
                if single == 0 {
                    return Err(FsError::Corrupt);
                }
                self.fsck_mark_block(single, block_used)?;
                let mut table = zero_block();
                self.dev.read(single, &mut table)?;
                let keep = used_blocks - direct;
                for slot in 0..INDIRECT_ENTRIES {
                    if slot >= keep && get_u32(&table, slot * 4) != 0 {
                        return Err(FsError::Corrupt);
                    }
                }
            } else if single != 0 {
                return Err(FsError::Corrupt);
            }
            return Ok(());
        }

        if used_blocks > direct {
            if single == 0 {
                return Err(FsError::Corrupt);
            }
            self.fsck_mark_block(single, block_used)?;
            let mut table = zero_block();
            self.dev.read(single, &mut table)?;
            let keep = core::cmp::min(used_blocks - direct, INDIRECT_ENTRIES);
            for slot in 0..INDIRECT_ENTRIES {
                if slot >= keep && get_u32(&table, slot * 4) != 0 {
                    return Err(FsError::Corrupt);
                }
            }
        } else if single != 0 {
            return Err(FsError::Corrupt);
        }

        let double_index = direct + 1;
        let double = inode.blocks[double_index];
        if used_blocks > direct + INDIRECT_ENTRIES {
            if double == 0 {
                return Err(FsError::Corrupt);
            }
            self.fsck_mark_block(double, block_used)?;
            let mut table = zero_block();
            self.dev.read(double, &mut table)?;
            let double_used = used_blocks - direct - INDIRECT_ENTRIES;
            let keep_tables = (double_used + INDIRECT_ENTRIES - 1) / INDIRECT_ENTRIES;
            for slot in 0..INDIRECT_ENTRIES {
                let second = get_u32(&table, slot * 4);
                if slot < keep_tables {
                    if second == 0 {
                        return Err(FsError::Corrupt);
                    }
                    self.fsck_mark_block(second, block_used)?;
                    let mut second_table = zero_block();
                    self.dev.read(second, &mut second_table)?;
                    let keep = if slot + 1 == keep_tables {
                        double_used - slot * INDIRECT_ENTRIES
                    } else {
                        INDIRECT_ENTRIES
                    };
                    for entry in 0..INDIRECT_ENTRIES {
                        if entry >= keep && get_u32(&second_table, entry * 4) != 0 {
                            return Err(FsError::Corrupt);
                        }
                    }
                } else if second != 0 {
                    return Err(FsError::Corrupt);
                }
            }
        } else if double != 0 {
            return Err(FsError::Corrupt);
        }
        Ok(())
    }

    /// Vòng 163: thư mục gốc có TRỐNG không (không tính "." và "..").
    /// Dùng để quyết định có được phép format khi fsck phát hiện bất nhất
    /// hay không: chỉ sửa khi chắc chắn không mất dữ liệu người dùng.
    pub fn root_is_empty(&mut self) -> bool {
        let mut index = 0usize;
        while let Ok(Some((_, name, len))) = self.read_dir_entry(ROOT_INO, index) {
            index += 1;
            let entry = &name[..len.min(MAX_NAME)];
            if entry != b"." && entry != b".." {
                return false;
            }
        }
        true
    }

    pub fn fsck(&mut self) -> Result<(), FsError> {
        let mut superblock = zero_block();
        self.dev.read(1, &mut superblock)?;
        let info = validate_super(&superblock, self.dev.block_count())?;
        let inode_limit = info.inode_limit;
        let dir_limit_bytes = info.dir_block_limit as u32 * BLOCK_SIZE as u32;
        let mut used = [false; MAX_INODES];
        let mut nlink = [0u16; MAX_INODES];
        let mut block_used = [0u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE];
        let mut stack = [0u16; MAX_INODES];
        let mut stack_len = 1usize;
        stack[0] = ROOT_INO;
        used[ROOT_INO as usize - 1] = true;
        nlink[ROOT_INO as usize - 1] = 1;
        while stack_len != 0 {
            stack_len -= 1;
            let ino = stack[stack_len];
            let inode = self.inodes[ino as usize - 1];
            if inode.kind == INODE_DIR {
                if inode.size % BLOCK_SIZE as u32 != 0
                    || inode.size > dir_limit_bytes
                    || inode.blocks_used() != (inode.size / BLOCK_SIZE as u32) as usize
                {
                    return Err(FsError::Corrupt);
                }
                for (block_index, block) in inode.blocks.iter().enumerate() {
                    if block_index >= inode.blocks_used() {
                        if *block != 0 {
                            return Err(FsError::Corrupt);
                        }
                        continue;
                    }
                    if *block < DATA_START || *block >= self.dev.block_count() {
                        return Err(FsError::Corrupt);
                    }
                    let mut data = zero_block();
                    self.dev.read(*block, &mut data)?;
                    for slot in 0..DIR_ENTRIES_PER_BLOCK {
                        let off = slot * DIR_ENTRY_SIZE;
                        let child = get_u16(&data, off);
                        if child == 0 {
                            continue;
                        }
                        let name_len = data[off + 2] as usize;
                        if name_len == 0
                            || name_len > MAX_NAME
                            || child as usize > inode_limit
                            || child == 0
                        {
                            return Err(FsError::Corrupt);
                        }
                        let name = &data[off + 3..off + 3 + name_len];
                        if name == b"."
                            || name == b".."
                            || name.contains(&b'/')
                            || name.contains(&0)
                        {
                            return Err(FsError::Corrupt);
                        }
                        let child_inode = self.inodes[child as usize - 1];
                        if child_inode.kind == INODE_FREE {
                            return Err(FsError::Corrupt);
                        }
                        if !used[child as usize - 1] {
                            used[child as usize - 1] = true;
                            if stack_len >= MAX_INODES {
                                return Err(FsError::Corrupt);
                            }
                            stack[stack_len] = child;
                            stack_len += 1;
                        }
                        nlink[child as usize - 1] = nlink[child as usize - 1].saturating_add(1);
                        if child_inode.kind == INODE_DIR {
                            nlink[ino as usize - 1] = nlink[ino as usize - 1].saturating_add(1);
                        }
                    }
                }
            }
        }
        for index in 0..inode_limit {
            let inode = self.inodes[index];
            if inode.kind == INODE_FREE {
                continue;
            }
            if !used[index] {
                return Err(FsError::Corrupt);
            }
            if inode.kind != INODE_FILE && inode.kind != INODE_DIR && inode.kind != INODE_SYMLINK {
                return Err(FsError::Corrupt);
            }
            if inode.size as usize > self.max_file_size {
                return Err(FsError::Corrupt);
            }
            if inode.kind == INODE_FILE || inode.kind == INODE_SYMLINK {
                if inode.blocks_used() != ((inode.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) {
                    return Err(FsError::Corrupt);
                }
            }
            if inode.nlink != nlink[index] {
                return Err(FsError::Corrupt);
            }
            let used_blocks = inode.blocks_used();
            if inode.kind == INODE_FILE || inode.kind == INODE_SYMLINK {
                if used_blocks > self.max_file_blocks {
                    return Err(FsError::Corrupt);
                }
                for block_index in 0..used_blocks {
                    let block = self.file_block(&inode, block_index)?;
                    // 291.3 (r291): block 0 trong phạm vi `size` là LỖ của file
                    // thưa — hợp lệ, không có gì để đánh dấu. Bản cũ gọi
                    // `fsck_mark_block(0)` ⇒ `0 < DATA_START` ⇒ Corrupt.
                    if block == 0 {
                        continue;
                    }
                    self.fsck_mark_block(block, &mut block_used)?;
                }
                self.fsck_file_tables(&inode, used_blocks, &mut block_used)?;
            } else {
                let mut count = 0usize;
                for (block_index, block) in inode.blocks.iter().enumerate() {
                    if *block == 0 {
                        continue;
                    }
                    if block_index >= used_blocks {
                        return Err(FsError::Corrupt);
                    }
                    self.fsck_mark_block(*block, &mut block_used)?;
                    count += 1;
                }
                if count != used_blocks {
                    return Err(FsError::Corrupt);
                }
            }
        }
        // P1-1: chỉ kiểm trong vùng FS; ngoài `capacity` là đĩa thừa, không
        // phải lỗi của FS.
        for block in DATA_START..self.capacity {
            let bit = self.bitmap[block as usize / 8] & (1 << (block as usize % 8)) != 0;
            let expected = block_used[block as usize / 8] & (1 << (block as usize % 8)) != 0;
            if bit != expected {
                return Err(FsError::Corrupt);
            }
        }
        for block in 0..DATA_START {
            if self.bitmap[block as usize / 8] & (1 << (block as usize % 8)) != 0 {
                return Err(FsError::Corrupt);
            }
        }
        Ok(())
    }

    fn load_metadata(&mut self) -> Result<(), FsError> {
        let mut block = zero_block();
        for index in 0..BITMAP_BLOCKS as usize {
            self.dev.read(BITMAP_START + index as u32, &mut block)?;
            self.bitmap[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE].copy_from_slice(&block);
        }
        // Rebuild the O(1) usage counter and the allocation cursor from the
        // on-disk bitmap (mount / replay path).  `used_blocks` only counts
        // the DATA region, matching `alloc_block`'s domain; `next_free`
        // points at the first hole so the first allocation is cheap.
        // P1-1: quét theo biên của FS, không theo dung lượng thiết bị.
        let capacity = self.capacity;
        self.used_blocks = 0;
        self.next_free = capacity;
        for block_no in DATA_START..capacity {
            if self.block_is_used(block_no) {
                self.used_blocks = self.used_blocks.saturating_add(1);
            } else if self.next_free == capacity {
                self.next_free = block_no;
            }
        }
        for index in 0..INODE_BLOCKS as usize {
            self.dev.read(INODE_START + index as u32, &mut block)?;
            for slot in 0..8 {
                let ino = index * 8 + slot;
                self.inodes[ino] = inode_decode(&block[slot * INODE_SIZE..(slot + 1) * INODE_SIZE]);
            }
        }
        Ok(())
    }

    fn replay(&mut self) -> Result<(), FsError> {
        let mut commit = zero_block();
        if self.dev.read(JOURNAL_COMMIT_LBA, &mut commit).is_err() {
            return Ok(());
        }
        if &commit[..4] != b"VC01" {
            return Ok(());
        }
        let seq = get_u32(&commit, 4);
        let expected = get_u32(&commit, 8);
        let mut header = zero_block();
        if self.dev.read(JOURNAL_START, &mut header).is_err() {
            self.replay_status = 2;
            return Ok(());
        }
        if &header[..4] != b"VJ01" || get_u32(&header, 4) != seq {
            self.replay_status = 2;
            let zero = zero_block();
            let _ = self.dev.write(JOURNAL_COMMIT_LBA, &zero);
            return Ok(());
        }
        let count = header[8] as usize;
        if count == 0 || count > 16 {
            self.replay_status = 2;
            let zero = zero_block();
            let _ = self.dev.write(JOURNAL_COMMIT_LBA, &zero);
            return Ok(());
        }
        for index in 0..count {
            let mut data = zero_block();
            if self
                .dev
                .read(JOURNAL_DATA_START + index as u32, &mut data)
                .is_err()
            {
                self.replay_status = 2;
                return Ok(());
            }
            let start = index * BLOCK_SIZE;
            self.journal[start..start + BLOCK_SIZE].copy_from_slice(&data);
        }
        let mut crc = crc32(&header[..80]);
        for index in 0..count {
            crc = crc32_continue(
                crc,
                &self.journal[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE],
            );
        }
        if crc != expected {
            self.replay_status = 2;
            let zero = zero_block();
            let _ = self.dev.write(JOURNAL_COMMIT_LBA, &zero);
            return Ok(());
        }
        for index in 0..count {
            let target = get_u32(&header, 16 + index * 4);
            if target == 0 || target >= self.dev.block_count() {
                self.replay_status = 2;
                return Ok(());
            }
            let mut data = zero_block();
            data.copy_from_slice(&self.journal[index * BLOCK_SIZE..(index + 1) * BLOCK_SIZE]);
            if self.dev.write(target, &data).is_err() {
                return Err(FsError::Io);
            }
        }
        self.replay_status = 1;
        self.dev.flush()?;
        let zero = zero_block();
        let _ = self.dev.write(JOURNAL_COMMIT_LBA, &zero);
        Ok(())
    }

    fn read_staged(&mut self, lba: u32) -> Result<[u8; BLOCK_SIZE], FsError> {
        for index in 0..self.tx_count {
            if self.targets[index] == lba {
                let start = index * BLOCK_SIZE;
                let mut block = zero_block();
                block.copy_from_slice(&self.journal[start..start + BLOCK_SIZE]);
                return Ok(block);
            }
        }
        let mut block = zero_block();
        self.dev.read(lba, &mut block)?;
        Ok(block)
    }

    fn stage(&mut self, lba: u32, data: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba == 0 || lba >= self.dev.block_count() {
            return Err(FsError::Invalid);
        }
        for index in 0..self.tx_count {
            if self.targets[index] == lba {
                let start = index * BLOCK_SIZE;
                self.journal[start..start + BLOCK_SIZE].copy_from_slice(data);
                return Ok(());
            }
        }
        if self.tx_count >= MAX_TX_WRITES {
            return Err(FsError::TooManyWrites);
        }
        let index = self.tx_count;
        self.targets[index] = lba;
        let start = index * BLOCK_SIZE;
        self.journal[start..start + BLOCK_SIZE].copy_from_slice(data);
        self.tx_count += 1;
        Ok(())
    }

    fn commit(&mut self) -> Result<(), FsError> {
        if self.tx_count == 0 {
            return Ok(());
        }
        let count = self.tx_count;
        for index in 0..count {
            let start = index * BLOCK_SIZE;
            let mut data = zero_block();
            data.copy_from_slice(&self.journal[start..start + BLOCK_SIZE]);
            self.dev.write(JOURNAL_DATA_START + index as u32, &data)?;
        }
        let mut header = zero_block();
        header[..4].copy_from_slice(b"VJ01");
        put_u32(&mut header, 4, self.seq);
        header[8] = count as u8;
        for index in 0..count {
            put_u32(&mut header, 16 + index * 4, self.targets[index]);
        }
        let mut crc = crc32(&header[..80]);
        for index in 0..count {
            let start = index * BLOCK_SIZE;
            crc = crc32_continue(crc, &self.journal[start..start + BLOCK_SIZE]);
        }
        put_u32(&mut header, 80, crc);
        self.dev.write(JOURNAL_START, &header)?;
        let mut commit = zero_block();
        commit[..4].copy_from_slice(b"VC01");
        put_u32(&mut commit, 4, self.seq);
        put_u32(&mut commit, 8, crc);
        self.dev.write(JOURNAL_COMMIT_LBA, &commit)?;
        for index in 0..count {
            let start = index * BLOCK_SIZE;
            let mut data = zero_block();
            data.copy_from_slice(&self.journal[start..start + BLOCK_SIZE]);
            self.dev.write(self.targets[index], &data)?;
        }
        // Never clear the journal commit until dirty data blocks and their
        // write-back cache have reached the device.
        self.dev.flush()?;
        // T26 (đèn quyết định cuối): sau khi flush, đọc lại TỪNG target bằng
        // đường BỎ CACHE và so với ảnh journal. Lệch ⇒ lệnh ghi không tới đĩa
        // dù commit đã "thành công". Đây là phép đo phân biệt hai nhánh còn lại.
        for index in 0..count {
            let mut on_disk = zero_block();
            if self
                .dev
                .read_uncached(self.targets[index], &mut on_disk)
                .is_ok()
            {
                let start = index * BLOCK_SIZE;
                if on_disk[..] != self.journal[start..start + BLOCK_SIZE] {
                    COMMIT_LOST.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                COMMIT_VERIFIED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
        self.dev.write(JOURNAL_COMMIT_LBA, &zero_block())?;
        self.seq = self.seq.wrapping_add(1);
        self.tx_count = 0;
        Ok(())
    }

    fn stage_inode(&mut self, ino: u16) -> Result<(), FsError> {
        let index = ino as usize - 1;
        if index >= self.inode_limit {
            return Err(FsError::Invalid);
        }
        let table_block = index / 8;
        let mut block = zero_block();
        for slot in 0..8 {
            let entry = table_block * 8 + slot;
            inode_encode(
                &self.inodes[entry],
                &mut block[slot * INODE_SIZE..(slot + 1) * INODE_SIZE],
            );
        }
        self.stage(INODE_START + table_block as u32, &block)
    }

    fn stage_bitmap(&mut self) -> Result<(), FsError> {
        for index in 0..BITMAP_BLOCKS as usize {
            let mut block = zero_block();
            let start = index * BLOCK_SIZE;
            block.copy_from_slice(&self.bitmap[start..start + BLOCK_SIZE]);
            self.stage(BITMAP_START + index as u32, &block)?;
        }
        Ok(())
    }

    fn block_is_used(&self, block: u32) -> bool {
        // P1-1: chặn theo CẢ capacity của FS và độ dài bitmap (phòng thủ).
        block < self.capacity
            && (block as usize) / 8 < self.bitmap.len()
            && self.bitmap[block as usize / 8] & (1 << (block as usize % 8)) != 0
    }

    fn mark_used(&mut self, block: u32) {
        if block >= self.capacity || block as usize / 8 >= self.bitmap.len() {
            return;
        }
        let byte = block as usize / 8;
        let bit = 1u8 << (block as usize % 8);
        if self.bitmap[byte] & bit == 0 {
            self.bitmap[byte] |= bit;
            // Only the DATA region feeds the usage counter.
            if block >= DATA_START {
                self.used_blocks = self.used_blocks.saturating_add(1);
            }
        }
    }

    fn mark_free(&mut self, block: u32) {
        // Vòng 83: block number đọc từ đĩa (free_inode_blocks) có thể rác.
        // Không kiểm tra biên thì bitmap[block/8] ghi ra ngoài mảng.
        if block >= self.capacity || block as usize / 8 >= self.bitmap.len() {
            return;
        }
        let byte = block as usize / 8;
        let bit = 1u8 << (block as usize % 8);
        if self.bitmap[byte] & bit != 0 {
            self.bitmap[byte] &= !bit;
            if block >= DATA_START {
                self.used_blocks = self.used_blocks.saturating_sub(1);
                // Pull the cursor back so the freed block is reused soon.
                if block < self.next_free {
                    self.next_free = block;
                }
            }
        }
    }

    fn alloc_block(&mut self) -> Result<u32, FsError> {
        // P1-1: cấp phát trong biên FS, không trong biên thiết bị.
        let capacity = self.capacity;
        // Scan from the cursor, wrapping once, instead of always restarting
        // at DATA_START.  The common case (cursor at a free block) is O(1).
        let start = self.next_free.clamp(DATA_START, capacity);
        for pass in 0..2 {
            let from = if pass == 0 { start } else { DATA_START };
            let to = if pass == 0 { capacity } else { start };
            let mut block = from;
            while block < to {
                if !self.block_is_used(block) {
                    self.mark_used(block);
                    // Advance past the block just handed out.
                    self.next_free = if block + 1 < capacity {
                        block + 1
                    } else {
                        DATA_START
                    };
                    return Ok(block);
                }
                block += 1;
            }
        }
        // Keep the cursor in range even when the disk is full.
        self.next_free = DATA_START;
        Err(FsError::NoSpace)
    }

    fn file_block(&mut self, inode: &Inode, block_index: usize) -> Result<u32, FsError> {
        if inode.kind == INODE_DIR {
            return Ok(inode.blocks.get(block_index).copied().unwrap_or(0));
        }
        if block_index < self.direct_blocks {
            return Ok(inode.blocks[block_index]);
        }
        // P1-2 (r290): chỉ số block đến từ `inode.size` trên đĩa, tức dữ liệu
        // không tin được. Vượt sức chứa tối đa của FS ⇒ `Corrupt`, KHÔNG được
        // để `get_u32` đọc ngoài mảng (bản cũ panic và giết kernel).
        if block_index >= self.max_file_blocks {
            return Err(FsError::Corrupt);
        }
        let relative = block_index - self.direct_blocks;
        let indirect = inode.blocks[self.direct_blocks];
        if indirect == 0 {
            return Ok(0);
        }
        if self.version < 4 && relative >= INDIRECT_ENTRIES {
            // Không có double-indirect ở định dạng cũ ⇒ chỉ số này bịa.
            return Err(FsError::Corrupt);
        }
        if self.version >= 4 && relative >= INDIRECT_ENTRIES {
            let double_index = self.direct_blocks + 1;
            let double = inode.blocks[double_index];
            if double == 0 {
                return Ok(0);
            }
            let double_relative = relative - INDIRECT_ENTRIES;
            let slot = double_relative / INDIRECT_ENTRIES;
            // P1-2: bảng double-indirect chỉ có `INDIRECT_ENTRIES` ô.
            if slot >= INDIRECT_ENTRIES {
                return Err(FsError::Corrupt);
            }
            let double_table = self.read_staged(double)?;
            let table = get_u32(&double_table, slot * 4);
            if table == 0 {
                return Ok(0);
            }
            let table = self.read_staged(table)?;
            return Ok(get_u32(&table, (double_relative % INDIRECT_ENTRIES) * 4));
        }
        let block = self.read_staged(indirect)?;
        Ok(get_u32(&block, relative * 4))
    }

    fn set_file_block(
        &mut self,
        inode: &mut Inode,
        block_index: usize,
        block_no: u32,
    ) -> Result<(), FsError> {
        if inode.kind == INODE_DIR || block_index < self.direct_blocks {
            inode.blocks[block_index] = block_no;
            return Ok(());
        }
        let relative = block_index - self.direct_blocks;
        if self.version >= 4 && relative >= INDIRECT_ENTRIES {
            let double_index = self.direct_blocks + 1;
            let double_relative = relative - INDIRECT_ENTRIES;
            let slot = double_relative / INDIRECT_ENTRIES;
            let double = inode.blocks[double_index];
            let double = if double == 0 {
                if block_no == 0 {
                    return Ok(());
                }
                let allocated = self.alloc_block()?;
                self.stage(allocated, &zero_block())?;
                self.stage_bitmap()?;
                inode.blocks[double_index] = allocated;
                allocated
            } else {
                double
            };
            let mut double_table = self.read_staged(double)?;
            let table = get_u32(&double_table, slot * 4);
            let table = if table == 0 {
                if block_no == 0 {
                    return Ok(());
                }
                let allocated = self.alloc_block()?;
                self.stage(allocated, &zero_block())?;
                self.stage_bitmap()?;
                put_u32(&mut double_table, slot * 4, allocated);
                self.stage(double, &double_table)?;
                allocated
            } else {
                table
            };
            let mut block = self.read_staged(table)?;
            put_u32(
                &mut block,
                (double_relative % INDIRECT_ENTRIES) * 4,
                block_no,
            );
            return self.stage(table, &block);
        }
        let indirect = inode.blocks[self.direct_blocks];
        let indirect = if indirect == 0 {
            if block_no == 0 {
                return Ok(());
            }
            let allocated = self.alloc_block()?;
            self.stage(allocated, &zero_block())?;
            self.stage_bitmap()?;
            inode.blocks[self.direct_blocks] = allocated;
            allocated
        } else {
            indirect
        };
        let mut table = self.read_staged(indirect)?;
        put_u32(&mut table, relative * 4, block_no);
        self.stage(indirect, &table)
    }

    fn free_inode_blocks(&mut self, inode: &mut Inode) {
        let used = inode.blocks_used();
        for index in 0..used {
            if let Ok(block) = self.file_block(inode, index) {
                if block != 0 {
                    self.mark_free(block);
                }
            }
        }
        if inode.kind == INODE_FILE {
            let indirect = inode.blocks[self.direct_blocks];
            if indirect != 0 {
                self.mark_free(indirect);
                inode.blocks[self.direct_blocks] = 0;
            }
            if self.version >= 4 {
                let double_index = self.direct_blocks + 1;
                let double = inode.blocks[double_index];
                if double != 0 {
                    if let Ok(table) = self.read_staged(double) {
                        for slot in 0..INDIRECT_ENTRIES {
                            let second = get_u32(&table, slot * 4);
                            if second != 0 {
                                self.mark_free(second);
                            }
                        }
                    }
                    self.mark_free(double);
                    inode.blocks[double_index] = 0;
                }
            }
        }
    }

    /// 291.4 (r291): trả một inode vừa cấp về trạng thái tự do (rollback).
    fn release_inode(&mut self, ino: u16) {
        if let Some(inode) = self.inodes.get_mut(ino as usize - 1) {
            inode.kind = INODE_FREE;
            inode.nlink = 0;
            inode.size = 0;
            inode.blocks = [0; MAX_BLOCKS_PER_INODE];
        }
    }

    fn alloc_inode(&mut self) -> Result<u16, FsError> {
        for index in 1..self.inode_limit {
            if self.inodes[index].kind == INODE_FREE {
                let ino = (index + 1) as u16;
                self.inodes[index] = Inode {
                    kind: INODE_FILE,
                    mode: 0b0111,
                    uid: 0,
                    nlink: 1,
                    size: 0,
                    generation: self.seq ^ index as u32,
                    blocks: [0; MAX_BLOCKS_PER_INODE],
                };
                return Ok(ino);
            }
        }
        Err(FsError::NoInode)
    }

    fn can_read(&self, inode: &Inode, uid: u16) -> bool {
        uid == 0
            || (uid == inode.uid && inode.mode & 1 != 0)
            || (uid != inode.uid && inode.mode & 4 != 0)
    }
    fn can_write(&self, inode: &Inode, uid: u16) -> bool {
        uid == 0
            || (uid == inode.uid && inode.mode & 2 != 0)
            || (uid != inode.uid && inode.mode & 8 != 0)
    }

    fn next_component<'a>(path: &'a [u8], pos: &mut usize) -> Result<Option<&'a [u8]>, FsError> {
        while *pos < path.len() && path[*pos] == b'/' {
            *pos += 1;
        }
        if *pos >= path.len() {
            return Ok(None);
        }
        let start = *pos;
        while *pos < path.len() && path[*pos] != b'/' {
            *pos += 1;
        }
        let name = &path[start..*pos];
        if name.len() > MAX_NAME {
            return Err(FsError::NameTooLong);
        }
        if name == b"." || name == b".." {
            return Err(FsError::Invalid);
        }
        Ok(Some(name))
    }

    fn lookup_child(&mut self, dir: u16, name: &[u8]) -> Result<Option<u16>, FsError> {
        let inode = self.inodes[dir as usize - 1];
        if inode.kind != INODE_DIR {
            return Err(FsError::NotDir);
        }
        for block_index in 0..inode.blocks_used() {
            let mut block = zero_block();
            self.dev.read(inode.blocks[block_index], &mut block)?;
            for slot in 0..DIR_ENTRIES_PER_BLOCK {
                let off = slot * DIR_ENTRY_SIZE;
                let ino = get_u16(&block, off);
                if ino == 0 {
                    continue;
                }
                let len = block[off + 2] as usize;
                // Vòng 83 (audit #2 P0-1): độ dài tên đến từ đĩa phải được
                // kiểm tra trước khi cắt slice, nếu không một byte hỏng là đủ
                // để panic trong VFS server.
                if len == 0 || len > MAX_NAME || off + 3 + len > BLOCK_SIZE {
                    continue;
                }
                if len == name.len() && &block[off + 3..off + 3 + len] == name {
                    LOOK_HIT[0].store(inode.size, core::sync::atomic::Ordering::Relaxed);
                    LOOK_HIT[1].store(
                        inode.blocks_used() as u32,
                        core::sync::atomic::Ordering::Relaxed,
                    );
                    LOOK_HIT[2].store(inode.blocks[0], core::sync::atomic::Ordering::Relaxed);
                    LOOK_HIT[3].store(ino as u32, core::sync::atomic::Ordering::Relaxed);
                    return Ok(Some(ino));
                }
            }
        }
        Ok(None)
    }

    fn readlink_inode(&mut self, ino: u16) -> Result<([u8; MAX_PATH], usize), FsError> {
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind != INODE_SYMLINK {
            return Err(FsError::Invalid);
        }
        let len = core::cmp::min(inode.size as usize, MAX_PATH);
        let mut target = [0u8; MAX_PATH];
        if len != 0 {
            self.read(ino, 0, &mut target[..len], 0)?;
        }
        Ok((target, len))
    }

    fn lookup_path_follow(
        &mut self,
        path: &[u8],
        uid: u16,
        follow_final: bool,
    ) -> Result<u16, FsError> {
        let _ = uid;
        if path.is_empty() || path[0] != b'/' || path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        let mut buf = [0u8; MAX_PATH];
        buf[..path.len()].copy_from_slice(path);
        let mut path_len = path.len();
        let mut pos = 0usize;
        let mut ino = ROOT_INO;
        let mut symlinks = 0usize;
        loop {
            let current = self.inodes[ino as usize - 1];
            if current.kind != INODE_DIR {
                return Err(FsError::NotDir);
            }
            while pos < path_len && buf[pos] == b'/' {
                pos += 1;
            }
            if pos >= path_len {
                return Ok(ino);
            }
            let name_start = pos;
            while pos < path_len && buf[pos] != b'/' {
                pos += 1;
            }
            let name = &buf[name_start..pos];
            if name.len() > MAX_NAME || name == b"." || name == b".." {
                return Err(if name.len() > MAX_NAME {
                    FsError::NameTooLong
                } else {
                    FsError::Invalid
                });
            }
            let parent = ino;
            let child = self.lookup_child(parent, name)?.ok_or(FsError::NotFound)?;
            let child_inode = self.inodes[child as usize - 1];
            let mut probe = pos;
            while probe < path_len && buf[probe] == b'/' {
                probe += 1;
            }
            let has_more = probe < path_len;
            if child_inode.kind == INODE_SYMLINK && (follow_final || has_more) {
                symlinks += 1;
                if symlinks > MAX_SYMLINK_DEPTH {
                    return Err(FsError::Invalid);
                }
                let (target, target_len) = self.readlink_inode(child)?;
                if target_len == 0 || target[0] != b'/' && name_start == 0 {
                    return Err(FsError::Invalid);
                }
                let suffix = &buf[pos..path_len];
                let mut next = [0u8; MAX_PATH];
                let prefix_len = if target[0] == b'/' { 0 } else { name_start };
                if prefix_len != 0 {
                    next[..prefix_len].copy_from_slice(&buf[..prefix_len]);
                }
                if prefix_len + target_len + suffix.len() > MAX_PATH {
                    return Err(FsError::Invalid);
                }
                next[prefix_len..prefix_len + target_len].copy_from_slice(&target[..target_len]);
                next[prefix_len + target_len..prefix_len + target_len + suffix.len()]
                    .copy_from_slice(suffix);
                path_len = prefix_len + target_len + suffix.len();
                buf = next;
                pos = 0;
                ino = if target[0] == b'/' { ROOT_INO } else { parent };
                continue;
            }
            ino = child;
            if !has_more {
                return Ok(ino);
            }
        }
    }

    fn lookup_path(&mut self, path: &[u8], uid: u16) -> Result<u16, FsError> {
        self.lookup_path_follow(path, uid, true)
    }

    fn resolve_parent(
        &mut self,
        path: &[u8],
        uid: u16,
    ) -> Result<(u16, [u8; MAX_NAME], usize), FsError> {
        if path.len() < 2 || path[0] != b'/' || path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        let mut pos = 0usize;
        let mut last_start = 0usize;
        let mut last_len = 0usize;
        while let Some(name) = Self::next_component(path, &mut pos)? {
            last_start = pos.saturating_sub(name.len());
            last_len = name.len();
        }
        if last_len == 0 {
            return Err(FsError::Invalid);
        }
        let parent = if last_start == 0 {
            ROOT_INO
        } else {
            self.lookup_path(&path[..last_start], uid)?
        };
        let parent_inode = self.inodes[parent as usize - 1];
        if parent_inode.kind != INODE_DIR {
            return Err(FsError::NotDir);
        }
        if !self.can_write(&parent_inode, uid) {
            return Err(FsError::NoAccess);
        }
        let mut last = [0u8; MAX_NAME];
        last[..last_len].copy_from_slice(&path[last_start..last_start + last_len]);
        Ok((parent, last, last_len))
    }

    fn write_dir_entry(&mut self, dir: u16, name: &[u8], child: u16) -> Result<(), FsError> {
        let mut inode = self.inodes[dir as usize - 1];
        for block_index in 0..inode.blocks_used() {
            let mut block = self.read_staged(inode.blocks[block_index])?;
            for slot in 0..DIR_ENTRIES_PER_BLOCK {
                let off = slot * DIR_ENTRY_SIZE;
                if get_u16(&block, off) == 0 {
                    put_u16(&mut block, off, child);
                    block[off + 2] = name.len() as u8;
                    block[off + 3..off + 3 + name.len()].copy_from_slice(name);
                    self.stage(inode.blocks[block_index], &block)?;
                    return Ok(());
                }
            }
        }
        if inode.blocks_used() >= self.dir_block_limit {
            return Err(FsError::NoSpace);
        }
        let block_no = self.alloc_block()?;
        let mut block = zero_block();
        put_u16(&mut block, 0, child);
        block[2] = name.len() as u8;
        block[3..3 + name.len()].copy_from_slice(name);
        inode.blocks[inode.blocks_used()] = block_no;
        inode.size = inode.size.saturating_add(BLOCK_SIZE as u32);
        self.inodes[dir as usize - 1] = inode;
        self.stage(block_no, &block)?;
        self.stage_inode(dir)?;
        self.stage_bitmap()?;
        Ok(())
    }

    fn remove_dir_entry(&mut self, dir: u16, name: &[u8]) -> Result<u16, FsError> {
        let inode = self.inodes[dir as usize - 1];
        for block_index in 0..inode.blocks_used() {
            let mut block = self.read_staged(inode.blocks[block_index])?;
            for slot in 0..DIR_ENTRIES_PER_BLOCK {
                let off = slot * DIR_ENTRY_SIZE;
                let ino = get_u16(&block, off);
                if ino == 0 {
                    continue;
                }
                let len = block[off + 2] as usize;
                // Vòng 83 (audit #2 P0-1): độ dài tên đến từ đĩa phải được
                // kiểm tra trước khi cắt slice, nếu không một byte hỏng là đủ
                // để panic trong VFS server.
                if len == 0 || len > MAX_NAME || off + 3 + len > BLOCK_SIZE {
                    continue;
                }
                if len == name.len() && &block[off + 3..off + 3 + len] == name {
                    put_u16(&mut block, off, 0);
                    block[off + 2] = 0;
                    for byte in block[off + 3..off + 3 + MAX_NAME].iter_mut() {
                        *byte = 0;
                    }
                    self.stage(inode.blocks[block_index], &block)?;
                    return Ok(ino);
                }
            }
        }
        Err(FsError::NotFound)
    }

    pub fn create(&mut self, path: &[u8], uid: u16, kind: u8) -> Result<u16, FsError> {
        if kind != INODE_FILE && kind != INODE_DIR {
            return Err(FsError::Invalid);
        }
        self.tx_count = 0;
        let (parent, name, name_len) = self.resolve_parent(path, uid)?;
        if self.lookup_child(parent, &name[..name_len])?.is_some() {
            return Err(FsError::Exists);
        }
        let ino = self.alloc_inode()?;
        let inode = &mut self.inodes[ino as usize - 1];
        inode.kind = kind;
        inode.uid = uid;
        inode.nlink = 1;
        inode.size = 0;
        self.stage_inode(ino)?;
        self.stage_bitmap()?;
        // 291.4 (r291): nếu bước ghi entry thư mục hỏng (thư mục đầy là ca
        // thật), các thay đổi ĐÃ STAGE (inode + bitmap) phải được ROLLBACK.
        // Bản cũ để nguyên: lần commit sau sẽ ghi inode đã cấp mà không có
        // entry ⇒ rò rỉ inode, `fsck` báo lệch.
        if let Err(error) = self.write_dir_entry(parent, &name[..name_len], ino) {
            self.release_inode(ino);
            self.tx_count = 0;
            return Err(error);
        }
        if kind == INODE_DIR {
            let parent_inode = &mut self.inodes[parent as usize - 1];
            parent_inode.nlink = parent_inode.nlink.saturating_add(1);
            self.stage_inode(parent)?;
        }
        self.commit()?;
        Ok(ino)
    }

    pub fn mkdir(&mut self, path: &[u8], uid: u16) -> Result<u16, FsError> {
        self.create(path, uid, INODE_DIR)
    }

    pub fn symlink(&mut self, target: &[u8], path: &[u8], uid: u16) -> Result<u16, FsError> {
        if target.is_empty() || target.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        if path.len() < 2 || path[0] != b'/' || path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        self.tx_count = 0;
        let (parent, name, name_len) = self.resolve_parent(path, uid)?;
        if self.lookup_child(parent, &name[..name_len])?.is_some() {
            return Err(FsError::Exists);
        }
        let ino = self.alloc_inode()?;
        let block_no = self.alloc_block()?;
        let mut block = zero_block();
        block[..target.len()].copy_from_slice(target);
        self.stage(block_no, &block)?;
        {
            let inode = &mut self.inodes[ino as usize - 1];
            inode.kind = INODE_SYMLINK;
            inode.mode = 0b0111;
            inode.uid = uid;
            inode.nlink = 1;
            inode.size = target.len() as u32;
            inode.blocks[0] = block_no;
        }
        self.stage_bitmap()?;
        self.stage_inode(ino)?;
        if let Err(error) = self.write_dir_entry(parent, &name[..name_len], ino) {
            self.tx_count = 0;
            return Err(error);
        }
        self.commit()?;
        Ok(ino)
    }

    pub fn readlink(&mut self, path: &[u8], uid: u16) -> Result<([u8; MAX_PATH], usize), FsError> {
        let (parent, name, name_len) = self.resolve_parent(path, uid)?;
        let ino = self
            .lookup_child(parent, &name[..name_len])?
            .ok_or(FsError::NotFound)?;
        self.readlink_inode(ino)
    }

    pub fn lookup(&mut self, path: &[u8], uid: u16) -> Result<u16, FsError> {
        self.lookup_path(path, uid)
    }

    /// Vòng 139: dung lượng dữ liệu còn dùng được — trả (đã dùng, tổng).
    /// Chỉ tính vùng DATA (bỏ superblock/bitmap/inode/journal) đúng như
    /// `alloc_block`, để con số khớp với khả năng cấp phát thật.
    ///
    /// O(1): `used_blocks` được duy trì bởi `mark_used`/`mark_free` và dựng
    /// lại trong `load_metadata`, nên không cần quét bitmap 8162 bit nữa.
    pub fn usage(&self) -> (u32, u32) {
        let total = self.capacity.saturating_sub(DATA_START);
        (core::cmp::min(self.used_blocks, total), total)
    }

    pub fn info(&self, ino: u16) -> Result<InodeInfo, FsError> {
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind == INODE_FREE {
            return Err(FsError::NotFound);
        }
        Ok(InodeInfo {
            kind: inode.kind,
            mode: inode.mode,
            uid: inode.uid,
            nlink: inode.nlink,
            size: inode.size,
        })
    }

    pub fn read(
        &mut self,
        ino: u16,
        offset: usize,
        out: &mut [u8],
        uid: u16,
    ) -> Result<usize, FsError> {
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind == INODE_FREE {
            return Err(FsError::BadFd);
        }
        if inode.kind == INODE_DIR {
            return Err(FsError::IsDir);
        }
        if !self.can_read(&inode, uid) {
            return Err(FsError::NoAccess);
        }
        if offset >= inode.size as usize {
            return Ok(0);
        }
        let count = core::cmp::min(out.len(), inode.size as usize - offset);
        let mut done = 0usize;
        while done < count {
            let block_index = (offset + done) / BLOCK_SIZE;
            let in_block = (offset + done) % BLOCK_SIZE;
            let block_no = self.file_block(&inode, block_index)?;
            let chunk = core::cmp::min(BLOCK_SIZE - in_block, count - done);
            // 291.3 (r291): block 0 trong phạm vi `size` là LỖ của file thưa
            // (ghi quá EOF tạo lỗ) — đọc ra 0, KHÔNG phải Corrupt. Bản cũ trả
            // Corrupt nên ghi ở offset 1 MiB làm cả file không đọc được.
            if block_no == 0 {
                out[done..done + chunk].fill(0);
                done += chunk;
                continue;
            }
            let mut block = zero_block();
            self.dev.read(block_no, &mut block)?;
            out[done..done + chunk].copy_from_slice(&block[in_block..in_block + chunk]);
            done += chunk;
        }
        Ok(count)
    }

    pub fn write(
        &mut self,
        ino: u16,
        offset: usize,
        data: &[u8],
        uid: u16,
    ) -> Result<usize, FsError> {
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind == INODE_FREE {
            return Err(FsError::BadFd);
        }
        if inode.kind == INODE_DIR {
            return Err(FsError::IsDir);
        }
        if !self.can_write(&inode, uid) {
            return Err(FsError::NoAccess);
        }
        let max_file_size = self.max_file_size;
        let end = offset.checked_add(data.len()).ok_or(FsError::Invalid)?;
        if end > max_file_size {
            return Err(FsError::NoSpace);
        }
        let mut done = 0usize;
        // TẠM REVERT batch (#8) để bisect x86 EIO: quay về 1 commit mỗi
        // 512B chunk như trước, giữ nguyên mọi cải tiến khác (CRC bảng,
        // cache 32, counter, cursor, fsync barrier).
        let staged: usize = 0;
        while done < data.len() {
            self.tx_count = 0;
            let mut current = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
            let absolute = offset + done;
            let block_index = absolute / BLOCK_SIZE;
            if block_index >= self.max_file_blocks {
                if staged > 0 {
                    self.commit()?;
                }
                return Err(FsError::NoSpace);
            }
            let in_block = absolute % BLOCK_SIZE;
            let chunk = core::cmp::min(BLOCK_SIZE - in_block, data.len() - done);
            let mut block_no = self.file_block(&current, block_index)?;
            let mut block = if block_no != 0 {
                let mut block = zero_block();
                self.dev.read(block_no, &mut block)?;
                block
            } else {
                block_no = self.alloc_block()?;
                self.set_file_block(&mut current, block_index, block_no)?;
                self.stage_bitmap()?;
                zero_block()
            };
            block[in_block..in_block + chunk].copy_from_slice(&data[done..done + chunk]);
            // TẠM REVERT: stage thẳng + commit ngay (bản cũ).
            self.stage(block_no, &block)?;
            current.size = core::cmp::max(current.size, (absolute + chunk) as u32);
            self.inodes[ino as usize - 1] = current;
            self.stage_inode(ino)?;
            self.commit()?;
            let _ = staged;
            done += chunk;
        }
        Ok(data.len())
    }

    fn free_double_tables(&mut self, inode: &mut Inode) -> Result<(), FsError> {
        if self.version < 4 {
            return Ok(());
        }
        let double_index = self.direct_blocks + 1;
        let double = inode.blocks[double_index];
        if double == 0 {
            return Ok(());
        }
        let table = self.read_staged(double)?;
        for slot in 0..INDIRECT_ENTRIES {
            let second = get_u32(&table, slot * 4);
            if second != 0 {
                self.mark_free(second);
            }
        }
        self.mark_free(double);
        inode.blocks[double_index] = 0;
        Ok(())
    }

    /// Drop indirect-table entries and table blocks that are beyond the new
    /// file length.  Data blocks are freed by `truncate` before this runs.
    fn trim_indirect(&mut self, inode: &mut Inode, new_blocks: usize) -> Result<(), FsError> {
        let direct = self.direct_blocks;
        let single = inode.blocks[direct];
        if self.version < 4 {
            if new_blocks <= direct {
                if single != 0 {
                    self.mark_free(single);
                    inode.blocks[direct] = 0;
                }
            } else if single != 0 {
                let mut table = self.read_staged(single)?;
                for slot in (new_blocks - direct)..INDIRECT_ENTRIES {
                    put_u32(&mut table, slot * 4, 0);
                }
                self.stage(single, &table)?;
            }
            return Ok(());
        }

        if new_blocks <= direct {
            if single != 0 {
                self.mark_free(single);
                inode.blocks[direct] = 0;
            }
            return self.free_double_tables(inode);
        }

        if single != 0 {
            let mut table = self.read_staged(single)?;
            let keep = core::cmp::min(new_blocks - direct, INDIRECT_ENTRIES);
            for slot in keep..INDIRECT_ENTRIES {
                put_u32(&mut table, slot * 4, 0);
            }
            self.stage(single, &table)?;
        }
        if new_blocks <= direct + INDIRECT_ENTRIES {
            return self.free_double_tables(inode);
        }

        let double_index = direct + 1;
        let double = inode.blocks[double_index];
        if double == 0 {
            return Ok(());
        }
        let double_used = new_blocks - direct - INDIRECT_ENTRIES;
        let keep_slots = (double_used + INDIRECT_ENTRIES - 1) / INDIRECT_ENTRIES;
        let mut table = self.read_staged(double)?;
        for slot in 0..keep_slots {
            let second = get_u32(&table, slot * 4);
            if second == 0 {
                continue;
            }
            let mut second_table = self.read_staged(second)?;
            let keep_entries = if slot + 1 == keep_slots {
                double_used - slot * INDIRECT_ENTRIES
            } else {
                INDIRECT_ENTRIES
            };
            for entry in keep_entries..INDIRECT_ENTRIES {
                put_u32(&mut second_table, entry * 4, 0);
            }
            self.stage(second, &second_table)?;
        }
        for slot in keep_slots..INDIRECT_ENTRIES {
            let second = get_u32(&table, slot * 4);
            if second != 0 {
                self.mark_free(second);
                put_u32(&mut table, slot * 4, 0);
            }
        }
        self.stage(double, &table)
    }

    pub fn truncate(&mut self, ino: u16, size: usize, uid: u16) -> Result<(), FsError> {
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind == INODE_FILE && !self.can_write(&inode, uid) {
            return Err(FsError::NoAccess);
        }
        if size > self.max_file_size {
            return Err(FsError::NoSpace);
        }
        let mut current = inode;
        let old_size = current.size as usize;
        let old_blocks = current.blocks_used();
        let new_blocks = (size + BLOCK_SIZE - 1) / BLOCK_SIZE;
        if new_blocks > self.max_file_blocks {
            return Err(FsError::NoSpace);
        }
        self.tx_count = 0;
        if size < old_size && old_size % BLOCK_SIZE != 0 {
            let block_index = size / BLOCK_SIZE;
            let block_no = self.file_block(&current, block_index)?;
            if block_no != 0 {
                let mut block = zero_block();
                self.dev.read(block_no, &mut block)?;
                for byte in block[size % BLOCK_SIZE..].iter_mut() {
                    *byte = 0;
                }
                self.stage(block_no, &block)?;
            }
        } else if size > old_size && old_size % BLOCK_SIZE != 0 {
            let block_index = old_size / BLOCK_SIZE;
            let block_no = self.file_block(&current, block_index)?;
            if block_no != 0 {
                let mut block = zero_block();
                self.dev.read(block_no, &mut block)?;
                for byte in block[old_size % BLOCK_SIZE..].iter_mut() {
                    *byte = 0;
                }
                self.stage(block_no, &block)?;
            }
        }
        if new_blocks < old_blocks {
            for index in new_blocks..old_blocks {
                let block = self.file_block(&current, index)?;
                if block != 0 {
                    self.mark_free(block);
                }
                if index < self.direct_blocks {
                    current.blocks[index] = 0;
                }
            }
            self.trim_indirect(&mut current, new_blocks)?;
            current.size = size as u32;
            self.inodes[ino as usize - 1] = current;
            self.stage_inode(ino)?;
            self.stage_bitmap()?;
            return self.commit();
        }
        if new_blocks > old_blocks {
            for index in old_blocks..new_blocks {
                self.tx_count = 0;
                let mut current = self.inodes[ino as usize - 1];
                let block = self.alloc_block()?;
                self.set_file_block(&mut current, index, block)?;
                self.stage(block, &zero_block())?;
                self.stage_bitmap()?;
                current.size = core::cmp::max(current.size, ((index + 1) * BLOCK_SIZE) as u32);
                self.inodes[ino as usize - 1] = current;
                self.stage_inode(ino)?;
                self.commit()?;
            }
        }
        let mut current = self.inodes[ino as usize - 1];
        current.size = size as u32;
        self.inodes[ino as usize - 1] = current;
        self.stage_inode(ino)?;
        self.stage_bitmap()?;
        self.commit()
    }

    /// r296: chuyển tiếp số liệu eviction của thiết bị (chỉ để chẩn đoán).
    pub fn evict_stats(&self) -> (u64, u64) {
        self.dev.evict_stats()
    }

    /// r296 (DIRSNAP): chụp lại ĐÚNG LÚC readdir trả "hết" — ba đường đọc
    /// (cache, RAW bỏ cache, ảnh journal đang chờ) trên cùng một block, cộng
    /// `isize` của inode và kết quả `lookup` ngay tại chỗ.
    ///
    /// Vì sao cần: `SLO`/`LOOK` là đèn "lần gần nhất" **không có `seq`**, nên
    /// không được dùng để khẳng định hai lần đọc cùng thời điểm. Đèn này có
    /// `seq` (server in kèm) và đọc cả ba đường trong CÙNG một lời gọi.
    pub fn dir_snapshot(&mut self, dir: u16, index: usize, probe: &[u8]) -> Option<DirSnap> {
        let inode = *self.inodes.get(dir as usize - 1)?;
        if inode.kind != INODE_DIR {
            return None;
        }
        let mut snap = DirSnap {
            blocks: inode.blocks_used() as u32,
            blk0: inode.blocks.first().copied().unwrap_or(0),
            isize: inode.size,
            index: index as u32,
            cached: false,
            dirty: false,
            n_cache: 0,
            n_raw: 0,
            n_jrnl: 0,
            lookup: 0,
            lookup_ino: 0,
            tx: self.tx_count as u32,
            names: [[0u8; MAX_NAME]; DIR_ENTRIES_PER_BLOCK],
            name_len: [0u8; DIR_ENTRIES_PER_BLOCK],
            inos: [0u16; DIR_ENTRIES_PER_BLOCK],
        };
        for block_index in 0..inode.blocks_used() {
            let lba = inode.blocks[block_index];
            if block_index == 0 {
                let (cached, dirty) = self.dev.slot_state(lba);
                snap.cached = cached;
                snap.dirty = dirty;
            }
            let mut block = zero_block();
            if self.dev.read(lba, &mut block).is_ok() {
                snap.n_cache += count_dir_entries(&block);
                if block_index == 0 {
                    for slot in 0..DIR_ENTRIES_PER_BLOCK {
                        let off = slot * DIR_ENTRY_SIZE;
                        let ino = get_u16(&block, off);
                        let len = block[off + 2] as usize;
                        snap.inos[slot] = ino;
                        if ino == 0 || len == 0 || len > MAX_NAME {
                            continue;
                        }
                        snap.name_len[slot] = len as u8;
                        snap.names[slot][..len].copy_from_slice(&block[off + 3..off + 3 + len]);
                    }
                }
            }
            let mut raw = zero_block();
            if self.dev.read_uncached(lba, &mut raw).is_ok() {
                snap.n_raw += count_dir_entries(&raw);
            }
            // Ảnh "nếu áp journal đang chờ": block có đang nằm trong vùng
            // staging (`self.journal` + `self.targets`) hay không.
            let mut journal_hit = false;
            for entry in 0..self.tx_count {
                if self.targets[entry] == lba {
                    let start = entry * BLOCK_SIZE;
                    let mut staged = zero_block();
                    staged.copy_from_slice(&self.journal[start..start + BLOCK_SIZE]);
                    snap.n_jrnl += count_dir_entries(&staged);
                    journal_hit = true;
                }
            }
            if !journal_hit {
                snap.n_jrnl += count_dir_entries(&block);
            }
        }
        // `lookup` cùng tên ngay tại đây, để so trực tiếp với đường liệt kê.
        snap.lookup = match self.lookup_child(dir, probe) {
            Ok(Some(ino)) => {
                snap.lookup_ino = ino;
                1
            }
            Ok(None) => 0,
            Err(_) => 2,
        };
        Some(snap)
    }

    pub fn read_dir_entry(
        &mut self,
        dir: u16,
        index: usize,
    ) -> Result<Option<(u16, [u8; MAX_NAME], usize)>, FsError> {
        Ok(self
            .read_dir_entry_full(dir, index)?
            .map(|(ino, name, len, _)| (ino, name, len)))
    }

    /// Directory entry with the child kind inline.  Powers the
    /// `READDIR_BULK` path (#10): the server packs `(ino, kind, len, name)`
    /// records without an extra `STAT_PATH` IPC per entry, so `ls` no longer
    /// pays N stats for N entries.
    pub fn read_dir_entry_full(
        &mut self,
        dir: u16,
        index: usize,
    ) -> Result<Option<(u16, [u8; MAX_NAME], usize, u8)>, FsError> {
        let inode = *self.inodes.get(dir as usize - 1).ok_or(FsError::BadFd)?;
        if inode.kind != INODE_DIR {
            return Err(FsError::NotDir);
        }
        let mut seen = 0usize;
        for block_index in 0..inode.blocks_used() {
            let mut block = zero_block();
            self.dev.read(inode.blocks[block_index], &mut block)?;
            for slot in 0..DIR_ENTRIES_PER_BLOCK {
                let off = slot * DIR_ENTRY_SIZE;
                let ino = get_u16(&block, off);
                if ino == 0 {
                    continue;
                }
                // Vòng 88 (audit #4): entry hỏng vẫn chiếm một index. Trước
                // đây `continue` không tăng `seen` nên entry hỏng ở index N
                // khiến entry hợp lệ ngay sau nó được trả cho cả N và N+1
                // ([A,corrupt,B,C] -> A,B,B,C).
                let len = block[off + 2] as usize;
                let corrupt = len == 0 || len > MAX_NAME || off + 3 + len > BLOCK_SIZE;
                if seen == index {
                    if corrupt {
                        return Err(FsError::Corrupt);
                    }
                    let mut name = [0u8; MAX_NAME];
                    name[..len].copy_from_slice(&block[off + 3..off + 3 + len]);
                    let kind = self
                        .inodes
                        .get(ino as usize - 1)
                        .map(|child| child.kind)
                        .unwrap_or(INODE_FILE);
                    return Ok(Some((ino, name, len, kind)));
                }
                seen += 1;
            }
        }
        // T26: tới đây là "hết" — đo lại bằng đường RAW (bỏ cache) để biết
        // ảnh nào đúng. Lệch ⇒ bug cache/inode, không phải hết thật.
        let mut raw_seen = 0u32;
        for block_index in 0..inode.blocks_used() {
            let mut raw = zero_block();
            if self.dev.read(inode.blocks[block_index], &mut raw).is_ok() {
                for slot in 0..DIR_ENTRIES_PER_BLOCK {
                    if get_u16(&raw, slot * DIR_ENTRY_SIZE) != 0 {
                        raw_seen += 1;
                    }
                }
            }
        }
        // Hình dạng thư mục tại đúng thời điểm "hết" (T26): size, blocks_used,
        // và số entry không rỗng của từng block.
        let mut per_block = [0u32; 2];
        for block_index in 0..inode.blocks_used().min(2) {
            let mut raw = zero_block();
            if self.dev.read(inode.blocks[block_index], &mut raw).is_ok() {
                for slot in 0..DIR_ENTRIES_PER_BLOCK {
                    if get_u16(&raw, slot * DIR_ENTRY_SIZE) != 0 {
                        per_block[block_index] += 1;
                    }
                }
            }
        }
        // T26: so ảnh CACHE với ảnh ĐĨA THẬT (bỏ hẳn cache) của block 0.
        // Đèn `DIR` trước đây dùng `dev.read` cho cả hai phía nên không chứng
        // minh được gì (subagent chỉ ra). Nay một phía đi `read_uncached`.
        let mut disk_mismatch = 0u32;
        if inode.blocks_used() > 0 {
            let mut cached = zero_block();
            let mut on_disk = zero_block();
            let cached_ok = self.dev.read(inode.blocks[0], &mut cached).is_ok();
            let disk_ok = self
                .dev
                .read_uncached(inode.blocks[0], &mut on_disk)
                .is_ok();
            if cached_ok && disk_ok && cached != on_disk {
                disk_mismatch = 1;
            }
        }
        DIR_SHAPE[3].store(disk_mismatch, core::sync::atomic::Ordering::Relaxed);
        // Ghi 8 ino đầu của block 0 để so với những gì `lookup_child` đã thấy.
        if inode.blocks_used() > 0 {
            let mut probe = zero_block();
            if self.dev.read(inode.blocks[0], &mut probe).is_ok() {
                for slot in 0..8 {
                    SLOT_INOS[slot].store(
                        get_u16(&probe, slot * DIR_ENTRY_SIZE) as u32,
                        core::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
        }
        DIR_SHAPE[0].store(inode.size, core::sync::atomic::Ordering::Relaxed);
        DIR_SHAPE[1].store(
            inode.blocks_used() as u32,
            core::sync::atomic::Ordering::Relaxed,
        );
        DIR_SHAPE[2].store(per_block[0], core::sync::atomic::Ordering::Relaxed);
        DIR_SHAPE[3].store(per_block[1], core::sync::atomic::Ordering::Relaxed);
        DIR_PROBE[0].store(index as u32, core::sync::atomic::Ordering::Relaxed);
        DIR_PROBE[1].store(seen as u32, core::sync::atomic::Ordering::Relaxed);
        DIR_PROBE[2].store(raw_seen, core::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }

    fn dir_empty(&mut self, dir: u16) -> Result<bool, FsError> {
        let inode = self.inodes[dir as usize - 1];
        for block_index in 0..inode.blocks_used() {
            let mut block = zero_block();
            self.dev.read(inode.blocks[block_index], &mut block)?;
            for slot in 0..DIR_ENTRIES_PER_BLOCK {
                if get_u16(&block, slot * DIR_ENTRY_SIZE) != 0 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Create a second directory entry for an existing regular file.  The
    /// inode link count is journaled together with the new directory entry.
    pub fn link(&mut self, target: &[u8], new_path: &[u8], uid: u16) -> Result<(), FsError> {
        if target.len() > MAX_PATH || new_path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        self.tx_count = 0;
        let target_ino = self.lookup_path(target, uid)?;
        let target_inode = self.inodes[target_ino as usize - 1];
        if target_inode.kind == INODE_DIR {
            return Err(FsError::IsDir);
        }
        if target_inode.kind == INODE_FREE {
            return Err(FsError::NotFound);
        }
        if target_inode.nlink == u16::MAX {
            return Err(FsError::NoSpace);
        }
        let (parent, name, name_len) = self.resolve_parent(new_path, uid)?;
        if self.lookup_child(parent, &name[..name_len])?.is_some() {
            return Err(FsError::Exists);
        }
        let old_nlink = target_inode.nlink;
        {
            let inode = &mut self.inodes[target_ino as usize - 1];
            inode.nlink = inode.nlink.saturating_add(1);
        }
        self.stage_inode(target_ino)?;
        if let Err(error) = self.write_dir_entry(parent, &name[..name_len], target_ino) {
            let inode = &mut self.inodes[target_ino as usize - 1];
            inode.nlink = old_nlink;
            self.tx_count = 0;
            return Err(error);
        }
        self.commit()
    }

    pub fn unlink(&mut self, path: &[u8], uid: u16) -> Result<(), FsError> {
        self.tx_count = 0;
        let (parent, name, name_len) = self.resolve_parent(path, uid)?;
        let ino = self
            .lookup_child(parent, &name[..name_len])?
            .ok_or(FsError::NotFound)?;
        let inode = self.inodes[ino as usize - 1];
        if inode.kind == INODE_DIR && !self.dir_empty(ino)? {
            return Err(FsError::NotEmpty);
        }
        self.remove_dir_entry(parent, &name[..name_len])?;
        if inode.kind == INODE_DIR {
            let parent_inode = &mut self.inodes[parent as usize - 1];
            parent_inode.nlink = parent_inode.nlink.saturating_sub(1);
            self.stage_inode(parent)?;
        }
        let mut inode = self.inodes[ino as usize - 1];
        inode.nlink = inode.nlink.saturating_sub(1);
        if inode.nlink == 0 {
            self.free_inode_blocks(&mut inode);
            inode = Inode::default();
            self.inodes[ino as usize - 1] = inode;
            self.stage_inode(ino)?;
            self.stage_bitmap()?;
        } else {
            self.inodes[ino as usize - 1] = inode;
            self.stage_inode(ino)?;
        }
        self.commit()
    }

    pub fn rename(&mut self, old: &[u8], new: &[u8], uid: u16) -> Result<(), FsError> {
        if old.len() > MAX_PATH || new.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        if new.len() > old.len() && &new[..old.len()] == old && new[old.len()] == b'/' {
            return Err(FsError::Invalid);
        }
        self.tx_count = 0;
        let (old_parent, old_name, old_len) = self.resolve_parent(old, uid)?;
        let (new_parent, new_name, new_len) = self.resolve_parent(new, uid)?;
        let ino = self
            .lookup_child(old_parent, &old_name[..old_len])?
            .ok_or(FsError::NotFound)?;
        if self
            .lookup_child(new_parent, &new_name[..new_len])?
            .is_some()
        {
            return Err(FsError::Exists);
        }
        let inode = self.inodes[ino as usize - 1];
        self.remove_dir_entry(old_parent, &old_name[..old_len])?;
        self.write_dir_entry(new_parent, &new_name[..new_len], ino)?;
        if inode.kind == INODE_DIR && old_parent != new_parent {
            let old_parent_inode = &mut self.inodes[old_parent as usize - 1];
            old_parent_inode.nlink = old_parent_inode.nlink.saturating_sub(1);
            self.stage_inode(old_parent)?;
            let new_parent_inode = &mut self.inodes[new_parent as usize - 1];
            new_parent_inode.nlink = new_parent_inode.nlink.saturating_add(1);
            self.stage_inode(new_parent)?;
        }
        self.commit()
    }

    /// Flush all staged-but-uncommitted transaction data, then the device.
    /// (#11) This is the real durability barrier: `write()` batches staged
    /// blocks and may return with a partial transaction still open, so
    /// `fsync` must commit before flushing.  Callers that need POSIX-like
    /// "fsync makes it durable" semantics should use this.
    pub fn fsync(&mut self) -> Result<(), FsError> {
        if self.tx_count > 0 {
            self.commit()?;
        }
        self.dev.flush()
    }

    /// Queue one complete write transaction without applying it.  This is a
    /// test/host-tool hook used to create a valid or torn journal image.
    pub fn inject_pending_write(
        &mut self,
        path: &[u8],
        data: &[u8],
        uid: u16,
        torn: bool,
    ) -> Result<(), FsError> {
        let ino = match self.lookup(path, uid) {
            Ok(ino) => ino,
            Err(FsError::NotFound) => self.create(path, uid, INODE_FILE)?,
            Err(error) => return Err(error),
        };
        // Build the transaction but write only journal/commit blocks, leaving
        // target blocks untouched so mount must replay.
        self.tx_count = 0;
        let inode = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
        let mut current = inode;
        let end = data.len().min(MAX_FILE_SIZE);
        let first = 0usize;
        let last = end.saturating_sub(1) / BLOCK_SIZE;
        for block_index in first..=last {
            let mut block = if current.blocks[block_index] != 0 {
                let mut block = zero_block();
                self.dev.read(current.blocks[block_index], &mut block)?;
                block
            } else {
                let block_no = self.alloc_block()?;
                current.blocks[block_index] = block_no;
                zero_block()
            };
            let start = block_index * BLOCK_SIZE;
            let chunk = core::cmp::min(BLOCK_SIZE, end - start);
            block[..chunk].copy_from_slice(&data[start..start + chunk]);
            self.stage(current.blocks[block_index], &block)?;
        }
        current.size = end as u32;
        self.inodes[ino as usize - 1] = current;
        self.stage_inode(ino)?;
        self.stage_bitmap()?;
        self.write_journal_only(torn)
    }

    fn write_journal_only(&mut self, torn: bool) -> Result<(), FsError> {
        let count = self.tx_count;
        for index in 0..count {
            let start = index * BLOCK_SIZE;
            let mut data = zero_block();
            data.copy_from_slice(&self.journal[start..start + BLOCK_SIZE]);
            self.dev.write(JOURNAL_DATA_START + index as u32, &data)?;
        }
        let mut header = zero_block();
        header[..4].copy_from_slice(b"VJ01");
        put_u32(&mut header, 4, self.seq);
        header[8] = count as u8;
        for index in 0..count {
            put_u32(&mut header, 16 + index * 4, self.targets[index]);
        }
        let mut crc = crc32(&header[..80]);
        for index in 0..count {
            let start = index * BLOCK_SIZE;
            crc = crc32_continue(crc, &self.journal[start..start + BLOCK_SIZE]);
        }
        put_u32(&mut header, 80, crc);
        self.dev.write(JOURNAL_START, &header)?;
        let mut commit = zero_block();
        commit[..4].copy_from_slice(b"VC01");
        put_u32(&mut commit, 4, self.seq);
        put_u32(&mut commit, 8, if torn { crc ^ 0xdead_beef } else { crc });
        self.dev.write(JOURNAL_COMMIT_LBA, &commit)?;
        self.tx_count = 0;
        self.seq = self.seq.wrapping_add(1);
        Ok(())
    }
}

fn crc32_continue(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for byte in data {
        let index = ((crc ^ (*byte as u32)) & 0xff) as usize;
        crc = CRC32_TABLE[index] ^ (crc >> 8);
    }
    !crc
}

/// Reference bitwise CRC-32 kept only as a test oracle: it recomputes the
/// digest one bit at a time so the table-driven `crc32()`/`crc32_continue()`
/// above can be cross-checked against an independent implementation.
#[cfg(test)]
fn crc32_bitwise(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Reference bitwise CRC-32 continuation, mirroring `crc32_continue()`
/// without the lookup table.
#[cfg(test)]
fn crc32_continue_bitwise(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

impl Inode {
    pub fn blocks_used(&self) -> usize {
        if self.kind == INODE_FREE {
            return 0;
        }
        // Vòng 83 (audit #2 P0-2): size trên đĩa có thể bị hỏng (ví dụ dir
        // size > 6144). Nếu không kẹp theo số slot thật của `blocks`, các
        // vòng lặp bên dưới sẽ index `[u32; MAX_BLOCKS_PER_INODE]` quá biên
        // và panic trong VFS server (mọi IPC sau đó treo).
        let blocks = (self.size as usize).div_ceil(BLOCK_SIZE);
        if self.kind == INODE_DIR {
            // Chỉ thư mục mới index trực tiếp `blocks[]`; file lớn dùng
            // `file_block()` nên có thể vượt 12 slot một cách hợp lệ.
            return core::cmp::min(blocks, MAX_BLOCKS_PER_INODE);
        }
        blocks
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    #[derive(Clone)]
    struct MemDev {
        data: Vec<u8>,
    }
    impl MemDev {
        fn new() -> Self {
            Self {
                data: vec![0u8; TOTAL_BLOCKS as usize * BLOCK_SIZE],
            }
        }
        fn with_blocks(blocks: u32) -> Self {
            Self {
                data: vec![0u8; blocks as usize * BLOCK_SIZE],
            }
        }
    }
    impl BlockDev for MemDev {
        fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
            let start = lba as usize * BLOCK_SIZE;
            out.copy_from_slice(&self.data[start..start + BLOCK_SIZE]);
            Ok(())
        }
        fn write(&mut self, lba: u32, input: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
            let start = lba as usize * BLOCK_SIZE;
            self.data[start..start + BLOCK_SIZE].copy_from_slice(input);
            Ok(())
        }
        fn block_count(&self) -> u32 {
            (self.data.len() / BLOCK_SIZE) as u32
        }
    }

    struct CountingDev {
        inner: MemDev,
        reads: usize,
        writes: usize,
        flushes: usize,
    }
    impl CountingDev {
        fn new() -> Self {
            Self {
                inner: MemDev::new(),
                reads: 0,
                writes: 0,
                flushes: 0,
            }
        }
    }
    impl BlockDev for CountingDev {
        fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
            self.reads += 1;
            self.inner.read(lba, out)
        }
        fn write(&mut self, lba: u32, input: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
            self.writes += 1;
            self.inner.write(lba, input)
        }
        fn block_count(&self) -> u32 {
            self.inner.block_count()
        }
        fn flush(&mut self) -> Result<(), FsError> {
            self.flushes += 1;
            Ok(())
        }
    }

    struct CrashDev {
        data: Vec<u8>,
        writes: usize,
        fail_at: usize,
        failed: bool,
    }
    impl BlockDev for CrashDev {
        fn read(&mut self, lba: u32, out: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
            let start = lba as usize * BLOCK_SIZE;
            out.copy_from_slice(&self.data[start..start + BLOCK_SIZE]);
            Ok(())
        }
        fn write(&mut self, lba: u32, input: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
            if self.writes == self.fail_at {
                self.failed = true;
                return Err(FsError::Io);
            }
            self.writes += 1;
            let start = lba as usize * BLOCK_SIZE;
            self.data[start..start + BLOCK_SIZE].copy_from_slice(input);
            Ok(())
        }
        fn block_count(&self) -> u32 {
            (self.data.len() / BLOCK_SIZE) as u32
        }
    }

    fn fresh() -> Fs<MemDev> {
        let mut dev = MemDev::new();
        format(&mut dev).unwrap();
        Fs::mount(dev).unwrap()
    }
    fn reopen(fs: &Fs<MemDev>) -> Fs<MemDev> {
        Fs::mount(fs.dev.clone()).unwrap()
    }

    /// P1-2 (r290, đỏ-trước): chỉ số block bịa trong inode phải trả `Corrupt`,
    /// không được panic. Bản cũ tính chỉ số từ `inode.size` (dữ liệu trên đĩa)
    /// rồi đọc thẳng bảng indirect ⇒ `index out of bounds`.
    #[test]
    fn read_with_a_forged_size_returns_corrupt_instead_of_panicking() {
        let mut fs = fresh();
        let ino = fs.create(b"/forged", 0, INODE_FILE).unwrap();
        // Đủ block để inode có bảng indirect thật (vượt vùng direct).
        let direct = fs.direct_blocks;
        let payload = vec![0xa5u8; BLOCK_SIZE * (direct + 2)];
        fs.write(ino, 0, &payload, 0).unwrap();
        assert_ne!(
            fs.inodes[ino as usize - 1].blocks[direct],
            0,
            "phải có indirect"
        );
        // Bịa: size khổng lồ + con trỏ double-indirect rác (đĩa hỏng/độc).
        fs.inodes[ino as usize - 1].size = u32::MAX;
        fs.inodes[ino as usize - 1].blocks[direct + 1] = 7;
        let mut out = [0u8; 32];
        let err = fs.read(ino, 0x3fff_f000, &mut out, 0);
        assert!(
            matches!(err, Err(FsError::Corrupt) | Err(FsError::Invalid)),
            "phải trả lỗi, không panic; nhận {:?}",
            err
        );
    }

    /// 291.3 (r291, đỏ-trước): ghi quá EOF tạo LỖ; đọc phần lỗ phải ra 0 và
    /// cả file vẫn đọc được. Bản cũ: `read` thấy block 0 ⇒ `Corrupt` ⇒ ghi xa
    /// EOF làm hỏng toàn bộ file.
    #[test]
    fn writing_past_eof_leaves_a_readable_zero_filled_hole() {
        let mut fs = fresh();
        let ino = fs.create(b"/sparse", 0, INODE_FILE).unwrap();
        let payload = [0x7au8; 16];
        // 64 KiB = block 128 ⇒ nằm trong vùng single-indirect.
        let offset = 64 * 1024;
        assert_eq!(fs.write(ino, offset, &payload, 0).unwrap(), payload.len());

        let mut out = [0u8; 16];
        assert_eq!(fs.read(ino, offset, &mut out, 0).unwrap(), 16);
        assert_eq!(out, payload, "phần đã ghi phải đúng");

        let mut hole = [0xffu8; 64];
        assert_eq!(fs.read(ino, 0, &mut hole, 0).unwrap(), 64);
        assert!(hole.iter().all(|b| *b == 0), "phần lỗ phải đọc ra 0");
        fs.fsck().unwrap();
    }

    /// 291.4 (r291, đỏ-trước): `create` hỏng giữa đường (thư mục đầy) phải
    /// ROLLBACK inode đã cấp — nếu không, lần commit sau ghi một inode mồ côi
    /// (không entry nào trỏ tới) và bộ đếm inode của FS bị rò rỉ.
    #[test]
    fn failed_create_rolls_back_the_allocated_inode() {
        // Thư mục gốc chỉ 1 block (12 entry) nhưng 64 inode ⇒ thư mục ĐẦY
        // trước khi cạn inode, đúng nhánh `write_dir_entry` hỏng.
        let mut dev = MemDev::new();
        format_with_options(
            &mut dev,
            FormatOptions {
                total_blocks: 0,
                inode_limit: MAX_INODES as u16,
                dir_block_limit: 1,
            },
        )
        .unwrap();
        let mut fs = Fs::mount(dev).unwrap();
        let alive = |fs: &Fs<MemDev>| {
            fs.inodes
                .iter()
                .filter(|inode| inode.kind != INODE_FREE)
                .count()
        };
        let baseline = alive(&fs);
        let mut created = 0usize;
        let mut failure = None;
        let mut index = 0usize;
        while index < 400 {
            // Tên DÀI để thư mục đầy trước khi cạn inode (mỗi entry ~100 B).
            let name = std::format!("/{}{index:02}", "n".repeat(24));
            match fs.create(name.as_bytes(), 0, INODE_FILE) {
                Ok(_) => created += 1,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
            index += 1;
        }
        let error = failure.expect("phải chạm giới hạn thư mục");
        assert!(
            matches!(error, FsError::NoSpace),
            "lỗi phải là NoSpace: {error:?}"
        );
        assert!(created > 0);
        assert_eq!(
            alive(&fs),
            baseline + created,
            "create hỏng không được để lại inode mồ côi (rò rỉ)"
        );
        // Và FS vẫn lành: một create mới thành công rồi fsck sạch.
        fs.fsck().unwrap();
    }

    /// P1-1 (r290, đỏ-trước): đĩa LỚN HƠN vùng đã format phải mount được.
    ///
    /// Kịch bản thật (bản soát 2026-11-10): `dd` 4 MiB → `mkfs` → `truncate -s
    /// 8M` → mount. Bản cũ lấy `dev.block_count()` làm biên nên quét bitmap
    /// quá vùng FS và panic `index out of bounds` (8 MiB = 16 384 block, bitmap
    /// chỉ phủ 8 192 block).
    #[test]
    fn mount_accepts_a_device_larger_than_the_formatted_area() {
        let mut small = MemDev::with_blocks(4_096);
        format(&mut small).unwrap();
        // "truncate -s 8M": cùng nội dung, thiết bị dài gấp bốn.
        let mut data = small.data;
        data.resize(16_384 * BLOCK_SIZE, 0);
        let big = MemDev { data };

        let mut fs = Fs::mount(big).expect("mount phải chấp nhận đĩa lớn hơn");
        // Biên cấp phát là vùng đã format (4 096), không phải 16 384.
        let (_, total) = fs.usage();
        assert_eq!(total, 4_096 - DATA_START);
        // Và FS vẫn dùng được bình thường.
        let ino = fs.create(b"/big", 0, INODE_FILE).unwrap();
        fs.write(ino, 0, &[0x5au8; BLOCK_SIZE], 0).unwrap();
        fs.fsck().unwrap();
    }

    #[test]
    fn fsck_detects_corrupt_bitmap_and_dangling_dir_entry() {
        // Vòng 217: `fsck()` được gọi ~42 lần trong test nhưng CHƯA BAO GIỜ được
        // chứng minh là BẮT ĐƯỢC hỏng — giống hệt `check_invariants` ở vòng 189.
        // Lưu ý: fsck kiểm theo `self.bitmap` trong bộ nhớ, nên mutation phải
        // sửa bitmap TRONG BỘ NHỚ, không phải block trên dev.
        let mut fs = fresh();
        let ino = fs.create(b"/keep", 0, INODE_FILE).unwrap();
        fs.write(ino, 0, &[0x11u8; BLOCK_SIZE], 0).unwrap();
        fs.fsck().unwrap();

        // (a) Bitmap nói block của file là FREE trong khi inode vẫn trỏ tới nó.
        let block = fs.inodes[ino as usize - 1].blocks[0];
        assert_ne!(block, 0);
        let byte = block as usize / 8;
        let bit = 1u8 << (block % 8);
        fs.bitmap[byte] &= !bit;
        assert_eq!(
            fs.fsck(),
            Err(FsError::Corrupt),
            "bitmap thiếu block phải bị bắt"
        );

        // Khôi phục rồi phá kiểu khác: entry trỏ tới inode đã free.
        fs.bitmap[byte] |= bit;
        fs.fsck().unwrap();
        fs.inodes[ino as usize - 1].kind = INODE_FREE;
        assert_eq!(
            fs.fsck(),
            Err(FsError::Corrupt),
            "dangling dir entry phải bị bắt"
        );
    }

    #[test]
    fn no_space_is_reported_and_old_data_survives() {
        // Vòng 216: đĩa ĐẦY phải báo `NoSpace` gọn, và dữ liệu đã ghi trước đó
        // phải còn đọc được. Đây là loại lỗi mà "hết chỗ" thường làm hỏng file
        // dở dang thay vì từ chối sạch.
        let mut dev = MemDev::with_blocks(64);
        format(&mut dev).unwrap();
        let mut fs = Fs::mount(dev).unwrap();
        let keep = fs.create(b"/keep", 0, INODE_FILE).unwrap();
        fs.write(keep, 0, b"before-full", 0).unwrap();
        // Ghi một file lớn cho tới khi hết chỗ.
        let big = fs.create(b"/big", 0, INODE_FILE).unwrap();
        let chunk = [0x5Au8; BLOCK_SIZE];
        let mut written = 0usize;
        let mut full_error = None;
        while written < 512 * BLOCK_SIZE {
            match fs.write(big, written, &chunk, 0) {
                Ok(n) => written += n,
                Err(error) => {
                    full_error = Some(error);
                    break;
                }
            }
        }
        assert_eq!(
            full_error,
            Some(FsError::NoSpace),
            "ghi tới khi hết đĩa phải trả NoSpace, không phải lỗi khác"
        );
        // Dữ liệu cũ phải nguyên vẹn và FS vẫn nhất quán.
        let mut buf = [0u8; 16];
        let n = fs.read(keep, 0, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"before-full");
        fs.fsck().unwrap();
    }

    #[test]
    fn other_uid_is_read_only_and_cannot_unlink() {
        // Vòng 215: kiểm ĐƯỜNG THẤT BẠI của quyền hạn (phần lớn test trước đây
        // chỉ đi đường thành công). Mode mặc định `0b0111` = owner rwx, other
        // chỉ đọc ⇒ uid khác ghi phải bị từ chối nhưng đọc vẫn được.
        let mut fs = fresh();
        // Tạo bằng uid 0: thư mục gốc thuộc uid 0, mode `0b0111` ⇒ uid khác
        // KHÔNG được tạo entry trong đó (chính điều này làm bản nháp của test
        // thất bại ở `create` — đó là hành vi ĐÚNG của FS, không phải lỗi).
        let ino = fs.create(b"/p", 0, INODE_FILE).unwrap();
        fs.write(ino, 0, b"secret", 0).unwrap();
        assert_eq!(fs.write(ino, 0, b"x", 7), Err(FsError::NoAccess));
        let mut buf = [0u8; 8];
        let n = fs.read(ino, 0, &mut buf, 7).unwrap();
        assert_eq!(&buf[..n], b"secret", "other vẫn phải ĐỌC được");
        // Xoá entry đòi quyền ghi trên thư mục cha (thuộc uid 0).
        assert_eq!(fs.unlink(b"/p", 7), Err(FsError::NoAccess));
        assert!(
            fs.lookup(b"/p", 5).is_ok(),
            "file còn nguyên sau khi bị từ chối"
        );
        // uid 0 (root) làm được mọi thứ — dùng để dọn.
        fs.unlink(b"/p", 0).unwrap();
    }

    #[test]
    fn truncate_shrinks_and_grows_with_zero_fill() {
        // Vòng 214: `truncate` chưa có test — nếu không trả block khi cắt thì
        // `df` giảm dần; nếu không zero phần đuôi thì đọc ra dữ liệu cũ (rò rỉ
        // nội dung giữa hai file trên cùng block).
        let mut fs = fresh();
        let ino = fs.create(b"/t", 0, INODE_FILE).unwrap();
        fs.write(ino, 0, &[0xAAu8; 3 * BLOCK_SIZE], 0).unwrap();
        let (used_full, _) = fs.usage();
        // Cắt còn 1 block: block phải được trả và size đúng.
        fs.truncate(ino, BLOCK_SIZE, 0).unwrap();
        assert_eq!(fs.info(ino).unwrap().size as usize, BLOCK_SIZE);
        let (used_one, _) = fs.usage();
        assert!(used_one < used_full, "cắt bớt block phải làm used giảm");
        let mut buf = [0u8; BLOCK_SIZE * 2];
        let n = fs.read(ino, 0, &mut buf, 0).unwrap();
        assert_eq!(n, BLOCK_SIZE, "đọc sau khi cắt chỉ trả phần còn lại");
        assert!(buf[..n].iter().all(|&b| b == 0xAA));
        // Mở rộng lại: phần mới phải là 0, không phải rác cũ.
        fs.truncate(ino, 2 * BLOCK_SIZE, 0).unwrap();
        assert_eq!(fs.info(ino).unwrap().size as usize, 2 * BLOCK_SIZE);
        let mut grown = [0xFFu8; BLOCK_SIZE * 2];
        let n = fs.read(ino, 0, &mut grown, 0).unwrap();
        assert_eq!(n, 2 * BLOCK_SIZE);
        assert!(grown[..BLOCK_SIZE].iter().all(|&b| b == 0xAA));
        assert!(
            grown[BLOCK_SIZE..].iter().all(|&b| b == 0),
            "phần mở rộng phải bằng 0, không phải dữ liệu cũ"
        );
        // Cắt về 0 phải trả hết block của file.
        fs.truncate(ino, 0, 0).unwrap();
        assert_eq!(fs.info(ino).unwrap().size, 0);
        let (used_zero, _) = fs.usage();
        assert!(used_zero < used_one);
    }

    #[test]
    fn usage_tracks_allocation_and_unlink() {
        // Vòng 213: `df` (STATFS) lấy số liệu từ `usage()`; nếu bitmap không
        // nhả block khi xoá thì `df` báo thiếu chỗ dần dần — rất khó thấy.
        let mut fs = fresh();
        // Mốc so sánh lấy SAU khi đã có một file: block của thư mục gốc (nếu
        // được cấp) không phải thứ ta muốn đo — thứ cần đo là block của FILE.
        let keep = fs.create(b"/keep", 0, INODE_FILE).unwrap();
        fs.write(keep, 0, b"x", 0).unwrap();
        let (used0, total0) = fs.usage();
        let ino = fs.create(b"/f", 0, INODE_FILE).unwrap();
        fs.write(ino, 0, &[7u8; 3 * BLOCK_SIZE], 0).unwrap();
        let (used1, total1) = fs.usage();
        assert!(used1 > used0, "ghi 3 block phải làm used tăng");
        assert_eq!(total1, total0);
        fs.unlink(b"/f", 0).unwrap();
        let (used2, total2) = fs.usage();
        assert_eq!(
            used2, used0,
            "xoá file phải trả block về bitmap (nếu không, df báo thiếu chỗ dần)"
        );
        assert_eq!(total2, total0, "tổng dung lượng không đổi");
    }

    #[test]
    fn rename_moves_entry_and_rejects_missing_or_duplicate() {
        // Vòng 212: `rename` chưa có test nào — trong khi nó là lệnh dễ làm hỏng
        // FS nhất (đổi tên xong mà journal sai là mất cả cây).
        let mut fs = fresh();
        fs.create(b"/a.txt", 0, INODE_FILE).unwrap();
        fs.rename(b"/a.txt", b"/b.txt", 0).unwrap();
        assert!(fs.lookup(b"/a.txt", 0).is_err(), "tên cũ phải biến mất");
        assert!(fs.lookup(b"/b.txt", 0).is_ok(), "tên mới phải tra được");
        // Đích không tồn tại nguồn ⇒ NotFound; đích đã tồn tại ⇒ Exists.
        assert_eq!(fs.rename(b"/missing", b"/x", 0), Err(FsError::NotFound));
        fs.create(b"/c.txt", 0, INODE_FILE).unwrap();
        assert_eq!(fs.rename(b"/b.txt", b"/c.txt", 0), Err(FsError::Exists));
        // Dữ liệu phải còn nguyên sau khi đổi tên (ghi trước, đọc sau).
        let ino = fs.lookup(b"/c.txt", 0).unwrap();
        fs.write(ino, 0, b"hello", 0).unwrap();
        fs.rename(b"/c.txt", b"/d.txt", 0).unwrap();
        let ino = fs.lookup(b"/d.txt", 0).unwrap();
        let mut buf = [0u8; 8];
        let n = fs.read(ino, 0, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn rename_rejects_directory_cycles_and_dotdot() {
        // Vòng 212: đổi tên thư mục vào CHÍNH cây con của nó sẽ tạo chu trình —
        // mount sau đó không dạo được cây. `..` bị `next_component` từ chối nên
        // không thể lách bằng đường dẫn tương đối.
        let mut fs = fresh();
        fs.mkdir(b"/a", 0).unwrap();
        fs.mkdir(b"/a/b", 0).unwrap();
        assert_eq!(fs.rename(b"/a", b"/a/b", 0), Err(FsError::Invalid));
        assert_eq!(fs.rename(b"/a", b"/a/b/new", 0), Err(FsError::Invalid));
        assert_eq!(fs.rename(b"/a", b"/x/../y", 0), Err(FsError::Invalid));
        // Đổi tên thư mục bình thường vẫn phải chạy.
        fs.rename(b"/a", b"/z", 0).unwrap();
        assert!(fs.lookup(b"/z/b", 0).is_ok(), "con phải còn nguyên");
    }

    #[test]
    fn cached_device_hits_and_write_through() {
        let mut cache = CachedDev::new(CountingDev::new());
        let data = [0xab; BLOCK_SIZE];
        cache.write(3, &data).unwrap();
        assert_eq!(cache.inner().writes, 1);

        let mut out = [0u8; BLOCK_SIZE];
        cache.read(3, &mut out).unwrap();
        assert_eq!(out, data);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 0);
        assert_eq!(cache.inner().reads, 0);

        cache.read(4, &mut out).unwrap();
        assert_eq!(out, [0u8; BLOCK_SIZE]);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.inner().reads, 1);
    }

    #[test]
    fn cached_device_write_backs_file_data_only() {
        let mut cache = CachedDev::new(CountingDev::new());
        let data = [0x5a; BLOCK_SIZE];
        cache.write(DATA_START, &data).unwrap();
        assert_eq!(cache.inner().writes, 0);
        let mut out = [0u8; BLOCK_SIZE];
        cache.read(DATA_START, &mut out).unwrap();
        assert_eq!(out, data);
        cache.flush().unwrap();
        assert_eq!(cache.inner().writes, 1);
        assert_eq!(cache.inner().flushes, 1);

        // Journal/metadata blocks stay write-through even with caching.
        cache.write(JOURNAL_START, &data).unwrap();
        assert_eq!(cache.inner().writes, 2);
    }

    #[test]
    fn cached_device_evicts_round_robin() {
        let mut cache = CachedDev::new(CountingDev::new());
        let mut out = [0u8; BLOCK_SIZE];
        for lba in 0..CACHE_SLOTS as u32 {
            cache.write(lba + 1, &[lba as u8; BLOCK_SIZE]).unwrap();
        }
        cache.read(1, &mut out).unwrap();
        assert_eq!(out[0], 0);
        assert_eq!(cache.stats().hits, 1);

        // The next miss replaces slot 0 (LBA 1).
        cache.read(CACHE_SLOTS as u32 + 1, &mut out).unwrap();
        cache.read(1, &mut out).unwrap();
        assert_eq!(cache.stats().misses, 2);
    }

    #[test]
    fn cached_device_wraps_filesystem() {
        let mut dev = CountingDev::new();
        format(&mut dev).unwrap();
        let mut fs = Fs::mount(CachedDev::new(dev)).unwrap();
        let file = fs.create(b"/cache", 0, INODE_FILE).unwrap();
        fs.write(file, 0, b"through-cache", 0).unwrap();

        let mut out = [0u8; 13];
        fs.read(file, 0, &mut out, 0).unwrap();
        assert_eq!(&out, b"through-cache");
        assert!(fs.dev().stats().hits > 0);
        assert!(fs.dev().stats().writes > 0);
        fs.fsync().unwrap();
        assert!(fs.dev().inner().flushes >= 1);
        fs.fsck().unwrap();
    }

    #[test]
    fn basic_namespace_and_data() {
        let mut fs = fresh();
        fs.fsck().unwrap();
        let dir = fs.mkdir(b"/d", 1).unwrap();
        let file = fs.create(b"/d/a", 1, INODE_FILE).unwrap();
        let data = [0x5au8; 600];
        fs.write(file, 0, &data, 1).unwrap();
        let mut out = [0u8; 600];
        assert_eq!(fs.read(file, 0, &mut out, 1).unwrap(), 600);
        assert_eq!(out, data);
        assert_eq!(fs.info(file).unwrap().size, 600);
        assert_eq!(fs.read_dir_entry(ROOT_INO, 0).unwrap().unwrap().0, dir);
        assert_eq!(fs.read_dir_entry(dir, 0).unwrap().unwrap().0, file);
        fs.rename(b"/d/a", b"/d/b", 1).unwrap();
        assert!(fs.lookup(b"/d/a", 1).is_err());
        assert_eq!(fs.lookup(b"/d/b", 1).unwrap(), file);
        fs.truncate(file, 100, 1).unwrap();
        assert_eq!(fs.info(file).unwrap().size, 100);
        fs.fsck().unwrap();
        let mut fs2 = reopen(&fs);
        fs2.fsck().unwrap();
        assert_eq!(fs2.lookup(b"/d/b", 1).unwrap(), file);
        fs2.unlink(b"/d/b", 1).unwrap();
        fs2.unlink(b"/d", 1).unwrap();
        fs2.fsck().unwrap();
    }

    #[test]
    fn journal_replay_and_torn_commit() {
        let mut fs = fresh();
        let file = fs.create(b"/j", 0, INODE_FILE).unwrap();
        fs.write(file, 0, b"old", 0).unwrap();
        let bytes = *b"new-data";
        // Build a pending write and leave the commit for mount to replay.
        fs.inject_pending_write(b"/j", &bytes, 0, false).unwrap();
        let mut fs2 = reopen(&fs);
        assert_eq!(fs2.replayed(), 1);
        let mut out = [0u8; 8];
        assert_eq!(fs2.read(file, 0, &mut out, 0).unwrap(), 8);
        assert_eq!(&out, &bytes);
        fs2.fsck().unwrap();

        // Torn commit must be ignored and the old target data preserved.
        fs.write(file, 0, b"stable", 0).unwrap();
        fs.inject_pending_write(b"/j", b"must-not-land", 0, true)
            .unwrap();
        let mut fs3 = reopen(&fs);
        assert_eq!(fs3.replayed(), 2);
        let mut out = [0u8; 6];
        assert_eq!(fs3.read(file, 0, &mut out, 0).unwrap(), 6);
        assert_eq!(&out, b"stable");
        fs3.fsck().unwrap();
    }

    #[test]
    fn vf03_image_still_mounts_with_legacy_geometry() {
        let mut dev = MemDev::new();
        format(&mut dev).unwrap();
        let mut superblock = zero_block();
        dev.read(1, &mut superblock).unwrap();
        superblock[..4].copy_from_slice(b"VF03");
        put_u32(&mut superblock, 4, 3);
        let crc = crc32(&superblock[..56]);
        put_u32(&mut superblock, 56, crc);
        dev.write(1, &superblock).unwrap();

        let mut fs = Fs::mount(dev).unwrap();
        assert_eq!(fs.version, 3);
        assert_eq!(fs.direct_blocks, MAX_DIRECT_BLOCKS);
        let ino = fs.create(b"/old", 1, INODE_FILE).unwrap();
        assert_eq!(
            fs.write(ino, 0, &[0x7a; 11 * BLOCK_SIZE], 1).unwrap(),
            11 * BLOCK_SIZE
        );
        fs.fsck().unwrap();
        let mut out = [0u8; 11 * BLOCK_SIZE];
        assert_eq!(fs.read(ino, 0, &mut out, 1).unwrap(), out.len());
        assert_eq!(out, [0x7a; 11 * BLOCK_SIZE]);
    }

    #[test]
    fn crash_every_write_keeps_fs_consistent() {
        let baseline = fresh();
        // Determine the write count of one mutation.
        let mut probe = Fs::mount(CrashDev {
            data: baseline.dev.data.clone(),
            writes: 0,
            fail_at: usize::MAX,
            failed: false,
        })
        .unwrap();
        probe.mkdir(b"/crash", 0).unwrap();
        let total = probe.dev.writes;
        assert!(total > 3);
        for k in 0..=total {
            let dev = CrashDev {
                data: baseline.dev.data.clone(),
                writes: 0,
                fail_at: k,
                failed: false,
            };
            let mut fs = match Fs::mount(dev) {
                Ok(fs) => fs,
                Err(_) => continue,
            };
            let result = fs.mkdir(b"/crash", 0);
            assert!(result.is_err() || fs.lookup(b"/crash", 0).is_ok());
            let mut image = MemDev {
                data: fs.dev.data.clone(),
            };
            let mut mounted = match Fs::mount(image.clone()) {
                Ok(fs) => fs,
                Err(error) => panic!("mount failed at k={k}: {error:?}"),
            };
            mounted.fsck().unwrap();
            let _ = &mut image;
        }
    }

    #[test]
    fn errors_and_limits() {
        let mut fs = fresh();
        assert_eq!(fs.lookup(b"/missing", 1), Err(FsError::NotFound));
        fs.create(b"/f", 1, INODE_FILE).unwrap();
        assert_eq!(fs.create(b"/f", 1, INODE_FILE), Err(FsError::Exists));
        let file = fs.lookup(b"/f", 1).unwrap();
        assert_eq!(fs.write(file, 0, b"x", 2), Err(FsError::NoAccess));
        assert_eq!(fs.lookup(b"/f/x", 1), Err(FsError::NotDir));
        assert_eq!(fs.read(ROOT_INO, 0, &mut [0u8; 1], 1), Err(FsError::IsDir));
        fs.mkdir(b"/d", 1).unwrap();
        fs.create(b"/d/inside", 1, INODE_FILE).unwrap();
        assert_eq!(fs.unlink(b"/d", 1), Err(FsError::NotEmpty));
        let long = [b'a'; MAX_NAME + 1];
        let mut path = [0u8; MAX_PATH];
        path[0] = b'/';
        path[1..1 + long.len()].copy_from_slice(&long);
        assert_eq!(
            fs.mkdir(&path[..1 + long.len()], 1),
            Err(FsError::NameTooLong)
        );
        let too_large = vec![0u8; MAX_FILE_SIZE + 1];
        assert_eq!(fs.write(file, 0, &too_large, 1), Err(FsError::NoSpace));
        let fits = vec![0u8; MAX_FILE_SIZE_V3];
        assert_eq!(fs.write(file, 0, &fits, 1).unwrap(), MAX_FILE_SIZE_V3);
        let mut made = 0usize;
        loop {
            let mut name = [0u8; 8];
            name[..3].copy_from_slice(b"/iX");
            name[2] = b'0' + (made % 10) as u8;
            name[3] = b'0' + ((made / 10) % 10) as u8;
            match fs.create(&name[..4], 1, INODE_FILE) {
                Ok(_) => made += 1,
                Err(FsError::NoInode) => break,
                Err(error) => panic!("unexpected {error:?}"),
            }
            assert!(made < MAX_INODES);
        }
        assert!(made > 0);
    }

    #[test]
    fn legacy_migration() {
        let mut dev = MemDev::new();
        let nonce = b"legacy-nonce";
        dev.data[..4].copy_from_slice(b"VF01");
        dev.data[4..8].copy_from_slice(&(nonce.len() as u32).to_le_bytes());
        dev.data[8..8 + nonce.len()].copy_from_slice(nonce);
        let mut fs = Fs::mount(dev).unwrap();
        assert!(fs.migrated());
        let ino = fs.lookup(b"/scratch", 0).unwrap();
        let mut out = [0u8; 16];
        let len = fs.read(ino, 0, &mut out, 0).unwrap();
        assert_eq!(&out[..len], nonce);
        assert_eq!(&fs.dev.data[..4], &[0u8; 4]);
        fs.fsck().unwrap();
    }

    enum Op {
        Create,
        Write,
        Truncate,
        Unlink,
        Rename,
        Mkdir,
    }

    fn apply_op<D: BlockDev>(fs: &mut Fs<D>, op: &Op) -> Result<(), FsError> {
        match op {
            Op::Create => fs.create(b"/new", 1, INODE_FILE).map(|_| ()),
            Op::Write => {
                let ino = fs.lookup(b"/a", 1)?;
                fs.write(ino, 0, b"new-value", 1).map(|_| ())
            }
            Op::Truncate => {
                let ino = fs.lookup(b"/a", 1)?;
                fs.truncate(ino, 1, 1)
            }
            Op::Unlink => fs.unlink(b"/a", 1),
            Op::Rename => fs.rename(b"/a", b"/renamed", 1),
            Op::Mkdir => fs.mkdir(b"/newdir", 1).map(|_| ()),
        }
    }

    fn baseline_image() -> Vec<u8> {
        let mut fs = fresh();
        let a = fs.create(b"/a", 1, INODE_FILE).unwrap();
        fs.write(a, 0, b"old-value", 1).unwrap();
        fs.create(b"/b", 1, INODE_FILE).unwrap();
        fs.fsck().unwrap();
        fs.dev.data
    }

    #[test]
    fn crash_every_write_for_mutations() {
        let baseline = baseline_image();
        for op in [
            Op::Create,
            Op::Write,
            Op::Truncate,
            Op::Unlink,
            Op::Rename,
            Op::Mkdir,
        ] {
            let mut probe = Fs::mount(CrashDev {
                data: baseline.clone(),
                writes: 0,
                fail_at: usize::MAX,
                failed: false,
            })
            .unwrap();
            let _ = apply_op(&mut probe, &op);
            let total = probe.dev.writes;
            for k in 0..=total {
                let mut fs = Fs::mount(CrashDev {
                    data: baseline.clone(),
                    writes: 0,
                    fail_at: k,
                    failed: false,
                })
                .unwrap();
                let _ = apply_op(&mut fs, &op);
                let image = fs.dev.data.clone();
                let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
                mounted.fsck().unwrap_or_else(|e| {
                    panic!(
                        "op={} k={k} fsck {e:?}",
                        match op {
                            Op::Create => "create",
                            Op::Write => "write",
                            Op::Truncate => "truncate",
                            Op::Unlink => "unlink",
                            Op::Rename => "rename",
                            Op::Mkdir => "mkdir",
                        }
                    )
                });
                match op {
                    Op::Write => {
                        let ino = mounted.lookup(b"/a", 1).unwrap();
                        let mut data = [0u8; 16];
                        let len = mounted.read(ino, 0, &mut data, 1).unwrap();
                        assert!(
                            len == 9 && &data[..len] == b"new-value"
                                || len == 9 && &data[..len] == b"old-value"
                        );
                    }
                    Op::Truncate => {
                        let ino = mounted.lookup(b"/a", 1).unwrap();
                        let size = mounted.info(ino).unwrap().size;
                        assert!(size == 9 || size == 1);
                    }
                    Op::Create => {
                        let exists = mounted.lookup(b"/new", 1).is_ok();
                        let _ = exists; // both states are valid after a crash
                    }
                    Op::Mkdir => {
                        let exists = mounted.lookup(b"/newdir", 1).is_ok();
                        let _ = exists;
                    }
                    Op::Rename => {
                        let a = mounted.lookup(b"/a", 1).is_ok();
                        let b = mounted.lookup(b"/renamed", 1).is_ok();
                        assert!((a && !b) || (!a && b));
                    }
                    Op::Unlink => {
                        let a = mounted.lookup(b"/a", 1);
                        assert!(a.is_ok() || a == Err(FsError::NotFound));
                    }
                }
            }
        }
    }

    #[test]
    fn random_2000_operations_with_checks() {
        let mut fs = fresh();
        let ino = fs.create(b"/rand", 1, INODE_FILE).unwrap();
        let mut model = vec![0u8; 1024];
        let mut model_len = 0usize;
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed
        };
        for step in 0..2000usize {
            let op = next() % 3;
            let offset_limit = core::cmp::max(model_len, 1);
            let offset = if model_len == 0 {
                0
            } else {
                (next() as usize) % (model_len + 1)
            };
            let _ = offset_limit;
            let len = core::cmp::min((next() as usize) % 300, MAX_FILE_SIZE - offset);
            if op == 0 {
                let mut bytes = [0u8; 300];
                for byte in bytes[..len].iter_mut() {
                    *byte = next() as u8;
                }
                if fs.write(ino, offset, &bytes[..len], 1).is_ok() {
                    model[offset..offset + len].copy_from_slice(&bytes[..len]);
                    model_len = model_len.max(offset + len);
                }
            } else if op == 1 {
                let new_len = (next() as usize) % 600;
                if fs.truncate(ino, new_len, 1).is_ok() {
                    if new_len > model_len {
                        for byte in model[model_len..new_len].iter_mut() {
                            *byte = 0;
                        }
                    }
                    model_len = new_len;
                }
            } else {
                let mut out = [0u8; 300];
                let count = fs.read(ino, offset, &mut out[..len], 1).unwrap_or(0);
                if count != 0 {
                    assert_eq!(&out[..count], &model[offset..offset + count]);
                }
            }
            if step % 100 == 0 {
                fs.fsck().unwrap_or_else(|e| panic!("step {step}: {e:?}"));
            }
            if step % 37 == 0 {
                // Crash the next mutation at a random write boundary and
                // require either the old or the fully applied model.
                let old_model = model.clone();
                let old_len = model_len;
                let mut bytes = [0u8; 64];
                for byte in bytes.iter_mut() {
                    *byte = next() as u8;
                }
                let crash_at = (next() as usize) % 80;
                let mut crash = Fs::mount(CrashDev {
                    data: fs.dev.data.clone(),
                    writes: 0,
                    fail_at: crash_at,
                    failed: false,
                })
                .unwrap();
                let _result = crash.write(ino, 0, &bytes, 1);
                let image = crash.dev.data;
                let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
                mounted.fsck().unwrap();
                let mut out = vec![0u8; 1024];
                let count = mounted.read(ino, 0, &mut out, 1).unwrap();
                let mut expected = old_model.clone();
                expected[..64].copy_from_slice(&bytes);
                let expected_len = old_len.max(64);
                let old_matches = count == old_len && &out[..count] == &old_model[..old_len];
                let new_matches =
                    count == expected_len && &out[..count] == &expected[..expected_len];
                assert!(old_matches || new_matches, "crash produced a partial write");
                if new_matches {
                    model = expected;
                    model_len = expected_len;
                }
                fs = mounted;
            }
        }
        fs.fsck().unwrap();
    }

    fn small_fs(options: FormatOptions) -> Fs<MemDev> {
        let mut dev = MemDev::with_blocks(48);
        format_with_options(&mut dev, options).unwrap();
        Fs::mount(dev).unwrap()
    }

    #[test]
    fn small_disk_enospc_then_free_and_reuse() {
        let mut fs = small_fs(FormatOptions {
            total_blocks: 48,
            inode_limit: 64,
            dir_block_limit: 11,
        });
        let full = fs.create(b"/full", 1, INODE_FILE).unwrap();
        assert_eq!(
            fs.write(full, 0, &[0x11; 12 * BLOCK_SIZE], 1).unwrap(),
            12 * BLOCK_SIZE
        );
        let rest = fs.create(b"/rest", 1, INODE_FILE).unwrap();
        assert_eq!(
            fs.write(rest, 0, &[0x22; 4 * BLOCK_SIZE], 1).unwrap(),
            4 * BLOCK_SIZE
        );
        let third = fs.create(b"/third", 1, INODE_FILE).unwrap();
        assert_eq!(
            fs.write(third, 0, &[0x33; BLOCK_SIZE], 1),
            Err(FsError::NoSpace)
        );
        let image = fs.dev.data.clone();
        let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
        mounted.fsck().unwrap();
        assert_eq!(mounted.unlink(b"/full", 1), Ok(()));
        let again = mounted.create(b"/again", 1, INODE_FILE).unwrap();
        assert_eq!(
            mounted.write(again, 0, &[0x44; 8 * BLOCK_SIZE], 1).unwrap(),
            8 * BLOCK_SIZE
        );
        mounted.fsck().unwrap();
    }

    #[test]
    fn one_block_directory_reaches_full_then_reuses_slot() {
        let mut fs = small_fs(FormatOptions {
            total_blocks: 48,
            inode_limit: 64,
            dir_block_limit: 1,
        });
        for index in 0..DIR_ENTRIES_PER_BLOCK {
            let mut path = [0u8; 5];
            path[..2].copy_from_slice(b"/f");
            path[2] = b'0' + (index / 10) as u8;
            path[3] = b'0' + (index % 10) as u8;
            fs.create(&path[..4], 1, INODE_FILE).unwrap();
        }
        let mut extra = [0u8; 5];
        extra[..2].copy_from_slice(b"/f");
        extra[2] = b'1';
        extra[3] = b'6';
        assert_eq!(fs.create(&extra[..4], 1, INODE_FILE), Err(FsError::NoSpace));
        let image = fs.dev.data.clone();
        let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
        mounted.fsck().unwrap();
        assert_eq!(mounted.unlink(b"/f00", 1), Ok(()));
        mounted.create(b"/g", 1, INODE_FILE).unwrap();
        mounted.fsck().unwrap();
    }

    #[test]
    fn crash_near_full_small_image_keeps_all_or_nothing() {
        let mut dev = MemDev::with_blocks(49);
        format_with_options(
            &mut dev,
            FormatOptions {
                total_blocks: 49,
                inode_limit: 64,
                dir_block_limit: 11,
            },
        )
        .unwrap();
        let mut fs = Fs::mount(dev).unwrap();
        let full = fs.create(b"/full", 1, INODE_FILE).unwrap();
        fs.write(full, 0, &[0x11; 8 * BLOCK_SIZE], 1).unwrap();
        let rest = fs.create(b"/rest", 1, INODE_FILE).unwrap();
        fs.write(rest, 0, &[0x22; 5 * BLOCK_SIZE], 1).unwrap();
        fs.create(b"/spare", 1, INODE_FILE).unwrap();
        fs.fsck().unwrap();
        let baseline = fs.dev.data.clone();
        let mut probe = Fs::mount(CrashDev {
            data: baseline.clone(),
            writes: 0,
            fail_at: usize::MAX,
            failed: false,
        })
        .unwrap();
        let spare = probe.lookup(b"/spare", 1).unwrap();
        probe.write(spare, 0, b"one", 1).unwrap();
        let total = probe.dev.writes;
        assert!(total >= 5);
        for k in 0..=total {
            let mut crashed = Fs::mount(CrashDev {
                data: baseline.clone(),
                writes: 0,
                fail_at: k,
                failed: false,
            })
            .unwrap();
            let spare = crashed.lookup(b"/spare", 1).unwrap();
            let _ = crashed.write(spare, 0, b"one", 1);
            let image = crashed.dev.data;
            let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
            mounted.fsck().unwrap();
            let spare = mounted.lookup(b"/spare", 1).unwrap();
            let size = mounted.info(spare).unwrap().size;
            assert!(
                size == 0 || size == 3,
                "partial write at k={k}: size={size}"
            );
        }
    }

    #[test]
    fn corrupt_dir_entry_length_does_not_panic() {
        // Vòng 83 (audit #2 P0-1): một byte độ dài tên hỏng trên đĩa từng làm
        // VFS server panic (slice index) → mọi IPC treo.
        let mut fs = fresh();
        let dir = fs.create(b"/d", 1, INODE_DIR).unwrap();
        fs.create(b"/d/a", 1, INODE_FILE).unwrap();
        let block_no = fs.inodes[dir as usize - 1].blocks[0];
        assert!(block_no != 0);
        let mut dev = fs.dev.clone();
        let off = block_no as usize * BLOCK_SIZE;
        dev.data[off + 2] = 200; // len rác, lớn hơn MAX_NAME
        let mut fs = Fs::mount(dev).unwrap();
        // Không được panic; entry hỏng bị bỏ qua.
        let _ = fs.lookup(b"/d/a", 0);
        let _ = fs.read_dir_entry(dir, 0);
    }

    #[test]
    fn corrupt_dir_entry_keeps_readdir_indices_aligned() {
        // Vòng 88 (audit #4 P2): entry hỏng phải chiếm một index. Trước đây
        // [a, corrupt, c] đọc index 1 trả c và index 2 cũng trả c.
        let mut fs = fresh();
        let dir = fs.create(b"/d", 1, INODE_DIR).unwrap();
        fs.create(b"/d/a", 1, INODE_FILE).unwrap();
        fs.create(b"/d/b", 1, INODE_FILE).unwrap();
        fs.create(b"/d/c", 1, INODE_FILE).unwrap();
        let block_no = fs.inodes[dir as usize - 1].blocks[0];
        assert!(block_no != 0);
        let mut dev = fs.dev.clone();
        let off = block_no as usize * BLOCK_SIZE;
        // Slot 0 = "a"; làm hỏng trường độ dài của slot 1 ("b").
        dev.data[off + DIR_ENTRY_SIZE + 2] = 200;
        let mut fs = Fs::mount(dev).unwrap();
        let first = fs.read_dir_entry(dir, 0).unwrap().unwrap();
        assert_eq!(&first.1[..first.2], b"a");
        // Index 1 là entry hỏng: báo lỗi thay vì trả lại entry kế tiếp.
        assert_eq!(fs.read_dir_entry(dir, 1), Err(FsError::Corrupt));
        // Index 2 là "c", không phải "c" lặp lại.
        let third = fs.read_dir_entry(dir, 2).unwrap().unwrap();
        assert_eq!(&third.1[..third.2], b"c");
        assert!(fs.read_dir_entry(dir, 3).unwrap().is_none());
    }

    #[test]
    fn corrupt_dir_size_beyond_block_slots_does_not_panic() {
        // Vòng 83 (audit #2 P0-2): dir size > 12 block làm index `blocks[]`
        // quá biên trong các vòng lặp thư mục.
        let mut fs = fresh();
        let dir = fs.create(b"/big", 1, INODE_DIR).unwrap();
        fs.inodes[dir as usize - 1].size = 7000;
        let _ = fs.read_dir_entry(dir, 0);
        let _ = fs.lookup(b"/big", 0);
        assert!(fs.inodes[dir as usize - 1].blocks_used() <= MAX_BLOCKS_PER_INODE);
    }

    #[test]
    fn large_file_uses_single_indirect_and_frees_it_on_truncate() {
        let mut fs = fresh();
        let ino = fs.create(b"/large", 1, INODE_FILE).unwrap();
        let mut data = [0u8; 60 * 1024];
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = (index * 31 + 7) as u8;
        }
        assert_eq!(fs.write(ino, 0, &data, 1).unwrap(), data.len());
        assert_eq!(fs.info(ino).unwrap().size as usize, data.len());
        {
            let inode = fs.inodes[ino as usize - 1];
            std::println!(
                "DEBUG size={} used={} direct={:?}",
                inode.size,
                inode.blocks_used(),
                inode.blocks
            );
            let ind = inode.blocks[V4_SINGLE_INDIRECT_INDEX];
            std::println!(
                "DEBUG indirect={} data0={} data10={} data11={}",
                ind,
                inode.blocks[0],
                inode.blocks[V4_DIRECT_BLOCKS],
                inode.blocks[V4_DOUBLE_INDIRECT_INDEX]
            );
            if ind != 0 {
                let mut b = zero_block();
                fs.dev.read(ind, &mut b).unwrap();
                std::println!(
                    "DEBUG entries {} {} {}",
                    get_u32(&b, 0),
                    get_u32(&b, 4),
                    get_u32(&b, 428)
                );
            }
        }
        fs.fsck().unwrap();
        let mut out = [0u8; 4096];
        let mut offset = 0usize;
        while offset < data.len() {
            let len = core::cmp::min(out.len(), data.len() - offset);
            assert_eq!(fs.read(ino, offset, &mut out[..len], 1).unwrap(), len);
            assert_eq!(&out[..len], &data[offset..offset + len]);
            offset += len;
        }
        fs.truncate(ino, 1000, 1).unwrap();
        assert_eq!(fs.info(ino).unwrap().size, 1000);
        fs.fsck().unwrap();
        let mut small = [0u8; 1000];
        assert_eq!(fs.read(ino, 0, &mut small, 1).unwrap(), 1000);
        assert_eq!(&small, &data[..1000]);
        fs.fsck().unwrap();
    }

    #[test]
    fn double_indirect_grows_beyond_single_indirect() {
        let mut fs = fresh();
        let ino = fs.create(b"/double", 1, INODE_FILE).unwrap();
        let blocks = 200usize;
        let mut data = vec![0u8; blocks * BLOCK_SIZE];
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = (index * 13 + 11) as u8;
        }
        assert_eq!(fs.write(ino, 0, &data, 1).unwrap(), data.len());
        let inode = fs.inodes[ino as usize - 1];
        assert_eq!(inode.blocks_used(), blocks);
        assert_ne!(inode.blocks[V4_SINGLE_INDIRECT_INDEX], 0);
        assert_ne!(inode.blocks[V4_DOUBLE_INDIRECT_INDEX], 0);
        fs.fsck().unwrap();

        let mut out = [0u8; 4096];
        let mut offset = 0usize;
        while offset < data.len() {
            let len = core::cmp::min(out.len(), data.len() - offset);
            assert_eq!(fs.read(ino, offset, &mut out[..len], 1).unwrap(), len);
            assert_eq!(&out[..len], &data[offset..offset + len]);
            offset += len;
        }

        fs.truncate(ino, 100 * BLOCK_SIZE, 1).unwrap();
        assert_eq!(
            fs.inodes[ino as usize - 1].blocks[V4_DOUBLE_INDIRECT_INDEX],
            0
        );
        fs.fsck().unwrap();
        fs.truncate(ino, 5 * BLOCK_SIZE, 1).unwrap();
        assert_eq!(
            fs.inodes[ino as usize - 1].blocks[V4_SINGLE_INDIRECT_INDEX],
            0
        );
        fs.fsck().unwrap();
    }

    #[test]
    fn crash_during_first_indirect_block_allocation_keeps_fsck_clean() {
        let mut fs = fresh();
        let ino = fs.create(b"/indirect", 1, INODE_FILE).unwrap();
        fs.write(ino, 0, &[0x5a; 10 * BLOCK_SIZE], 1).unwrap();
        fs.fsck().unwrap();
        let baseline = fs.dev.data.clone();
        let mut probe = Fs::mount(CrashDev {
            data: baseline.clone(),
            writes: 0,
            fail_at: usize::MAX,
            failed: false,
        })
        .unwrap();
        probe
            .write(ino, 10 * BLOCK_SIZE, &[0xa5; BLOCK_SIZE], 1)
            .unwrap();
        let total = probe.dev.writes;
        assert!(total >= 5);
        for k in 0..=total {
            let mut crashed = Fs::mount(CrashDev {
                data: baseline.clone(),
                writes: 0,
                fail_at: k,
                failed: false,
            })
            .unwrap();
            let _ = crashed.write(ino, 10 * BLOCK_SIZE, &[0xa5; BLOCK_SIZE], 1);
            let image = crashed.dev.data;
            let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
            mounted.fsck().unwrap();
            let size = mounted.info(ino).unwrap().size as usize;
            assert!(size == 10 * BLOCK_SIZE || size == 11 * BLOCK_SIZE);
        }
    }

    #[test]
    fn symlink_follows_final_and_intermediate_components() {
        let mut fs = fresh();
        let target = fs.create(b"/target", 1, INODE_FILE).unwrap();
        fs.write(target, 0, b"symlink-data", 1).unwrap();
        fs.symlink(b"/target", b"/link", 1).unwrap();
        assert_eq!(fs.lookup(b"/link", 1).unwrap(), target);
        let (name, len) = fs.readlink(b"/link", 1).unwrap();
        assert_eq!(&name[..len], b"/target");
        fs.fsck().unwrap();

        fs.mkdir(b"/d", 1).unwrap();
        let inside = fs.create(b"/d/f", 1, INODE_FILE).unwrap();
        fs.symlink(b"/d", b"/dlink", 1).unwrap();
        assert_eq!(fs.lookup(b"/dlink/f", 1).unwrap(), inside);
        fs.symlink(b"d", b"/relative", 1).unwrap();
        assert_eq!(fs.lookup(b"/relative/f", 1).unwrap(), inside);
        fs.fsck().unwrap();

        fs.symlink(b"/missing", b"/dangling", 1).unwrap();
        assert_eq!(fs.lookup(b"/dangling", 1), Err(FsError::NotFound));
        fs.symlink(b"/loop", b"/loop", 1).unwrap();
        assert_eq!(fs.lookup(b"/loop", 1), Err(FsError::Invalid));
        fs.fsck().unwrap();

        fs.unlink(b"/link", 1).unwrap();
        assert_eq!(fs.lookup(b"/link", 1), Err(FsError::NotFound));
        assert_eq!(fs.lookup(b"/target", 1).unwrap(), target);
        fs.fsck().unwrap();
    }

    #[test]
    fn symlink_rejects_empty_target_and_existing_name() {
        let mut fs = fresh();
        assert_eq!(fs.symlink(b"", b"/x", 1), Err(FsError::Invalid));
        fs.symlink(b"/target", b"/x", 1).unwrap();
        assert_eq!(fs.symlink(b"/other", b"/x", 1), Err(FsError::Exists));
        assert_eq!(fs.readlink(b"/x", 1).unwrap().1, 7);
        fs.fsck().unwrap();
    }

    #[test]
    fn hard_link_keeps_data_until_last_name_is_removed() {
        let mut fs = fresh();
        let ino = fs.create(b"/a", 1, INODE_FILE).unwrap();
        let data = [0x5au8; 3 * BLOCK_SIZE];
        fs.write(ino, 0, &data, 1).unwrap();
        fs.link(b"/a", b"/b", 1).unwrap();
        fs.fsck().unwrap();
        assert_eq!(fs.info(ino).unwrap().nlink, 2);
        assert_eq!(fs.lookup(b"/b", 1).unwrap(), ino);
        let mut out = [0u8; 3 * BLOCK_SIZE];
        fs.read(ino, 0, &mut out, 1).unwrap();
        assert_eq!(out, data);
        fs.unlink(b"/a", 1).unwrap();
        fs.fsck().unwrap();
        assert_eq!(fs.info(ino).unwrap().nlink, 1);
        let mut out = [0u8; BLOCK_SIZE];
        let b = fs.lookup(b"/b", 1).unwrap();
        assert_eq!(fs.read(b, 0, &mut out, 1).unwrap(), BLOCK_SIZE);
        assert_eq!(&out, &data[..BLOCK_SIZE]);
        fs.unlink(b"/b", 1).unwrap();
        assert_eq!(fs.lookup(b"/b", 1), Err(FsError::NotFound));
        fs.fsck().unwrap();
    }

    #[test]
    fn mount_rejects_device_smaller_than_superblock_needs() {
        // Vòng 166 (driver block): đĩa nhỏ hơn yêu cầu tối thiểu phải bị từ
        // chối ngay, không được panic hay ghi bừa ra ngoài phạm vi.
        let mut small = MemDev::with_blocks(DATA_START as u32 + 1);
        // format cần ít nhất DATA_START + 2 block (superblock + bitmap + dữ liệu).
        assert_eq!(format(&mut small), Err(FsError::NoSpace));
        assert!(Fs::mount(small).is_err());
    }

    #[test]
    fn format_rejects_device_without_data_blocks() {
        // Đĩa chỉ đủ cho superblock + bitmap nhưng không có block dữ liệu.
        let mut tiny = MemDev::with_blocks(DATA_START as u32);
        assert!(format(&mut tiny).is_err());
    }

    #[test]
    fn writes_land_in_the_right_block_at_both_edges() {
        // Vòng 166: driver phải đúng ở hai biên — block 0 (superblock) và
        // block dữ liệu cuối cùng; ghi rồi đọc lại phải khớp.
        let mut dev = MemDev::new();
        format(&mut dev).unwrap();
        let last = dev.block_count() - 1;
        let mut pattern = [0u8; BLOCK_SIZE];
        for (i, byte) in pattern.iter_mut().enumerate() {
            *byte = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        dev.write(last, &pattern).unwrap();
        let mut back = [0u8; BLOCK_SIZE];
        dev.read(last, &mut back).unwrap();
        assert_eq!(back, pattern);
        // Superblock vẫn đọc được sau khi ghi block cuối.
        let mut sb = [0u8; BLOCK_SIZE];
        dev.read(1, &mut sb).unwrap();
        assert_eq!(&sb[0..4], b"VF04");
    }

    #[test]
    fn root_is_empty_reports_user_entries_only() {
        // Vòng 163: quyết định "được phép format lại khi fsck lỗi" dựa vào
        // việc thư mục gốc có dữ liệu người dùng hay không.
        let mut fs = fresh();
        assert!(fs.root_is_empty(), "FS mới phải coi như trống");
        fs.mkdir(b"/d", 1).unwrap();
        assert!(!fs.root_is_empty(), "có thư mục con thì không còn trống");
        fs.unlink(b"/d", 1).unwrap();
        assert!(fs.root_is_empty(), "xoá hết thì trống lại");
        fs.create(b"/f", 1, INODE_FILE).unwrap();
        assert!(!fs.root_is_empty(), "có file thì không trống");
        fs.fsck().unwrap();
    }

    #[test]
    fn hard_link_rejects_missing_existing_and_directory_targets() {
        let mut fs = fresh();
        assert_eq!(fs.link(b"/missing", b"/b", 1), Err(FsError::NotFound));
        fs.create(b"/a", 1, INODE_FILE).unwrap();
        fs.create(b"/b", 1, INODE_FILE).unwrap();
        assert_eq!(fs.link(b"/a", b"/b", 1), Err(FsError::Exists));
        fs.mkdir(b"/d", 1).unwrap();
        assert_eq!(fs.link(b"/d", b"/e", 1), Err(FsError::IsDir));
        fs.fsck().unwrap();
    }

    /// (#8) Batch-write regression: two 256B chunks in the same 512B block
    /// (the `/prog.elf` staging pattern) must both land with the right size.
    #[test]
    fn issue8_batch_two_chunks_same_block() {
        let mut fs = fresh();
        let ino = fs.create(b"/a", 1, INODE_FILE).unwrap();
        let chunk1 = [0xAAu8; 256];
        let chunk2 = [0xBBu8; 256];
        // Exact root staging pattern: truncate on first open, then chunks.
        fs.truncate(ino, 0, 1).unwrap();
        assert_eq!(fs.write(ino, 0, &chunk1, 1).unwrap(), 256);
        assert_eq!(fs.write(ino, 256, &chunk2, 1).unwrap(), 256);
        assert_eq!(fs.info(ino).unwrap().size, 512);
        let mut out = [0u8; 512];
        let n = fs.read(ino, 0, &mut out, 1).unwrap();
        assert_eq!(n, 512);
        assert!(out[..256].iter().all(|&b| b == 0xAA));
        assert!(out[256..].iter().all(|&b| b == 0xBB));
        fs.fsck().unwrap();
    }

    #[test]
    fn journal_txn_limit_coalesces_and_rejects_overflow() {
        // Mốc 3: txn journal tối đa 16 block — stage trùng LBA gộp (không
        // tốn slot), LBA thứ 17 bị TooManyWrites thay vì tràn journal.
        let mut fs = fresh();
        let filler = [0x5au8; BLOCK_SIZE];
        for lba in 30..30 + 16 {
            fs.stage(lba, &filler).unwrap();
        }
        // Stage lại LBA cũ: gộp, không tốn slot mới.
        fs.stage(30, &filler).unwrap();
        fs.stage(45, &filler).unwrap();
        // LBA mới thứ 17: từ chối.
        assert_eq!(fs.stage(100, &filler), Err(FsError::TooManyWrites));
        // Commit + fsck vẫn sạch (journal không hỏng).
        fs.commit().unwrap();
        fs.fsck().unwrap();
    }

    /// (#8) Table-driven CRC matches the bitwise oracle on the standard
    /// vector ("123456789" -> 0xCBF43926) plus pseudo-random buffers, and
    /// `crc32_continue` chains identically in both implementations.
    #[test]
    fn crc32_table_matches_bitwise_oracle() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32_bitwise(b"123456789"), 0xCBF43926);
        let mut buf = [0u8; 600];
        let mut x: u32 = 0x12345678;
        for slot in buf.iter_mut() {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            *slot = (x >> 24) as u8;
        }
        assert_eq!(crc32(&buf), crc32_bitwise(&buf));
        let (head, tail) = buf.split_at(211);
        let chained = crc32_continue(crc32(head), tail);
        let chained_ref = crc32_continue_bitwise(crc32_bitwise(head), tail);
        assert_eq!(chained, chained_ref);
        assert_eq!(chained, crc32(&buf));
    }

    #[test]
    fn exec_reads_elf_from_fs_and_parses() {
        // Mốc 9 (vòng 18, bước 1): exec tương lai cần FS -> ELF bytes ->
        // parse. Viết 1 ELF64 tối giản vào FS, đọc ra qua read(), parse
        // bằng vifast-elf, kiểm tra entry + segment.
        let mut fs = fresh();
        // ELF64 LE exec, machine riscv (243), 1 PT_LOAD R+X.
        let mut elf = vec![0u8; 0x1000 + 64];
        elf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        elf[4] = 2; // 64-bit
        elf[5] = 1; // LE
        elf[6] = 1; // version
        elf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        elf[18..20].copy_from_slice(&243u16.to_le_bytes()); // EM_RISCV
        elf[20..24].copy_from_slice(&1u32.to_le_bytes());
        elf[24..32].copy_from_slice(&0x10000u64.to_le_bytes()); // entry
        elf[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
        elf[52..54].copy_from_slice(&64u16.to_le_bytes()); // ehsize
        elf[54..56].copy_from_slice(&56u16.to_le_bytes()); // phentsize
        elf[56..58].copy_from_slice(&1u16.to_le_bytes()); // phnum
        let ph = 64usize;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes()); // R+X
        elf[ph + 8..ph + 16].copy_from_slice(&0x1000u64.to_le_bytes()); // offset
        elf[ph + 16..ph + 24].copy_from_slice(&0x10000u64.to_le_bytes()); // vaddr
        elf[ph + 24..ph + 32].copy_from_slice(&0x10000u64.to_le_bytes());
        elf[ph + 32..ph + 40].copy_from_slice(&64u64.to_le_bytes()); // filesz
        elf[ph + 40..ph + 48].copy_from_slice(&64u64.to_le_bytes()); // memsz
        elf[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes()); // align
        for (i, byte) in elf.iter_mut().enumerate().skip(0x1000) {
            *byte = (i & 0xff) as u8;
        }
        let ino = fs.create(b"/init", 1, INODE_FILE).unwrap();
        fs.write(ino, 0, &elf, 1).unwrap();
        fs.fsck().unwrap();
        // Đọc lại qua FS read (nhiều chunk, qua biên block).
        let mut back = vec![0u8; elf.len()];
        let mut done = 0usize;
        while done < back.len() {
            let mut out = [0u8; BLOCK_SIZE];
            let n = fs.read(ino, done, &mut out, 1).unwrap();
            assert!(n > 0);
            back[done..done + n].copy_from_slice(&out[..n]);
            done += n;
        }
        assert_eq!(back, elf);
        // Parse bằng vifast-elf trong cửa sổ user riscv.
        let image = vifast_elf::parse(&back, 243, 0x10000, 0x10000 + 0x1000000).unwrap();
        assert_eq!(image.entry, 0x10000);
        assert_eq!(image.segment_count(), 1);
        let seg = image.segment(0).unwrap();
        assert!(seg.executable() && !seg.writable());
    }
}
