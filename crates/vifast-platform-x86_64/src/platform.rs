//! x86_64 UEFI platform: GDT/TSS/IDT, ring3, legacy PIC/PIT and user paging.

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

fn poll_maintenance() {
    // No AP bootstrap yet; x86 has no maintenance IPI to service.
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

    zero_table(pml4);
    zero_table(pdpt);
    zero_table(identity);

    /* PML4[0] -> PDPT; PDPT[0] covers 0..1 GiB identity for kernel/UEFI
    code, stacks, GDT/TSS/IDT and page tables. */
    (*pml4).0[0] = table_pa(pdpt) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    (*pdpt).0[0] = table_pa(identity) | PAGE_PRESENT | PAGE_WRITE;
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

fn assert_current_tcb() {
    let state = cpu_state();
    if state.is_null() {
        return;
    }
    let expected = X86Platform::kernel()
        .current()
        .map(|tcb| tcb.index as usize)
        .unwrap_or(usize::MAX);
    assert_eq!(
        unsafe { (*state).current_tcb },
        expected,
        "per-CPU current_tcb mismatch"
    );
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    CPU_STATES[index].hart_id = hartid;
    CPU_STATES[index].current_tcb = 0;
    CPU_STATES[index].trap_stack_top = core::ptr::addr_of!(TRAP_STACKS[index]) as u64 + 16384;
    if index == 0 {
        write_msr(IA32_GS_BASE, core::ptr::addr_of!(CPU_STATES[0]) as u64);
        write_msr(IA32_KERNEL_GS_BASE, 0);
    }
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
            build_kernel_page_tables();
            init_cpu(info.hartid);
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
            setup_gdt_tss_idt();
            // x86_reload_segments loads GS with a null-base selector, which
            // clobbers IA32_GS_BASE.  Re-publish the per-CPU pointer after it.
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
        assert!(!BKL.is_held(), "BKL held before user return");
        let hart: usize;
        let trap_top: u64;
        unsafe {
            core::arch::asm!("mov {}, gs:[0]", out(reg) hart);
            core::arch::asm!("mov {}, gs:[16]", out(reg) trap_top);
        }
        assert_eq!(hart, 0, "unexpected x86 per-CPU hart");
        assert!(trap_top != 0, "x86 per-CPU trap stack missing");
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

    fn start_smp() {
        X86_64::write_str("SMP SKIP (x86 AP bootstrap pending)\r\n");
    }
}

// UEFI x86 uses the Win64 ABI: Rust entry points are `efiapi`, and trap.S
// passes the first three arguments in rcx/rdx/r8.
extern "efiapi" {
    fn int80_entry();
    fn irq0_entry();
    fn pf_entry();
    static x86_exception_table: [u64; 32];
    fn x86_reload_segments();
}

#[no_mangle]
extern "efiapi" fn x86_exception_handler(frame: *const u64, vector: u64, error: u64) -> ! {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
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
    assert!(!BKL.is_held(), "BKL held before user fault return");
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_page_fault(_frame: *const u64, error: u64) -> ! {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
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
    assert!(!BKL.is_held(), "BKL held before user page-fault return");
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_trap_handler(frame: *mut TrapFrame) {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
    let frame = unsafe { &mut *frame };
    let number = frame.rax;
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.rcx, frame.r8, frame.r9,
    ];
    vifast_runtime::syscall::dispatch::<X86Platform>(frame, number, args);
    drop(bkl);
    assert!(!BKL.is_held(), "BKL held on x86 syscall return");
}

#[no_mangle]
extern "efiapi" fn x86_irq_handler(frame: *mut TrapFrame) {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
    let frame = unsafe { &mut *frame };
    vifast_runtime::timer::on_timer::<X86Platform>(frame);
    drop(bkl);
    assert!(!BKL.is_held(), "BKL held on x86 IRQ return");
}
