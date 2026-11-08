//! Minimal, allocation-free ELF64 loader validation.
//!
//! The crate only parses and validates the parts a microkernel root task needs:
//! the ELF header and `PT_LOAD` program headers.  Mapping is deliberately left
//! to the caller so this logic can be host-tested without unsafe code.

#![no_std]
#![forbid(unsafe_code)]

pub const ELF_HEADER_SIZE: usize = 64;
pub const PROGRAM_HEADER_SIZE: usize = 56;
pub const PAGE_SIZE: u64 = 4096;
pub const MAX_LOAD_SEGMENTS: usize = 16;

pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_TLS: u32 = 7;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;
pub const ET_EXEC: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    Not64Bit,
    NotLittleEndian,
    BadIdentVersion,
    NotExecutable,
    BadVersion,
    BadHeaderSize,
    BadProgramHeaderSize,
    TooManySegments,
    HeaderOutOfBounds,
    BadProgramHeader,
    BadAlignment,
    Misaligned,
    FileRange,
    MemorySmallerThanFile,
    Overflow,
    OutsideUserRange,
    BadUserRange,
    WrongMachine,
    UnsupportedSegment,
    NotReadable,
    WriteAndExecute,
    Overlap,
    NoSegments,
    BadEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadSegment {
    pub file_offset: usize,
    pub file_size: usize,
    pub memory_size: usize,
    pub vaddr: u64,
    pub flags: u32,
}

impl LoadSegment {
    pub fn readable(&self) -> bool {
        self.flags & PF_R != 0
    }

    pub fn writable(&self) -> bool {
        self.flags & PF_W != 0
    }

    pub fn executable(&self) -> bool {
        self.flags & PF_X != 0
    }

    pub fn page_count(&self) -> usize {
        (self.memory_size + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize
    }

    pub fn memory_end(&self) -> Option<u64> {
        let pages = self.page_count() as u64;
        self.vaddr.checked_add(pages.checked_mul(PAGE_SIZE)?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfImage {
    pub entry: u64,
    pub machine: u16,
    segments: [Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    count: usize,
}

impl ElfImage {
    pub fn segment_count(&self) -> usize {
        self.count
    }

    pub fn segment(&self, index: usize) -> Option<&LoadSegment> {
        self.segments.get(index).and_then(Option::as_ref)
    }

    pub fn segments(&self) -> impl Iterator<Item = &LoadSegment> {
        self.segments.iter().flatten()
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn page_round_up(value: u64) -> Option<u64> {
    let mask = PAGE_SIZE - 1;
    value.checked_add(mask).map(|v| v & !mask)
}

fn ranges_overlap(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> bool {
    a_start < b_end && b_start < a_end
}

/// Parse an ELF64 little-endian executable for `expected_machine` inside
/// `[user_start, user_end)`.
///
/// The caller owns the architecture constants; this crate never redefines the
/// user address range.  `PT_LOAD` offsets and virtual addresses must be
/// congruent modulo the page size, all ranges must fit the supplied user
/// window, and no segment may be writable and executable together.
pub fn parse(
    bytes: &[u8],
    expected_machine: u16,
    user_start: u64,
    user_end: u64,
) -> Result<ElfImage, ElfError> {
    parse_impl(bytes, expected_machine, user_start, user_end, true)
}

/// Parse chỉ ELF header + program header table (không cần toàn bộ nội dung
/// segment trong `bytes`). Dùng cho exec-from-FS: loader đọc 1KiB đầu file
/// để lấy entry/segment, rồi nạp từng page trực tiếp từ FS vào frame đích
/// (mốc 9, vòng 84). Mọi kiểm tra khác giữ nguyên như `parse`.
pub fn parse_metadata(
    bytes: &[u8],
    expected_machine: u16,
    user_start: u64,
    user_end: u64,
) -> Result<ElfImage, ElfError> {
    parse_impl(bytes, expected_machine, user_start, user_end, false)
}

fn parse_impl(
    bytes: &[u8],
    expected_machine: u16,
    user_start: u64,
    user_end: u64,
    require_file_bytes: bool,
) -> Result<ElfImage, ElfError> {
    if bytes.len() < ELF_HEADER_SIZE {
        return Err(ElfError::TooSmall);
    }
    if user_start >= user_end {
        return Err(ElfError::BadUserRange);
    }
    if bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        return Err(ElfError::BadMagic);
    }
    if bytes[4] != 2 {
        return Err(ElfError::Not64Bit);
    }
    if bytes[5] != 1 {
        return Err(ElfError::NotLittleEndian);
    }
    if bytes[6] != 1 {
        return Err(ElfError::BadIdentVersion);
    }
    if u16_at(bytes, 16) != Some(ET_EXEC) {
        return Err(ElfError::NotExecutable);
    }
    if u32_at(bytes, 20) != Some(1) {
        return Err(ElfError::BadVersion);
    }
    let machine = u16_at(bytes, 18).ok_or(ElfError::TooSmall)?;
    if machine != expected_machine {
        return Err(ElfError::WrongMachine);
    }
    let entry = u64_at(bytes, 24).ok_or(ElfError::TooSmall)?;
    let phoff = u64_at(bytes, 32).ok_or(ElfError::TooSmall)?;
    let ehsize = u16_at(bytes, 52).ok_or(ElfError::TooSmall)? as usize;
    let phentsize = u16_at(bytes, 54).ok_or(ElfError::TooSmall)? as usize;
    let phnum = u16_at(bytes, 56).ok_or(ElfError::TooSmall)? as usize;

    if ehsize < ELF_HEADER_SIZE {
        return Err(ElfError::BadHeaderSize);
    }
    if phentsize != PROGRAM_HEADER_SIZE {
        return Err(ElfError::BadProgramHeaderSize);
    }
    if phnum == 0 {
        return Err(ElfError::NoSegments);
    }
    if phnum > MAX_LOAD_SEGMENTS {
        return Err(ElfError::TooManySegments);
    }
    let phoff = usize::try_from(phoff).map_err(|_| ElfError::HeaderOutOfBounds)?;
    let table_len = phnum.checked_mul(phentsize).ok_or(ElfError::Overflow)?;
    let table_end = phoff.checked_add(table_len).ok_or(ElfError::Overflow)?;
    if table_end > bytes.len() {
        return Err(ElfError::HeaderOutOfBounds);
    }

    let mut image = ElfImage {
        entry,
        machine,
        segments: [None; MAX_LOAD_SEGMENTS],
        count: 0,
    };

    for index in 0..phnum {
        let base = phoff + index * phentsize;
        let p_type = u32_at(bytes, base).ok_or(ElfError::BadProgramHeader)?;
        match p_type {
            PT_LOAD => {}
            PT_DYNAMIC | PT_INTERP | PT_TLS => {
                return Err(ElfError::UnsupportedSegment);
            }
            // Linkers commonly emit these non-mapped metadata entries.
            _ => continue,
        }
        let flags = u32_at(bytes, base + 4).ok_or(ElfError::BadProgramHeader)?;
        let offset = u64_at(bytes, base + 8).ok_or(ElfError::BadProgramHeader)?;
        let vaddr = u64_at(bytes, base + 16).ok_or(ElfError::BadProgramHeader)?;
        let file_size = u64_at(bytes, base + 32).ok_or(ElfError::BadProgramHeader)?;
        let memory_size = u64_at(bytes, base + 40).ok_or(ElfError::BadProgramHeader)?;
        let align = u64_at(bytes, base + 48).ok_or(ElfError::BadProgramHeader)?;

        if flags & PF_R == 0 {
            return Err(ElfError::NotReadable);
        }
        if flags & PF_W != 0 && flags & PF_X != 0 {
            return Err(ElfError::WriteAndExecute);
        }
        if memory_size == 0 {
            continue;
        }
        if file_size > memory_size {
            return Err(ElfError::MemorySmallerThanFile);
        }
        if align != 0 && (!align.is_power_of_two() || align > PAGE_SIZE) {
            return Err(ElfError::BadAlignment);
        }
        if vaddr % PAGE_SIZE != offset % PAGE_SIZE {
            return Err(ElfError::Misaligned);
        }

        let file_offset = usize::try_from(offset).map_err(|_| ElfError::FileRange)?;
        let file_size_usize = usize::try_from(file_size).map_err(|_| ElfError::FileRange)?;
        let file_end = file_offset
            .checked_add(file_size_usize)
            .ok_or(ElfError::Overflow)?;
        if require_file_bytes && file_end > bytes.len() {
            return Err(ElfError::FileRange);
        }

        let memory_size_usize = usize::try_from(memory_size).map_err(|_| ElfError::Overflow)?;
        let memory_end = page_round_up(vaddr.checked_add(memory_size).ok_or(ElfError::Overflow)?)
            .ok_or(ElfError::Overflow)?;
        if vaddr < user_start || memory_end > user_end {
            return Err(ElfError::OutsideUserRange);
        }

        for previous in image.segments.iter().flatten() {
            let previous_end = previous.memory_end().ok_or(ElfError::Overflow)?;
            if ranges_overlap(vaddr, memory_end, previous.vaddr, previous_end) {
                return Err(ElfError::Overlap);
            }
        }

        let segment = LoadSegment {
            file_offset,
            file_size: file_size_usize,
            memory_size: memory_size_usize,
            vaddr,
            flags,
        };
        image.segments[image.count] = Some(segment);
        image.count += 1;
    }

    if image.count == 0 {
        return Err(ElfError::NoSegments);
    }
    let entry_is_executable = image.segments.iter().flatten().any(|segment| {
        segment.executable()
            && entry >= segment.vaddr
            && entry < segment.vaddr.saturating_add(segment.memory_size as u64)
    });
    if !entry_is_executable {
        return Err(ElfError::BadEntry);
    }
    Ok(image)
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn phdr(bytes: &mut [u8], index: usize, offset: u64, vaddr: u64, filesz: u64, memsz: u64) {
        let base = ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE;
        put_u32(bytes, base, PT_LOAD);
        put_u32(bytes, base + 4, PF_R | PF_X);
        put_u64(bytes, base + 8, offset);
        put_u64(bytes, base + 16, vaddr);
        put_u64(bytes, base + 32, filesz);
        put_u64(bytes, base + 40, memsz);
        put_u64(bytes, base + 48, PAGE_SIZE);
    }

    const TEST_MACHINE: u16 = 0x3e;
    const TEST_USER_START: u64 = 0x0000_0010_0000_0000;
    const TEST_USER_END: u64 = 0x0000_0020_0000_0000;

    fn parse_test(bytes: &[u8]) -> Result<ElfImage, ElfError> {
        parse(bytes, TEST_MACHINE, TEST_USER_START, TEST_USER_END)
    }

    fn metadata_test(bytes: &[u8]) -> Result<ElfImage, ElfError> {
        parse_metadata(bytes, TEST_MACHINE, TEST_USER_START, TEST_USER_END)
    }

    fn valid_elf() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x3000];
        bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        put_u16(&mut bytes, 16, ET_EXEC);
        put_u16(&mut bytes, 18, 0x3e);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 24, TEST_USER_START + 0x100);
        put_u64(&mut bytes, 32, ELF_HEADER_SIZE as u64);
        put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16);
        put_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16);
        put_u16(&mut bytes, 56, 1);
        phdr(&mut bytes, 0, 0x1000, TEST_USER_START, 0x100, 0x1000);
        bytes[0x1000..0x1100].fill(0x90);
        bytes
    }

    #[test]
    fn accepts_page_aligned_executable_segment() {
        let image = parse_test(&valid_elf()).unwrap();
        assert_eq!(image.segment_count(), 1);
        let segment = image.segment(0).unwrap();
        assert_eq!(segment.vaddr, TEST_USER_START);
        assert_eq!(segment.page_count(), 1);
        assert!(segment.readable());
        assert!(segment.executable());
        assert!(!segment.writable());
    }

    #[test]
    fn metadata_parse_accepts_header_plus_phdrs_only() {
        // Mốc 9 (vòng 84): exec-from-FS đọc 1KiB đầu file (header + phdr)
        // rồi nạp segment theo page. `parse` đòi toàn bộ segment trong
        // buffer (FileRange), `parse_metadata` chỉ cần header + phdr table.
        let full = valid_elf();
        assert!(parse_test(&full).is_ok());
        // Chỉ 1KiB: đủ header(64) + phdr(56) nhưng thiếu thân segment.
        let head = &full[..1024];
        assert_eq!(parse_test(head), Err(ElfError::FileRange));
        let image = metadata_test(head).expect("metadata parse");
        assert_eq!(image.entry, u64_at(&full, 24).unwrap());
        assert_eq!(image.segment_count(), 1);
        let segment = image.segment(0).unwrap();
        // Metadata trùng khớp với parse đầy đủ (cùng file).
        let reference = parse_test(&full).unwrap().segment(0).unwrap().clone();
        assert_eq!(segment.file_offset, reference.file_offset);
        assert_eq!(segment.vaddr, reference.vaddr);
        assert_eq!(segment.file_size, reference.file_size);
        assert!(segment.executable());
        // Header hỏng vẫn bị từ chối.
        let mut bad = full[..1024].to_vec();
        bad[0] = 0;
        assert_eq!(metadata_test(&bad), Err(ElfError::BadMagic));
    }

    #[test]
    fn accepts_unaligned_entry_inside_segment() {
        // Mốc 9 (vòng 100): ảnh ELF nhỏ đặt code ở offset 0x80 trong segment
        // và e_entry = vaddr + 0x80 (entry KHÔNG page-aligned). parse phải
        // chấp nhận: entry chỉ cần nằm trong segment, không cần trùng địa
        // chỉ trang — đây là điều kiện để thu nhỏ ảnh demo tiết kiệm headroom.
        let mut bytes = vec![0u8; 0x1100];
        bytes[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        bytes[16] = 2; // ET_EXEC
        put_u16(&mut bytes, 18, TEST_MACHINE);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 24, TEST_USER_START + 0x1000 + 0x80); // e_entry
        put_u64(&mut bytes, 32, 64); // e_phoff
        bytes[52] = 64;
        bytes[54] = 56;
        bytes[56] = 1;
        // PT_LOAD: offset 0x1000, vaddr USER_START+0x1000, filesz/memsz 0x100
        let base = ELF_HEADER_SIZE;
        put_u32(&mut bytes, base, PT_LOAD);
        put_u32(&mut bytes, base + 4, PF_R | PF_X);
        put_u64(&mut bytes, base + 8, 0x1000);
        put_u64(&mut bytes, base + 16, TEST_USER_START + 0x1000);
        put_u64(&mut bytes, base + 32, 0x100);
        put_u64(&mut bytes, base + 40, 0x100);
        put_u64(&mut bytes, base + 48, PAGE_SIZE);
        let image = parse_test(&bytes).expect("unaligned entry");
        assert_eq!(image.entry, TEST_USER_START + 0x1080);
        assert_eq!(image.segment_count(), 1);
        let segment = image.segment(0).unwrap();
        assert_eq!(segment.page_count(), 1);
        // Loader: trang được map tại page_base và code rơi vào offset 0x80.
        let page_base = segment.vaddr & !(PAGE_SIZE - 1);
        assert_eq!(page_base, TEST_USER_START + 0x1000);
        assert_eq!(image.entry - page_base, 0x80);
    }

    #[test]
    fn rejects_bad_magic_and_class() {
        let mut bytes = valid_elf();
        bytes[0] = 0;
        assert_eq!(parse_test(&bytes), Err(ElfError::BadMagic));
        let mut bytes = valid_elf();
        bytes[4] = 1;
        assert_eq!(parse_test(&bytes), Err(ElfError::Not64Bit));
    }

    #[test]
    fn rejects_bad_program_header_table() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, 32, u64::MAX);
        assert_eq!(parse_test(&bytes), Err(ElfError::Overflow));

        let mut bytes = valid_elf();
        put_u64(&mut bytes, 32, 0x4000);
        assert_eq!(parse_test(&bytes), Err(ElfError::HeaderOutOfBounds));

        let mut bytes = valid_elf();
        put_u16(&mut bytes, 56, (MAX_LOAD_SEGMENTS + 1) as u16);
        assert_eq!(parse_test(&bytes), Err(ElfError::TooManySegments));
    }

    #[test]
    fn rejects_file_and_memory_ranges() {
        let mut bytes = valid_elf();
        let base = ELF_HEADER_SIZE;
        put_u64(&mut bytes, base + 32, 0x3000);
        put_u64(&mut bytes, base + 40, 0x3000);
        assert_eq!(parse_test(&bytes), Err(ElfError::FileRange));

        let mut bytes = valid_elf();
        put_u64(&mut bytes, base + 40, 0x80);
        assert_eq!(parse_test(&bytes), Err(ElfError::MemorySmallerThanFile));
    }

    #[test]
    fn rejects_load_segment_offset_overflow() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 8, u64::MAX & !0xfff);
        assert!(matches!(
            parse_test(&bytes),
            Err(ElfError::Overflow | ElfError::FileRange)
        ));
    }

    #[test]
    fn rejects_unaligned_and_wx_segments() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 16, TEST_USER_START + 1);
        assert_eq!(parse_test(&bytes), Err(ElfError::Misaligned));

        let mut bytes = valid_elf();
        put_u32(&mut bytes, ELF_HEADER_SIZE + 4, PF_R | PF_W | PF_X);
        assert_eq!(parse_test(&bytes), Err(ElfError::WriteAndExecute));
    }

    #[test]
    fn rejects_outside_user_range_and_overlap() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 16, 0x1000);
        assert_eq!(parse_test(&bytes), Err(ElfError::OutsideUserRange));

        let mut bytes = valid_elf();
        put_u16(&mut bytes, 56, 2);
        phdr(&mut bytes, 1, 0x2000, TEST_USER_START, 0x100, 0x1000);
        assert_eq!(parse_test(&bytes), Err(ElfError::Overlap));
    }

    #[test]
    fn rejects_bad_alignment_and_entry() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 48, 3);
        assert_eq!(parse_test(&bytes), Err(ElfError::BadAlignment));

        let mut bytes = valid_elf();
        put_u64(&mut bytes, 24, TEST_USER_START + 0x2000);
        assert_eq!(parse_test(&bytes), Err(ElfError::BadEntry));
    }

    #[test]
    fn rejects_dynamic_interpreted_and_tls_segments() {
        for segment_type in [PT_DYNAMIC, PT_INTERP, PT_TLS] {
            let mut bytes = valid_elf();
            put_u32(&mut bytes, ELF_HEADER_SIZE, segment_type);
            assert_eq!(parse_test(&bytes), Err(ElfError::UnsupportedSegment));
        }
    }

    #[test]
    fn rejects_wrong_machine_and_non_exec_type() {
        let mut bytes = valid_elf();
        put_u16(&mut bytes, 18, 0x03);
        assert_eq!(parse_test(&bytes), Err(ElfError::WrongMachine));

        let mut bytes = valid_elf();
        put_u16(&mut bytes, 16, 3);
        assert_eq!(parse_test(&bytes), Err(ElfError::NotExecutable));
    }

    #[test]
    fn rejects_image_outside_narrow_root_window() {
        let bytes = valid_elf();
        let root_base = TEST_USER_START + 0x20_0000;
        assert_eq!(
            parse(&bytes, TEST_MACHINE, root_base, root_base + 0x10_0000),
            Err(ElfError::OutsideUserRange)
        );
    }

    #[test]
    fn rejects_bad_user_range() {
        let bytes = valid_elf();
        assert_eq!(
            parse(&bytes, TEST_MACHINE, TEST_USER_START, TEST_USER_START),
            Err(ElfError::BadUserRange)
        );
    }

    #[test]
    fn rejects_memory_range_overflow() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 40, u64::MAX);
        assert_eq!(parse_test(&bytes), Err(ElfError::Overflow));
    }

    #[test]
    fn rejects_offset_vaddr_congruence_mismatch() {
        let mut bytes = valid_elf();
        put_u64(&mut bytes, ELF_HEADER_SIZE + 8, 0x1001);
        assert_eq!(parse_test(&bytes), Err(ElfError::Misaligned));
    }

    #[test]
    fn rejects_segments_sharing_a_page_even_without_byte_overlap() {
        let mut bytes = valid_elf();
        put_u16(&mut bytes, 56, 2);
        let base = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;
        put_u32(&mut bytes, base, PT_LOAD);
        put_u32(&mut bytes, base + 4, PF_R);
        put_u64(&mut bytes, base + 8, 0x1800);
        put_u64(&mut bytes, base + 16, TEST_USER_START + 0x800);
        put_u64(&mut bytes, base + 32, 0x100);
        put_u64(&mut bytes, base + 40, 0x100);
        put_u64(&mut bytes, base + 48, PAGE_SIZE);
        assert_eq!(parse_test(&bytes), Err(ElfError::Overlap));
    }

    #[test]
    fn rejects_load_segment_without_read_permission() {
        let mut bytes = valid_elf();
        put_u32(&mut bytes, ELF_HEADER_SIZE + 4, PF_X);
        assert_eq!(parse_test(&bytes), Err(ElfError::NotReadable));
    }

    #[test]
    fn rejects_missing_load_segments() {
        let mut bytes = valid_elf();
        put_u16(&mut bytes, 56, 0);
        assert_eq!(parse_test(&bytes), Err(ElfError::NoSegments));
    }
}
