//! ViFastOs FS v3: a fixed-size, journaled filesystem for user-space VFS.
//!
//! The filesystem is deliberately small: no allocation, 64 inodes, 11 direct
//! blocks plus one single-indirect block per file (69 KiB files), and one
//! 16-block write-ahead journal.  A large write is split into per-block
//! atomic transactions rather than claiming whole-file atomicity.
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
pub const MAX_DIRECT_BLOCKS: usize = 11;
pub const INDIRECT_ENTRIES: usize = BLOCK_SIZE / 4;
pub const MAX_FILE_BLOCKS: usize = MAX_DIRECT_BLOCKS + INDIRECT_ENTRIES;
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

pub fn crc32(data: &[u8]) -> u32 {
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
    block[..4].copy_from_slice(b"VF03");
    put_u32(&mut block, 4, 3);
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
        if &block[..4] == b"VF03" && get_u32(block, 4) == 3 {
            (
                3u8,
                MAX_DIRECT_BLOCKS,
                MAX_FILE_BLOCKS,
                MAX_DIR_BLOCKS,
                MAX_FILE_SIZE,
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
    let _ = total;
    Ok(SuperInfo {
        version,
        inode_limit,
        dir_block_limit,
        direct_blocks,
        max_file_blocks,
        max_file_size: size_limit,
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

pub struct Fs<D: BlockDev> {
    dev: D,
    inodes: [Inode; MAX_INODES],
    bitmap: [u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
    journal: [u8; (JOURNAL_BLOCKS as usize - 2) * BLOCK_SIZE],
    targets: [u32; 16],
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
            targets: [0; 16],
            tx_count: 0,
            seq: 1,
            replay_status: 0,
            migrated: false,
            inode_limit: MAX_INODES,
            dir_block_limit: MAX_DIR_BLOCKS,
            version: 3,
            direct_blocks: MAX_DIRECT_BLOCKS,
            max_file_blocks: MAX_FILE_BLOCKS,
            max_file_size: MAX_FILE_SIZE,
        }
    }

    /// Initialize a preallocated filesystem object in place.  Server uses
    /// this so the 13 KiB FS state lives in `.bss`, not on its one-page stack.
    pub fn initialize(&mut self) -> Result<(), FsError> {
        self.inodes = [EMPTY_INODE; MAX_INODES];
        self.bitmap = [0u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE];
        self.journal = [0u8; (JOURNAL_BLOCKS as usize - 2) * BLOCK_SIZE];
        self.targets = [0; 16];
        self.tx_count = 0;
        self.seq = 1;
        self.replay_status = 0;
        self.migrated = false;
        self.inode_limit = MAX_INODES;
        self.dir_block_limit = MAX_DIR_BLOCKS;
        self.version = 3;
        self.direct_blocks = MAX_DIRECT_BLOCKS;
        self.max_file_blocks = MAX_FILE_BLOCKS;
        self.max_file_size = MAX_FILE_SIZE;
        let capacity = self.dev.block_count();
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

    fn fsck_mark_block(
        &self,
        block: u32,
        block_used: &mut [u8; (BITMAP_BLOCKS as usize) * BLOCK_SIZE],
    ) -> Result<(), FsError> {
        if block < DATA_START || block >= self.dev.block_count() {
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
            if inode.kind != INODE_FILE && inode.kind != INODE_DIR {
                return Err(FsError::Corrupt);
            }
            if inode.size as usize > self.max_file_size {
                return Err(FsError::Corrupt);
            }
            if inode.kind == INODE_FILE {
                if inode.blocks_used() != ((inode.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) {
                    return Err(FsError::Corrupt);
                }
            }
            if inode.nlink != nlink[index] {
                return Err(FsError::Corrupt);
            }
            let used_blocks = inode.blocks_used();
            if inode.kind == INODE_FILE {
                if used_blocks > self.max_file_blocks {
                    return Err(FsError::Corrupt);
                }
                for block_index in 0..used_blocks {
                    let block = self.file_block(&inode, block_index)?;
                    self.fsck_mark_block(block, &mut block_used)?;
                }
                let indirect = inode.blocks[self.direct_blocks];
                if used_blocks > self.direct_blocks {
                    if indirect == 0 {
                        return Err(FsError::Corrupt);
                    }
                    self.fsck_mark_block(indirect, &mut block_used)?;
                    let mut table = zero_block();
                    self.dev.read(indirect, &mut table)?;
                    for slot in 0..INDIRECT_ENTRIES {
                        let block = get_u32(&table, slot * 4);
                        if slot >= used_blocks - self.direct_blocks && block != 0 {
                            return Err(FsError::Corrupt);
                        }
                    }
                } else if indirect != 0 {
                    return Err(FsError::Corrupt);
                }
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
        for block in DATA_START..self.dev.block_count() {
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
        if self.tx_count >= 16 {
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
        block < self.dev.block_count()
            && self.bitmap[block as usize / 8] & (1 << (block as usize % 8)) != 0
    }

    fn mark_used(&mut self, block: u32) {
        self.bitmap[block as usize / 8] |= 1 << (block as usize % 8);
    }

    fn mark_free(&mut self, block: u32) {
        self.bitmap[block as usize / 8] &= !(1 << (block as usize % 8));
    }

    fn alloc_block(&mut self) -> Result<u32, FsError> {
        for block in DATA_START..self.dev.block_count() {
            if !self.block_is_used(block) {
                self.mark_used(block);
                return Ok(block);
            }
        }
        Err(FsError::NoSpace)
    }

    fn file_block(&mut self, inode: &Inode, block_index: usize) -> Result<u32, FsError> {
        if inode.kind == INODE_DIR {
            return Ok(inode.blocks.get(block_index).copied().unwrap_or(0));
        }
        if block_index < self.direct_blocks {
            return Ok(inode.blocks[block_index]);
        }
        let indirect = inode.blocks[self.direct_blocks];
        if indirect == 0 {
            return Ok(0);
        }
        let block = self.read_staged(indirect)?;
        Ok(get_u32(&block, (block_index - self.direct_blocks) * 4))
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
        let indirect = inode.blocks[self.direct_blocks];
        let indirect = if indirect == 0 {
            let allocated = self.alloc_block()?;
            self.stage(allocated, &zero_block())?;
            self.stage_bitmap()?;
            inode.blocks[self.direct_blocks] = allocated;
            allocated
        } else {
            indirect
        };
        let mut table = self.read_staged(indirect)?;
        put_u32(&mut table, (block_index - self.direct_blocks) * 4, block_no);
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
                if len == name.len() && &block[off + 3..off + 3 + len] == name {
                    return Ok(Some(ino));
                }
            }
        }
        Ok(None)
    }

    fn lookup_path(&mut self, path: &[u8], uid: u16) -> Result<u16, FsError> {
        if path.is_empty() || path[0] != b'/' || path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        let mut ino = ROOT_INO;
        let mut pos = 0usize;
        while let Some(name) = Self::next_component(path, &mut pos)? {
            let current = self.inodes[ino as usize - 1];
            if current.kind != INODE_DIR {
                return Err(FsError::NotDir);
            }
            if !self.can_read(&current, uid) && uid != 0 {
                // Directory search is not a data read; keep traversal allowed
                // for permission probes such as badge 2 reading /priv.
            }
            ino = self.lookup_child(ino, name)?.ok_or(FsError::NotFound)?;
        }
        Ok(ino)
    }

    fn resolve_parent(
        &mut self,
        path: &[u8],
        uid: u16,
    ) -> Result<(u16, [u8; MAX_NAME], usize), FsError> {
        if path.len() < 2 || path[0] != b'/' || path.len() > MAX_PATH {
            return Err(FsError::Invalid);
        }
        let mut parent = ROOT_INO;
        let mut pos = 0usize;
        let mut last = [0u8; MAX_NAME];
        let mut last_len = 0usize;
        while let Some(name) = Self::next_component(path, &mut pos)? {
            let next_pos = pos;
            let mut probe = next_pos;
            let has_more = match Self::next_component(path, &mut probe)? {
                Some(_) => true,
                None => false,
            };
            if !has_more {
                last[..name.len()].copy_from_slice(name);
                last_len = name.len();
                break;
            }
            let inode = self.inodes[parent as usize - 1];
            if inode.kind != INODE_DIR {
                return Err(FsError::NotDir);
            }
            parent = self.lookup_child(parent, name)?.ok_or(FsError::NotFound)?;
        }
        if last_len == 0 {
            return Err(FsError::Invalid);
        }
        let parent_inode = self.inodes[parent as usize - 1];
        if parent_inode.kind != INODE_DIR {
            return Err(FsError::NotDir);
        }
        if !self.can_write(&parent_inode, uid) {
            return Err(FsError::NoAccess);
        }
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
        self.write_dir_entry(parent, &name[..name_len], ino)?;
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

    pub fn lookup(&mut self, path: &[u8], uid: u16) -> Result<u16, FsError> {
        self.lookup_path(path, uid)
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
            if block_index >= inode.blocks_used() || block_no == 0 {
                return Err(FsError::Corrupt);
            }
            let mut block = zero_block();
            self.dev.read(block_no, &mut block)?;
            let chunk = core::cmp::min(BLOCK_SIZE - in_block, count - done);
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
        while done < data.len() {
            self.tx_count = 0;
            let mut current = *self.inodes.get(ino as usize - 1).ok_or(FsError::BadFd)?;
            let absolute = offset + done;
            let block_index = absolute / BLOCK_SIZE;
            if block_index >= self.max_file_blocks {
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
            self.stage(block_no, &block)?;
            current.size = core::cmp::max(current.size, (absolute + chunk) as u32);
            self.inodes[ino as usize - 1] = current;
            self.stage_inode(ino)?;
            self.commit()?;
            done += chunk;
        }
        Ok(data.len())
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
            let indirect = current.blocks[self.direct_blocks];
            let keep_indirect = new_blocks > self.direct_blocks;
            for index in new_blocks..old_blocks {
                let block = self.file_block(&current, index)?;
                if block != 0 {
                    self.mark_free(block);
                }
                if index < self.direct_blocks {
                    current.blocks[index] = 0;
                } else if keep_indirect && indirect != 0 {
                    self.set_file_block(&mut current, index, 0)?;
                }
            }
            if !keep_indirect && indirect != 0 {
                self.mark_free(indirect);
                current.blocks[self.direct_blocks] = 0;
            }
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

    pub fn read_dir_entry(
        &mut self,
        dir: u16,
        index: usize,
    ) -> Result<Option<(u16, [u8; MAX_NAME], usize)>, FsError> {
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
                if seen == index {
                    let mut name = [0u8; MAX_NAME];
                    let len = block[off + 2] as usize;
                    name[..len].copy_from_slice(&block[off + 3..off + 3 + len]);
                    return Ok(Some((ino, name, len)));
                }
                seen += 1;
            }
        }
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

    pub fn fsync(&mut self) -> Result<(), FsError> {
        Ok(())
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
        if self.kind == INODE_DIR {
            return (self.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE;
        }
        (self.size as usize + BLOCK_SIZE - 1) / BLOCK_SIZE
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
        assert_eq!(
            fs.write(file, 0, &[0u8; MAX_FILE_SIZE + 1], 1),
            Err(FsError::NoSpace)
        );
        assert_eq!(
            fs.write(file, 0, &[0u8; MAX_FILE_SIZE], 1).unwrap(),
            MAX_FILE_SIZE
        );
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
        let mut model = [0u8; MAX_FILE_SIZE];
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
                let old_model = model;
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
                let mut out = [0u8; MAX_FILE_SIZE];
                let count = mounted.read(ino, 0, &mut out, 1).unwrap();
                let mut expected = old_model;
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
            fs.write(full, 0, &[0x11; 11 * BLOCK_SIZE], 1).unwrap(),
            11 * BLOCK_SIZE
        );
        let rest = fs.create(b"/rest", 1, INODE_FILE).unwrap();
        assert_eq!(
            fs.write(rest, 0, &[0x22; 6 * BLOCK_SIZE], 1).unwrap(),
            6 * BLOCK_SIZE
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
        fs.write(full, 0, &[0x11; 11 * BLOCK_SIZE], 1).unwrap();
        let rest = fs.create(b"/rest", 1, INODE_FILE).unwrap();
        fs.write(rest, 0, &[0x22; 6 * BLOCK_SIZE], 1).unwrap();
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
            let ind = inode.blocks[MAX_DIRECT_BLOCKS];
            std::println!(
                "DEBUG indirect={} data0={} data10={} data11={}",
                ind,
                inode.blocks[0],
                inode.blocks[10],
                inode.blocks[11]
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
    fn crash_during_first_indirect_block_allocation_keeps_fsck_clean() {
        let mut fs = fresh();
        let ino = fs.create(b"/indirect", 1, INODE_FILE).unwrap();
        fs.write(ino, 0, &[0x5a; 11 * BLOCK_SIZE], 1).unwrap();
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
            .write(ino, 11 * BLOCK_SIZE, &[0xa5; BLOCK_SIZE], 1)
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
            let _ = crashed.write(ino, 11 * BLOCK_SIZE, &[0xa5; BLOCK_SIZE], 1);
            let image = crashed.dev.data;
            let mut mounted = Fs::mount(MemDev { data: image }).unwrap();
            mounted.fsck().unwrap();
            let size = mounted.info(ino).unwrap().size as usize;
            assert!(size == 11 * BLOCK_SIZE || size == 12 * BLOCK_SIZE);
        }
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
}
