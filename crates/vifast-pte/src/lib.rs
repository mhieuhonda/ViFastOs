//! Safe, host-testable page-table descriptor formats.
//!
//! No MMU access happens here: these are pure bit encoders/decoders used by
//! the portable page-table walker and by every platform port.

#![no_std]
#![forbid(unsafe_code)]

use vifast_abi::{Rights, DEFAULT_PAGE_SIZE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pte {
    Invalid,
    Table(u64),
    Leaf(u64, Rights),
}

/// User virtual range shared by the three current ports.  Keeping this in the
/// safe PTE crate lets host tests check the real numbers used by the platforms.
pub const USER_VA_START: u64 = vifast_abi::user_layout::USER_VA_START as u64;
pub const USER_VA_END: u64 = vifast_abi::user_layout::USER_VA_END as u64;
pub const EM_RISCV: u16 = 243;
pub const EM_AARCH64: u16 = 183;
pub const EM_X86_64: u16 = 62;

/// Number of virtual addresses covered by a page-table level.
///
/// Level 0 is the root.  A level-3 table on a 4-level format covers one 4 KiB
/// page; a level-2 table covers 2 MiB; a level-1 table covers 1 GiB.  Returning
/// zero for an invalid level keeps the helper safe to call from assertions and
/// tests.
pub const fn table_span(levels: usize, level: usize, page_size: u64) -> u64 {
    if levels == 0 || level >= levels {
        return 0;
    }
    let shift = 9 * (levels - 1 - level);
    match page_size.checked_shl(shift as u32) {
        Some(span) => span,
        None => 0,
    }
}

pub const fn table_span_default(levels: usize, level: usize) -> u64 {
    table_span(levels, level, DEFAULT_PAGE_SIZE)
}

fn rights(r: bool, w: bool, x: bool) -> Rights {
    let mut out = Rights::NONE;
    if r {
        out = out | Rights::READ;
    }
    if w {
        out = out | Rights::WRITE;
    }
    if x {
        out = out | Rights::EXEC;
    }
    out
}

pub mod riscv {
    use super::*;

    pub const LEVELS: usize = 3;
    pub const ELF_MACHINE: u16 = crate::EM_RISCV;
    pub const USER_VA_START: u64 = crate::USER_VA_START;
    pub const USER_VA_END: u64 = crate::USER_VA_END;
    /// Root entries copied from the kernel root into a retyped VSpace:
    /// entry 0 is the low device map, entry 2 is the 0x8000_0000 kernel RAM.
    pub const KERNEL_SHARED_SLOTS: &[usize] = &[0, 2];

    pub const fn table_span(level: usize) -> u64 {
        crate::table_span(LEVELS, level, DEFAULT_PAGE_SIZE)
    }
    pub const PTE_V: u64 = 1;
    pub const PTE_R: u64 = 2;
    pub const PTE_W: u64 = 4;
    pub const PTE_X: u64 = 8;
    pub const PTE_U: u64 = 16;
    /// A (accessed) / D (dirty): kernel này không xử lý page fault do thiếu
    /// A/D (không có Svade/Svadu handler), nên luôn bật sẵn — đúng đặc tả cho
    /// kiểu kernel "set once" và tránh fault phụ thuộc cấu hình CPU.
    pub const PTE_A: u64 = 64;
    pub const PTE_D: u64 = 128;
    const PTE_PPN_SHIFT: u64 = 10;

    pub fn index(va: u64, level: usize) -> usize {
        ((va >> (12 + 9 * (LEVELS - 1 - level))) & 0x1ff) as usize
    }

    pub fn encode_leaf(pa: u64, access: Rights) -> u64 {
        let mut flags = PTE_V | PTE_U | PTE_A | PTE_D;
        let writable = access.contains(Rights::WRITE);
        if access.contains(Rights::READ) || writable {
            flags |= PTE_R;
        }
        if writable {
            flags |= PTE_W;
        }
        if access.contains(Rights::EXEC) {
            flags |= PTE_X;
        }
        ((pa >> 12) << PTE_PPN_SHIFT) | flags
    }

    pub fn encode_table(pa: u64) -> u64 {
        ((pa >> 12) << PTE_PPN_SHIFT) | PTE_V
    }

    pub fn decode_at(entry: u64, _level: usize) -> Pte {
        if entry & PTE_V == 0 {
            return Pte::Invalid;
        }
        let pa = (entry >> PTE_PPN_SHIFT) << 12;
        if entry & (PTE_R | PTE_W | PTE_X) != 0 {
            Pte::Leaf(
                pa,
                rights(entry & PTE_R != 0, entry & PTE_W != 0, entry & PTE_X != 0),
            )
        } else {
            Pte::Table(pa)
        }
    }

    pub fn decode(entry: u64) -> Pte {
        decode_at(entry, LEVELS - 1)
    }

    pub fn valid_rights(access: Rights) -> bool {
        !access.contains(Rights::WRITE) || access.contains(Rights::READ)
    }

    pub fn is_user(entry: u64) -> bool {
        entry & PTE_U != 0
    }
}

pub mod x86_64 {
    use super::*;

    pub const LEVELS: usize = 4;
    pub const ELF_MACHINE: u16 = crate::EM_X86_64;
    pub const USER_VA_START: u64 = crate::USER_VA_START;
    pub const USER_VA_END: u64 = crate::USER_VA_END;
    /// PDPT[0] points at the shared 0..1 GiB kernel PD.  PML4[0] itself is
    /// private to each VSpace and points at the private PDPT.
    pub const KERNEL_SHARED_SLOTS: &[usize] = &[0, 3];

    pub const fn table_span(level: usize) -> u64 {
        crate::table_span(LEVELS, level, DEFAULT_PAGE_SIZE)
    }
    pub const P: u64 = 1;
    pub const W: u64 = 2;
    pub const U: u64 = 4;
    pub const PS: u64 = 0x80;
    pub const NX: u64 = 1 << 63;
    pub const PA_MASK: u64 = 0x000f_ffff_ffff_f000;

    pub fn index(va: u64, level: usize) -> usize {
        ((va >> (12 + 9 * (LEVELS - 1 - level))) & 0x1ff) as usize
    }

    pub fn encode_leaf(pa: u64, access: Rights) -> u64 {
        let mut flags = P | U;
        if access.contains(Rights::WRITE) {
            flags |= W;
        }
        if !access.contains(Rights::EXEC) {
            flags |= NX;
        }
        (pa & PA_MASK) | flags
    }

    pub fn encode_table(pa: u64) -> u64 {
        (pa & PA_MASK) | P | W | U
    }

    pub fn decode_at(entry: u64, level: usize) -> Pte {
        if entry & P == 0 {
            return Pte::Invalid;
        }
        let pa = entry & PA_MASK;
        if level == LEVELS - 1 || entry & PS != 0 {
            Pte::Leaf(pa, rights(true, entry & W != 0, entry & NX == 0))
        } else {
            Pte::Table(pa)
        }
    }

    pub fn decode(entry: u64) -> Pte {
        decode_at(entry, LEVELS - 1)
    }

    pub fn is_user(entry: u64) -> bool {
        entry & U != 0
    }
}

pub mod aarch64 {
    use super::*;

    pub const LEVELS: usize = 4;
    pub const ELF_MACHINE: u16 = crate::EM_AARCH64;
    pub const USER_VA_START: u64 = crate::USER_VA_START;
    pub const USER_VA_END: u64 = crate::USER_VA_END;
    /// L1[0] contains the low MMIO blocks (GIC/PL011); L1[1] contains the
    /// 0x4000_0000 kernel RAM blocks.  Both tables are shared kernel tables.
    pub const KERNEL_SHARED_SLOTS: &[usize] = &[0, 1];

    pub const fn table_span(level: usize) -> u64 {
        crate::table_span(LEVELS, level, DEFAULT_PAGE_SIZE)
    }
    pub const AF: u64 = 1 << 10;
    pub const SH_INNER: u64 = 0b11 << 8;
    pub const AP_USER_RW: u64 = 1 << 6;
    pub const AP_USER_RO: u64 = (1 << 6) | (1 << 7);
    pub const PXN: u64 = 1 << 53;
    pub const UXN: u64 = 1 << 54;
    /// P0-1 (r289): bit 11 = nG (non-global). Bắt buộc với mọi leaf user, nếu
    /// không `tlbi aside1is` (theo ASID) sẽ KHÔNG xoá entry và hai VSpace cùng
    /// VA sẽ thấy entry của nhau.
    pub const NG: u64 = 1 << 11;
    pub const PA_MASK: u64 = 0x0000_ffff_ffff_f000;

    pub fn index(va: u64, level: usize) -> usize {
        ((va >> (12 + 9 * (LEVELS - 1 - level))) & 0x1ff) as usize
    }

    pub fn encode_leaf(pa: u64, access: Rights) -> u64 {
        let ap = if access.contains(Rights::WRITE) {
            AP_USER_RW
        } else {
            AP_USER_RO
        };
        let uxn = if access.contains(Rights::EXEC) {
            0
        } else {
            UXN
        };
        // P0-1 (r289): nG cho MỌI leaf user (xem `NG`).
        (pa & PA_MASK) | 0b11 | AF | SH_INNER | ap | PXN | uxn | NG
    }

    pub fn encode_table(pa: u64) -> u64 {
        (pa & PA_MASK) | 0b11
    }

    pub fn decode_at(entry: u64, level: usize) -> Pte {
        if entry & 0b11 == 0 {
            return Pte::Invalid;
        }
        let pa = entry & PA_MASK;
        let writable = entry & (1 << 7) == 0 && entry & AP_USER_RW != 0;
        let executable = entry & UXN == 0;
        let access = rights(true, writable, executable);
        if level == LEVELS - 1 {
            Pte::Leaf(pa, access)
        } else if entry & 0b11 == 0b01 {
            Pte::Leaf(pa, access)
        } else {
            Pte::Table(pa)
        }
    }

    pub fn decode(entry: u64) -> Pte {
        decode_at(entry, LEVELS - 1)
    }

    pub fn is_user(entry: u64) -> bool {
        entry & (0b11 << 6) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P0-1 (r289, đỏ-trước): mọi leaf user của aarch64 phải có bit nG (11).
    ///
    /// Thiếu nG thì entry là "global", mà `tlbi aside1is` **không** xoá entry
    /// global; hai VSpace cùng map một VA (client/server đều map `CODE_VA`) sẽ
    /// thấy entry của VSpace trước ⇒ chạy nhầm mã. QEMU flush rộng tay nên lỗi
    /// này xanh trên QEMU và sai trên máy thật.
    #[test]
    fn aarch64_leaf_is_non_global() {
        for access in [
            Rights::READ,
            Rights::READ | Rights::WRITE,
            Rights::READ | Rights::EXEC,
            Rights::ALL,
        ] {
            let leaf = aarch64::encode_leaf(0x1234_5000, access);
            assert!(
                leaf & (1 << 11) != 0,
                "leaf aarch64 phải có nG (bit 11), entry={:#x}",
                leaf
            );
        }
        // Bảng (table) thì không mang nG — chỉ leaf mới có nghĩa TLB.
        let table = aarch64::encode_table(0x1234_5000);
        assert_eq!(table & (1 << 11), 0, "table descriptor không được có nG");
    }

    #[test]
    fn riscv_roundtrip_and_wx() {
        let leaf = riscv::encode_leaf(0x1234_5000, Rights::READ | Rights::EXEC);
        match riscv::decode(leaf) {
            Pte::Leaf(pa, rights) => {
                assert_eq!(pa, 0x1234_5000);
                assert!(rights.contains(Rights::READ));
                assert!(rights.contains(Rights::EXEC));
                assert!(!rights.contains(Rights::WRITE));
            }
            _ => panic!("not leaf"),
        }
        let table = riscv::encode_table(0x2000);
        assert_eq!(riscv::decode(table), Pte::Table(0x2000));
        assert_eq!(riscv::index(0x10_0000_0000, 0), 64);
        assert_eq!(riscv::index(0xffff_ffff, 0), 3);
    }

    #[test]
    fn x86_roundtrip_and_nx() {
        let leaf = x86_64::encode_leaf(0x9000, Rights::READ | Rights::WRITE);
        match x86_64::decode_at(leaf, 3) {
            Pte::Leaf(pa, rights) => {
                assert_eq!(pa, 0x9000);
                assert!(rights.contains(Rights::WRITE));
                assert!(!rights.contains(Rights::EXEC));
            }
            _ => panic!("not leaf"),
        }
        let table = x86_64::encode_table(0x4000);
        assert_eq!(x86_64::decode_at(table, 2), Pte::Table(0x4000));
    }

    #[test]
    fn aarch64_table_page_and_block() {
        let table = aarch64::encode_table(0x4000);
        assert_eq!(aarch64::decode_at(table, 1), Pte::Table(0x4000));
        let page = aarch64::encode_leaf(0x5000, Rights::READ | Rights::EXEC);
        assert_eq!(
            aarch64::decode_at(page, 3),
            Pte::Leaf(0x5000, Rights::READ | Rights::EXEC)
        );
        let block = (0x4000_0000u64) | 0b01 | aarch64::AF;
        match aarch64::decode_at(block, 1) {
            Pte::Leaf(pa, _) => assert_eq!(pa, 0x4000_0000),
            _ => panic!("not block"),
        }
    }

    #[test]
    fn elf_machine_constants_match_architectures() {
        assert_eq!(riscv::ELF_MACHINE, 243);
        assert_eq!(aarch64::ELF_MACHINE, 183);
        assert_eq!(x86_64::ELF_MACHINE, 62);
    }

    #[test]
    fn index_boundaries() {
        assert_eq!(riscv::index(0x10_0000_0000, 0), 64);
        assert_eq!(riscv::index(0x3fff_ffff, 0), 0);
        assert_eq!(riscv::index(0xffff_ffff_ffff_ffff, 0), 511);
        assert_eq!(x86_64::index(0x10_0000_0000, 0), 0);
        assert_eq!(aarch64::index(0x10_0000_0000, 0), 0);
        assert_eq!(riscv::index(0x3fff_ffff, 1), 511);
        assert_eq!(riscv::index(0xffff_ffff_ffff_ffff, 1), 511);
    }

    #[test]
    fn permission_bits_are_encoded() {
        assert!(!riscv::valid_rights(Rights::WRITE));
        assert!(riscv::valid_rights(Rights::READ | Rights::WRITE));
        let riscv_w = riscv::encode_leaf(0x1000, Rights::WRITE);
        assert!(riscv_w & riscv::PTE_R != 0);
        assert!(riscv_w & riscv::PTE_W != 0);
        let x86_rx = x86_64::encode_leaf(0x2000, Rights::READ | Rights::EXEC);
        assert!(x86_rx & x86_64::U != 0);
        assert!(x86_rx & x86_64::NX == 0);
        let arm_rw = aarch64::encode_leaf(0x3000, Rights::READ | Rights::WRITE);
        assert!(arm_rw & aarch64::UXN != 0);
        assert!(arm_rw & aarch64::AP_USER_RW != 0);
        match aarch64::decode_at(arm_rw, 3) {
            Pte::Leaf(pa, rights) => {
                assert_eq!(pa, 0x3000);
                assert!(rights.contains(Rights::READ));
                assert!(rights.contains(Rights::WRITE));
                assert!(!rights.contains(Rights::EXEC));
            }
            _ => panic!("not AArch64 RW page"),
        }
        assert!(aarch64::is_user(arm_rw));
        let arm_ro_x = aarch64::encode_leaf(0x4000, Rights::READ | Rights::EXEC);
        match aarch64::decode_at(arm_ro_x, 3) {
            Pte::Leaf(pa, rights) => {
                assert_eq!(pa, 0x4000);
                assert!(rights.contains(Rights::READ));
                assert!(!rights.contains(Rights::WRITE));
                assert!(rights.contains(Rights::EXEC));
            }
            _ => panic!("not AArch64 RX page"),
        }
        let arm_kernel = arm_rw & !(0b11 << 6);
        assert!(!aarch64::is_user(arm_kernel));
    }

    #[test]
    fn aarch64_l3_permissions_and_el0_bit() {
        let arm_rw = aarch64::encode_leaf(0x3000, Rights::READ | Rights::WRITE);
        assert_eq!(
            aarch64::decode_at(arm_rw, 3),
            Pte::Leaf(0x3000, Rights::READ | Rights::WRITE)
        );
        assert!(aarch64::is_user(arm_rw));
        assert!(arm_rw & aarch64::UXN != 0);

        let arm_rx = aarch64::encode_leaf(0x4000, Rights::READ | Rights::EXEC);
        assert_eq!(
            aarch64::decode_at(arm_rx, 3),
            Pte::Leaf(0x4000, Rights::READ | Rights::EXEC)
        );
        assert!(aarch64::is_user(arm_rx));
        assert!(arm_rx & aarch64::UXN == 0);

        let arm_kernel = arm_rw & !(0b11 << 6);
        assert!(!aarch64::is_user(arm_kernel));
        assert_eq!(
            aarch64::decode_at(arm_kernel, 3),
            Pte::Leaf(0x3000, Rights::READ)
        );
    }

    #[test]
    fn x86_large_leaf_is_recognized() {
        let block = 0x4000_0000u64 | x86_64::P | x86_64::PS;
        match x86_64::decode_at(block, 1) {
            Pte::Leaf(pa, _) => assert_eq!(pa, 0x4000_0000),
            _ => panic!("not block"),
        }
    }

    #[test]
    fn user_ranges_exclude_kernel_identity() {
        // The 64 GiB base is above the RISC-V identity map and above the
        // x86/AArch64 low 1 GiB kernel mapping.
        assert!(USER_VA_START >= 0x8000_0000);
        assert!(USER_VA_START >= 1 << 30);
        assert!(USER_VA_END > USER_VA_START);
        assert_eq!(USER_VA_END - USER_VA_START, 0x10_0000_0000);

        // Sv39 user addresses must remain in the lower half.
        assert!(riscv::USER_VA_END <= 1 << 38);

        // The whole range is inside the shared PML4/L0 entry 0 on x86 and
        // AArch64, but the level-1 indices start above the low 1 GiB
        // identity mapping.
        assert_eq!(x86_64::index(x86_64::USER_VA_START, 0), 0);
        assert_eq!(x86_64::index(x86_64::USER_VA_END - 1, 0), 0);
        assert_eq!(x86_64::index(x86_64::USER_VA_START, 1), 64);
        assert_eq!(x86_64::index(x86_64::USER_VA_END - 1, 1), 127);
        assert_eq!(aarch64::index(aarch64::USER_VA_START, 0), 0);
        assert_eq!(aarch64::index(aarch64::USER_VA_END - 1, 0), 0);
        assert_eq!(aarch64::index(aarch64::USER_VA_START, 1), 64);
        assert_eq!(aarch64::index(aarch64::USER_VA_END - 1, 1), 127);

        // RISC-V uses its own root entry 64..127; it never touches the
        // identity root entry 2.
        assert_eq!(riscv::index(riscv::USER_VA_START, 0), 64);
        assert_eq!(riscv::index(riscv::USER_VA_END - 1, 0), 127);
        assert_eq!(riscv::index(riscv::USER_VA_START, 1), 0);
        assert_eq!(riscv::index(riscv::USER_VA_END - 1, 1), 511);
    }

    #[test]
    fn kernel_shared_slots_do_not_intersect_user_entries() {
        assert_eq!(riscv::index(0x1000_0000, 0), 0);
        assert_eq!(riscv::index(0x8000_0000, 0), 2);
        assert_eq!(riscv::KERNEL_SHARED_SLOTS, &[0, 2]);
        assert_eq!(riscv::index(riscv::USER_VA_START, 0), 64);

        assert_eq!(x86_64::index(0, 1), 0);
        assert_eq!(x86_64::KERNEL_SHARED_SLOTS, &[0, 3]);
        assert_eq!(x86_64::index(x86_64::USER_VA_START, 1), 64);

        // AArch64 virt maps low MMIO in L1[0] and RAM at 0x4000_0000 in
        // L1[1]; both must remain kernel slots.
        assert_eq!(aarch64::index(0x0800_0000, 1), 0);
        assert_eq!(aarch64::index(0x4000_0000, 1), 1);
        assert_eq!(aarch64::KERNEL_SHARED_SLOTS, &[0, 1]);
        assert_eq!(aarch64::index(aarch64::USER_VA_START, 1), 64);
        for slot in aarch64::KERNEL_SHARED_SLOTS {
            assert!(*slot < aarch64::index(aarch64::USER_VA_START, 1));
        }
    }

    #[test]
    fn table_spans_match_each_format() {
        assert_eq!(riscv::table_span(2), 4096);
        assert_eq!(riscv::table_span(1), 0x20_0000);
        assert_eq!(riscv::table_span(0), 0x4000_0000);
        assert_eq!(x86_64::table_span(3), 4096);
        assert_eq!(x86_64::table_span(2), 0x20_0000);
        assert_eq!(x86_64::table_span(1), 0x4000_0000);
        assert_eq!(x86_64::table_span(0), 0x80_0000_0000);
        assert_eq!(aarch64::table_span(3), 4096);
        assert_eq!(aarch64::table_span(2), 0x20_0000);
        assert_eq!(aarch64::table_span(1), 0x4000_0000);
        assert_eq!(aarch64::table_span(0), 0x80_0000_0000);
        assert_eq!(riscv::table_span(3), 0);
        assert_eq!(x86_64::table_span(4), 0);
    }
}
