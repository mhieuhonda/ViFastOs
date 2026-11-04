//! x86_64 UEFI platform: GDT/TSS/IDT, ring3, legacy PIC/PIT and user paging.

use vifast_abi::{KError, Rights};
use vifast_arch::Arch;
use vifast_kernel::Kernel;
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
const USER_STACK_A_TOP: usize = vifast_abi::user_layout::STACK_A_TOP;
const USER_STACK_B_TOP: usize = vifast_abi::user_layout::STACK_B_TOP;
const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const PAGE_PRESENT: u64 = 1;
const PAGE_WRITE: u64 = 2;
const PAGE_USER: u64 = 4;
const PAGE_LARGE: u64 = 0x80;
const PAGE_NX: u64 = 1 << 63;

core::arch::global_asm!(include_str!("trap.S"));

#[allow(dead_code)]
#[repr(align(4096))]
struct Page([u8; 4096]);

#[allow(dead_code)]
#[repr(align(4096))]
struct Stack([u8; 32768]);

#[repr(align(4096))]
struct Table([u64; 512]);

static mut PML4: Table = Table([0; 512]);
static mut PDPT: Table = Table([0; 512]);
static mut PD_IDENTITY: Table = Table([0; 512]);
static mut PD_USER: Table = Table([0; 512]);
static mut PT_USER: Table = Table([0; 512]);
static mut USER_PAGE: Page = Page([0; 4096]);
static mut USER_STACK: Stack = Stack([0; 32768]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_FRAME: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 32] = [TrapFrame::empty(); 32];
static mut TRAP_STACK: Stack = Stack([0; 32768]);
static mut IST_STACK: Stack = Stack([0; 32768]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
static mut SMAP_ENABLED: bool = false;

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
static mut TSS: Tss = Tss::zero();

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

unsafe fn build_user_page_tables() {
    let pml4 = core::ptr::addr_of_mut!(PML4) as *mut Table;
    let pdpt = core::ptr::addr_of_mut!(PDPT) as *mut Table;
    let identity = core::ptr::addr_of_mut!(PD_IDENTITY) as *mut Table;
    let user_pd = core::ptr::addr_of_mut!(PD_USER) as *mut Table;
    let user_pt = core::ptr::addr_of_mut!(PT_USER) as *mut Table;

    zero_table(pml4);
    zero_table(pdpt);
    zero_table(identity);
    zero_table(user_pd);
    zero_table(user_pt);

    /* PML4[0] -> PDPT; PDPT[0] covers 0..1 GiB identity for kernel/UEFI
    code, stacks, GDT/TSS/IDT and page tables. */
    (*pml4).0[0] = table_pa(pdpt) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    (*pdpt).0[0] = table_pa(identity) | PAGE_PRESENT | PAGE_WRITE;
    for i in 0..512usize {
        (*identity).0[i] = (i as u64 * 0x20_0000) | PAGE_PRESENT | PAGE_WRITE | PAGE_LARGE;
    }

    /* User region starts at 64 GiB, in PDPT[64], outside the kernel
    identity map.  PT_USER provides 4 KiB mappings for code, stack and data. */
    (*pdpt).0[64] = table_pa(user_pd) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    (*user_pd).0[0] = table_pa(user_pt) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;

    let code_pa = core::ptr::addr_of!(USER_PAGE) as u64;
    let stack_pa = core::ptr::addr_of!(USER_STACK) as u64;
    (*user_pt).0[0] = code_pa | PAGE_PRESENT | PAGE_USER;
    for i in 0..8usize {
        (*user_pt).0[1 + i] =
            (stack_pa + (i as u64) * 4096) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | PAGE_NX;
    }
}

fn user_range(ptr: u64, len: usize, base: usize, size: usize) -> bool {
    ptr >= base as u64 && ptr as usize + len <= base + size
}

unsafe fn setup_gdt_tss_idt() {
    let tss = core::ptr::addr_of_mut!(TSS);
    (*tss).rsp0 = core::ptr::addr_of!(TRAP_STACK) as u64 + 16384;
    (*tss).ist[0] = core::ptr::addr_of!(IST_STACK) as u64 + 32768;
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

    fn install_kernel(&self, _root_pa: u64) {}

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

    fn zero_frame(&self, pa: u64, size: usize) {
        unsafe {
            core::ptr::write_bytes(pa as *mut u8, 0, size);
        }
    }
}

static PAGE_OPS: PageOps = PageOps;

static mut KERNEL_STORAGE: Kernel = Kernel::new_const();

impl Platform for X86Platform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;

    fn stack_a_top() -> usize {
        USER_STACK_A_TOP
    }
    fn stack_b_top() -> usize {
        USER_STACK_B_TOP
    }

    fn init_platform(_info: PlatformInfo) {}

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
            enable_smep_smap();
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

    fn code_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_PAGE) as *mut u8, 4096)
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

    fn build_page_tables() {
        unsafe {
            build_user_page_tables();
        }
    }

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(PML4) as u64
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }
    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !user_range(ptr, out.len(), USER_CODE_VA, 4096)
            && !user_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            && !user_range(ptr, out.len(), USER_DATA_VA, 4096)
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
        if !user_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            && !user_range(ptr, input.len(), USER_DATA_VA, 4096)
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
    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) core::ptr::addr_of!(PML4) as u64);
            core::arch::asm!(
                "push {ss}", "push {rsp}", "push {rflags}", "push {cs}", "push {rip}", "iretq",
                ss = in(reg) context.ss,
                rsp = in(reg) context.rsp,
                rflags = in(reg) context.rflags,
                cs = in(reg) context.cs,
                rip = in(reg) context.rip,
            );
        }
        unreachable!()
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
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_page_fault(_frame: *const u64, error: u64) -> ! {
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
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    let number = frame.rax;
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.rcx, frame.r8, frame.r9,
    ];
    vifast_runtime::syscall::dispatch::<X86Platform>(frame, number, args);
}

#[no_mangle]
extern "efiapi" fn x86_irq_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    vifast_runtime::timer::on_timer::<X86Platform>(frame);
}
