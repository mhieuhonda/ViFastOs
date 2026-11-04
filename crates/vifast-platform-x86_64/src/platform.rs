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

core::arch::global_asm!(include_str!("trap.S"));

#[allow(dead_code)]
#[repr(align(4096))]
struct Page([u8; 4096]);

#[allow(dead_code)]
#[repr(align(4096))]
struct Stack([u8; 32768]);

static mut USER_PAGE: Page = Page([0; 4096]);
static mut USER_STACK: Stack = Stack([0; 32768]);
static mut USER_FRAME: Page = Page([0; 4096]);
static mut CONTEXTS: [TrapFrame; 32] = [TrapFrame::empty(); 32];
static mut TRAP_STACK: Stack = Stack([0; 32768]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();

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

unsafe fn invlpg(va: u64) {
    core::arch::asm!("invlpg [{}]", in(reg) va);
}

unsafe fn make_user_page(va: u64, executable: bool) -> bool {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    let mut table = cr3 & 0x000f_ffff_ffff_f000;
    for shift in [39u32, 30, 21] {
        let entry = (table as *mut u64).add(((va >> shift) & 0x1ff) as usize);
        let mut value = core::ptr::read_volatile(entry);
        if value & 1 == 0 {
            return false;
        }
        if value & 0x80 != 0 {
            value |= (1 << 2) | (1 << 1);
            if executable {
                value &= !(1u64 << 63);
            }
            core::ptr::write_volatile(entry, value);
            invlpg(va);
            return true;
        }
        value |= 1 << 2;
        core::ptr::write_volatile(entry, value);
        table = value & 0x000f_ffff_ffff_f000;
    }
    let entry = (table as *mut u64).add(((va >> 12) & 0x1ff) as usize);
    let mut value = core::ptr::read_volatile(entry);
    if value & 1 == 0 {
        return false;
    }
    value |= (1 << 2) | (1 << 1);
    if executable {
        value &= !(1u64 << 63);
    } else {
        value |= 1u64 << 63;
    }
    core::ptr::write_volatile(entry, value);
    invlpg(va);
    true
}

unsafe fn setup_gdt_tss_idt() {
    let tss = core::ptr::addr_of_mut!(TSS);
    (*tss).rsp0 = core::ptr::addr_of!(TRAP_STACK) as u64 + 16384;
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

fn user_readable(ptr: u64, len: usize) -> bool {
    let code = core::ptr::addr_of!(USER_PAGE) as u64;
    let data = core::ptr::addr_of!(USER_FRAME) as u64;
    (ptr >= code && ptr as usize + len <= code as usize + 4096)
        || (ptr >= data && ptr as usize + len <= data as usize + 4096)
}

fn user_writable(ptr: u64, len: usize) -> bool {
    let data = core::ptr::addr_of!(USER_FRAME) as u64;
    ptr >= data && ptr as usize + len <= data as usize + 4096
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

impl Platform for X86Platform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = 0;
    const USER_DATA_VA: usize = 0;
    const DIRECT_VSPACE: bool = true;

    fn user_code_base() -> usize {
        core::ptr::addr_of!(USER_PAGE) as usize
    }
    fn stack_a_top() -> usize {
        core::ptr::addr_of!(USER_STACK) as usize + 0x4000
    }
    fn stack_b_top() -> usize {
        core::ptr::addr_of!(USER_STACK) as usize + 0x8000
    }

    fn init_platform(_info: PlatformInfo) {}

    fn prepare_user() {
        unsafe {
            setup_gdt_tss_idt();
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
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_FRAME) as *mut u8, 4096)
        }
    }

    fn build_page_tables() {}

    fn vspace_root() -> u64 {
        let cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
        }
        cr3
    }

    fn map_user_page(va: u64, pa: u64, _rights: Rights) -> Result<(), KError> {
        if va != pa {
            return Err(KError::InvalidArg);
        }
        let cr0: u64;
        unsafe {
            core::arch::asm!("mov {}, cr0", out(reg) cr0);
            core::arch::asm!("mov cr0, {}", in(reg) cr0 & !(1 << 16));
        }
        let ok = unsafe { make_user_page(pa, false) };
        unsafe {
            core::arch::asm!("mov cr0, {}", in(reg) cr0);
        }
        if ok {
            Ok(())
        } else {
            Err(KError::MissingPageTable)
        }
    }

    fn unmap_user_page(va: u64) -> Result<(), KError> {
        unsafe {
            invlpg(va);
        }
        Ok(())
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !user_readable(ptr, out.len()) {
            return Err(KError::InvalidArg);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
        }
        Ok(())
    }

    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError> {
        if !user_writable(ptr, input.len()) {
            return Err(KError::InvalidArg);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
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
        let code = core::ptr::addr_of!(USER_PAGE) as u64;
        let stack = core::ptr::addr_of!(USER_STACK) as u64;
        let cr0: u64;
        unsafe {
            core::arch::asm!("mov {}, cr0", out(reg) cr0);
            core::arch::asm!("mov cr0, {}", in(reg) cr0 & !(1 << 16));
        }
        if !unsafe { make_user_page(code, true) } {
            X86_64::write_str("user code page not mapped\r\n");
            X86_64::exit(1);
        }
        for i in 0..8u64 {
            if !unsafe { make_user_page(stack + i * 4096, false) } {
                X86_64::write_str("user stack page not mapped\r\n");
                X86_64::exit(1);
            }
        }
        unsafe {
            core::arch::asm!("mov cr0, {}", in(reg) cr0);
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

extern "C" {
    fn int80_entry();
    fn irq0_entry();
    fn x86_reload_segments();
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
