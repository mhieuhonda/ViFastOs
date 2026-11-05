//! AArch64 QEMU `virt` platform: 4-level MMU, GICv2, generic timer and PSCI.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::bkl::BigKernelLock;
use vifast_arch::{Arch, CpuState, MAX_HARTS};
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_aarch64::Aarch64;

pub struct Aarch64Platform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
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
const ATTR_DEVICE: u64 = 1 << 2;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

#[derive(Clone, Copy)]
#[allow(dead_code)]
#[repr(align(4096))]
struct UserStack([u8; 32768]);

static mut L0: Table = Table([0; 512]);
static mut L1: Table = Table([0; 512]);
static mut L2_LOW: Table = Table([0; 512]);
static mut L2_KERNEL: Table = Table([0; 512]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_DATA: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 128] = [TrapFrame::empty(); 128];
static mut TIMER_INTERVAL: u64 = 0;
static mut TRAP_STACKS: [UserStack; MAX_HARTS] = [UserStack([0; 32768]); MAX_HARTS];
static mut CPU_STATES: [CpuState; MAX_HARTS] = [CpuState::empty(); MAX_HARTS];
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 131072]);
static mut BOOT_POOL: BootPool = BootPool([0; 131072]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);
static IPI_PENDING: AtomicBool = AtomicBool::new(false);
static IPI_ACK: AtomicUsize = AtomicUsize::new(0);
static BKL: BigKernelLock = BigKernelLock::new();

fn poll_maintenance() {
    let state = cpu_state();
    if state.is_null() || unsafe { (*state).hart_id } == 0 {
        return;
    }
    if IPI_PENDING.swap(false, Ordering::SeqCst) {
        unsafe {
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
        IPI_ACK.fetch_add(1, Ordering::SeqCst);
    }
}

fn assert_irqs_disabled() {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
    }
    assert!(daif & (1 << 7) != 0, "BKL entered with IRQ unmasked");
}

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
    write32(GICD + 0x100, (1 << TIMER_IRQ) | 1);
    write8(GICD + 0x400 + TIMER_IRQ as u64, 0xA0);
    write8(GICD + 0x800 + TIMER_IRQ as u64, 1);
    write32(GICC + 0x004, 0xFF);
    write32(GICC + 0x000, 1);
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    CPU_STATES[index].hart_id = hartid;
    CPU_STATES[index].current_tcb = 0;
    let stack = core::ptr::addr_of!(TRAP_STACKS[index]) as u64;
    CPU_STATES[index].trap_stack_top = stack + 16384;
    let state = core::ptr::addr_of!(CPU_STATES[index]) as u64;
    core::arch::asm!("msr tpidr_el1, {}", in(reg) state);
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
    // Maintenance-only hart: do NOT take BKL here.  Hart 0 holds BKL while
    // waiting for the TLB-shootdown ACK from this loop.
    unsafe { init_cpu(1) };
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        if IPI_PENDING.swap(false, Ordering::SeqCst) {
            unsafe {
                core::arch::asm!("dsb ish");
                core::arch::asm!("isb");
            }
            IPI_ACK.fetch_add(1, Ordering::SeqCst);
        }
        core::hint::spin_loop();
    }
}

fn request_ipi() -> bool {
    if SECONDARY_ONLINE.load(Ordering::SeqCst) == 0 {
        return false;
    }
    IPI_ACK.store(0, Ordering::SeqCst);
    IPI_PENDING.store(true, Ordering::SeqCst);
    // GICv2 SGIR: SGI 0, target list = CPU1.
    write32(GICD + 0xF00, 1 << 16);
    let mut spins = 0usize;
    while IPI_ACK.load(Ordering::SeqCst) == 0 && spins < 5_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    IPI_ACK.load(Ordering::SeqCst) > 0
}

fn cpu_state() -> *mut CpuState {
    let state: *mut CpuState;
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el1", out(reg) state);
    }
    state
}

fn publish_current_tcb(tcb: Option<usize>) {
    let state = cpu_state();
    unsafe {
        (*state).current_tcb = tcb.unwrap_or(usize::MAX);
    }
}

fn active_root() -> u64 {
    unsafe { (*cpu_state()).active_root }
}

fn active_asid() -> u16 {
    unsafe { (*cpu_state()).active_asid }
}

fn set_active_root(root: u64, asid: u16) {
    unsafe {
        (*cpu_state()).active_root = root;
        (*cpu_state()).active_asid = asid;
    }
}

fn assert_current_tcb() {
    let state = cpu_state();
    let expected = Aarch64Platform::kernel()
        .current()
        .map(|tcb| tcb.index as usize)
        .unwrap_or(usize::MAX);
    assert_eq!(
        unsafe { (*state).current_tcb },
        expected,
        "per-CPU current_tcb mismatch"
    );
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
        vifast_pte::aarch64::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::aarch64::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
            core::arch::asm!("dsb ishst");
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::aarch64::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::aarch64::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::aarch64::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::aarch64::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        2
    }

    fn install_kernel(&self, root_pa: u64, aux_pa: u64) {
        unsafe {
            let root = root_pa as *mut u64;
            let aux = aux_pa as *mut u64;
            *root = table_desc(aux_pa);
            let shared_l1 = core::ptr::addr_of!(L1) as *const u64;
            for slot in vifast_pte::aarch64::KERNEL_SHARED_SLOTS {
                *aux.add(*slot) = *shared_l1.add(*slot);
            }
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0x4000_0000, 0x4080_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::aarch64::USER_VA_START,
            vifast_pte::aarch64::USER_VA_END,
        )
    }

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

    fn sync_icache(&self, pa: u64, size: usize) {
        if size == 0 {
            return;
        }
        let ctr: u64;
        unsafe {
            core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
            let dline = 4u64 << (ctr & 0xf);
            let iline = 4u64 << ((ctr >> 16) & 0xf);
            let end = pa.saturating_add(size as u64);
            let mut addr = pa & !(dline - 1);
            while addr < end {
                core::arch::asm!("dc civac, {}", in(reg) addr, options(nostack));
                addr = addr.saturating_add(dline);
            }
            core::arch::asm!("dsb ish", options(nostack));
            addr = pa & !(iline - 1);
            while addr < end {
                core::arch::asm!("ic ivau, {}", in(reg) addr, options(nostack));
                addr = addr.saturating_add(iline);
            }
            core::arch::asm!("dsb ish", options(nostack));
            core::arch::asm!("isb", options(nostack));
        }
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::aarch64::is_user(pte)
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
    for level in 0..vifast_pte::aarch64::LEVELS {
        let idx = vifast_pte::aarch64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::aarch64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                let kernel_writable = pte & (0b11 << 6) == 0;
                return rights.contains(Rights::READ)
                    && !vifast_pte::aarch64::is_user(pte)
                    && (!need_write || kernel_writable)
                    && (!need_exec || pte & vifast_pte::aarch64::PXN == 0);
            }
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
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

unsafe fn build_kernel_page_tables() {
    let l0 = core::ptr::addr_of_mut!(L0) as *mut Table;
    let l1 = core::ptr::addr_of_mut!(L1) as *mut Table;
    let low = core::ptr::addr_of_mut!(L2_LOW) as *mut Table;
    let kernel = core::ptr::addr_of_mut!(L2_KERNEL) as *mut Table;
    (*l0).0[0] = table_desc(l1 as u64);
    (*l1).0[0] = table_desc(low as u64);
    (*l1).0[1] = table_desc(kernel as u64);
    (*low).0[64] = block_desc(0x0800_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    (*low).0[72] = block_desc(0x0900_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    (*low).0[80] = block_desc(0x0a00_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    for i in 0..4usize {
        let pa = 0x4000_0000u64 + i as u64 * 2 * 1024 * 1024;
        (*kernel).0[i] = block_desc(pa, 0, 0, 0, UXN, SH_INNER);
    }
}

static PAGE_OPS: PageOps = PageOps;

static mut KERNEL_STORAGE: Kernel = Kernel::new_const();

impl Platform for Aarch64Platform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::aarch64::ELF_MACHINE;

    fn init_platform(info: PlatformInfo) {
        unsafe {
            build_kernel_page_tables();
            init_cpu(info.hartid);
        }
    }

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

    fn boot_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BOOT_POOL) as *mut u8, 131072)
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

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(L0) as u64
    }

    fn switch_root(root_pa: u64, asid: u16) -> bool {
        unsafe {
            if root_pa == 0 || (root_pa == active_root() && asid == active_asid()) {
                return false;
            }
            set_active_root(root_pa, asid);
            // TCR_EL1.AS=1 enables 16-bit ASIDs; TTBR0 carries the tag.
            let tcr: u64 = 16 | (1 << 23) | (2 << 32) | (1 << 36);
            core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
            let ttbr = root_pa | ((asid as u64) << 48);
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) ttbr);
            core::arch::asm!("tlbi aside1is, {}", in(reg) asid as u64);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
        true
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }

    fn kernel_image_end() -> u64 {
        core::ptr::addr_of!(__kernel_end) as u64
    }

    fn check_kernel_mappings(root_pa: u64) -> bool {
        kernel_range_ok(root_pa)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(vector_table) as u64,
                false,
                true,
            )
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(TRAP_STACKS) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(KERNEL_STORAGE) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, GICD, true, false)
            && kernel_leaf_ok(root_pa, GICC, true, false)
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, out.len(), USER_CODE_VA, 0x2_0000)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, out.len(), USER_DATA_VA, 4096)
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
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
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, input.len(), USER_DATA_VA, 4096)
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
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

    fn set_context_arg0(context: &mut TrapFrame, value: u64) {
        context.regs[0] = value;
    }

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        assert!(!BKL.is_held(), "BKL held before user return");
        let tcr: u64 = 16 | (1 << 23) | (2 << 32) | (1 << 36);
        let mair: u64 = 0xFF;
        unsafe {
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
            let cpu: *const CpuState;
            core::arch::asm!("mrs {}, tpidr_el1", out(reg) cpu);
            assert!(!cpu.is_null(), "AArch64 per-CPU pointer is null");
            let trap_top = (*cpu).trap_stack_top;
            core::arch::asm!("msr spsel, #1");
            core::arch::asm!("mov sp, {}", in(reg) trap_top);
            core::arch::asm!("msr sp_el0, {}", in(reg) context.sp);
            core::arch::asm!("msr elr_el1, {}", in(reg) context.elr);
            core::arch::asm!("msr spsr_el1, {}", in(reg) context.spsr);
            core::arch::asm!("mov x0, {}", in(reg) context.regs[0]);
            core::arch::asm!("eret", options(noreturn));
        }
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

    fn init_block() {
        if vifast_virtio::init() {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                Aarch64Platform,
                "VIRTIO OK base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        } else {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                Aarch64Platform,
                "VIRTIO SKIP base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        }
    }

    fn block_read(lba: u64, out: &mut [u8; 512]) -> bool {
        vifast_virtio::read_sector(lba, out)
    }

    fn block_write(lba: u64, input: &[u8; 512]) -> bool {
        vifast_virtio::write_sector(lba, input)
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
            if request_ipi() {
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "SMP OK (harts=2, psci={}, ipi=1)",
                    result as i64
                );
            } else {
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "SMP OK (harts=2, psci={}, ipi=pending)",
                    result as i64
                );
            }
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
    static __kernel_end: u8;
    fn secondary_entry();
}

#[no_mangle]
extern "C" fn aarch64_trap_handler(frame: *mut TrapFrame) {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
    let frame = unsafe { &mut *frame };
    let ec = (frame.esr >> 26) & 0x3f;
    if ec != 0x15 {
        vifast_runtime::fault::suspend::<Aarch64Platform>(frame, frame.esr, frame.far);
        drop(bkl);
        assert!(!BKL.is_held(), "BKL held on fault return");
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
    drop(bkl);
    assert!(!BKL.is_held(), "BKL held on syscall return");
}

#[no_mangle]
extern "C" fn aarch64_irq_handler(frame: *mut TrapFrame) {
    assert_irqs_disabled();
    let bkl = BKL.lock_with(poll_maintenance);
    assert_current_tcb();
    let frame = unsafe { &mut *frame };
    let iar = read32(GICC + 0x00C);
    let irq = iar & 0x3FF;
    if irq != TIMER_IRQ {
        write32(GICC + 0x010, irq);
        drop(bkl);
        assert!(!BKL.is_held(), "BKL held on IRQ return");
        return;
    }
    vifast_runtime::timer::on_timer::<Aarch64Platform>(frame);
    drop(bkl);
    assert!(!BKL.is_held(), "BKL held on timer return");
}
