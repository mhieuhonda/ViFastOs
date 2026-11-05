//! x86_64 UEFI platform: GDT/TSS/IDT, ring3, legacy PIC/PIT and user paging.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::bkl::BigKernelLock;
use vifast_arch::{Arch, CpuState, MAX_HARTS};
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_x86_64::X86_64;

pub struct X86Platform;

const USER_DATA_SEL: u64 = 0x18;
const USER_CODE_SEL: u64 = 0x20;
const TSS_SEL: u16 = 0x28;
const USER_DATA_RPL3: u64 = USER_DATA_SEL | 3;
const USER_CODE_RPL3: u64 = USER_CODE_SEL | 3;

const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const PAGE_PRESENT: u64 = 1;
const PAGE_WRITE: u64 = 2;
const PAGE_USER: u64 = 4;
const PAGE_LARGE: u64 = 0x80;
const PAGE_NX: u64 = 1 << 63;
const IA32_EFER: u32 = 0xC000_0080;
const IA32_GS_BASE: u32 = 0xC000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
const EFER_NXE: u64 = 1 << 11;

core::arch::global_asm!(include_str!("trap.S"));

#[derive(Clone, Copy)]
#[allow(dead_code)]
#[repr(align(4096))]
struct Stack([u8; 32768]);

#[repr(align(4096))]
struct Table([u64; 512]);

static mut PML4: Table = Table([0; 512]);
static mut PDPT: Table = Table([0; 512]);
static mut PD_IDENTITY: Table = Table([0; 512]);
static mut PD_HIGH: Table = Table([0; 512]);
const LAPIC_DEFAULT: u64 = 0xFEE0_0000;
const PAGE_PCD: u64 = 1 << 4;
const PAGE_PWT: u64 = 1 << 3;

#[allow(dead_code)]
#[repr(align(4096))]
struct SplitTables([[u64; 512]; 8]);
static mut SPLIT_TABLES: SplitTables = SplitTables([[0; 512]; 8]);
static mut SPLIT_USED: usize = 0;
static mut NX_ENABLED: bool = false;
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_FRAME: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 128] = [TrapFrame::empty(); 128];
static mut TRAP_STACKS: [Stack; MAX_HARTS] = [Stack([0; 32768]); MAX_HARTS];
static mut IST_STACKS: [Stack; MAX_HARTS] = [Stack([0; 32768]); MAX_HARTS];
static mut CPU_STATES: [CpuState; MAX_HARTS] = [CpuState::empty(); MAX_HARTS];
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 131072]);
static mut BOOT_POOL: BootPool = BootPool([0; 131072]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
static mut SMAP_ENABLED: bool = false;
static mut PCID_ENABLED: bool = false;
static BKL: BigKernelLock = BigKernelLock::new();

fn bkl_lock() -> vifast_arch::bkl::BigKernelGuard {
    BKL.lock_with_hart(X86Platform::hart_id(), poll_maintenance)
}

fn assert_bkl_released() {
    let hart = X86Platform::hart_id();
    assert!(
        !BKL.held_by(hart),
        "BKL held on current hart before user return"
    );
}
static mut ACPI_CPUS: usize = 0;
static mut ACPI_BSP: u32 = 0;
static mut ACPI_LAPIC_BASE: u64 = 0;
static mut AP_TRAMPOLINE: u64 = 0;
static mut AP_APIC_ID: u32 = 0;
static mut AP_STARTED: bool = false;
static mut AP_LONG_ONLINE: bool = false;
static AP_ONLINE: AtomicBool = AtomicBool::new(false);
static AP_MAINT_SEQ: AtomicUsize = AtomicUsize::new(0);
static AP_MAINT_ACK: AtomicUsize = AtomicUsize::new(0);
static AP_MAINT_KIND: AtomicU32 = AtomicU32::new(0);
const APIC_MAINT_VECTOR: u32 = 0x30;
const APIC_TIMER_VECTOR: u32 = 0x31;
const APIC_HALT_VECTOR: u32 = 0x32;
static AP_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static AP_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static AP_USER_TICKS: AtomicUsize = AtomicUsize::new(0);
static AP_PREEMPTS: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_WAKES: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_SEEN: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_SENT: AtomicUsize = AtomicUsize::new(0);
static AP_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static AP_IDLE_WINDOW: AtomicBool = AtomicBool::new(false);
static USER_ENTRIES: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static RESCHED_SENT: AtomicUsize = AtomicUsize::new(0);
static RESCHED_WAKE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_WAIT_BLOCKS: AtomicUsize = AtomicUsize::new(0);
static POST_STARTED: AtomicBool = AtomicBool::new(false);
static POST_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static POST_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static POST_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static WAKE_SENT_THIS_WINDOW: AtomicBool = AtomicBool::new(false);
static AP_IDLE_GENERATION: AtomicUsize = AtomicUsize::new(0);
static WAKE_LAST_GENERATION: AtomicUsize = AtomicUsize::new(usize::MAX);
static AP_RESCHED_PENDING: AtomicBool = AtomicBool::new(false);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static LAPIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static BSP_PIT_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_RATE_BASE: AtomicUsize = AtomicUsize::new(0);
const AP_MAINT_TLB: u32 = 1;
const AP_MAINT_ICACHE: u32 = 2;
const TRAMPOLINE_STAGE_OFFSET: u64 = 0x802;

unsafe fn identity_u32(address: u64) -> Option<u32> {
    if address.saturating_add(4) > 0x4000_0000 {
        return None;
    }
    Some(core::ptr::read_unaligned(address as *const u32))
}

unsafe fn identity_u64(address: u64) -> Option<u64> {
    if address.saturating_add(8) > 0x4000_0000 {
        return None;
    }
    Some(core::ptr::read_unaligned(address as *const u64))
}

unsafe fn identity_table(address: u64, max: usize) -> Option<&'static [u8]> {
    if address < 0x1000 || address.saturating_add(8) > 0x4000_0000 {
        return None;
    }
    let length = identity_u32(address + 4)? as usize;
    if length < 36 || length > max || address.saturating_add(length as u64) > 0x4000_0000 {
        return None;
    }
    Some(core::slice::from_raw_parts(address as *const u8, length))
}

unsafe fn init_acpi(rsdp: u64) {
    if rsdp == 0 {
        X86_64::write_str("ACPI SKIP (no RSDP)\r\n");
        return;
    }
    if rsdp >= 0x4000_0000 {
        X86_64::write_str("ACPI SKIP (above identity map)\r\n");
        return;
    }
    let Some(revision) = (|| {
        let length = identity_u32(rsdp + 20)? as usize;
        if length < 36 || length > 4096 || rsdp.saturating_add(length as u64) > 0x4000_0000 {
            return None;
        }
        Some(core::slice::from_raw_parts(rsdp as *const u8, length)[15])
    })() else {
        X86_64::write_str("ACPI SKIP (bad RSDP length)\r\n");
        return;
    };
    if revision < 2 {
        X86_64::write_str("ACPI SKIP (RSDP v1)\r\n");
        return;
    }
    let Some(xsdt_address) = identity_u64(rsdp + 24) else {
        X86_64::write_str("ACPI SKIP (above identity map)\r\n");
        return;
    };
    let Some(rsdp_len) = identity_u32(rsdp + 20) else {
        return;
    };
    let rsdp_table = core::slice::from_raw_parts(rsdp as *const u8, rsdp_len as usize);
    let Some(xsdt) = identity_table(xsdt_address, 4096) else {
        X86_64::write_str("ACPI SKIP (bad XSDT)\r\n");
        return;
    };
    let mut madt_address = 0;
    let mut offset = 36;
    while offset + 8 <= xsdt.len() {
        let address = core::ptr::read_unaligned(xsdt.as_ptr().add(offset) as *const u64);
        if identity_u32(address).map(|value| value == u32::from_le_bytes(*b"APIC")) == Some(true) {
            madt_address = address;
            break;
        }
        offset += 8;
    }
    if madt_address == 0 {
        X86_64::write_str("ACPI SKIP (no MADT)\r\n");
        return;
    }
    let Some(madt) = identity_table(madt_address, 8192) else {
        X86_64::write_str("ACPI SKIP (bad MADT)\r\n");
        return;
    };
    match vifast_acpi::parse(rsdp_table, xsdt, madt, madt_address) {
        Ok(parsed) => {
            ACPI_CPUS = parsed.cpu_count;
            ACPI_BSP = parsed.bsp;
            ACPI_LAPIC_BASE = parsed.lapic_base;
            AP_APIC_ID = 0;
            for cpu in parsed.cpus[..parsed.cpu_count].iter() {
                if cpu.enabled && cpu.apic_id != parsed.bsp {
                    AP_APIC_ID = cpu.apic_id;
                    break;
                }
            }
            vifast_kernel::klog!(
                X86Platform,
                "ACPI MADT OK (cpus={}, bsp={}, lapic={:#x})",
                parsed.cpu_count,
                parsed.bsp,
                parsed.lapic_base
            );
        }
        Err(error) => {
            vifast_kernel::klog!(X86Platform, "ACPI SKIP ({:?})", error);
        }
    }
}

fn poll_maintenance() {
    unsafe {
        if read_msr(IA32_GS_BASE) != core::ptr::addr_of!(CPU_STATES[1]) as u64 {
            return;
        }
        process_maintenance();
    }
}

fn assert_irqs_disabled() {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) flags);
    }
    assert!(flags & (1 << 9) == 0, "BKL entered with IF set");
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DescriptorTablePtr {
    limit: u16,
    base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn zero() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    reserved0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap: u16,
}

impl Tss {
    const fn zero() -> Self {
        Tss {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap: 0,
        }
    }
}

static mut GDT: [u64; 7] = [0; 7];
static mut GDT_PTR: DescriptorTablePtr = DescriptorTablePtr { limit: 0, base: 0 };
static mut IDT: [IdtEntry; 256] = [IdtEntry::zero(); 256];
static mut IDT_PTR: DescriptorTablePtr = DescriptorTablePtr { limit: 0, base: 0 };
static mut AP_GDT: [u64; 7] = [0; 7];
static mut AP_GDT_PTR: DescriptorTablePtr = DescriptorTablePtr { limit: 0, base: 0 };
static mut TSSS: [Tss; MAX_HARTS] = [Tss::zero(); MAX_HARTS];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl TrapFrame {
    const fn empty() -> Self {
        TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            rip: 0,
            cs: 0,
            rflags: 0,
            rsp: 0,
            ss: 0,
        }
    }

    fn user(entry: usize, stack: usize) -> Self {
        let mut frame = Self::empty();
        frame.rip = entry as u64;
        frame.cs = USER_CODE_RPL3;
        frame.rflags = 0x202;
        frame.rsp = stack as u64;
        frame.ss = USER_DATA_RPL3;
        frame
    }
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value);
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value);
    value
}

unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", in("dx") port, out("ax") value);
    value
}

unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value);
}

const ATA_BASE: u16 = 0x1F0;
const ATA_CTRL: u16 = 0x3F6;

unsafe fn ata_wait_bsy() -> bool {
    let mut spins = 0u32;
    while inb(ATA_BASE + 7) & 0x80 != 0 {
        spins += 1;
        if spins > 10_000_000 {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

unsafe fn ata_wait_drq() -> bool {
    let mut spins = 0u32;
    loop {
        let status = inb(ATA_BASE + 7);
        if status & 0x80 == 0 && status & 0x08 != 0 {
            return true;
        }
        if status & 0x01 != 0 || spins > 10_000_000 {
            return false;
        }
        spins += 1;
        core::hint::spin_loop();
    }
}

unsafe fn ata_prepare(lba: u64) -> bool {
    if lba >= 1 << 28 || !ata_wait_bsy() {
        return false;
    }
    outb(ATA_BASE + 6, 0xE0 | ((lba >> 24) as u8 & 0x0f));
    // 400 ns delay: four alternate-status reads.
    for _ in 0..4 {
        let _ = inb(ATA_CTRL);
    }
    outb(ATA_BASE + 2, 1);
    outb(ATA_BASE + 3, lba as u8);
    outb(ATA_BASE + 4, (lba >> 8) as u8);
    outb(ATA_BASE + 5, (lba >> 16) as u8);
    true
}

unsafe fn ata_read(lba: u64, out: &mut [u8; 512]) -> bool {
    if !ata_prepare(lba) {
        return false;
    }
    outb(ATA_BASE + 7, 0x20);
    if !ata_wait_drq() {
        return false;
    }
    for i in 0..256 {
        let word = inw(ATA_BASE);
        out[i * 2] = word as u8;
        out[i * 2 + 1] = (word >> 8) as u8;
    }
    true
}

unsafe fn ata_write(lba: u64, input: &[u8; 512]) -> bool {
    if !ata_prepare(lba) {
        return false;
    }
    outb(ATA_BASE + 7, 0x30);
    if !ata_wait_drq() {
        return false;
    }
    for i in 0..256 {
        let word = input[i * 2] as u16 | ((input[i * 2 + 1] as u16) << 8);
        outw(ATA_BASE, word);
    }
    outb(ATA_BASE + 7, 0xE7); // flush cache
    let _ = ata_wait_bsy();
    true
}

fn table_pa(table: *const Table) -> u64 {
    table as u64
}

unsafe fn zero_table(table: *mut Table) {
    for entry in (*table).0.iter_mut() {
        *entry = 0;
    }
}

unsafe fn enable_smep_smap() {
    let leaf7 = core::arch::x86_64::__cpuid(7);
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    if leaf7.ebx & (1 << 7) != 0 {
        cr4 |= 1 << 20; // SMEP
    }
    if leaf7.ebx & (1 << 20) != 0 {
        cr4 |= 1 << 21; // SMAP
        SMAP_ENABLED = true;
    }
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
}

unsafe fn enable_pcid() {
    let leaf1 = core::arch::x86_64::__cpuid(1);
    if leaf1.ecx & (1 << 17) == 0 {
        return;
    }
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    cr4 |= 1 << 17;
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
    PCID_ENABLED = true;
}

unsafe fn enable_nx() -> bool {
    let ext = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if ext < 0x8000_0001 || core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) == 0 {
        return false;
    }
    let low: u32;
    let high: u32;
    core::arch::asm!("rdmsr", in("ecx") IA32_EFER, out("eax") low, out("edx") high);
    let efer = (((high as u64) << 32) | low as u64) | EFER_NXE;
    core::arch::asm!(
        "wrmsr",
        in("ecx") IA32_EFER,
        in("eax") efer as u32,
        in("edx") (efer >> 32) as u32,
    );
    true
}

/// Split one 2 MiB identity leaf into 4 KiB leaves so individual kernel data
/// pages can be marked non-executable without affecting the text page.
unsafe fn split_identity_2m(region: usize) -> *mut u64 {
    let identity = core::ptr::addr_of_mut!(PD_IDENTITY) as *mut Table;
    let entry = &mut (*identity).0[region];
    if *entry & PAGE_LARGE == 0 {
        return (*entry & !0xfff) as *mut u64;
    }
    let slot = SPLIT_USED;
    if slot >= 8 {
        return core::ptr::null_mut();
    }
    SPLIT_USED += 1;
    let table = core::ptr::addr_of_mut!(SPLIT_TABLES) as *mut u64;
    let table = table.add(slot * 512);
    let base = region as u64 * 0x20_0000;
    for index in 0..512 {
        *table.add(index) = (base + index as u64 * 4096) | PAGE_PRESENT | PAGE_WRITE;
    }
    *entry = table as u64 | PAGE_PRESENT | PAGE_WRITE;
    table
}

unsafe fn mark_nx_range(start: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = match start.checked_add(len as u64 - 1) {
        Some(end) => end,
        None => return false,
    };
    let mut va = start & !0xfff;
    loop {
        let region = (va >> 21) as usize;
        let index = ((va >> 12) & 0x1ff) as usize;
        let table = split_identity_2m(region);
        if table.is_null() {
            return false;
        }
        *table.add(index) |= PAGE_NX;
        if va >= (end & !0xfff) {
            break;
        }
        va += 4096;
    }
    true
}

fn kernel_data_ranges() -> [(u64, usize); 14] {
    [
        (
            core::ptr::addr_of!(TRAP_STACKS) as u64,
            core::mem::size_of::<[Stack; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(IST_STACKS) as u64,
            core::mem::size_of::<[Stack; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(CONTEXTS) as u64,
            core::mem::size_of::<[TrapFrame; 32]>(),
        ),
        (
            core::ptr::addr_of!(GDT) as u64,
            core::mem::size_of::<[u64; 7]>(),
        ),
        (
            core::ptr::addr_of!(TSSS) as u64,
            core::mem::size_of::<[Tss; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(IDT) as u64,
            core::mem::size_of::<[IdtEntry; 256]>(),
        ),
        (
            core::ptr::addr_of!(BOOT_POOL) as u64,
            core::mem::size_of::<BootPool>(),
        ),
        (
            core::ptr::addr_of!(USER_FRAME) as u64,
            core::mem::size_of::<DataPool>(),
        ),
        (
            core::ptr::addr_of!(METADATA) as u64,
            core::mem::size_of::<Metadata>(),
        ),
        (
            core::ptr::addr_of!(KERNEL_STORAGE) as u64,
            core::mem::size_of::<Kernel>(),
        ),
        (
            core::ptr::addr_of!(PML4) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(PDPT) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(PD_IDENTITY) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(SPLIT_TABLES) as u64,
            core::mem::size_of::<SplitTables>(),
        ),
    ]
}

unsafe fn build_kernel_page_tables() {
    let pml4 = core::ptr::addr_of_mut!(PML4) as *mut Table;
    let pdpt = core::ptr::addr_of_mut!(PDPT) as *mut Table;
    let identity = core::ptr::addr_of_mut!(PD_IDENTITY) as *mut Table;
    let high = core::ptr::addr_of_mut!(PD_HIGH) as *mut Table;

    zero_table(pml4);
    zero_table(pdpt);
    zero_table(identity);
    zero_table(high);

    /* PML4[0] -> PDPT; PDPT[0] covers 0..1 GiB identity for kernel/UEFI
    code, stacks, GDT/TSS/IDT and page tables. */
    (*pml4).0[0] = table_pa(pdpt) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    (*pdpt).0[0] = table_pa(identity) | PAGE_PRESENT | PAGE_WRITE;
    // LAPIC lives in 3..4 GiB.  Share this supervisor-only 2 MiB MMIO
    // mapping through the same PDPT copied into every VSpace.
    let lapic_base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    (*pdpt).0[3] = table_pa(high) | PAGE_PRESENT | PAGE_WRITE;
    let high_index = ((lapic_base >> 21) & 0x1ff) as usize;
    (*high).0[high_index] = (lapic_base & !0x1f_ffff)
        | PAGE_PRESENT
        | PAGE_WRITE
        | PAGE_LARGE
        | PAGE_PCD
        | PAGE_PWT
        | PAGE_NX;
    for i in 0..512usize {
        (*identity).0[i] = (i as u64 * 0x20_0000) | PAGE_PRESENT | PAGE_WRITE | PAGE_LARGE;
    }
    NX_ENABLED = enable_nx();
    if NX_ENABLED {
        for (start, len) in kernel_data_ranges() {
            if !mark_nx_range(start, len) {
                NX_ENABLED = false;
                break;
            }
        }
    }
}

fn user_range(ptr: u64, len: usize, base: usize, size: usize) -> bool {
    ptr >= base as u64 && ptr as usize + len <= base + size
}

fn active_root_matches_current() -> bool {
    unsafe {
        if KERNEL_PTR.is_null() || active_root() == 0 {
            return false;
        }
        let kernel = &*KERNEL_PTR;
        let Some(tcb) = kernel.current() else {
            return false;
        };
        kernel.current_root(tcb) == Some(active_root())
    }
}

unsafe fn write_msr(msr: u32, value: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
    );
}

unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high);
    ((high as u64) << 32) | low as u64
}

fn cpu_state() -> *mut CpuState {
    unsafe { read_msr(IA32_GS_BASE) as *mut CpuState }
}

fn publish_current_tcb(tcb: Option<usize>) {
    let state = cpu_state();
    unsafe {
        if !state.is_null() {
            (*state).current_tcb = tcb.unwrap_or(usize::MAX);
        }
    }
}

fn active_root() -> u64 {
    let state = cpu_state();
    unsafe {
        if state.is_null() {
            0
        } else {
            (*state).active_root
        }
    }
}

fn active_asid() -> u16 {
    let state = cpu_state();
    unsafe {
        if state.is_null() {
            0
        } else {
            (*state).active_asid
        }
    }
}

fn set_active_root(root: u64, asid: u16) {
    let state = cpu_state();
    unsafe {
        if !state.is_null() {
            (*state).active_root = root;
            (*state).active_asid = asid;
        }
    }
}

fn assert_current_tcb(where_: &str) {
    let state = cpu_state();
    if state.is_null() {
        return;
    }
    let hart = unsafe { (*state).hart_id };
    let expected = X86Platform::kernel()
        .current_on(hart)
        .map(|tcb| tcb.index as usize)
        .unwrap_or(usize::MAX);
    let actual = unsafe { (*state).current_tcb };
    if actual != expected {
        vifast_kernel::klog!(
            X86Platform,
            "CURRENT MISMATCH where={} hart={} actual={} expected={}",
            where_,
            unsafe { (*state).hart_id },
            actual,
            expected
        );
        X86Platform::exit(1);
    }
}

static mut LAPIC_READY: bool = false;

unsafe fn configure_lapic() -> u32 {
    let base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    let svr = (base + 0xF0) as *mut u32;
    let tpr = (base + 0x80) as *mut u32;
    let id = core::ptr::read_volatile((base + 0x20) as *const u32) >> 24;
    core::ptr::write_volatile(tpr, 0);
    core::ptr::write_volatile(svr, 0x100 | 0xFF);
    id
}

unsafe fn init_lapic() {
    if LAPIC_READY {
        return;
    }
    let id = configure_lapic();
    let base = lapic_base();
    core::ptr::write_volatile((base + 0x350) as *mut u32, 0x700);
    let lint0 = core::ptr::read_volatile((base + 0x350) as *const u32);
    let lint1 = core::ptr::read_volatile((base + 0x360) as *const u32);
    LAPIC_READY = true;
    vifast_kernel::klog!(X86Platform, "LAPIC OK (id={})", id);
    vifast_kernel::klog!(X86Platform, "BSP LINT0={:#x} LINT1={:#x}", lint0, lint1);
}

unsafe fn lapic_base() -> u64 {
    if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    }
}

fn count_user_entry() {
    let hart = X86Platform::hart_id();
    if let Some(counter) = USER_ENTRIES.get(hart) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe fn lapic_eoi() {
    core::ptr::write_volatile((lapic_base() + 0xB0) as *mut u32, 0);
}

unsafe fn process_maintenance() {
    let seq = AP_MAINT_SEQ.load(Ordering::Acquire);
    if AP_MAINT_ACK.load(Ordering::Acquire) >= seq {
        return;
    }
    let kind = AP_MAINT_KIND.swap(0, Ordering::AcqRel);
    if kind & AP_MAINT_TLB != 0 {
        core::arch::asm!("invlpg [{}]", in(reg) 0u64);
    }
    if kind & AP_MAINT_ICACHE != 0 {
        core::arch::asm!("wbinvd");
    }
    AP_MAINT_ACK.store(seq, Ordering::Release);
}

unsafe fn rdtsc() -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
    ((high as u64) << 32) | low as u64
}

unsafe fn calibrate_tsc_pit() {
    // PIT channel 2, mode 0, 10 ms one-shot; gate enabled through port 0x61.
    let divisor: u16 = 11932;
    let control = (inb(0x61) & !0x02) | 0x01;
    outb(0x61, control);
    outb(0x43, 0xB0);
    outb(0x42, divisor as u8);
    outb(0x42, (divisor >> 8) as u8);
    let start = rdtsc();
    let mut spins = 0u64;
    while inb(0x61) & 0x20 == 0 && spins < 100_000_000 {
        core::arch::asm!("pause");
        spins += 1;
    }
    let end = rdtsc();
    let elapsed = end.wrapping_sub(start);
    let hz = if spins >= 100_000_000 || elapsed == 0 {
        2_500_000_000
    } else {
        ((elapsed as u128 * 1_193_182u128) / divisor as u128) as u64
    };
    TSC_HZ.store(hz, Ordering::Release);
    vifast_kernel::klog!(X86Platform, "TSC HZ={}", hz);
}

unsafe fn tsc_delay_us(micros: u64) {
    let hz = TSC_HZ.load(Ordering::Acquire);
    let cycles = if hz == 0 {
        micros.saturating_mul(2_500)
    } else {
        (hz as u128 * micros as u128 / 1_000_000) as u64
    };
    tsc_delay(cycles);
}

unsafe fn calibrate_lapic_timer() {
    let base = lapic_base();
    let tsc = TSC_HZ.load(Ordering::Acquire).max(1);
    // Mask the timer and run a one-shot with the largest count for 10 ms.
    core::ptr::write_volatile((base + 0x320) as *mut u32, 1 << 16);
    core::ptr::write_volatile((base + 0x3E0) as *mut u32, 0xB);
    core::ptr::write_volatile((base + 0x380) as *mut u32, 0xFFFF_FFFF);
    let start = rdtsc();
    let target = tsc / 100;
    while rdtsc().wrapping_sub(start) < target {
        core::arch::asm!("pause");
    }
    let elapsed = rdtsc().wrapping_sub(start).max(1);
    let current = core::ptr::read_volatile((base + 0x390) as *const u32);
    let delta = 0xFFFF_FFFFu64.saturating_sub(current as u64);
    let hz = if delta == 0 {
        100_000_000
    } else {
        ((delta as u128 * tsc as u128) / elapsed as u128) as u64
    };
    let hz = hz.max(1_000);
    LAPIC_TIMER_HZ.store(hz, Ordering::Release);
    vifast_kernel::klog!(X86Platform, "LAPIC TIMER HZ={}", hz);
    let initial = (hz / 100).max(1_000) as u32;
    core::ptr::write_volatile((base + 0x3E0) as *mut u32, 0xB);
    core::ptr::write_volatile((base + 0x380) as *mut u32, 0);
    core::ptr::write_volatile((base + 0x320) as *mut u32, (1 << 17) | APIC_TIMER_VECTOR);
    core::ptr::write_volatile((base + 0x380) as *mut u32, initial);
}

unsafe fn ap_idle_loop() -> ! {
    calibrate_lapic_timer();
    loop {
        let (picked, ready) = {
            let bkl = bkl_lock();
            let picked = X86Platform::kernel().pick_on(1);
            let ready = X86Platform::kernel().has_ready_on(1);
            drop(bkl);
            (picked, ready)
        };
        if let Some(tcb) = picked {
            publish_current_tcb(Some(tcb.index as usize));
            if vifast_runtime::ensure_current_root::<X86Platform>().is_err() {
                X86Platform::exit(1);
            }
            let context = X86Platform::contexts()[tcb.index as usize];
            <X86Platform as Platform>::enter_user(&context);
        }
        if ready {
            continue;
        }
        AP_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        let generation = AP_IDLE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = generation;
        WAKE_SENT_THIS_WINDOW.store(false, Ordering::SeqCst);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        AP_IDLE_WINDOW.store(true, Ordering::SeqCst);
        core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
        AP_IDLE_WINDOW.store(false, Ordering::SeqCst);
    }
}

unsafe fn install_real_trampoline() {
    let base = AP_TRAMPOLINE;
    if base == 0 || base >= 0xA0000 || base & 0xfff != 0 {
        return;
    }
    let page = base as *mut u8;
    core::ptr::write_bytes(page, 0, 4096);
    // 16-bit real mode: magic, GDT, protected mode, then far jump to 32-bit.
    let vector_byte = (base >> 12) as u8;
    let real = [
        0xFA, // cli
        0xB8,
        0x00,
        vector_byte, // mov ax, vector << 8
        0x8E,
        0xD8, // mov ds, ax
        0xC7,
        0x06,
        0x00,
        0x08,
        0x5A,
        0xA5, // mov word [0800], A55A
        0x0F,
        0x01,
        0x16,
        0x40,
        0x03, // lgdt [0340]
        0x0F,
        0x20,
        0xC0, // mov eax, cr0
        0x0C,
        0x01, // or al, 1
        0x0F,
        0x22,
        0xC0, // mov cr0, eax
        0x66,
        0xEA,
        0,
        0,
        0,
        0,
        0x08,
        0x00, // ljmp 08:page+0x30
    ];
    let far_start = real.len() - 8;
    core::ptr::copy_nonoverlapping(real.as_ptr(), page, far_start);
    let real_stage = [0xC6, 0x06, 0x02, 0x08, 0x01]; // mov byte [0802], 1
    core::ptr::copy_nonoverlapping(real_stage.as_ptr(), page.add(far_start), real_stage.len());
    core::ptr::copy_nonoverlapping(
        real[far_start..].as_ptr(),
        page.add(far_start + real_stage.len()),
        8,
    );
    core::ptr::write_unaligned(
        page.add(far_start + real_stage.len() + 2) as *mut u32,
        (base as u32).wrapping_add(0x30),
    );
    // 32-bit protected mode: flat segments, CR3, PAE, EFER.LME|NXE, paging.
    let pm = [
        0x66, 0xB8, 0x10, 0x00, // mov ax, 0x10
        0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xE0, 0x8E, 0xE8, 0x8E, 0xD0, 0x8B, 0x1D, 0, 0, 0,
        0, // mov ebx, [page+0x200]
        0x0F, 0x22, 0xDB, // mov cr3, ebx
        0x0F, 0x20, 0xE0, // mov eax, cr4
        0x0D, 0x20, 0x00, 0x00, 0x00, // or eax, 0x20
        0x0F, 0x22, 0xE0, // mov cr4, eax
        0xB9, 0x80, 0x00, 0x00, 0xC0, // mov ecx, 0xC0000080
        0x0F, 0x32, // rdmsr
        0x0D, 0x00, 0x09, 0x00, 0x00, // or eax, 0x900
        0x0F, 0x30, // wrmsr
        0x0F, 0x20, 0xC0, // mov eax, cr0
        0x0D, 0x01, 0x00, 0x00, 0x80, // or eax, 0x80000001
        0x0F, 0x22, 0xC0, // mov cr0, eax
        0xEA, 0, 0, 0, 0, 0x18, 0x00, // ljmp 18:page+0x80
    ];
    let pm_offset = 0x30;
    let pm_far = pm.len() - 7;
    core::ptr::copy_nonoverlapping(pm.as_ptr(), page.add(pm_offset), pm_far);
    let mut pm_stage = [
        0xC6, 0x05, 0, 0, 0, 0, 0x02, // mov byte [abs], 2
    ];
    core::ptr::write_unaligned(
        pm_stage.as_mut_ptr().add(2) as *mut u32,
        (base as u32).wrapping_add(0x802),
    );
    core::ptr::copy_nonoverlapping(
        pm_stage.as_ptr(),
        page.add(pm_offset + pm_far),
        pm_stage.len(),
    );
    core::ptr::copy_nonoverlapping(
        pm[pm_far..].as_ptr(),
        page.add(pm_offset + pm_far + pm_stage.len()),
        7,
    );
    core::ptr::write_unaligned(
        page.add(pm_offset + 0x10) as *mut u32,
        (base as u32).wrapping_add(0x200),
    );
    core::ptr::write_unaligned(
        page.add(pm_offset + pm_far + pm_stage.len() + 1) as *mut u32,
        (base as u32).wrapping_add(0x80),
    );
    #[cfg(feature = "test-ap-stall")]
    core::ptr::copy_nonoverlapping(
        [0xEB, 0xFE].as_ptr(),
        page.add(pm_offset + pm_far + pm_stage.len()),
        2,
    );
    // 64-bit entry: load stack and AP entry address.
    let mut lm = [
        0x66, 0xB8, 0x10, 0x00, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xE0, 0x8E, 0xE8, 0x8E, 0xD0, 0x48,
        0xBC, 0, 0, 0, 0, 0, 0, 0, 0, // mov rsp, imm64
        0x48, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, // mov rax, imm64
        0xFF, 0xE0,
    ];
    let lm_offset = 0x80;
    core::ptr::copy_nonoverlapping(lm.as_mut_ptr(), page.add(lm_offset), lm.len());
    let stack = core::ptr::addr_of!(TRAP_STACKS[1]) as u64 + 0x4000;
    let entry = ap_entry as *const () as u64;
    core::ptr::write_unaligned(page.add(lm_offset + 16) as *mut u64, stack);
    core::ptr::write_unaligned(page.add(lm_offset + 26) as *mut u64, entry);
    // Data: CR3 at 0x200, GDT at 0x300/0x340, stack/entry patched in code.
    let pml4 = core::ptr::addr_of!(PML4) as u32;
    core::ptr::write_unaligned(page.add(0x200) as *mut u32, pml4);
    let gdt = [
        0u64,
        0x00CF_9A00_0000_FFFFu64,
        0x00CF_9200_0000_FFFFu64,
        0x00AF_9A00_0000_FFFFu64,
        0x00AF_9200_0000_FFFFu64,
    ];
    core::ptr::copy_nonoverlapping(
        gdt.as_ptr() as *const u8,
        page.add(0x300),
        core::mem::size_of_val(&gdt),
    );
    let gdt_desc = [0x27u8, 0, 0, 0, 0, 0, 0, 0];
    core::ptr::copy_nonoverlapping(gdt_desc.as_ptr(), page.add(0x340), 8);
    core::ptr::write_unaligned(
        page.add(0x342) as *mut u32,
        (base as u32).wrapping_add(0x300),
    );
}

#[allow(asm_sub_register)]
#[no_mangle]
pub extern "C" fn ap_entry() -> ! {
    unsafe {
        core::arch::asm!("cli");
        let stage_ptr = (AP_TRAMPOLINE + TRAMPOLINE_STAGE_OFFSET) as *mut u8;
        core::ptr::write_volatile(stage_ptr, 3);
        setup_ap_gdt_tss_idt();
        init_cpu(1);
        let mut tr: u16;
        core::arch::asm!("str {}", out(reg) tr);
        assert_eq!(tr, TSS_SEL, "AP TSS selector mismatch");
        assert_eq!(
            read_msr(IA32_GS_BASE),
            core::ptr::addr_of!(CPU_STATES[1]) as u64,
            "AP GS base mismatch"
        );
        let _ = configure_lapic();
        let base = lapic_base();
        core::ptr::write_volatile((base + 0x350) as *mut u32, 1 << 16);
        core::ptr::write_volatile((base + 0x360) as *mut u32, 1 << 16);
        enable_smep_smap();
        core::ptr::write_volatile(stage_ptr, 4);
        AP_ONLINE.store(true, Ordering::SeqCst);
        AP_LONG_ONLINE = true;
        ap_idle_loop()
    }
}

pub fn panic_halt_others() {
    unsafe {
        let base = lapic_base();
        let target = if X86Platform::hart_id() == 0 {
            if !AP_ONLINE.load(Ordering::SeqCst) {
                return;
            }
            AP_APIC_ID
        } else {
            0
        };
        core::ptr::write_volatile((base + 0x310) as *mut u32, target << 24);
        send_icr(base, APIC_HALT_VECTOR);
    }
}

unsafe fn trampoline_stage() -> u8 {
    if AP_TRAMPOLINE == 0 {
        0
    } else {
        core::ptr::read_volatile((AP_TRAMPOLINE + TRAMPOLINE_STAGE_OFFSET) as *const u8)
    }
}

unsafe fn wait_stage(target: u8) -> u8 {
    let mut stage = trampoline_stage();
    for _ in 0..40 {
        if stage >= target {
            return stage;
        }
        tsc_delay_us(500);
        stage = trampoline_stage();
    }
    stage
}

unsafe fn wait_icr_idle(base: u64) -> bool {
    for _ in 0..1_000_000 {
        if core::ptr::read_volatile((base + 0x300) as *const u32) & (1 << 12) == 0 {
            return true;
        }
        core::arch::asm!("pause");
    }
    false
}

unsafe fn send_icr(base: u64, low: u32) {
    // Destination was written to ICR high (0x310) before this low write.
    core::ptr::write_volatile((base + 0x300) as *mut u32, low);
}

unsafe fn tsc_delay(cycles: u64) {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
    let start = ((high as u64) << 32) | low as u64;
    let mut now = start;
    while now.wrapping_sub(start) < cycles {
        core::arch::asm!("pause");
        let low: u32;
        let high: u32;
        core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
        now = ((high as u64) << 32) | low as u64;
    }
}

unsafe fn start_ap_real() {
    if AP_STARTED || AP_TRAMPOLINE == 0 || AP_APIC_ID == 0 {
        return;
    }
    AP_STARTED = true;
    let base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    let vector = (AP_TRAMPOLINE >> 12) as u32;
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4500);
    if !wait_icr_idle(base) {
        X86_64::write_str("SMP FAIL (ap timeout stage=real)\r\n");
        return;
    }
    tsc_delay_us(10_000);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4600 | vector);
    if !wait_icr_idle(base) {
        X86_64::write_str("SMP FAIL (ap timeout stage=real)\r\n");
        return;
    }
    tsc_delay_us(200);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4600 | vector);
    let _ = wait_icr_idle(base);
    let stage = wait_stage(1);
    if stage < 1 {
        vifast_kernel::klog!(X86Platform, "SMP FAIL (ap timeout stage={})", stage);
        #[cfg(feature = "test-ap-stall")]
        X86Platform::exit(1);
        #[cfg(not(feature = "test-ap-stall"))]
        return;
    }
    let apic = AP_APIC_ID;
    vifast_kernel::klog!(X86Platform, "AP REAL MODE OK (apic={})", apic);
    let stage = wait_stage(3);
    if stage < 3 {
        vifast_kernel::klog!(X86Platform, "SMP FAIL (ap timeout stage={})", stage);
        #[cfg(feature = "test-ap-stall")]
        X86Platform::exit(1);
        #[cfg(not(feature = "test-ap-stall"))]
        return;
    }
    vifast_kernel::klog!(X86Platform, "AP LONG MODE OK (cpu=1)");
    let seq = AP_MAINT_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    AP_MAINT_KIND.fetch_or(AP_MAINT_TLB | AP_MAINT_ICACHE, Ordering::AcqRel);
    AP_IPI_SENT.fetch_add(1, Ordering::AcqRel);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, APIC_MAINT_VECTOR);
    for _ in 0..100_000 {
        if AP_MAINT_ACK.load(Ordering::Acquire) >= seq && AP_IPI_SEEN.load(Ordering::Acquire) > 0 {
            BSP_RATE_BASE.store(BSP_PIT_TICKS.load(Ordering::Acquire), Ordering::Release);
            vifast_kernel::klog!(X86Platform, "SMP OK (harts=2, ipi=1)");
            return;
        }
        core::arch::asm!("pause");
    }
    vifast_kernel::klog!(
        X86Platform,
        "SMP FAIL (ap timeout stage={})",
        trampoline_stage()
    );
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    CPU_STATES[index].hart_id = hartid;
    CPU_STATES[index].current_tcb = 0;
    CPU_STATES[index].trap_stack_top = core::ptr::addr_of!(TRAP_STACKS[index]) as u64 + 16384;
    write_msr(IA32_GS_BASE, core::ptr::addr_of!(CPU_STATES[index]) as u64);
    write_msr(IA32_KERNEL_GS_BASE, 0);
}
unsafe fn setup_ap_gdt_tss_idt() {
    let tss = core::ptr::addr_of_mut!(TSSS[1]);
    (*tss).rsp0 = core::ptr::addr_of!(TRAP_STACKS[1]) as u64 + 16384;
    (*tss).ist[0] = core::ptr::addr_of!(IST_STACKS[1]) as u64 + 32768;
    (*tss).iomap = core::mem::size_of::<Tss>() as u16;
    let tss_base = tss as u64;
    let tss_limit = core::mem::size_of::<Tss>() as u64 - 1;
    let tss_low = (tss_limit & 0xffff)
        | ((tss_base & 0xffffff) << 16)
        | (0x89u64 << 40)
        | (((tss_limit >> 16) & 0xf) << 48)
        | (((tss_base >> 24) & 0xff) << 56);
    let gdt = core::ptr::addr_of_mut!(AP_GDT);
    (*gdt)[0] = 0;
    (*gdt)[1] = 0x00AF9A000000FFFF;
    (*gdt)[2] = 0x00AF92000000FFFF;
    (*gdt)[3] = 0x00AFF2000000FFFF;
    (*gdt)[4] = 0x00AFFA000000FFFF;
    (*gdt)[5] = tss_low;
    (*gdt)[6] = tss_base >> 32;
    core::ptr::write_volatile(
        core::ptr::addr_of_mut!(AP_GDT_PTR),
        DescriptorTablePtr {
            limit: 7 * 8 - 1,
            base: gdt as u64,
        },
    );
    core::arch::asm!("lgdt [{}]", in(reg) core::ptr::addr_of!(AP_GDT_PTR));
    x86_reload_segments();
    core::arch::asm!("mov ax, 0x28", "ltr ax");
    core::arch::asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_PTR));
}

unsafe fn setup_gdt_tss_idt() {
    let tss = core::ptr::addr_of_mut!(TSSS[0]);
    (*tss).rsp0 = core::ptr::addr_of!(TRAP_STACKS[0]) as u64 + 16384;
    (*tss).ist[0] = core::ptr::addr_of!(IST_STACKS[0]) as u64 + 32768;
    (*tss).iomap = core::mem::size_of::<Tss>() as u16;
    let tss_base = tss as u64;
    let tss_limit = core::mem::size_of::<Tss>() as u64 - 1;
    let tss_low = (tss_limit & 0xffff)
        | ((tss_base & 0xffffff) << 16)
        | (0x89u64 << 40)
        | (((tss_limit >> 16) & 0xf) << 48)
        | (((tss_base >> 24) & 0xff) << 56);
    let tss_high = tss_base >> 32;
    let gdt = core::ptr::addr_of_mut!(GDT);
    (*gdt)[0] = 0;
    (*gdt)[1] = 0x00AF9A000000FFFF;
    (*gdt)[2] = 0x00AF92000000FFFF;
    (*gdt)[3] = 0x00AFF2000000FFFF;
    (*gdt)[4] = 0x00AFFA000000FFFF;
    (*gdt)[5] = tss_low;
    (*gdt)[6] = tss_high;
    core::ptr::write_volatile(
        core::ptr::addr_of_mut!(GDT_PTR),
        DescriptorTablePtr {
            limit: 7 * 8 - 1,
            base: gdt as u64,
        },
    );
    core::arch::asm!("lgdt [{}]", in(reg) core::ptr::addr_of!(GDT_PTR));
    x86_reload_segments();
    core::arch::asm!("ltr ax", in("ax") TSS_SEL);

    let idt = core::ptr::addr_of_mut!(IDT);
    for entry in (*idt).iter_mut() {
        *entry = IdtEntry::zero();
    }
    let handler = int80_entry as *const () as u64;
    (*idt)[0x80] = IdtEntry {
        offset_low: handler as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0xEE,
        offset_mid: (handler >> 16) as u16,
        offset_high: (handler >> 32) as u32,
        zero: 0,
    };
    for (vector, handler) in x86_exception_table.iter().enumerate() {
        if vector == 0x0E {
            continue;
        }
        let handler = *handler;
        (*idt)[vector] = IdtEntry {
            offset_low: handler as u16,
            selector: 0x08,
            ist: if matches!(vector, 2 | 8 | 18) { 1 } else { 0 },
            type_attr: 0x8E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        };
    }
    let pf = pf_entry as *const () as u64;
    (*idt)[0x0E] = IdtEntry {
        offset_low: pf as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (pf >> 16) as u16,
        offset_high: (pf >> 32) as u32,
        zero: 0,
    };
    let timer = irq0_entry as *const () as u64;
    (*idt)[0x20] = IdtEntry {
        offset_low: timer as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (timer >> 16) as u16,
        offset_high: (timer >> 32) as u32,
        zero: 0,
    };
    for (vector, handler) in [
        (APIC_MAINT_VECTOR, ap_ipi_entry as *const () as u64),
        (APIC_TIMER_VECTOR, ap_timer_entry as *const () as u64),
        (APIC_HALT_VECTOR, ap_halt_entry as *const () as u64),
    ] {
        (*idt)[vector as usize] = IdtEntry {
            offset_low: handler as u16,
            selector: 0x08,
            ist: 0,
            type_attr: 0x8E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        };
    }
    core::ptr::write_volatile(
        core::ptr::addr_of_mut!(IDT_PTR),
        DescriptorTablePtr {
            limit: 256 * 16 - 1,
            base: idt as u64,
        },
    );
    core::arch::asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_PTR));
}

unsafe fn setup_pic_pit() {
    outb(0x20, 0x11);
    outb(0xA0, 0x11);
    outb(0x21, 0x20);
    outb(0xA1, 0x28);
    outb(0x21, 0x04);
    outb(0xA1, 0x02);
    outb(0x21, 0x01);
    outb(0xA1, 0x01);
    outb(0x21, 0xFE);
    outb(0xA1, 0xFF);
    let divisor: u16 = 11932;
    outb(0x43, 0x36);
    outb(0x40, divisor as u8);
    outb(0x40, (divisor >> 8) as u8);
}

impl Arch for X86Platform {
    const NAME: &'static str = X86_64::NAME;
    fn init() {
        X86_64::init();
    }
    fn putc(byte: u8) {
        X86_64::putc(byte);
    }
    fn now() -> u64 {
        X86_64::now()
    }
    fn irq_disable() {
        X86_64::irq_disable();
    }
    fn irq_enable() {
        X86_64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        X86_64::send_ipi(hart);
    }
    fn halt() -> ! {
        X86_64::halt()
    }
    fn exit(code: u8) -> ! {
        X86_64::exit(code)
    }
}

impl vifast_kernel::console::Console for X86Platform {
    const NAME: &'static str = X86_64::NAME;

    fn write_byte(byte: u8) {
        X86_64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        vifast_pte::x86_64::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::x86_64::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::x86_64::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::x86_64::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::x86_64::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::x86_64::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        2
    }

    fn install_kernel(&self, root_pa: u64, aux_pa: u64) {
        unsafe {
            let root = root_pa as *mut u64;
            let aux = aux_pa as *mut u64;
            let shared_pdpt = core::ptr::addr_of!(PDPT) as *const u64;
            *root = aux_pa | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
            *aux = *shared_pdpt;
            // PDPT[3] carries the supervisor-only LAPIC MMIO window.
            *aux.add(3) = *shared_pdpt.add(3);
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0, 0x4000_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::x86_64::USER_VA_START,
            vifast_pte::x86_64::USER_VA_END,
        )
    }

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) va);
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
            core::arch::asm!("mov cr3, {}", in(reg) cr3);
        }
    }

    fn sync_icache(&self, _pa: u64, _size: usize) {
        // x86 has coherent instruction and data caches.
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::x86_64::is_user(pte)
    }

    fn copy_to_frame(&self, pa: u64, bytes: &[u8]) {
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), pa as *mut u8, bytes.len());
        }
    }

    fn copy_frame_to_frame(&self, src: u64, dst: u64, size: usize) {
        unsafe {
            core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, size);
        }
    }

    fn zero_frame(&self, pa: u64, size: usize) {
        unsafe {
            core::ptr::write_bytes(pa as *mut u8, 0, size);
        }
    }
}

fn kernel_leaf_ok(root_pa: u64, va: u64, need_write: bool, need_exec: bool) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::x86_64::LEVELS {
        let idx = vifast_pte::x86_64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::x86_64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                return rights.contains(Rights::READ)
                    && !vifast_pte::x86_64::is_user(pte)
                    && (!need_write || pte & vifast_pte::x86_64::W != 0)
                    && (!need_exec || pte & vifast_pte::x86_64::NX == 0);
            }
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
}

fn kernel_leaf_nx(root_pa: u64, va: u64) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::x86_64::LEVELS {
        let idx = vifast_pte::x86_64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::x86_64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, _) => return pte & PAGE_NX != 0,
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
}

fn kernel_range_nx(root_pa: u64, start: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = match start.checked_add(len as u64 - 1) {
        Some(end) => end,
        None => return false,
    };
    let mut va = start & !0xfff;
    loop {
        if !kernel_leaf_nx(root_pa, va) {
            return false;
        }
        if va >= (end & !0xfff) {
            break;
        }
        va += 4096;
    }
    true
}

fn kernel_data_nx(root_pa: u64) -> bool {
    if unsafe { !NX_ENABLED } {
        return true;
    }
    kernel_data_ranges()
        .into_iter()
        .all(|(start, len)| kernel_range_nx(root_pa, start, len))
}

fn kernel_range_ok(root_pa: u64) -> bool {
    let (start, end) = PageOps.kernel_accessible_range();
    let mut va = start;
    while va < end {
        if !kernel_leaf_ok(root_pa, va, false, false) {
            return false;
        }
        va = match va.checked_add(4096) {
            Some(next) => next,
            None => break,
        };
    }
    true
}

static PAGE_OPS: PageOps = PageOps;

static mut KERNEL_STORAGE: Kernel = Kernel::new_const();

impl Platform for X86Platform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::x86_64::ELF_MACHINE;

    fn init_platform(info: PlatformInfo) {
        unsafe {
            init_acpi(info.acpi_rsdp);
            AP_TRAMPOLINE = info.ap_trampoline;
            build_kernel_page_tables();
            install_real_trampoline();
            init_cpu(info.hartid);
            calibrate_tsc_pit();
            // APs start before prepare_user(); install the shared IDT first so
            // their D4 idle window never runs with an empty IDT.
            setup_gdt_tss_idt();
            let trampoline = AP_TRAMPOLINE;
            if trampoline != 0 {
                vifast_kernel::klog!(X86Platform, "AP TRAMPOLINE OK (page={:#x})", trampoline);
            } else {
                vifast_kernel::klog!(X86Platform, "SMP FAIL (trampoline alloc)");
            }
        }
    }
    fn set_current_tcb(tcb: Option<usize>) {
        publish_current_tcb(tcb);
    }

    fn hart_id() -> usize {
        let state = cpu_state();
        if state.is_null() {
            0
        } else {
            unsafe { (*state).hart_id }
        }
    }

    fn init_kernel() -> &'static mut Kernel {
        unsafe {
            let kernel = &mut *core::ptr::addr_of_mut!(KERNEL_STORAGE);
            kernel.reset();
            kernel
        }
    }

    fn prepare_user() {
        unsafe {
            // The GDT/TSS/IDT was installed in init_platform before the AP
            // started; do not rebuild it while the AP may be idle with IF=1.
            init_cpu(0);
            enable_smep_smap();
            enable_pcid();
            if NX_ENABLED {
                vifast_kernel::klog!(X86Platform, "KERNEL DATA NX OK");
            }
            if PCID_ENABLED {
                vifast_kernel::klog!(X86Platform, "PCID ENABLED");
            }
        }
    }

    fn set_kernel(kernel: *mut Kernel) {
        unsafe {
            KERNEL_PTR = kernel;
        }
    }
    fn kernel() -> &'static mut Kernel {
        unsafe { &mut *KERNEL_PTR }
    }

    fn boot_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BOOT_POOL) as *mut u8, 131072)
        }
    }

    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_FRAME) as *mut u8, 262144)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 65536)
        }
    }

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(PML4) as u64
    }

    fn switch_root(root_pa: u64, asid: u16) -> bool {
        unsafe {
            if root_pa == 0 || (root_pa == active_root() && asid == active_asid()) {
                return false;
            }
            set_active_root(root_pa, asid);
            let cr3 = if PCID_ENABLED {
                root_pa | (((asid as u64) & 0x0fff) << 12)
            } else {
                root_pa
            };
            // A normal CR3 write invalidates the target PCID.  Other address
            // spaces keep their tagged TLB entries.
            core::arch::asm!("mov cr3, {}", in(reg) cr3);
            init_lapic();
            start_ap_real();
        }
        true
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }

    fn kernel_image_end() -> u64 {
        0
    }

    fn check_kernel_mappings(root_pa: u64) -> bool {
        kernel_range_ok(root_pa)
            && kernel_leaf_ok(
                root_pa,
                if unsafe { ACPI_LAPIC_BASE == 0 } {
                    LAPIC_DEFAULT
                } else {
                    unsafe { ACPI_LAPIC_BASE }
                },
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, unsafe { x86_exception_table[0] }, false, true)
            && kernel_data_nx(root_pa)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(TRAP_STACKS) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(IST_STACKS) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(GDT) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(TSSS) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(IDT) as u64, true, false)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(KERNEL_STORAGE) as u64,
                true,
                false,
            )
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if !user_range(ptr, out.len(), USER_CODE_VA, 0x2_0000)
            && !user_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            && !user_range(ptr, out.len(), USER_DATA_VA, 4096)
            && !user_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            && !user_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            && !user_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            && !user_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
        {
            return Err(KError::InvalidArg);
        }
        unsafe {
            if SMAP_ENABLED {
                core::arch::asm!("stac");
            }
            core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
            if SMAP_ENABLED {
                core::arch::asm!("clac");
            }
        }
        Ok(())
    }

    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if !user_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            && !user_range(ptr, input.len(), USER_DATA_VA, 4096)
            && !user_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            && !user_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            && !user_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            && !user_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
        {
            return Err(KError::InvalidArg);
        }
        unsafe {
            if SMAP_ENABLED {
                core::arch::asm!("stac");
            }
            core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
            if SMAP_ENABLED {
                core::arch::asm!("clac");
            }
        }
        Ok(())
    }

    fn context_for_user(entry: usize, stack: usize) -> TrapFrame {
        TrapFrame::user(entry, stack)
    }

    fn set_context_arg0(context: &mut TrapFrame, value: u64) {
        context.rdi = value;
    }
    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        let hart: usize;
        let trap_top: u64;
        unsafe {
            core::arch::asm!("mov {}, gs:[0]", out(reg) hart);
            core::arch::asm!("mov {}, gs:[16]", out(reg) trap_top);
        }
        assert!(hart < 2, "unexpected x86 per-CPU hart");
        assert!(trap_top != 0, "x86 per-CPU trap stack missing");
        assert_bkl_released();
        count_user_entry();
        // Entering ring 3: restore the user GS base and keep CpuState in
        // IA32_KERNEL_GS_BASE (standard swapgs convention).
        unsafe {
            core::arch::asm!("swapgs", options(nostack));
        }
        let base = context as *const TrapFrame as usize;
        unsafe {
            core::arch::asm!(
                "push {ss}", "push {rsp}", "push {rflags}", "push {cs}", "push {rip}",
                "mov r10, {base}",
                "mov r15, [r10 + {off_r15}]",
                "mov r14, [r10 + {off_r14}]",
                "mov r13, [r10 + {off_r13}]",
                "mov r12, [r10 + {off_r12}]",
                "mov r11, [r10 + {off_r11}]",
                "mov r9, [r10 + {off_r9}]",
                "mov r8, [r10 + {off_r8}]",
                "mov rbp, [r10 + {off_rbp}]",
                "mov rdi, [r10 + {off_rdi}]",
                "mov rsi, [r10 + {off_rsi}]",
                "mov rdx, [r10 + {off_rdx}]",
                "mov rcx, [r10 + {off_rcx}]",
                "mov rbx, [r10 + {off_rbx}]",
                "mov rax, [r10 + {off_rax}]",
                "mov r10, [r10 + {off_r10}]",
                "iretq",
                ss = in(reg) context.ss,
                rsp = in(reg) context.rsp,
                rflags = in(reg) context.rflags,
                cs = in(reg) context.cs,
                rip = in(reg) context.rip,
                base = in(reg) base,
                off_r15 = const core::mem::offset_of!(TrapFrame, r15),
                off_r14 = const core::mem::offset_of!(TrapFrame, r14),
                off_r13 = const core::mem::offset_of!(TrapFrame, r13),
                off_r12 = const core::mem::offset_of!(TrapFrame, r12),
                off_r11 = const core::mem::offset_of!(TrapFrame, r11),
                off_r10 = const core::mem::offset_of!(TrapFrame, r10),
                off_r9 = const core::mem::offset_of!(TrapFrame, r9),
                off_r8 = const core::mem::offset_of!(TrapFrame, r8),
                off_rbp = const core::mem::offset_of!(TrapFrame, rbp),
                off_rdi = const core::mem::offset_of!(TrapFrame, rdi),
                off_rsi = const core::mem::offset_of!(TrapFrame, rsi),
                off_rdx = const core::mem::offset_of!(TrapFrame, rdx),
                off_rcx = const core::mem::offset_of!(TrapFrame, rcx),
                off_rbx = const core::mem::offset_of!(TrapFrame, rbx),
                off_rax = const core::mem::offset_of!(TrapFrame, rax),
                options(noreturn),
            );
        }
    }

    fn write_result(
        context: &mut TrapFrame,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    ) {
        context.rax = error as u64;
        context.rdi = value;
        context.rsi = label;
        context.rdx = data[0];
        context.rcx = data[1];
        context.r8 = data[2];
        context.r9 = data[3];
    }

    fn start_timer() {
        unsafe {
            setup_pic_pit();
        }
    }
    fn reload_timer() {}
    fn ack_timer() {
        unsafe {
            outb(0x20, 0x20);
        }
    }

    fn block_read(lba: u64, out: &mut [u8; 512]) -> bool {
        unsafe { ata_read(lba, out) }
    }

    fn block_write(lba: u64, input: &[u8; 512]) -> bool {
        unsafe { ata_write(lba, input) }
    }

    fn secondary_wakeups() -> (usize, usize) {
        (
            AP_TIMER_WAKES.load(Ordering::Relaxed),
            AP_IPI_WAKES.load(Ordering::Relaxed),
        )
    }

    fn secondary_timer_ticks() -> usize {
        AP_TIMER_TICKS.load(Ordering::Relaxed)
    }

    fn secondary_user_ticks() -> usize {
        AP_USER_TICKS.load(Ordering::Relaxed)
    }

    fn secondary_preempts() -> usize {
        AP_PREEMPTS.load(Ordering::Relaxed)
    }

    fn secondary_idle_entries() -> usize {
        AP_IDLE_ENTRIES.load(Ordering::Relaxed)
    }

    fn secondary_wait_blocked() {
        let count = SECONDARY_WAIT_BLOCKS.fetch_add(1, Ordering::SeqCst) + 1;
        if count == 2 {
            POST_TIMER_TICKS.store(0, Ordering::Relaxed);
            POST_TIMER_WAKES.store(0, Ordering::Relaxed);
            POST_IDLE_ENTRIES.store(0, Ordering::Relaxed);
            POST_STARTED.store(true, Ordering::SeqCst);
        }
    }

    fn post_idle_stats() -> Option<(usize, usize, usize)> {
        if !POST_STARTED.load(Ordering::SeqCst) {
            return None;
        }
        Some((
            POST_TIMER_TICKS.load(Ordering::Relaxed),
            POST_IDLE_ENTRIES.load(Ordering::Relaxed),
            POST_TIMER_WAKES.load(Ordering::Relaxed),
        ))
    }

    fn post_idle_ready() -> bool {
        if !POST_STARTED.load(Ordering::SeqCst) {
            return false;
        }
        let ticks = POST_TIMER_TICKS.load(Ordering::Relaxed);
        ticks >= 5
            && POST_IDLE_ENTRIES.load(Ordering::Relaxed) >= ticks.saturating_sub(2)
            && POST_TIMER_WAKES.load(Ordering::Relaxed) >= ticks.saturating_sub(2)
    }

    fn wake_hart(hart: usize) {
        if hart == 1 && AP_ONLINE.load(Ordering::SeqCst) && AP_IDLE_WINDOW.load(Ordering::SeqCst) {
            let generation = AP_IDLE_GENERATION.load(Ordering::SeqCst);
            if WAKE_LAST_GENERATION.swap(generation, Ordering::AcqRel) != generation {
                unsafe {
                    let base = lapic_base();
                    AP_IPI_SENT.fetch_add(1, Ordering::AcqRel);
                    RESCHED_SENT.fetch_add(1, Ordering::Relaxed);
                    AP_RESCHED_PENDING.store(true, Ordering::Release);
                    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
                    send_icr(base, APIC_MAINT_VECTOR);
                }
            }
        }
    }

    fn sched_harts() -> u32 {
        0b11
    }

    fn can_wait_for_work() -> bool {
        true
    }

    fn wait_for_work(hart: usize) {
        loop {
            let ready = {
                let bkl = bkl_lock();
                let ready = X86Platform::kernel().has_ready_on(hart);
                drop(bkl);
                ready
            };
            if ready {
                return;
            }
            if hart == 1 {
                AP_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
                AP_IDLE_GENERATION.fetch_add(1, Ordering::SeqCst);
                WAKE_SENT_THIS_WINDOW.store(false, Ordering::SeqCst);
                if POST_STARTED.load(Ordering::Relaxed) {
                    POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
                }
                AP_IDLE_WINDOW.store(true, Ordering::SeqCst);
                unsafe {
                    core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
                }
                AP_IDLE_WINDOW.store(false, Ordering::SeqCst);
            } else {
                core::hint::spin_loop();
            }
        }
    }

    fn user_entries(hart: usize) -> usize {
        USER_ENTRIES
            .get(hart)
            .map(|counter| counter.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    fn resched_sent() -> usize {
        RESCHED_SENT.load(Ordering::Relaxed)
    }

    fn resched_wake() -> usize {
        RESCHED_WAKE.load(Ordering::Relaxed)
    }

    fn bkl_stats() -> [(usize, usize); 2] {
        [BKL.stats(0), BKL.stats(1)]
    }

    fn idle_entries() -> usize {
        AP_IDLE_ENTRIES.load(Ordering::Relaxed)
    }

    fn timer_rate() -> Option<(usize, usize)> {
        let base = BSP_RATE_BASE.load(Ordering::Acquire);
        Some((
            AP_TIMER_TICKS.load(Ordering::Relaxed),
            BSP_PIT_TICKS.load(Ordering::Relaxed).saturating_sub(base),
        ))
    }

    fn ipi_count() -> Option<(usize, usize)> {
        Some((
            AP_IPI_SENT.load(Ordering::Relaxed),
            AP_IPI_SEEN.load(Ordering::Relaxed),
        ))
    }

    fn start_smp() {
        // The AP is started from switch_root after the kernel page tables and
        // LAPIC are active.  D2: AP kernel IF=0, idle/poll via pause.
    }
}

// UEFI x86 uses the Win64 ABI: Rust entry points are `efiapi`, and trap.S
// passes the first three arguments in rcx/rdx/r8.
extern "efiapi" {
    fn int80_entry();
    fn irq0_entry();
    fn ap_ipi_entry();
    fn ap_timer_entry();
    fn ap_halt_entry();
    fn pf_entry();
    static x86_exception_table: [u64; 32];
    fn x86_reload_segments();
}

#[no_mangle]
extern "efiapi" fn x86_exception_handler(frame: *const u64, vector: u64, error: u64) -> ! {
    if X86Platform::hart_id() != 0 {
        vifast_kernel::klog!(
            X86Platform,
            "AP EXCEPTION vector={} err={:#x} rip={:#x} cs={:#x} rsp={:#x}",
            vector,
            error,
            unsafe { *frame.add(17) },
            unsafe { *frame.add(18) },
            unsafe { *frame.add(20) }
        );
        X86Platform::exit(1);
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("exception");
    if matches!(vector, 2 | 8 | 18) {
        vifast_kernel::klog!(
            X86Platform,
            "FATAL EXCEPTION vector={} err={:#x}",
            vector,
            error
        );
        X86Platform::exit(1);
    }
    let cs = unsafe { *frame.add(18) };
    if cs & 3 == 0 {
        vifast_kernel::klog!(
            X86Platform,
            "KERNEL EXCEPTION vector={} err={:#x} cs={:#x}",
            vector,
            error,
            cs
        );
        X86Platform::exit(1);
    }
    let mut context = TrapFrame::empty();
    vifast_runtime::fault::suspend::<X86Platform>(&mut context, vector, error);
    drop(bkl);
    assert_bkl_released();
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_page_fault(_frame: *const u64, error: u64) -> ! {
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("pagefault");
    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
    }
    /* error bit 2 is U/S: 1 means user mode, 0 means kernel mode. */
    if error & 0x4 == 0 {
        vifast_kernel::klog!(
            X86Platform,
            "KERNEL PAGE FAULT cr2={:#x} err={:#x}",
            cr2,
            error
        );
        X86_64::exit(1);
    }
    let mut context = TrapFrame::empty();
    vifast_runtime::fault::suspend::<X86Platform>(&mut context, error, cr2);
    drop(bkl);
    assert_bkl_released();
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_trap_handler(frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("trap");
    let frame = unsafe { &mut *frame };
    let number = frame.rax;
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.rcx, frame.r8, frame.r9,
    ];
    let waiting = vifast_runtime::syscall::dispatch::<X86Platform>(frame, number, args);
    drop(bkl);
    if waiting && X86Platform::hart_id() == 1 {
        loop {
            <X86Platform as Platform>::wait_for_work(1);
            let bkl = bkl_lock();
            if vifast_runtime::syscall::resume_blocked::<X86Platform>(frame) {
                drop(bkl);
                assert_bkl_released();
                count_user_entry();
                return;
            }
            drop(bkl);
        }
    }
    assert_bkl_released();
    count_user_entry();
}

#[no_mangle]
extern "efiapi" fn x86_irq_handler(frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    if X86Platform::hart_id() != 0 {
        vifast_kernel::klog!(X86Platform, "AP PIC IRQ");
        panic!("AP PIC IRQ");
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("irq0");
    if cpu_state() == unsafe { core::ptr::addr_of!(CPU_STATES[0]) as *mut _ } {
        BSP_PIT_TICKS.fetch_add(1, Ordering::Relaxed);
    }
    let frame = unsafe { &mut *frame };
    vifast_runtime::timer::on_timer::<X86Platform>(frame);
    drop(bkl);
    assert_bkl_released();
}

#[no_mangle]
extern "efiapi" fn x86_ap_irq_handler(frame: *mut TrapFrame, vector: u64) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    assert_irqs_disabled();
    let cs = unsafe { (*frame).cs };
    if cs & 3 == 0 {
        // D4: the only legal ring-0 interrupt path is the idle window.
        if !AP_IDLE_WINDOW.load(Ordering::SeqCst) {
            vifast_kernel::klog!(X86Platform, "KERNEL IRQ OUTSIDE IDLE");
            X86Platform::exit(1);
        }
        if vector == APIC_TIMER_VECTOR as u64 {
            AP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            AP_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
            if POST_STARTED.load(Ordering::Relaxed) {
                POST_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
                POST_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
            }
        } else if vector == APIC_MAINT_VECTOR as u64 {
            AP_IPI_SEEN.fetch_add(1, Ordering::Relaxed);
            AP_IPI_WAKES.fetch_add(1, Ordering::Relaxed);
            if AP_RESCHED_PENDING.swap(false, Ordering::AcqRel) {
                RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
            }
            unsafe { process_maintenance() };
        }
        unsafe { lapic_eoi() };
        return;
    }
    // Ring-3 path keeps the existing preemption contract.
    let bkl = bkl_lock();
    assert_current_tcb("ap-user-irq");
    if vector == APIC_TIMER_VECTOR as u64 {
        unsafe { lapic_eoi() };
        AP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        AP_USER_TICKS.fetch_add(1, Ordering::Relaxed);
        let changed = vifast_runtime::timer::on_timer::<X86Platform>(unsafe { &mut *frame });
        if changed {
            AP_PREEMPTS.fetch_add(1, Ordering::Relaxed);
        }
    } else if vector == APIC_MAINT_VECTOR as u64 {
        AP_IPI_SEEN.fetch_add(1, Ordering::Relaxed);
        if AP_RESCHED_PENDING.swap(false, Ordering::AcqRel) {
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
        }
        unsafe {
            process_maintenance();
            lapic_eoi();
        }
    }
    count_user_entry();
    drop(bkl);
    assert_bkl_released();
}
