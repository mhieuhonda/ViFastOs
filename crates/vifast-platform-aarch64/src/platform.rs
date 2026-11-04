//! AArch64 QEMU `virt` platform: 4-level MMU, GICv2, generic timer and PSCI.

use core::sync::atomic::{AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::Arch;
use vifast_kernel::Kernel;
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_aarch64::Aarch64;

pub struct Aarch64Platform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
pub const USER_STACK_A_TOP: usize = vifast_abi::user_layout::STACK_A_TOP;
pub const USER_STACK_B_TOP: usize = vifast_abi::user_layout::STACK_B_TOP;
const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const USER_SPSR: u64 = 0x340;
const GICD: u64 = 0x0800_0000;
const GICC: u64 = 0x0801_0000;
const TIMER_IRQ: u32 = 30;
const TIMER_HZ: u64 = 100;

const DESC_TABLE: u64 = 0b11;
const DESC_BLOCK: u64 = 0b1;
const AF: u64 = 1 << 10;
const SH_INNER: u64 = 0b11 << 8;
const AP_USER_RW: u64 = 1 << 6;
const AP_USER_RO: u64 = (1 << 6) | (1 << 7);
const ATTR_DEVICE: u64 = 1 << 2;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

#[allow(dead_code)]
#[repr(align(4096))]
struct Page([u8; 4096]);

#[allow(dead_code)]
#[repr(align(4096))]
struct UserStack([u8; 32768]);

static mut L0: Table = Table([0; 512]);
static mut L1: Table = Table([0; 512]);
static mut L2_LOW: Table = Table([0; 512]);
static mut L2_KERNEL: Table = Table([0; 512]);
static mut L2_USER: Table = Table([0; 512]);
static mut L3_USER: Table = Table([0; 512]);
static mut USER_CODE: Page = Page([0; 4096]);
static mut USER_STACK: UserStack = UserStack([0; 32768]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_DATA: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 32] = [TrapFrame::empty(); 32];
static mut TIMER_INTERVAL: u64 = 0;
static mut TRAP_STACK: UserStack = UserStack([0; 32768]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    regs: [u64; 31],
    sp: u64,
    elr: u64,
    spsr: u64,
    esr: u64,
    far: u64,
}

impl TrapFrame {
    const fn empty() -> TrapFrame {
        TrapFrame {
            regs: [0; 31],
            sp: 0,
            elr: 0,
            spsr: 0,
            esr: 0,
            far: 0,
        }
    }

    fn user(entry: usize, stack: usize) -> TrapFrame {
        let mut frame = TrapFrame::empty();
        frame.sp = stack as u64;
        frame.elr = entry as u64;
        frame.spsr = USER_SPSR;
        frame
    }
}

fn table_desc(pa: u64) -> u64 {
    pa | DESC_TABLE
}
fn block_desc(pa: u64, attr: u64, ap: u64, pxn: u64, uxn: u64, sh: u64) -> u64 {
    pa | DESC_BLOCK | attr | ap | sh | AF | pxn | uxn
}
fn page_desc(pa: u64, attr: u64, ap: u64, pxn: u64, uxn: u64, sh: u64) -> u64 {
    pa | 0b11 | attr | ap | sh | AF | pxn | uxn
}

fn read32(addr: u64) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}
fn write32(addr: u64, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}
fn write8(addr: u64, value: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

fn init_gic() {
    write32(GICD + 0x000, 1);
    write32(GICD + 0x100, 1 << TIMER_IRQ);
    write8(GICD + 0x400 + TIMER_IRQ as u64, 0xA0);
    write8(GICD + 0x800 + TIMER_IRQ as u64, 1);
    write32(GICC + 0x004, 0xFF);
    write32(GICC + 0x000, 1);
}

fn init_timer() {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq);
        TIMER_INTERVAL = (freq / TIMER_HZ).max(1);
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) TIMER_INTERVAL);
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64);
    }
}

fn page_in_range(ptr: u64, len: usize, base: usize, size: usize) -> bool {
    ptr >= base as u64 && ptr as usize + len <= base + size
}

fn pan_supported() -> bool {
    let mmfr1: u64;
    unsafe {
        core::arch::asm!("mrs {}, id_aa64mmfr1_el1", out(reg) mmfr1);
    }
    ((mmfr1 >> 20) & 0xF) != 0
}

#[no_mangle]
pub extern "C" fn rust_secondary(_context: u64) -> ! {
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

impl Arch for Aarch64Platform {
    const NAME: &'static str = Aarch64::NAME;
    fn init() {
        Aarch64::init();
    }
    fn putc(byte: u8) {
        Aarch64::putc(byte);
    }
    fn now() -> u64 {
        Aarch64::now()
    }
    fn irq_disable() {
        Aarch64::irq_disable();
    }
    fn irq_enable() {
        Aarch64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        Aarch64::send_ipi(hart);
    }
    fn halt() -> ! {
        Aarch64::halt()
    }
    fn exit(code: u8) -> ! {
        Aarch64::exit(code)
    }
}

impl vifast_kernel::console::Console for Aarch64Platform {
    const NAME: &'static str = Aarch64::NAME;

    fn write_byte(byte: u8) {
        Aarch64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        4
    }

    fn index(&self, va: u64, level: usize) -> usize {
        ((va >> (12 + 9 * (3 - level))) & 0x1ff) as usize
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
        let ap = if rights.contains(Rights::WRITE) {
            AP_USER_RW
        } else {
            AP_USER_RO
        };
        let uxn = if rights.contains(Rights::EXEC) {
            0
        } else {
            UXN
        };
        page_desc(pa, 0, ap, PXN, uxn, SH_INNER)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        table_desc(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        if entry & 0b11 == 0 {
            return vifast_kernel::Pte::Invalid;
        }
        let pa = entry & 0x0000_ffff_ffff_f000;
        if entry & 0b11 == 0b11 {
            vifast_kernel::Pte::Table(pa)
        } else {
            vifast_kernel::Pte::Leaf(pa, Rights::READ)
        }
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        if entry & 0b11 == 0 {
            return vifast_kernel::Pte::Invalid;
        }
        let pa = entry & 0x0000_ffff_ffff_f000;
        if level == 3 {
            return vifast_kernel::Pte::Leaf(pa, Rights::READ);
        }
        if entry & 0b11 == 0b01 {
            vifast_kernel::Pte::Leaf(pa, Rights::READ)
        } else {
            vifast_kernel::Pte::Table(pa)
        }
    }

    fn install_kernel(&self, _root_pa: u64) {}

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("tlbi vae1is, {}", in(reg) va);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            core::arch::asm!("tlbi vmalle1is");
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
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

impl Platform for Aarch64Platform {
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
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_CODE) as *mut u8, 4096)
        }
    }
    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_DATA) as *mut u8, 262144)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 65536)
        }
    }

    fn build_page_tables() {
        unsafe {
            let l0 = core::ptr::addr_of_mut!(L0) as *mut Table;
            let l1 = core::ptr::addr_of_mut!(L1) as *mut Table;
            let low = core::ptr::addr_of_mut!(L2_LOW) as *mut Table;
            let kernel = core::ptr::addr_of_mut!(L2_KERNEL) as *mut Table;
            let user_l2 = core::ptr::addr_of_mut!(L2_USER) as *mut Table;
            let user_l3 = core::ptr::addr_of_mut!(L3_USER) as *mut Table;
            (*l0).0[0] = table_desc(l1 as u64);
            (*l1).0[0] = table_desc(low as u64);
            (*l1).0[1] = table_desc(kernel as u64);
            /* Shared 64 GiB user window: L1[64] -> L2_USER -> L3_USER. */
            (*l1).0[64] = table_desc(user_l2 as u64);
            (*user_l2).0[0] = table_desc(user_l3 as u64);
            (*low).0[64] = block_desc(0x0800_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
            (*low).0[72] = block_desc(0x0900_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
            let code_pa = core::ptr::addr_of!(USER_CODE) as u64;
            let stack_pa = core::ptr::addr_of!(USER_STACK) as u64;
            (*user_l3).0[0] = page_desc(code_pa, 0, AP_USER_RO, PXN, 0, SH_INNER);
            for i in 0..8usize {
                (*user_l3).0[1 + i] = page_desc(
                    stack_pa + (i as u64) * 4096,
                    0,
                    AP_USER_RW,
                    PXN,
                    UXN,
                    SH_INNER,
                );
            }
            for i in 0..4usize {
                let pa = 0x4000_0000u64 + i as u64 * 2 * 1024 * 1024;
                (*kernel).0[i] = block_desc(pa, 0, 0, 0, UXN, SH_INNER);
            }
        }
    }

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(L0) as u64
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }
    fn map_user_page(va: u64, pa: u64, rights: Rights) -> Result<(), KError> {
        if va < USER_CODE_VA as u64 || va >= USER_CODE_VA as u64 + 0x20_0000 {
            return Err(KError::InvalidArg);
        }
        if pa % 4096 != 0 {
            return Err(KError::Alignment);
        }
        if rights.contains(Rights::WRITE) && rights.contains(Rights::EXEC) {
            return Err(KError::NoRights);
        }
        let ap = if rights.contains(Rights::WRITE) {
            AP_USER_RW
        } else {
            AP_USER_RO
        };
        let uxn = if rights.contains(Rights::EXEC) {
            0
        } else {
            UXN
        };
        let desc = page_desc(pa, 0, ap, PXN, uxn, SH_INNER);
        unsafe {
            let l3 = core::ptr::addr_of_mut!(L3_USER) as *mut Table;
            let index = ((va >> 12) & 0x1ff) as usize;
            (*l3).0[index] = desc;
            core::arch::asm!("tlbi vae1is, {}", in(reg) va);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
        Ok(())
    }

    fn unmap_user_page(va: u64) -> Result<(), KError> {
        if va < USER_CODE_VA as u64 || va >= USER_CODE_VA as u64 + 0x20_0000 {
            return Err(KError::InvalidArg);
        }
        unsafe {
            let l3 = core::ptr::addr_of_mut!(L3_USER) as *mut Table;
            let index = ((va >> 12) & 0x1ff) as usize;
            (*l3).0[index] = 0;
            core::arch::asm!("tlbi vae1is, {}", in(reg) va);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
        Ok(())
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if page_in_range(ptr, out.len(), USER_CODE_VA, 4096)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, out.len(), USER_DATA_VA, 4096)
        {
            unsafe {
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #0");
                }
                core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #1");
                }
            }
            Ok(())
        } else {
            Err(KError::InvalidArg)
        }
    }

    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError> {
        if page_in_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, input.len(), USER_DATA_VA, 4096)
        {
            unsafe {
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #0");
                }
                core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #1");
                }
            }
            Ok(())
        } else {
            Err(KError::InvalidArg)
        }
    }

    fn context_for_user(entry: usize, stack: usize) -> TrapFrame {
        TrapFrame::user(entry, stack)
    }

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        let l0_pa = core::ptr::addr_of!(L0) as u64;
        let tcr: u64 = 16 | (1 << 23) | (2 << 32);
        let mair: u64 = 0xFF;
        unsafe {
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) l0_pa);
            core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
            core::arch::asm!("msr mair_el1, {}", in(reg) mair);
            let mut sctlr: u64;
            core::arch::asm!("mrs {}, sctlr_el1", out(reg) sctlr);
            sctlr &= !(1 << 19);
            if pan_supported() {
                sctlr |= 1 << 23;
            }
            sctlr |= (1 << 0) | (1 << 2) | (1 << 12);
            core::arch::asm!("msr vbar_el1, {}", in(reg) core::ptr::addr_of!(vector_table) as u64);
            core::arch::asm!("dsb sy; isb");
            core::arch::asm!("msr sctlr_el1, {}", in(reg) sctlr);
            core::arch::asm!("isb");
            let trap_top = core::ptr::addr_of!(TRAP_STACK) as u64 + 16384;
            core::arch::asm!("msr spsel, #1");
            core::arch::asm!("mov sp, {}", in(reg) trap_top);
            core::arch::asm!("msr sp_el0, {}", in(reg) context.sp);
            core::arch::asm!("msr elr_el1, {}", in(reg) context.elr);
            core::arch::asm!("msr spsr_el1, {}", in(reg) context.spsr);
            core::arch::asm!("eret");
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
        context.regs[0] = error as u64;
        context.regs[1] = value;
        context.regs[2] = label;
        context.regs[3] = data[0];
        context.regs[4] = data[1];
        context.regs[5] = data[2];
        context.regs[6] = data[3];
    }

    fn start_timer() {
        init_gic();
        init_timer();
    }

    fn reload_timer() {
        unsafe {
            core::arch::asm!("msr cntp_tval_el0, {}", in(reg) TIMER_INTERVAL);
            core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64);
        }
    }

    fn ack_timer() {
        write32(GICC + 0x010, TIMER_IRQ);
    }

    fn start_smp() {
        let entry = secondary_entry as *const () as u64;
        let result: u64;
        unsafe {
            core::arch::asm!(
                "hvc #0",
                in("x0") 0xC400_0003u64,
                in("x1") 1u64,
                in("x2") entry,
                in("x3") 0u64,
                lateout("x0") result,
            );
        }
        let mut spins = 0u32;
        while SECONDARY_ONLINE.load(Ordering::SeqCst) == 0 && spins < 10_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if SECONDARY_ONLINE.load(Ordering::SeqCst) > 0 {
            vifast_kernel::klog!(Aarch64Platform, "SMP OK (harts=2, psci={})", result as i64);
        } else {
            vifast_kernel::klog!(Aarch64Platform, "SMP SKIP (psci={})", result as i64);
        }
    }
}

#[no_mangle]
extern "C" fn aarch64_kernel_fault() -> ! {
    let esr: u64;
    let elr: u64;
    let far: u64;
    unsafe {
        core::arch::asm!("mrs {}, esr_el1", out(reg) esr);
        core::arch::asm!("mrs {}, elr_el1", out(reg) elr);
        core::arch::asm!("mrs {}, far_el1", out(reg) far);
    }
    vifast_kernel::klog!(
        Aarch64Platform,
        "KERNEL EXCEPTION ESR={:#x} ELR={:#x} FAR={:#x}",
        esr,
        elr,
        far
    );
    Aarch64::exit(1)
}

extern "C" {
    static vector_table: u8;
    fn secondary_entry();
}

#[no_mangle]
extern "C" fn aarch64_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    let ec = (frame.esr >> 26) & 0x3f;
    if ec != 0x15 {
        vifast_runtime::fault::suspend::<Aarch64Platform>(frame, frame.esr, frame.far);
        return;
    }
    let number = frame.regs[8];
    let args = [
        frame.regs[0],
        frame.regs[1],
        frame.regs[2],
        frame.regs[3],
        frame.regs[4],
        frame.regs[5],
    ];
    vifast_runtime::syscall::dispatch::<Aarch64Platform>(frame, number, args);
}

#[no_mangle]
extern "C" fn aarch64_irq_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    let iar = read32(GICC + 0x00C);
    let irq = iar & 0x3FF;
    if irq != TIMER_IRQ {
        write32(GICC + 0x010, irq);
        return;
    }
    vifast_runtime::timer::on_timer::<Aarch64Platform>(frame);
}
