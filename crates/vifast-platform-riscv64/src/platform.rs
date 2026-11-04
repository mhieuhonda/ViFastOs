//! RISC-V QEMU `virt` platform: Sv39 MMU, SBI timer, trap entry and SMP.

use core::sync::atomic::{AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::Arch;
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_riscv64::Riscv64;

pub struct RiscvPlatform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
pub const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
pub const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const USER_SSTATUS: u64 = 1 << 5;
const TIMER_INTERVAL: u64 = 100_000;
const PTE_V: u64 = 1;
const PTE_R: u64 = 2;
const PTE_W: u64 = 4;
const PTE_X: u64 = 8;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

static mut ROOT: Table = Table([0; 512]);
static mut KERNEL_L1: Table = Table([0; 512]);
static mut DEVICE_L1: Table = Table([0; 512]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_DATA: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 32] = [TrapFrame::empty(); 32];
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 131072]);
static mut BOOT_POOL: BootPool = BootPool([0; 131072]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
static mut ACTIVE_ROOT: u64 = 0;
#[allow(dead_code)]
#[repr(align(16))]
struct TrapStack([u8; 32768]);
static mut TRAP_STACK: TrapStack = TrapStack([0; 32768]);
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    regs: [u64; 32],
    sepc: u64,
    sstatus: u64,
    scause: u64,
}

impl TrapFrame {
    const fn empty() -> TrapFrame {
        TrapFrame {
            regs: [0; 32],
            sepc: 0,
            sstatus: 0,
            scause: 0,
        }
    }

    fn user(entry: usize, sp: usize) -> TrapFrame {
        let mut frame = TrapFrame::empty();
        frame.regs[2] = sp as u64;
        frame.sepc = entry as u64;
        frame.sstatus = USER_SSTATUS;
        frame
    }
}

fn pte(pa: u64, flags: u64) -> u64 {
    ((pa >> 12) << 10) | flags
}

fn table_pa(table: *const Table) -> u64 {
    table as u64
}

fn page_in_range(ptr: u64, len: usize, base: usize, size: usize) -> bool {
    ptr >= base as u64 && ptr as usize + len <= base + size
}

unsafe fn set_timer(at: u64) {
    core::arch::asm!(
        "ecall",
        in("a7") 0x5449_4D45usize,
        in("a6") 0usize,
        in("a0") at,
    );
}

#[no_mangle]
pub extern "C" fn rust_secondary(_hartid: usize) -> ! {
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

unsafe fn build_kernel_page_tables() {
    let root = core::ptr::addr_of_mut!(ROOT) as *mut Table;
    let kernel_l1 = core::ptr::addr_of_mut!(KERNEL_L1) as *mut Table;
    let device_l1 = core::ptr::addr_of_mut!(DEVICE_L1) as *mut Table;
    (*root).0[0] = pte(table_pa(device_l1), PTE_V);
    (*root).0[2] = pte(table_pa(kernel_l1), PTE_V);
    (*kernel_l1).0[0] = pte(0x8000_0000, PTE_V | PTE_R | PTE_W | PTE_X);
    (*kernel_l1).0[1] = pte(0x8020_0000, PTE_V | PTE_R | PTE_W | PTE_X);
    (*device_l1).0[128] = pte(0x1000_0000, PTE_V | PTE_R | PTE_W);
}

fn active_root_matches_current() -> bool {
    unsafe {
        if KERNEL_PTR.is_null() || ACTIVE_ROOT == 0 {
            return false;
        }
        let kernel = &*KERNEL_PTR;
        let Some(tcb) = kernel.current() else {
            return false;
        };
        kernel.current_root(tcb) == Some(ACTIVE_ROOT)
    }
}

impl Arch for RiscvPlatform {
    const NAME: &'static str = Riscv64::NAME;
    fn init() {
        Riscv64::init();
    }
    fn putc(byte: u8) {
        Riscv64::putc(byte);
    }
    fn now() -> u64 {
        Riscv64::now()
    }
    fn irq_disable() {
        Riscv64::irq_disable();
    }
    fn irq_enable() {
        Riscv64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        Riscv64::send_ipi(hart);
    }
    fn halt() -> ! {
        Riscv64::halt()
    }
    fn exit(code: u8) -> ! {
        Riscv64::exit(code)
    }
}

impl vifast_kernel::console::Console for RiscvPlatform {
    const NAME: &'static str = Riscv64::NAME;

    fn write_byte(byte: u8) {
        Riscv64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        vifast_pte::riscv::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::riscv::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
            core::arch::asm!("sfence.vma");
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::riscv::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::riscv::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::riscv::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::riscv::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        1
    }

    fn install_kernel(&self, root_pa: u64, _aux_pa: u64) {
        unsafe {
            let dst = root_pa as *mut u64;
            let src = core::ptr::addr_of!(ROOT) as *const u64;
            for slot in vifast_pte::riscv::KERNEL_SHARED_SLOTS {
                *dst.add(*slot) = *src.add(*slot);
            }
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0x8000_0000, 0x8040_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::riscv::USER_VA_START,
            vifast_pte::riscv::USER_VA_END,
        )
    }

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("sfence.vma {}, zero", in(reg) va);
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            core::arch::asm!("sfence.vma zero, zero");
        }
    }

    fn sync_icache(&self, _pa: u64, _size: usize) {
        // Local hart only until SMP I-cache shootdown is implemented.
        unsafe {
            core::arch::asm!("fence.i");
        }
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::riscv::is_user(pte)
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

/// Walk the real boot page table and verify a kernel leaf.
fn kernel_leaf_ok(root_pa: u64, va: u64, need_write: bool, need_exec: bool) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::riscv::LEVELS {
        let idx = vifast_pte::riscv::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::riscv::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                return rights.contains(Rights::READ)
                    && !vifast_pte::riscv::is_user(pte)
                    && (!need_write || pte & vifast_pte::riscv::PTE_W != 0)
                    && (!need_exec || pte & vifast_pte::riscv::PTE_X != 0);
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

static PAGE_OPS: PageOps = PageOps;

static mut KERNEL_STORAGE: Kernel = Kernel::new_const();

impl Platform for RiscvPlatform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::riscv::ELF_MACHINE;

    fn init_platform(_info: PlatformInfo) {
        unsafe {
            build_kernel_page_tables();
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
        table_pa(core::ptr::addr_of!(ROOT))
    }

    fn switch_root(root_pa: u64) -> bool {
        unsafe {
            if root_pa == 0 || root_pa == ACTIVE_ROOT {
                return false;
            }
            ACTIVE_ROOT = root_pa;
            let satp = (8u64 << 60) | (root_pa >> 12);
            core::arch::asm!("csrw satp, {}", in(reg) satp);
            core::arch::asm!("sfence.vma zero, zero");
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
            && kernel_leaf_ok(root_pa, trap_entry as *const () as u64, false, true)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(TRAP_STACK) as u64, true, false)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(KERNEL_STORAGE) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, 0x1000_0000, true, false)
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, out.len(), USER_CODE_VA, 4096)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, out.len(), USER_DATA_VA, 4096)
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
                /* SUM is kept clear while in S-mode; enable it only for the
                explicit copy window. */
                core::arch::asm!("csrs sstatus, {}", in(reg) 1u64 << 18);
                core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
                core::arch::asm!("csrc sstatus, {}", in(reg) 1u64 << 18);
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
                core::arch::asm!("csrs sstatus, {}", in(reg) 1u64 << 18);
                core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
                core::arch::asm!("csrc sstatus, {}", in(reg) 1u64 << 18);
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
        context.regs[10] = value;
    }

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        unsafe {
            core::arch::asm!("csrw sstatus, {}", in(reg) context.sstatus);
            core::arch::asm!("csrw sepc, {}", in(reg) context.sepc);
            core::arch::asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
            let trap_top = core::ptr::addr_of!(TRAP_STACK) as u64 + 32768;
            core::arch::asm!("csrw sscratch, {}", in(reg) trap_top);
            core::arch::asm!("csrs sie, {}", in(reg) 1u64 << 5);
            core::arch::asm!("mv sp, {}", in(reg) context.regs[2]);
            core::arch::asm!("mv a0, {}", in(reg) context.regs[10]);
            core::arch::asm!("sret", options(noreturn));
        }
    }

    fn write_result(
        context: &mut TrapFrame,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    ) {
        context.regs[10] = error as u64;
        context.regs[11] = value;
        context.regs[12] = label;
        context.regs[13] = data[0];
        context.regs[14] = data[1];
        context.regs[15] = data[2];
        context.regs[16] = data[3];
    }

    fn start_timer() {
        unsafe {
            set_timer(Riscv64::now() + TIMER_INTERVAL);
        }
    }

    fn reload_timer() {
        unsafe {
            set_timer(Riscv64::now() + TIMER_INTERVAL);
        }
    }

    fn ack_timer() {}

    fn start_smp() {
        let entry = secondary_entry as *const () as usize;
        let started = unsafe {
            let result: usize;
            core::arch::asm!(
                "ecall",
                in("a7") 0x4853_4dusize,
                in("a6") 0usize,
                in("a0") 1usize,
                in("a1") entry,
                in("a2") 0usize,
                lateout("a0") result,
            );
            result
        };
        let mut spins = 0u32;
        while SECONDARY_ONLINE.load(Ordering::SeqCst) == 0 && spins < 10_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if SECONDARY_ONLINE.load(Ordering::SeqCst) > 0 {
            vifast_kernel::klog!(RiscvPlatform, "SMP OK (harts=2)");
        } else {
            vifast_kernel::klog!(RiscvPlatform, "SMP SKIP (hsm_start={:#x})", started);
        }
    }
}

#[no_mangle]
extern "C" fn riscv_kernel_trap() -> ! {
    let cause: u64;
    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) cause);
    }
    vifast_kernel::klog!(RiscvPlatform, "KERNEL TRAP cause={:#x}", cause);
    Riscv64::exit(1)
}

extern "C" {
    fn trap_entry();
    fn secondary_entry();
    static __kernel_end: u8;
}

#[no_mangle]
extern "C" fn riscv_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    if frame.scause == 0x8000_0000_0000_0005 {
        vifast_runtime::timer::on_timer::<RiscvPlatform>(frame);
        return;
    }
    if frame.scause != 8 {
        /* SPP=1 means the fault came from S-mode.  That is a kernel bug,
        not a user fault, and must not be suspended. */
        if frame.sstatus & (1 << 8) != 0 {
            vifast_kernel::klog!(RiscvPlatform, "KERNEL FAULT cause={:#x}", frame.scause);
            Riscv64::exit(1);
        }
        let stval: u64;
        unsafe {
            core::arch::asm!("csrr {}, stval", out(reg) stval);
        }
        vifast_runtime::fault::suspend::<RiscvPlatform>(frame, frame.scause, stval);
        return;
    }
    let number = frame.regs[17];
    frame.sepc = frame.sepc.wrapping_add(4);
    let args = [
        frame.regs[10],
        frame.regs[11],
        frame.regs[12],
        frame.regs[13],
        frame.regs[14],
        frame.regs[15],
    ];
    vifast_runtime::syscall::dispatch::<RiscvPlatform>(frame, number, args);
}
